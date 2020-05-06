use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use bitcoin::{Address, Txid, SignedAmount};
use bitcoin_hashes::sha256;
use bitcoincore_rpc::json::{ListTransactionResult, GetTransactionResultDetailCategory as TxCategory};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::hd::{DerivationInfo, HDWatcher};
use crate::types::{Tx, TxEntry, TxStatus, Utxo};
use crate::util::address_to_scripthash;

#[cfg(feature = "electrum")]
use crate::electrum::get_status_hash;

pub struct Indexer {
    rpc: Arc<RpcClient>,
    watcher: HDWatcher,
    index: MemoryIndex,
}

#[derive(Debug)]
struct MemoryIndex {
    scripthashes: HashMap<sha256::Hash, ScriptEntry>,
    transactions: HashMap<Txid, TxEntry>,
}

#[derive(Debug)]
struct ScriptEntry {
    address: Address,
    derivation_info: DerivationInfo,
    history: BTreeSet<HistoryEntry>,
    //#[cfg(feature = "electrum")]
    //electrum_status_hash: Option<sha256::Hash>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct HistoryEntry {
    pub txid: Txid,
    pub status: TxStatus,
}

impl Indexer {
    pub fn new(rpc: Arc<RpcClient>, watcher: HDWatcher) -> Self {
        Indexer {
            rpc,
            watcher: watcher,
            index: MemoryIndex::new(),
        }
    }
    pub fn update(&mut self) -> Result<()> {
        self.update_transactions()?;

        self.watcher.do_imports(&self.rpc)?;

        Ok(())
    }

    fn update_transactions(&mut self) -> Result<()> {
        let index = &mut self.index;
        let watcher = &mut self.watcher;
        load_transactions_since(&self.rpc, 25, 0, &mut |chunk, tip_height| {
            for ltx in chunk {
                index.process_ltx(ltx, tip_height, watcher);
            }
        })?;

        // TODO: keep track of last known tip
        // TODO: keep track of how many new txs are returned on avg
        // TODO: remove confliced txids from index

        Ok(())
    }

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Result<Vec<Tx>> {
        self.index
            .get_history(scripthash)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|hist| {
                        Ok(Tx {
                            txid: hist.txid,
                            entry: self.index.get_tx(&hist.txid).or_err("missing tx")?.clone(),
                        })
                    })
                    .collect::<Result<Vec<Tx>>>()
            })
            .unwrap_or_else(|| Ok(vec![]))
    }

    #[cfg(feature = "electrum")]
    pub fn status_hash(&self, scripthash: &sha256::Hash) -> Option<sha256::Hash> {
        self.index.get_history(scripthash).map(get_status_hash)
    }

    /// Get the unspent utxos owned by scripthash
    pub fn list_unspent(&self, scripthash: &sha256::Hash, min_conf: usize) -> Result<Vec<Utxo>> {
        let address = self
            .index
            .get_address(scripthash)
            .or_err("unknown scripthash")?;

        loop {
            let tip_height = self.rpc.get_block_count()? as u32;
            let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

            // XXX include unsafe?
            let unspents = self.rpc.list_unspent(Some(min_conf), None, Some(&[&address]), Some(false), None)?;

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
}

impl MemoryIndex {
    fn new() -> Self {
        MemoryIndex {
            scripthashes: HashMap::new(),
            transactions: HashMap::new(),
        }
    }

    /// Process a transaction entry retrieved from "listtransactions"
    pub fn process_ltx(
        &mut self,
        ltx: ListTransactionResult,
        tip_height: u32,
        watcher: &mut HDWatcher,
    ) {
        debug!("process ltx: {:?}", ltx);

        // XXX stop early if we're familiar with this txid and its long confirmed?
        if !should_process(&ltx) {
            return;
        }

        let status = TxStatus::new(ltx.info.confirmations, tip_height);

        if !status.is_viable() {
            return self.purge_tx(&ltx.info.txid);
        }

        let txentry = TxEntry {
            status: status,
            fee: parse_fee(ltx.detail.fee),
        };
        self.index_tx_entry(&ltx.info.txid, txentry);

        let txhist = HistoryEntry {
            status,
            txid: ltx.info.txid,
        };
        self.index_address_history(ltx.detail.address, &ltx.detail.label.unwrap_or("".into()), txhist, watcher);
    }

    /*
    /// Process a transaction entry retrieved from "gettransaction"
    pub fn process_gtx(
        &mut self,
        gtx: GetTransactionResult,
        tip_height: u32,
        watcher: &mut HDWatcher,
    ) {
        let status = TxStatus::new(gtx.confirmations, tip_height);

        if !status.is_viable() {
            return self.purge_tx(&gtx.txid);
        }

        let txentry = TxEntry {
            status,
            fee: parse_fee(gtx.fee),
        };
        self.index_tx_entry(&gtx.txid, txentry);

        let txhist = HistoryEntry {
            status,
            txid: gtx.txid,
        };
        for detail in gtx.details {
            let category = TxCategory::from(detail.category); // XXX
            if !category.should_process() {
                continue;
            }

            self.index_address_history(detail.address, &detail.label, txhist.clone(), watcher);
        }
    }
    */

    /// Index transaction entry
    fn index_tx_entry(&mut self, txid: &Txid, txentry: TxEntry) {
        info!("index tx entry {:?}: {:?}", txid, txentry);

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

                if &curr_entry.status != &txentry.status {
                    changed_from = Some(curr_entry.status);
                    curr_entry.status = new_status;
                }
            })
            .or_insert_with(|| {
                info!("new tx: {:?}", txid);
                txentry
            });

        if let Some(old_status) = changed_from {
            self.update_tx_status(txid, old_status, new_status)
        }
    }

    /// Index address history entry
    fn index_address_history(
        &mut self,
        address: Address,
        label: &str,
        txhist: HistoryEntry,
        watcher: &mut HDWatcher,
    ) {
        debug!(
            "index address history {:?} {}: {:?}",
            address, label, txhist
        );
        let scripthash = address_to_scripthash(&address);

        let added = self
            .scripthashes
            .entry(scripthash)
            .or_insert_with(|| {
                let derivation_info = DerivationInfo::from_label(label);
                info!(
                    "new address {:?} ({:?}), marking as used",
                    address, derivation_info
                );
                watcher.mark_address(&derivation_info, true);

                ScriptEntry {
                    address,
                    derivation_info,
                    history: BTreeSet::new(),
                }
            })
            .history
            .insert(txhist);

        if added {
            info!("new history entry for {}", label)
        }
    }

    /// Update the scripthash history index to reflect the new tx status
    fn update_tx_status(
        &mut self,
        txid: &Txid,
        old_status: TxStatus,
        new_status: TxStatus,
    ) {
        if old_status == new_status {
            return;
        }

        info!(
            "transition tx {:?} status: {:?} -> {:?}",
            txid, old_status, new_status
        );

        let old_txhist = HistoryEntry {
            status: old_status,
            txid: *txid,
        };

        let new_txhist = HistoryEntry {
            status: new_status,
            txid: *txid,
        };

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

            // TODO optimize
            self.scripthashes
                .retain(|_scripthash, ScriptEntry { history, .. }| {
                    history.remove(&old_txhist);
                    history.len() > 0
                })
        }
    }

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Option<&BTreeSet<HistoryEntry>> {
        Some(&self.scripthashes.get(scripthash)?.history)
    }

    // get the address of a scripthash
    pub fn get_address(&self, scripthash: &sha256::Hash) -> Option<&Address> {
        self.scripthashes
            .get(scripthash)
            .map(|entry| &entry.address)
    }

    pub fn get_tx(&self, txid: &Txid) -> Option<&TxEntry> {
        self.transactions.get(txid)
    }
}

