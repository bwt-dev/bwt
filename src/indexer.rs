use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::{fmt, thread, time};

use serde::Serialize;

use bitcoin::{Address, BlockHash, OutPoint, Txid};
use bitcoincore_rpc::json::{
    GetTransactionResultDetailCategory as TxCategory, ListTransactionResult,
};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::Result;
use crate::store::{FundingInfo, MemoryStore, SpendingInfo, TxEntry};
use crate::types::{BlockId, InPoint, RescanSince, ScriptHash, TxStatus};
use crate::util::bitcoincore_ext::{Progress, RpcApiExt};
use crate::wallet::{KeyOrigin, WalletWatcher};

pub struct Indexer {
    rpc: Arc<RpcClient>,
    watcher: WalletWatcher,
    store: MemoryStore,
    tip: Option<BlockId>,
}

impl Indexer {
    pub fn new(rpc: Arc<RpcClient>, watcher: WalletWatcher) -> Self {
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

    pub fn watcher(&self) -> &WalletWatcher {
        &self.watcher
    }

    // continue to sync transactions and import addresses (with rescan) until no more new addresses
    // need to be imported. the initial sync does not collect the Changelog and does not emit updates.
    pub fn initial_sync(&mut self, progress_tx: Option<mpsc::Sender<Progress>>) -> Result<()> {
        let timer = time::Instant::now();

        info!("starting initial sync");
        self.watcher.check_imports(&self.rpc)?;

        let mut changelog = Changelog::new(false);
        let mut synced_tip;

        spawn_send_progress_thread(self.rpc.clone(), progress_tx);

        while {
            synced_tip = self.sync_transactions(&mut changelog)?;
            self.watcher.do_imports(&self.rpc, /*rescan=*/ true)?
        } { /* do while */ }

        self.sync_mempool(/*force_refresh=*/ true)?;

        let stats = self.store.stats();
        info!(
            "completed initial sync in {:?} up to height {} (total {} transactions and {} addresses)",
            timer.elapsed(),
            synced_tip.0,
            stats.transaction_count,
            stats.scripthash_count,
        );
        self.tip = Some(synced_tip);
        Ok(())
    }

    // initiate a regular sync to catch up with updates and import new addresses (no rescan)
    pub fn sync(&mut self) -> Result<Vec<IndexChange>> {
        let mut changelog = Changelog::new(self.tip.is_some());

        // detect reorgs and sync the whole history from scratch when they happen
        // XXX the reorg test is racey
        if let Some(BlockId(tip_height, ref tip_hash)) = self.tip {
            let best_chain_hash = self.rpc.get_block_hash(tip_height as u64)?;
            if best_chain_hash != *tip_hash {
                warn!(
                    "reorg detected, block height {} was {} and now is {}. fetching history from scratch...",
                    tip_height, tip_hash, best_chain_hash
                );

                // notify clients about the reorg, but don't collect additional events (apart from
                // ChainTip, added below)
                changelog.push(|| IndexChange::Reorg(tip_height, *tip_hash, best_chain_hash));
                changelog.track = false;

                // XXX start syncing from N blocks backs instead of from the beginning?
                self.tip = None;
            }
        }

        let synced_tip = self.sync_transactions(&mut changelog)?;
        let tip_updated = self.tip != Some(synced_tip);
        self.sync_mempool(/*force_refresh=*/ tip_updated)?;
        self.watcher.do_imports(&self.rpc, /*rescan=*/ false)?;

        let mut changelog = changelog.into_vec();

        if tip_updated {
            info!("synced up to height {}", synced_tip.0);
            changelog.push(IndexChange::ChainTip(synced_tip));
            self.tip = Some(synced_tip);
        }

        if !changelog.is_empty() && log_enabled!(log::Level::Debug) {
            debug!(
                "sync resulted in {} index changelog events",
                changelog.len()
            );
            for update in &changelog {
                debug!("  - {:?}", update);
            }
        }

        Ok(changelog)
    }

    fn sync_transactions(&mut self, changelog: &mut Changelog) -> Result<BlockId> {
        let since_block = self.tip.as_ref().map(|tip| &tip.1);
        let tip_height = self.rpc.get_block_count()? as u32;
        let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

        let result = self
            .rpc
            .list_since_block(since_block, Some(1), Some(true), Some(true))?;

        // Workaround for https://github.com/bitcoin/bitcoin/issues/19338,
        // listsinceblock is not atomic and could provide inconsistent results.
        if result.lastblock != tip_hash {
            warn!("chain tip moved while reading listsinceblock, retrying...");
            return self.sync_transactions(changelog);
        }

        for ltx in result.removed {
            // transactions that were re-added in the active chain will appear in `removed`
            // but with a positive confirmation count, ignore these.
            if ltx.info.confirmations < 0 {
                let tx_deleted = self.store.purge_tx(&ltx.info.txid);
                if tx_deleted {
                    changelog.push(|| IndexChange::TransactionReplaced(ltx.info.txid));
                }
            }
        }

        let mut buffered_outgoing: HashMap<Txid, i32> = HashMap::new();

        for ltx in result.transactions {
            // "listtransactions"/"listsinceblock" in fact lists transaction outputs and not transactions.
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
                    // indexing outgoing txs require fetching the list of spent prevouts and
                    // comparing them against the wallet's known funded outputs. we can't do that
                    // straightaway because the prevouts being spent might not be indexed yet, so
                    // the outgoing txs are buffered and processed at the end, after the txs funding
                    // the prevouts are guarranted to be indexed.
                    buffered_outgoing.insert(ltx.info.txid, ltx.info.confirmations);
                }
                // ignore mining-related transactions
                TxCategory::Generate | TxCategory::Immature | TxCategory::Orphan => (),
            };
        }

