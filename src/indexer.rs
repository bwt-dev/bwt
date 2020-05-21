use std::collections::HashMap;
use std::sync::Arc;
use std::{fmt, time};

use serde::Serialize;

use bitcoin::{BlockHash, OutPoint, Txid};
use bitcoincore_rpc::json::{
    GetTransactionResultDetailCategory as TxCategory, ListTransactionResult,
};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::hd::{HDWatcher, KeyOrigin};
use crate::store::{FundingInfo, MemoryStore, SpendingInfo};
use crate::types::{BlockId, InPoint, ScriptHash, TxStatus};

pub struct Indexer {
    rpc: Arc<RpcClient>,
    watcher: HDWatcher,
    store: MemoryStore,
    tip: Option<BlockId>,
}

impl Indexer {
    pub fn new(rpc: Arc<RpcClient>, watcher: HDWatcher) -> Self {
        Indexer {
            rpc,
            watcher,
            store: MemoryStore::new(),
            tip: None,
        }
    }

    pub fn store(&self) -> &MemoryStore {
        &self.store
    }

    pub fn watcher(&self) -> &HDWatcher {
        &self.watcher
    }

    // continue to sync transactions and import addresses (with rescan) until no more new addresses
    // need to be imported. the initial sync does not collect the Changelog and does not emit updates.
    pub fn initial_sync(&mut self) -> Result<()> {
        let timer = time::Instant::now();

        info!("starting initial sync");
        self.watcher.check_imports(&self.rpc)?;

        let (mut synced_tip, _) = self._sync()?;
        while self.watcher.do_imports(&self.rpc, /*rescan=*/ true)? {
            let (tip, _) = self._sync()?;
            synced_tip = tip;
        }

        info!(
            "completed initial sync in {:?} up to height {} (total {})",
            timer.elapsed(),
            synced_tip.0,
            self.store.stats_str(),
        );
        self.tip = Some(synced_tip);
        Ok(())
    }

    // initiate a regular sync to catch up with updates and import new addresses (no rescan)
    pub fn sync(&mut self) -> Result<Vec<IndexChange>> {
        let (synced_tip, mut changelog) = self._sync()?;
        self.watcher.do_imports(&self.rpc, /*rescan=*/ false)?;

        if self.tip.as_ref() != Some(&synced_tip) {
            info!("synced up to height {}", synced_tip.0);
            changelog.push(IndexChange::ChainTip(synced_tip.clone()));
            self.tip = Some(synced_tip);
        }

        Ok(changelog)
    }

    fn _sync(&mut self) -> Result<(BlockId, Vec<IndexChange>)> {
        // only track changes when we're catching up with updates
        let mut changelog = Changelog::new(self.tip.is_some());

        // detect reorgs and sync the whole history from scratch when they happen
        // XXX start syncing from N blocks backs instead of from the beginning?
        if let Some(BlockId(tip_height, ref tip_hash)) = self.tip {
            let best_chain_hash = self.rpc.get_block_hash(tip_height as u64)?;
            if best_chain_hash != *tip_hash {
                warn!(
                    "reorg detected, block height {} was {} and now is {}. fetching history from scratch...",
                    tip_height, tip_hash, best_chain_hash
                );

                changelog.push(|| IndexChange::Reorg(tip_height, *tip_hash, best_chain_hash));
                // notify clients about the reorg but don't collect additional events
                changelog.track = false;

                self.tip = None;
            }
        }

        let synced_tip = self.sync_transactions(&mut changelog)?;

        let changelog = changelog.into_vec();

        if !changelog.is_empty() {
            info!(
                "sync resulted in {} index changelog events",
                changelog.len()
            );
            if log_enabled!(log::Level::Debug) {
                for update in &changelog {
                    debug!("  - {:?}", update);
                }
            }
        }

        Ok((synced_tip, changelog))
    }

