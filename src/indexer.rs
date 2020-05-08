use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde::Serialize;

use bitcoin::{Address, BlockHash, OutPoint, SignedAmount, Txid};
use bitcoincore_rpc::json::{
    GetTransactionResultDetailCategory as TxCategory, ListTransactionResult,
};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::hd::{HDWatcher, KeyOrigin};
use crate::types::{BlockId, ScriptHash, TxInput, TxStatus, Utxo};
use crate::util::{address_to_scripthash, remove_if};

pub struct Indexer {
    rpc: Arc<RpcClient>,
    watcher: HDWatcher,
    index: MemoryIndex,
    tip: Option<BlockId>,
}

#[derive(Debug)]
struct MemoryIndex {
    scripthashes: HashMap<ScriptHash, ScriptEntry>,
    transactions: HashMap<Txid, TxEntry>,
    txo_spends: HashMap<OutPoint, TxInput>,
}

#[derive(Debug)]
struct ScriptEntry {
    address: Address,
    origin: KeyOrigin,
    history: BTreeSet<HistoryEntry>,
    //#[cfg(feature = "electrum")]
    //electrum_status_hash: Option<StatusHash>,
}

#[derive(Clone, Eq, PartialEq, Debug, Hash)]
pub struct HistoryEntry {
    pub txid: Txid,
    pub status: TxStatus,
}