        for (txid, confirmations) in buffered_outgoing {
            let status = TxStatus::from_confirmations(confirmations, tip_height);
            self.process_outgoing_tx(txid, status, changelog)
                .map_err(|err| warn!("failed processing outgoing payment: {:?}", err))
                .ok();
        }

        Ok(BlockId(tip_height, tip_hash))
    }

    // upsert the transaction while collecting the changelog
    fn upsert_tx(&mut self, txid: &Txid, status: TxStatus, changelog: &mut Changelog) {
        let tx_updated = self.store.upsert_tx(txid, status);
        if tx_updated {
            changelog.with(|changelog| {
                let tx_entry = self.store.get_tx_entry(txid).unwrap();
                changelog.extend(IndexChange::from_tx(txid, tx_entry));
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
        let address = some_or_ret!(ltx.detail.address);

        // XXX we assume that any address with a "bwt/..." label is ours, this may not necessarily be true.

        let txid = ltx.info.txid;
        let vout = ltx.detail.vout;
        let scripthash = ScriptHash::from(&address);
        let status = TxStatus::from_confirmations(ltx.info.confirmations, tip_height);
        let amount = ltx.detail.amount.to_unsigned().unwrap().as_sat(); // safe to unwrap, incoming payments cannot have negative amounts

        trace!(
            "processing incoming txout {}:{} scripthash={} address={} origin={:?} status={:?} amount={}",
            txid, vout, scripthash, address, origin, status, amount
        );

        self.upsert_tx(&txid, status, changelog);

        self.store.index_scripthash(&scripthash, &origin, &address);

        let txo_added =
            self.store
                .index_tx_output_funding(&txid, vout, FundingInfo(scripthash, amount));

        if txo_added {
            changelog.push(|| {
                IndexChange::TxoFunded(OutPoint::new(txid, vout), scripthash, amount, status)
            });
            self.watcher.mark_funded(&origin);
        }
    }

    fn process_outgoing_tx(
        &mut self,
        txid: Txid,
        status: TxStatus,
        changelog: &mut Changelog,
    ) -> Result<()> {
        trace!("processing outgoing tx txid={} status={:?}", txid, status);

        if let Some(tx_entry) = self.store.get_tx_entry(&txid) {
            // TODO keep a marker for processed transactions that had no spending inputs
            if !tx_entry.spending.is_empty() {
                // skip indexing spent inputs, but keep the status which might be more recent
                self.upsert_tx(&txid, status, changelog);
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
                let inpoint = InPoint::new(txid, vin as u32);
                let prevout = input.previous_output;
                let FundingInfo(scripthash, amount) = self.store.lookup_txo_fund(&prevout)?;

                #[cfg(feature = "track-spends")]
                self.store.index_txo_spend(prevout, inpoint);

                changelog.push(|| IndexChange::TxoSpent(inpoint, scripthash, prevout, status));

                // we could keep just the previous_output and lookup the scripthash and amount
                // from the corresponding FundingInfo, but we keep it here anyway for quick access
                let spending_info = SpendingInfo(scripthash, prevout, amount);
                Some((vin as u32, spending_info))
            })
            .collect();

        if !spending.is_empty() {
            self.upsert_tx(&txid, status, changelog);
            self.store.index_tx_inputs_spending(&txid, spending);
        }

        Ok(())
    }

    /// Update missing/outdated mempool entries for unconfirmed mempool transactions (or all mempool
    /// entries when force_refresh is set, during the initial sync or following a chain tip update)
    fn sync_mempool(&mut self, force_refresh: bool) -> Result<()> {
        let mempool = self.store.mempool_mut();

        for (txid, opt_entry) in mempool.iter_mut() {
            if force_refresh || opt_entry.is_none() {
                let rpc_entry = self.rpc.get_mempool_entry(txid)?;
                *opt_entry = Some(rpc_entry.into());
            }
        }

        // TODO use batch rpc

        Ok(())
    }

    /// Get historical events that happened at or after `min_block_height`, including unconfirmed,
    /// ordered with oldest first.
    ///
    /// Includes the `Transaction`, `TxoFunded` and `TxoSpent` events, and a *single* `ChainTip`
    /// event with the currently synced tip as the last entry (when bwt is synced).
    pub fn get_changelog_since(&self, min_block_height: u32) -> Vec<IndexChange> {
        self.store
            .get_history_since(min_block_height)
            .into_iter()
            .map(|txhist| {
                let tx_entry = self.store.get_tx_entry(&txhist.txid).unwrap();
                IndexChange::from_tx(&txhist.txid, tx_entry)
            })
            .flatten()
            .chain(self.tip.clone().map(IndexChange::ChainTip).into_iter())
            .collect()
    }

    pub fn track_address(&mut self, address: Address, rescan_since: RescanSince) -> Result<()> {
        self.watcher.track_address(address, rescan_since)
    }
}

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "category", content = "params")]
pub enum IndexChange {
    ChainTip(BlockId),
    Reorg(u32, BlockHash, BlockHash),

