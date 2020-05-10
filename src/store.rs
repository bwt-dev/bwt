use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use bitcoin::{Address, OutPoint, Txid};

use crate::hd::KeyOrigin;
use crate::types::{ScriptHash, TxStatus};

#[cfg(feature = "track-spends")]
use crate::{types::TxInput, util::remove_if};

#[derive(Debug)]
pub struct MemoryStore {
    scripthashes: HashMap<ScriptHash, ScriptEntry>,
    transactions: HashMap<Txid, TxEntry>,
    #[cfg(feature = "track-spends")]
    txo_spends: HashMap<OutPoint, TxInput>,
}

#[derive(Debug)]
struct ScriptEntry {
    address: Address,
    origin: KeyOrigin,
    history: BTreeSet<HistoryEntry>,
}

#[derive(Clone, Eq, PartialEq, Debug, Hash, Serialize)]
pub struct HistoryEntry {
    pub txid: Txid,
    #[serde(flatten)]
    pub status: TxStatus,
}

impl HistoryEntry {
    pub fn new(txid: Txid, status: TxStatus) -> Self {
        HistoryEntry { txid, status }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct TxEntry {
    #[serde(flatten)]
    pub status: TxStatus,
    pub fee: Option<u64>,
    pub funding: HashMap<u32, FundingInfo>,
    #[cfg(feature = "track-spends")]
    pub spending: HashMap<u32, SpendingInfo>,
}

impl TxEntry {
    pub fn new(status: TxStatus, fee: Option<u64>) -> Self {
        TxEntry {
            status,
            fee,
            funding: HashMap::new(),
            #[cfg(feature = "track-spends")]
            spending: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FundingInfo(pub ScriptHash, pub u64);

#[cfg(feature = "track-spends")]
#[derive(Debug, Clone, Serialize)]
pub struct SpendingInfo(pub ScriptHash, pub OutPoint, pub u64);

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore {
            scripthashes: HashMap::new(),
            transactions: HashMap::new(),
            #[cfg(feature = "track-spends")]
            txo_spends: HashMap::new(),
        }
    }

    pub fn track_scripthash(
        &mut self,
        scripthash: &ScriptHash,
        origin: &KeyOrigin,
        address: &Address,
    ) -> bool {
        debug!("tracking {:?} {:?} {:?}", origin, scripthash, address);

        let mut existed = false;

        self.scripthashes
            .entry(*scripthash)
            .and_modify(|curr_entry| {
                assert_eq!(
                    curr_entry.origin, *origin,
                    "unexpected stored origin for {:?}",
                    scripthash
                );
                existed = true;
            })
            .or_insert_with(|| ScriptEntry {
                address: address.clone(),
                origin: origin.clone(),
                history: BTreeSet::new(),
            });

        !existed
    }

    pub fn index_tx_entry(&mut self, txid: &Txid, mut txentry: TxEntry) -> bool {
        debug!("index tx entry {:?}: {:?}", txid, txentry);

        assert!(
            txentry.status.is_viable(),
            "should not index non-viable tx entries"
        );

        let new_status = txentry.status;
        let mut updated = false;
        let mut changed_from = None;

        self.transactions
            .entry(*txid)
            .and_modify(|curr_entry| {
                if let (None, &Some(_)) = (curr_entry.fee, &txentry.fee) {
                    curr_entry.fee = txentry.fee;
                }

                curr_entry.funding.extend(txentry.funding.drain());
                #[cfg(feature = "track-spends")]
                curr_entry.spending.extend(txentry.spending.drain());

                if &curr_entry.status != &txentry.status {
                    changed_from = Some(curr_entry.status);
                    curr_entry.status = new_status;
                    updated = true;
                }
            })
            .or_insert_with(|| {
                info!("new tx entry: {:?}", txid);
                updated = true;
                txentry
            });

        if let Some(old_status) = changed_from {
            self.tx_status_changed(txid, old_status, new_status);
        }

        updated
    }

    pub fn index_history_entry(&mut self, scripthash: &ScriptHash, txhist: HistoryEntry) -> bool {
        debug!(
            "index scripthash history for {:?}: {:?}",
            scripthash, txhist
        );

        let added = self
            .scripthashes
            .get_mut(scripthash)
            .expect("missing expected scripthash entry")
            .history
            .insert(txhist);

        if added {
            info!("new history entry added for {:?}", scripthash);
        }

        added
    }

    #[cfg(feature = "track-spends")]
    pub fn index_txo_spend(&mut self, spent_prevout: OutPoint, spending_input: TxInput) -> bool {
        debug!(
            "index txo spend: {:?} by {:?}",
            spent_prevout, spending_input
        );

        let added = self
            .txo_spends
            .insert(spent_prevout, spending_input)
            .is_none();
        added
    }

    /// Update the scripthash history index to reflect the new tx status
    pub fn tx_status_changed(&mut self, txid: &Txid, old_status: TxStatus, new_status: TxStatus) {
        if old_status == new_status {
            return;
        }

        info!(
            "transition tx {:?} status: {:?} -> {:?}",
            txid, old_status, new_status
        );

        let old_txhist = HistoryEntry::new(*txid, old_status);
        let new_txhist = HistoryEntry::new(*txid, new_status);

        /*
        let txentry = self
            .transactions
            .get(txid)
            .expect("missing expected tx entry");
        let affected_scripthashes = txentry
            .funding
            .iter()
            .map(|(_, scripthash)| scripthash)
            .chain(txentry.spending.iter().map(|(_, scripthash)| scripthash));

        for scripthash in affected_scripthashes {
            let scriptentry = self
                .scripthashes
                .get(scripthash)
                .expect("missing expected script entry");
            assert!(scriptentry.history.remove(&old_txhist));
            assert!(scriptentry.history.insert(new_txhist.clone()));
        }
        */

        // TODO optimize, keep txid->scripthashes map
        for (_scripthash, ScriptEntry { history, .. }) in &mut self.scripthashes {
            if history.remove(&old_txhist) {
                history.insert(new_txhist.clone());
            }
        }
    }

    pub fn purge_tx(&mut self, txid: &Txid) -> bool {
        info!("purge tx {:?}", txid);

        // XXX should replaced transactions be kept around instead of purged entirely?
        if let Some(old_entry) = self.transactions.remove(txid) {
            let old_txhist = HistoryEntry {
                status: old_entry.status,
                txid: *txid,
            };

            #[cfg(feature = "track-spends")]
            for (_, SpendingInfo(scripthash, prevout, _)) in old_entry.spending {
                // remove prevout spending edge, but only if it still references the purged tx
                #[cfg(feature = "track-spends")]
                remove_if(&mut self.txo_spends, prevout, |spending_input| {
                    spending_input.txid == *txid
                });

                self.scripthashes
                    .get_mut(&scripthash)
                    .map(|s| s.history.remove(&old_txhist));
            }

            // if we don't track spends, we have to iterate over the entire scripthash set in order
            // to purge history entries of transactions spending the removed tx.
            #[cfg(not(feature = "track-spends"))]
            self.scripthashes
                .retain(|_scripthash, ScriptEntry { history, .. }| {
                    history.remove(&old_txhist);
                    history.len() > 0
                });

            for (_, FundingInfo(scripthash, _)) in old_entry.funding {
                self.scripthashes
                    .get_mut(&scripthash)
                    .map(|s| s.history.remove(&old_txhist));
            }

            // TODO remove the scripthashes entirely if have no more history entries
            true
        } else {
            false
        }
    }

    pub fn lookup_txo_fund(&self, outpoint: &OutPoint) -> Option<FundingInfo> {
        self.transactions
            .get(&outpoint.txid)?
            .funding
            .get(&outpoint.vout)
            .cloned()
    }

    #[cfg(feature = "track-spends")]
    pub fn lookup_txo_spend(&self, outpoint: &OutPoint) -> Option<TxInput> {
        // XXX don't return non-viabla (double-spent) spends?
        self.txo_spends.get(outpoint).copied()
    }

    pub fn get_history(&self, scripthash: &ScriptHash) -> Option<&BTreeSet<HistoryEntry>> {
        Some(&self.scripthashes.get(scripthash)?.history)
    }

    pub fn get_tx_count(&self, scripthash: &ScriptHash) -> usize {
        self.scripthashes
            .get(scripthash)
            .map_or(0, |script_entry| script_entry.history.len())
    }

    pub fn get_tx_entry(&self, txid: &Txid) -> Option<&TxEntry> {
        self.transactions.get(txid)
    }

    pub fn get_script_info(&self, scripthash: &ScriptHash) -> Option<ScriptInfo> {
        let script_entry = self.scripthashes.get(scripthash)?;
        Some(ScriptInfo::new(*scripthash, script_entry))
    }

    pub fn get_script_address(&self, scripthash: &ScriptHash) -> Option<Address> {
        Some(self.scripthashes.get(scripthash)?.address.clone())
    }
    /// Get all history since `min_block_height`, including unconfirmed mempool transactions,
    /// for *all* tracked scripthashes
    pub fn get_history_since(&self, min_block_height: u32) -> BTreeSet<&HistoryEntry> {
        // XXX this is terribly inefficient. okayish for now, but should be rewritten not to
        // require a full scan at some point
        self.scripthashes
            .values()
            .map(|script_entry| {
                script_entry
                    .history
                    .iter()
                    .rev()
                    .take_while(|txhist| match txhist.status {
                        TxStatus::Confirmed(block_height) => block_height >= min_block_height,
                        TxStatus::Unconfirmed => true,
                        TxStatus::Conflicted => unreachable!(),
                    })
            })
            .flatten()
            .collect()
    }
}

#[derive(Serialize, Debug)]
pub struct ScriptInfo {
    scripthash: ScriptHash,
    address: Address,
    #[serde(skip_serializing_if = "KeyOrigin::is_standalone")]
    origin: KeyOrigin,
}

impl ScriptInfo {
    fn new(scripthash: ScriptHash, script_entry: &ScriptEntry) -> Self {
        ScriptInfo {
            scripthash: scripthash,
            address: script_entry.address.clone(),
            origin: script_entry.origin.clone(),
        }
    }
}

impl Ord for HistoryEntry {
    fn cmp(&self, other: &HistoryEntry) -> Ordering {
        self.status
            .cmp(&other.status)
            .then_with(|| self.txid.cmp(&other.txid))
    }
}

impl PartialOrd for HistoryEntry {
    fn partial_cmp(&self, other: &HistoryEntry) -> Option<Ordering> {
        Some(
            self.status
                .cmp(&other.status)
                .then_with(|| self.txid.cmp(&other.txid)),
        )
    }
}