impl HistoryEntry {
    fn new(txid: Txid, status: TxStatus) -> Self {
        HistoryEntry { txid, status }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TxEntry {
    pub status: TxStatus,
    pub fee: Option<u64>,
    pub funding: HashMap<u32, FundingInfo>,
    pub spending: HashMap<u32, SpendingInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FundingInfo(pub ScriptHash, pub u64);

#[derive(Debug, Clone, Serialize)]
pub struct SpendingInfo(pub ScriptHash, pub OutPoint, pub u64);

impl TxEntry {
    fn new(status: TxStatus, fee: Option<u64>) -> Self {
        TxEntry {
            status,
            fee,
            funding: HashMap::new(),
            spending: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Tx {
    pub txid: Txid,
    #[serde(flatten)]
    pub entry: TxEntry,
}

impl Indexer {
    pub fn new(rpc: Arc<RpcClient>, watcher: HDWatcher) -> Self {
        Indexer {
            rpc,
            watcher: watcher,
            index: MemoryIndex::new(),
            tip: None,
        }
    }

    pub fn dump(&self) {
        debug!("{:#?}", self.index);
    }

    pub fn sync(&mut self) -> Result<()> {
        // Detect reorgs and start syncing history from scratch when they happen
        if let Some(BlockId(tip_height, tip_hash)) = self.tip {
            let best_chain_hash = self.rpc.get_block_hash(tip_height as u64)?;
            if best_chain_hash != tip_hash {
                warn!(
                    "reorg detected, block height {} was {} and now is {}. fetching history from scratch...",
                    tip_height, tip_hash, best_chain_hash
                );
                self.tip = None;
            }
        }

        let synced_tip = self.sync_transactions()?;

        self.watcher.watch(&self.rpc)?;

        info!("synced up to {:?}", synced_tip);
        self.tip = Some(synced_tip);

        Ok(())
    }

    fn sync_transactions(&mut self) -> Result<BlockId> {
        let rpc = Arc::clone(&self.rpc);

        let start_height = self
            .tip
            .as_ref()
            .map_or(0, |BlockId(tip_height, _)| tip_height + 1);

        let mut pending_outgoing: HashMap<Txid, TxEntry> = HashMap::new();

        let synced_tip =
            load_transactions_since(&rpc, start_height, None, &mut |chunk, tip_height| {
                for ltx in chunk {
                    match ltx.detail.category {
                        TxCategory::Receive => {
                            self.process_incoming(ltx, tip_height);
                        }
                        TxCategory::Send => {
                            // outgoing payments are buffered and processed later so that the
                            // parent funding transaction is guaranteed to get indexed first
                            pending_outgoing.entry(ltx.info.txid).or_insert_with(|| {
                                let status = TxStatus::new(ltx.info.confirmations, tip_height);
                                TxEntry::new(status, parse_fee(ltx.detail.fee))
                            });
                        }
                        TxCategory::Generate | TxCategory::Immature => (),
                    };
                }
            })?;

        for (txid, txentry) in pending_outgoing {
            self.process_outgoing(txid, txentry)
                .map_err(|err| warn!("failed processing outgoing payment: {:?}", err))
                .ok();
        }

        Ok(synced_tip)
    }

    fn process_incoming(&mut self, ltx: ListTransactionResult, tip_height: u32) {
        // XXX stop early if we're familiar with this txid and its long confirmed

        let origin = match ltx
            .detail
            .label
            .as_ref()
            .and_then(|l| KeyOrigin::from_label(l))
        {
            Some(origin) => origin,
            None => return,
        };
        let status = TxStatus::new(ltx.info.confirmations, tip_height);

        debug!(
            "process incoming tx for {:?} origin {:?} with status {:?}: {:?}",
            ltx.detail.address, origin, status, ltx
        );

        if !status.is_viable() {
            self.index.purge_tx(&ltx.info.txid);
            return;
        }

        let scripthash = address_to_scripthash(&ltx.detail.address);
        let amount = ltx.detail.amount.to_unsigned().unwrap().as_sat(); // safe to unwrap, incoming payments cannot have negative amounts
        let funding_info = FundingInfo(scripthash, amount);

        let mut txentry = TxEntry::new(status, None);
        txentry.funding.insert(ltx.detail.vout, funding_info);

        self.index.index_tx_entry(&ltx.info.txid, txentry);

        self.index
            .track_scripthash(&scripthash, &origin, &ltx.detail.address);

        self.index
            .index_history_entry(&scripthash, HistoryEntry::new(ltx.info.txid, status));

        self.watcher.mark_funded(&origin);
    }

    fn process_outgoing(&mut self, txid: Txid, mut txentry: TxEntry) -> Result<()> {
        debug!(
            "processing outgoing tx {:?} with status {:?}",
            txid, txentry.status
        );

        if !txentry.status.is_viable() {
            self.index.purge_tx(&txid);
            return Ok(());
        }

        let tx = self.rpc.get_transaction(&txid, Some(true))?.transaction()?;

        for (vin, input) in tx.input.iter().enumerate() {
            if let Some(FundingInfo(scripthash, amount)) =
                self.index.lookup_txo_fund(&input.previous_output)
            {
                self.index
                    .index_history_entry(&scripthash, HistoryEntry::new(txid, txentry.status));

                self.index
                    .index_txo_spend(input.previous_output, TxInput::new(txid, vin as u32));

                // we could keep just the previous_output and lookup the scripthash and amount
                // from the corrospanding FundingInfo, but we keep it here anyway for quick access
                let spending_info = SpendingInfo(scripthash, input.previous_output, amount);
                txentry.spending.insert(vin as u32, spending_info);
            }
        }

        if !txentry.spending.is_empty() {
            self.index.index_tx_entry(&txid, txentry);
        }

        Ok(())
    }

    pub fn get_history(&self, scripthash: &ScriptHash) -> Result<Vec<Tx>> {
        self.index
            .get_history(scripthash)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|hist| {
                        let tx_entry =
                            self.index.get_tx_entry(&hist.txid).or_err("tx not found")?;
                        Ok(Tx {
                            txid: hist.txid,
                            entry: tx_entry.clone(),
                        })
                    })
                    .collect::<Result<Vec<Tx>>>()
            })
            .unwrap_or_else(|| Ok(vec![]))
    }

    pub fn raw_history_ref(&self, scripthash: &ScriptHash) -> Option<&BTreeSet<HistoryEntry>> {
        self.index.get_history(scripthash)
    }

    pub fn get_script_info(&self, scripthash: &ScriptHash) -> Option<ScriptInfo> {
        let script_entry = self.index.get_script_entry(scripthash)?;
        Some(ScriptInfo::new(scripthash, script_entry))
    }

    pub fn get_tx_entry(&self, txid: &Txid) -> Option<TxEntry> {
        self.index.get_tx_entry(txid).cloned()
    }

    /// Get the unspent utxos owned by scripthash
    // XXX Move to Query?
    pub fn list_unspent(&self, scripthash: &ScriptHash, min_conf: usize) -> Result<Vec<Utxo>> {
        let address = self
            .index
            .get_address(scripthash)
            .or_err("unknown scripthash")?;

        loop {
            let tip_height = self.rpc.get_block_count()? as u32;
            let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

            // XXX include unsafe?
            let unspents = self.rpc.list_unspent(
                Some(min_conf),
                None,
                Some(&[&address]),
                Some(false),
                None,
            )?;

            if tip_hash != self.rpc.get_best_block_hash()? {
                warn!("tip changed while fetching unspents, retrying...");
                continue;
            }

            return Ok(unspents
                .into_iter()
                .map(|unspent| Utxo::from_unspent(unspent, tip_height))
                .filter(|utxo| utxo.status.is_viable())
                .collect());
        }
    }

    pub fn find_tx_blockhash(&self, txid: &Txid) -> Result<Option<BlockHash>> {
        let txentry = self.index.get_tx_entry(txid).or_err("tx not found")?;
        Ok(match txentry.status {
            TxStatus::Confirmed(height) => Some(self.rpc.get_block_hash(height as u64)?),
            _ => None,
        })
    }

    pub fn lookup_txo_spend(&self, outpoint: &OutPoint) -> Option<TxInput> {
        self.index.lookup_txo_spend(outpoint)
    }
}

impl MemoryIndex {
    fn new() -> Self {
        MemoryIndex {
            scripthashes: HashMap::new(),
            transactions: HashMap::new(),
            txo_spends: HashMap::new(),
        }
    }

    fn track_scripthash(&mut self, scripthash: &ScriptHash, origin: &KeyOrigin, address: &Address) {
        debug!("tracking {:?} {:?} {:?}", origin, scripthash, address);

        self.scripthashes
            .entry(*scripthash)
            .and_modify(|curr_entry| {
                assert_eq!(
                    curr_entry.origin, *origin,
                    "unexpected stored origin for {:?}",
                    scripthash
                )
            })
            .or_insert_with(|| ScriptEntry {
                address: address.clone(),
                origin: origin.clone(),
                history: BTreeSet::new(),
            });
    }

    fn index_tx_entry(&mut self, txid: &Txid, mut txentry: TxEntry) {
        debug!("index tx entry {:?}: {:?}", txid, txentry);

        assert!(
            txentry.status.is_viable(),
            "should not index non-viable tx entries"
        );

        let new_status = txentry.status;
        let mut changed_from = None;

        self.transactions
            .entry(*txid)
            .and_modify(|curr_entry| {
                if let (None, &Some(_)) = (curr_entry.fee, &txentry.fee) {
                    curr_entry.fee = txentry.fee;
                }

                curr_entry.funding.extend(txentry.funding.drain());
                curr_entry.spending.extend(txentry.spending.drain());

                if &curr_entry.status != &txentry.status {
                    changed_from = Some(curr_entry.status);
                    curr_entry.status = new_status;
                }
            })
            .or_insert_with(|| {
                info!("new tx entry: {:?}", txid);
                txentry
            });

        if let Some(old_status) = changed_from {
            self.tx_status_changed(txid, old_status, new_status)
        }
    }

    fn index_history_entry(&mut self, scripthash: &ScriptHash, txhist: HistoryEntry) {
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
            info!("new history entry added for {:?}", scripthash)
        }
    }

    fn index_txo_spend(&mut self, spent_prevout: OutPoint, spending_input: TxInput) {
        debug!(
            "index txo spend: {:?} by {:?}",
            spent_prevout, spending_input
        );
        self.txo_spends.insert(spent_prevout, spending_input);
    }

    /// Update the scripthash history index to reflect the new tx status
    fn tx_status_changed(&mut self, txid: &Txid, old_status: TxStatus, new_status: TxStatus) {
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

    fn purge_tx(&mut self, txid: &Txid) {
        info!("purge tx {:?}", txid);

        if let Some(old_entry) = self.transactions.remove(txid) {
            let old_txhist = HistoryEntry {
                status: old_entry.status,
                txid: *txid,
            };

            for (_, SpendingInfo(scripthash, prevout, _)) in old_entry.spending {
                // remove prevout spending edge, but only if it still references the purged tx
                remove_if(&mut self.txo_spends, prevout, |spending_input| {
                    spending_input.txid == *txid
                });

                self.scripthashes
                    .get_mut(&scripthash)
                    .map(|s| s.history.remove(&old_txhist));
            }

            for (_, FundingInfo(scripthash, _)) in old_entry.funding {
                self.scripthashes
                    .get_mut(&scripthash)
                    .map(|s| s.history.remove(&old_txhist));
            }

            // TODO remove the scripthashes entirely if have no more history entries
        }
    }

    fn lookup_txo_fund(&self, outpoint: &OutPoint) -> Option<FundingInfo> {
        self.transactions
            .get(&outpoint.txid)?
            .funding
            .get(&outpoint.vout)
            .cloned()
    }

    fn lookup_txo_spend(&self, outpoint: &OutPoint) -> Option<TxInput> {
        // XXX don't return non-viabla (double-spent) spends?
        self.txo_spends.get(outpoint).copied()
    }

    fn get_script_entry(&self, scripthash: &ScriptHash) -> Option<&ScriptEntry> {
        self.scripthashes.get(scripthash)
    }

    fn get_history(&self, scripthash: &ScriptHash) -> Option<&BTreeSet<HistoryEntry>> {
        Some(&self.scripthashes.get(scripthash)?.history)
    }

    // get the address of a scripthash
    fn get_address(&self, scripthash: &ScriptHash) -> Option<&Address> {
        Some(&self.scripthashes.get(scripthash)?.address)
    }

    fn get_tx_entry(&self, txid: &Txid) -> Option<&TxEntry> {
        self.transactions.get(txid)
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

#[derive(Serialize, Debug)]
pub struct ScriptInfo {
    scripthash: ScriptHash,
    address: Address,
    #[serde(skip_serializing_if = "KeyOrigin::is_standalone")]
    origin: KeyOrigin,
}

impl ScriptInfo {
    fn new(scripthash: &ScriptHash, script_entry: &ScriptEntry) -> Self {
        ScriptInfo {
            scripthash: *scripthash,
            address: script_entry.address.clone(),
            origin: script_entry.origin.clone(),
        }
    }
}

// convert to a positive satoshi amount
fn parse_fee(fee: Option<SignedAmount>) -> Option<u64> {
    fee.map(|fee| fee.abs().as_sat() as u64)
}

const INIT_TX_PER_PAGE: usize = 25;
const MAX_TX_PER_PAGE: usize = 250;

// Fetch all unconfirmed transactions + transactions confirmed at or after start_height
fn load_transactions_since(
    rpc: &RpcClient,
    start_height: u32,
    init_per_page: Option<usize>,
    chunk_handler: &mut dyn FnMut(Vec<ListTransactionResult>, u32),
) -> Result<BlockId> {
    let mut per_page = init_per_page.unwrap_or(INIT_TX_PER_PAGE);
    let mut start_index = 0;
    let mut oldest_seen = None;

    let tip_height = rpc.get_block_count()? as u32;
    let tip_hash = rpc.get_block_hash(tip_height as u64)?;

    assert!(start_height <= tip_height + 1, "start_height too far");
    let max_confirmations = (tip_height + 1 - start_height) as i32;

    // TODO: if the newest entry has the exact same (txid,address,height) as the previous newest,
    // skip processing the entries entirely

    info!("syncing transactions {}..{}", start_height, tip_height,);
    while {
        debug!(
            "reading {} transactions starting at index {}",
            per_page, start_index
        );

        let mut chunk =
            rpc.list_transactions(None, Some(per_page), Some(start_index), Some(true))?;

        let mut exhausted = chunk.len() < per_page;

        // this is necessary because we rely on the tip height to derive the confirmed height
        // from the number of confirmations
        if tip_hash != rpc.get_best_block_hash()? {
            warn!("tip changed while fetching transactions, retrying...");
            return load_transactions_since(rpc, start_height, Some(per_page), chunk_handler);
        }

        // make sure we didn't miss any transactions by comparing the first entry of this page with
        // the last entry of the last page. note that the entry used for comprasion is popped off
        if let Some(ref oldest_seen) = oldest_seen {
            let marker = chunk.pop().or_err("missing marker tx")?;

            if oldest_seen != &(marker.info.txid, marker.detail.vout) {
                warn!("transaction set changed while fetching transactions, retrying...");
                return load_transactions_since(rpc, start_height, Some(per_page), chunk_handler);
            }
        }

        // process entries (if any)
        if let Some(oldest) = chunk.first() {
            oldest_seen = Some((oldest.info.txid.clone(), oldest.detail.vout));

            chunk.retain(|ltx| ltx.info.confirmations <= max_confirmations);
            exhausted = exhausted || chunk.is_empty();

            chunk_handler(chunk, tip_height);
        }
        !exhausted
    } {
        // -1 so we'll get the last entry of this page as the first of the next, as a marker for sanity check
        start_index = start_index + per_page - 1;
        per_page = MAX_TX_PER_PAGE.min(per_page * 2);
    }

    Ok(BlockId(tip_height, tip_hash))
}