    Transaction(Txid, TxStatus),
    TransactionReplaced(Txid),

    TxoFunded(OutPoint, ScriptHash, u64, TxStatus),
    TxoSpent(InPoint, ScriptHash, OutPoint, TxStatus),
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
            IndexChange::TxoFunded(_, ref scripthash, ..) => Some(scripthash),
            IndexChange::TxoSpent(_, ref scripthash, ..) => Some(scripthash),
            _ => None,
        }
    }

    // the outpoint created or spent, if any
    pub fn outpoint(&self) -> Option<&OutPoint> {
        match self {
            IndexChange::TxoFunded(ref outpoint, ..) => Some(outpoint),
            IndexChange::TxoSpent(_, _, ref outpoint, _) => Some(outpoint),
            _ => None,
        }
    }

    pub fn category_str(&self) -> &str {
        match self {
            Self::ChainTip(..) => "ChainTip",
            Self::Reorg(..) => "Reorg",

            Self::Transaction(..) => "Transaction",
            Self::TransactionReplaced(..) => "TransactionReplaced",

            Self::TxoFunded(..) => "TxoFunded",
            Self::TxoSpent(..) => "TxoSpent",
        }
    }

    // create all the changelog events inflicted by the transaction
    fn from_tx(txid: &Txid, tx_entry: &TxEntry) -> Vec<Self> {
        let mut changes = vec![IndexChange::Transaction(*txid, tx_entry.status)];

        changes.extend(tx_entry.funding.iter().map(|(vout, funding_info)| {
            let outpoint = OutPoint::new(*txid, *vout);
            let FundingInfo(scripthash, amount) = funding_info;
            IndexChange::TxoFunded(outpoint, *scripthash, *amount, tx_entry.status)
        }));

        changes.extend(tx_entry.spending.iter().map(|(vin, spending_info)| {
            let inpoint = InPoint::new(*txid, *vin);
            let SpendingInfo(scripthash, prevout, _) = spending_info;
            IndexChange::TxoSpent(inpoint, *scripthash, *prevout, tx_entry.status)
        }));

        changes
    }
}

impl fmt::Display for IndexChange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// Spawn a thread to poll getwalletinfo, log progress and send progress updates via mpsc
fn spawn_send_progress_thread(
    rpc: Arc<RpcClient>,
    progress_tx: Option<mpsc::Sender<Progress>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        // allow some time for the indexer to start the first set of imports
        thread::sleep(time::Duration::from_millis(250));

        if let Err(e) = rpc.wait_wallet_scan(progress_tx) {
            warn!("getwalletinfo failed: {:?}", e);
        }
    })
}