    fn sync_transactions(&mut self, changelog: &mut Changelog) -> Result<BlockId> {
        let start_height = self
            .tip
            .as_ref()
            .map_or(0, |BlockId(tip_height, _)| tip_height + 1);

        let mut buffered_outgoing: HashMap<Txid, (i32, u64)> = HashMap::new();

        let synced_tip = load_transactions_since(
            &self.rpc.clone(),
            start_height,
            None,
            &mut |chunk, tip_height, is_first| {
                // reset buffered txs whenever load_transactions_since() starts over, we'll get these
                // outgoing txs again later (with a potentially different, more updated, `confirmations` number)
                if is_first {
                    buffered_outgoing.clear();
                }

                for ltx in chunk {
                    if ltx.info.confirmations < 0 {
                        if self.store.purge_tx(&ltx.info.txid) {
                            changelog.push(|| IndexChange::TransactionReplaced(ltx.info.txid));
                        }
                        continue;
                    }

                    // "listtransactions" in fact lists transaction outputs and not transactions.
                    // for "receive" txs, it returns one entry per wallet-owned output in the tx.
                    // for "send" txs, it returns one entry for every output in the tx, owned or not.
                    match ltx.detail.category {
                        TxCategory::Receive => {
                            // incoming txouts are easy: bitcoind tells us the associated
                            // address and label, giving us all the information we need in
                            // order to save the txo to the index.
                            self.process_incoming_txo(ltx, tip_height, changelog);
                        }
                        TxCategory::Send => {
                            // outgoing txs are more tricky: bitcoind doesn't tell us which
                            // prevouts are being spent, so we have to fetch the transaction to
                            // determine it. we can't do that straightaway because prevouts being
                            // spent might not be indexed yet. instead, buffer outgoing txs and
                            // process them at the end, so that the parent txs funding the prevouts
                            // are guaranteed to get indexed first.
                            buffered_outgoing.entry(ltx.info.txid).or_insert_with(|| {
                                // "send" transactions must have a fee
                                let fee = ltx.detail.fee.unwrap().abs().as_sat() as u64;
                                (ltx.info.confirmations, fee)
                            });
                        }
                        // ignore mining-related transactions
                        TxCategory::Generate | TxCategory::Immature | TxCategory::Orphan => (),
                    };
                }
            },
        )?;

        for (txid, (confirmations, fee)) in buffered_outgoing {
            let status = TxStatus::from_confirmations(confirmations, synced_tip.0);
            self.process_outgoing_tx(txid, status, fee, changelog)
                .map_err(|err| warn!("failed processing outgoing payment: {:?}", err))
                .ok();
        }

        // TODO: complete fee information for incoming-only txs

        Ok(synced_tip)
    }

    // upsert the transaction while collecting changelog
    fn upsert_tx(
        &mut self,
        txid: &Txid,
        status: TxStatus,
        fee: Option<u64>,
        changelog: &mut Changelog,
    ) {
        let tx_updated = self.store.upsert_tx(txid, status, fee);
        if tx_updated {
            changelog.with(|changelog| {
                changelog.push(IndexChange::Transaction(*txid, status));

                // create an update entry for every affected scripthash
                let tx_entry = self.store.get_tx_entry(&txid).unwrap();
                changelog.extend(
                    tx_entry
                        .scripthashes()
                        .into_iter()
                        .map(|scripthash| IndexChange::History(*scripthash, *txid, status)),
                );
            });
        }
    }