impl Ord for HistoryEntry {
    fn cmp(&self, other: &HistoryEntry) -> Ordering {
        self.status.cmp(&other.status)
    }
}

impl PartialOrd for HistoryEntry {
    fn partial_cmp(&self, other: &HistoryEntry) -> Option<Ordering> {
        Some(self.status.cmp(&other.status))
    }
}

// convert to a positive satoshi amount
fn parse_fee(fee: Option<SignedAmount>) -> Option<u64> {
    fee.map(|fee| fee.abs().as_sat() as u64)
}

// Fetch all unconfirmed transactions + transactions confirmed at or after start_height
fn load_transactions_since(
    rpc: &RpcClient,
    init_per_page: usize,
    start_height: u32,
    chunk_handler: &mut dyn FnMut(Vec<ListTransactionResult>, u32),
) -> Result<()> {
    let mut per_page = init_per_page;
    let mut start_index = 0;
    let mut oldest_seen = None;

    let tip_height = rpc.get_block_count()? as u32;
    let tip_hash = rpc.get_block_hash(tip_height as u64)?;

    // TODO: if the newest entry has the exact same (txid,address,height) as the previous newest,
    // skip processing the entries entirely

    while {
        info!(
            "reading {} transactions starting at index {}",
            per_page, start_index
        );

        let mut chunk = rpc.list_transactions(None, Some(per_page), Some(start_index), Some(true))?;

        let mut exhausted = chunk.len() < per_page;

        // this is necessary because we rely on the tip height to derive the confirmed height
        // from the number of confirmations
        if tip_hash != rpc.get_best_block_hash()? {
            warn!("tip changed while fetching transactions, retrying...");
            return load_transactions_since(rpc, per_page, start_height, chunk_handler);
        }

        // make sure we didn't miss any transactions by comparing the first entry of this page with
        // the last entry of the last page. note that the entry used for comprasion is popped off
        if let Some(ref oldest_seen) = oldest_seen {
            let marker = chunk.pop().or_err("missing market tx")?;

            if oldest_seen != &(marker.info.txid, marker.detail.address) {
                warn!("transaction set changed while fetching transactions, retrying...");
                return load_transactions_since(rpc, per_page, start_height, chunk_handler);
            }
        }

        // process entries (if any)
        if let Some(oldest) = chunk.first() {
            oldest_seen = Some((oldest.info.txid.clone(), oldest.detail.address.clone()));

            exhausted = exhausted
                || (oldest.info.confirmations > 0
                    && tip_height - oldest.info.confirmations as u32 + 1 < start_height);

            chunk_handler(chunk, tip_height);
        }
        !exhausted
    } {
        // -1 so we'll get the last entry of this page as the first of the next, as a marker for sanity check
        start_index = start_index + per_page - 1;
        per_page = per_page * 2;
    }

    Ok(())
}

fn should_process(ltx: &ListTransactionResult) -> bool {
    match ltx.detail.category {
        TxCategory::Send | TxCategory::Receive => true,
        TxCategory::Generate | TxCategory::Immature /*| TxCategory::Orphan*/ => false,
    }
}