    fn process_incoming_txo(
        &mut self,
        ltx: ListTransactionResult,
        tip_height: u32,
        changelog: &mut Changelog,
    ) {
        let label = ltx.detail.label.as_ref();
        let origin = some_or_ret!(label.and_then(|l| KeyOrigin::from_label(l)));

        // XXX we assume that any address with a "bwt/..." label is ours, this may not necessarily be true.

        let txid = ltx.info.txid;
        let vout = ltx.detail.vout;
        let scripthash = ScriptHash::from(&ltx.detail.address);
        let status = TxStatus::from_confirmations(ltx.info.confirmations, tip_height);
        let amount = ltx.detail.amount.to_unsigned().unwrap().as_sat(); // safe to unwrap, incoming payments cannot have negative amounts

        trace!(
            "processing incoming txout {}:{} scripthash={} address={} origin={:?} status={:?} amount={}",
            txid, vout, scripthash, ltx.detail.address, origin, status, amount
        );

        self.upsert_tx(&txid, status, None, changelog);

        self.store
            .index_scripthash(&scripthash, &origin, &ltx.detail.address);

        let txo_added =
            self.store
                .index_tx_output_funding(&txid, vout, FundingInfo(scripthash, amount));

        if txo_added {
            changelog.push(|| IndexChange::History(scripthash, txid, status));
            changelog.push(|| IndexChange::TxoCreated(OutPoint::new(txid, vout), status));
            self.watcher.mark_funded(&origin);
        }
    }

    fn process_outgoing_tx(
        &mut self,
        txid: Txid,
        status: TxStatus,
        fee: u64,
        changelog: &mut Changelog,
    ) -> Result<()> {
        trace!("processing outgoing tx txid={} status={:?}", txid, status);

        if let Some(tx_entry) = self.store.get_tx_entry(&txid) {
            if !tx_entry.spending.is_empty() {
                // TODO keep a marker for processed transactions that had no spending inputs
                trace!("skipping outgoing tx {}, already indexed", txid);
                return Ok(());
            }
        }

        // TODO use batch rpc to fetch all buffered outgoing txs
        let tx = self.rpc.get_transaction(&txid, Some(true))?.transaction()?;

        let spending: HashMap<u32, SpendingInfo> = tx
            .input
            .iter()
            .enumerate()
            .filter_map(|(vin, input)| {
                let FundingInfo(scripthash, amount) =
                    self.store.lookup_txo_fund(&input.previous_output)?;
                let input_point = InPoint::new(txid, vin as u32);

                #[cfg(feature = "track-spends")]
                self.store
                    .index_txo_spend(input.previous_output, input_point);

                changelog.push(|| IndexChange::History(scripthash, txid, status));
                changelog
                    .push(|| IndexChange::TxoSpent(input.previous_output, input_point, status));

                // we could keep just the previous_output and lookup the scripthash and amount
                // from the corrospanding FundingInfo, but we keep it here anyway for quick access
                Some((
                    vin as u32,
                    SpendingInfo(scripthash, input.previous_output, amount),
                ))
            })
            .collect();

        if !spending.is_empty() {
            self.upsert_tx(&txid, status, Some(fee), changelog);
            self.store.index_tx_inputs_spending(&txid, spending);
        }

        Ok(())
    }
}

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "category", content = "params")]
pub enum IndexChange {
    ChainTip(BlockId),
    Reorg(u32, BlockHash, BlockHash),

    Transaction(Txid, TxStatus),
    TransactionReplaced(Txid),

    History(ScriptHash, Txid, TxStatus),
    TxoCreated(OutPoint, TxStatus),
    TxoSpent(OutPoint, InPoint, TxStatus),
}

struct Changelog {
    track: bool,
    changes: Vec<IndexChange>,
}

impl Changelog {
    fn new(track: bool) -> Self {
        Changelog {
            track,
            changes: vec![],
        }
    }
    fn push(&mut self, make_update: impl Fn() -> IndexChange) {
        if self.track {
            self.changes.push(make_update());
        }
    }
    fn with(&mut self, closure: impl Fn(&mut Vec<IndexChange>)) {
        if self.track {
            closure(&mut self.changes)
        }
    }
    fn into_vec(self) -> Vec<IndexChange> {
        self.changes
    }
}
impl IndexChange {
    // the scripthash affected by the update, if any
    pub fn scripthash(&self) -> Option<&ScriptHash> {
        match self {
            IndexChange::History(ref scripthash, ..) => Some(scripthash),
            _ => None,
        }
    }

    // the (previously) utxo spent by the update, if any
    pub fn outpoint(&self) -> Option<&OutPoint> {
        match self {
            IndexChange::TxoSpent(ref outpoint, ..) | IndexChange::TxoCreated(ref outpoint, ..) => {
                Some(outpoint)
            }
            _ => None,
        }
    }

    pub fn category_str(&self) -> &str {
        match self {
            Self::ChainTip(..) => "ChainTip",
            Self::Reorg(..) => "Reorg",

            Self::Transaction(..) => "Transaction",
            Self::TransactionReplaced(..) => "TransactionReplaced",

            Self::History(..) => "History",
            Self::TxoCreated(..) => "TxoCreated",
            Self::TxoSpent(..) => "TxoSpent",
        }
    }
}

impl fmt::Display for IndexChange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

const INIT_TX_PER_PAGE: usize = 500;
const DELTA_TX_PER_PAGE: usize = 25;
const MAX_TX_PER_PAGE: usize = 5000;

// Fetch all unconfirmed transactions + transactions confirmed at or after start_height
fn load_transactions_since(
    rpc: &RpcClient,
    start_height: u32,
    init_per_page: Option<usize>,
    chunk_handler: &mut dyn FnMut(Vec<ListTransactionResult>, u32, bool),
) -> Result<BlockId> {
    let mut start_index = 0;
    let mut per_page = init_per_page.unwrap_or_else(|| {
        if start_height == 0 {
            // start with larger pages if we're catching up for the first time
            INIT_TX_PER_PAGE
        } else {
            DELTA_TX_PER_PAGE
        }
    });
    let mut oldest_seen = None;

    let tip_height = rpc.get_block_count()? as u32;
    let tip_hash = rpc.get_block_hash(tip_height as u64)?;

    assert!(start_height <= tip_height + 1, "start_height too far");
    let max_confirmations = (tip_height + 1 - start_height) as i32;

    if start_height <= tip_height {
        info!(
            "syncing transactions since height {} + mempool transactions (tip height={} hash={})",
            start_height, tip_height, tip_hash,
        );
    } else {
        debug!("syncing mempool transactions (no new blocks)");
    }

    loop {
        debug!(
            "fetching {} transactions starting at index {}",
            per_page, start_index
        );

        let mut chunk =
            rpc.list_transactions(None, Some(per_page), Some(start_index), Some(true))?;

        // this is necessary because we rely on the tip height to derive the confirmed height
        // from the number of confirmations
        if tip_hash != rpc.get_best_block_hash()? {
            warn!("tip changed while fetching transactions, retrying...");
            return load_transactions_since(rpc, start_height, Some(per_page), chunk_handler);
        }

        // make sure we didn't miss any transactions by comparing the first entry of this page with
        // the last entry of the last page (the "marker")
        if let Some(oldest_seen) = &oldest_seen {
            let marker = chunk.pop().or_err("missing marker tx")?;

            if oldest_seen != &(marker.info.txid, marker.detail.vout) {
                warn!("transaction set changed while fetching transactions, retrying...");
                return load_transactions_since(rpc, start_height, Some(per_page), chunk_handler);
            }
        }
        // update the marker
        if let Some(oldest) = chunk.first() {
            oldest_seen = Some((oldest.info.txid, oldest.detail.vout));
        } else {
            break;
        }

        let chunk: Vec<ListTransactionResult> = chunk
            .into_iter()
            .rev()
            .take_while(|ltx| ltx.info.confirmations <= max_confirmations)
            .collect();

        let exhausted = if start_index == 0 {
            chunk.len() < per_page
        } else {
            // account for the removed marker tx
            chunk.len() < per_page - 1
        };

        chunk_handler(chunk, tip_height, start_index == 0);

        if exhausted {
            break;
        }

        // -1 so we'll get the last entry of this page as the first of the next, as a marker for sanity check
        start_index = start_index + per_page - 1;
        per_page = MAX_TX_PER_PAGE.min(per_page * 2);
    }

    Ok(BlockId(tip_height, tip_hash))
}
