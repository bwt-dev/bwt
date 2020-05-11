use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;

use bitcoin::{BlockHash, OutPoint, SignedAmount, Txid};
use bitcoincore_rpc::json::{
    GetTransactionResultDetailCategory as TxCategory, ListTransactionResult,
};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::hd::{HDWatcher, KeyOrigin};
use crate::store::{FundingInfo, HistoryEntry, MemoryStore, TxEntry};
use crate::types::{BlockId, ScriptHash, TxStatus};

#[cfg(feature = "track-spends")]
use crate::{store::SpendingInfo, types::TxInput};

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
            watcher: watcher,
            store: MemoryStore::new(),
            tip: None,
        }
    }

    pub fn store(&self) -> &MemoryStore {
        &self.store
    }

    pub fn sync(&mut self, track_updates: bool) -> Result<Vec<IndexUpdate>> {
        let mut updates = IndexUpdates::new(track_updates);

        // Detect reorgs and start syncing history from scratch when they happen
        if let Some(BlockId(tip_height, tip_hash)) = self.tip {
            let best_chain_hash = self.rpc.get_block_hash(tip_height as u64)?;
            if best_chain_hash != tip_hash {
                warn!(
                    "reorg detected, block height {} was {} and now is {}. fetching history from scratch...",
                    tip_height, tip_hash, best_chain_hash
                );

                // XXX start syncing from N blocks backs instead of from the beginning?
                self.tip = None;

                updates.push(|| IndexUpdate::Reorg(tip_height, tip_hash, best_chain_hash));
            }
        }

        let synced_tip = self.sync_transactions(&mut updates)?;

        self.watcher.watch(&self.rpc)?;

        if self.tip.as_ref() != Some(&synced_tip) {
            info!("synced up to {:?}", synced_tip);
            updates.push(|| IndexUpdate::ChainTip(synced_tip.clone()));
            self.tip = Some(synced_tip);
        }

        let updates = updates.into_vec();

        if !updates.is_empty() {
            info!("sync resulted in {} index updates", updates.len());
            debug!("{:#?}", updates);
        }

        Ok(updates)
    }

    fn sync_transactions(&mut self, updates: &mut IndexUpdates) -> Result<BlockId> {
        let start_height = self
            .tip
            .as_ref()
            .map_or(0, |BlockId(tip_height, _)| tip_height + 1);

        let mut pending_outgoing: HashMap<Txid, TxEntry> = HashMap::new();

        let synced_tip = load_transactions_since(
            &self.rpc.clone(),
            start_height,
            None,
            &mut |chunk, tip_height| {
                for ltx in chunk {
                    if ltx.info.confirmations < 0 {
                        if self.store.purge_tx(&ltx.info.txid) {
                            updates.push(|| IndexUpdate::TransactionReplaced(ltx.info.txid));
                        }
                        continue;
                    }
                    match ltx.detail.category {
                        TxCategory::Receive => {
                            self.process_incoming(ltx, tip_height, updates);
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
            },
        )?;

        for (txid, txentry) in pending_outgoing {
            self.process_outgoing(txid, txentry, updates)
                .map_err(|err| warn!("failed processing outgoing payment: {:?}", err))
                .ok();
        }

        Ok(synced_tip)
    }

    fn process_incoming(
        &mut self,
        ltx: ListTransactionResult,
        tip_height: u32,
        updates: &mut IndexUpdates,
    ) {
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

        let txid = ltx.info.txid;
        let scripthash = ScriptHash::from(&ltx.detail.address);
        let amount = ltx.detail.amount.to_unsigned().unwrap().as_sat(); // safe to unwrap, incoming payments cannot have negative amounts

        trace!(
            "processing incoming txout {}:{} scripthash={} address={} origin={:?} status={:?} amount={}",
            txid, ltx.detail.vout, scripthash, ltx.detail.address, origin, status, amount
        );

        let mut txentry = TxEntry::new(status, None);
        let funding_info = FundingInfo(scripthash, amount);
        txentry.funding.insert(ltx.detail.vout, funding_info);

        if self.store.index_tx_entry(&txid, txentry) {
            updates.push(|| IndexUpdate::Transaction(txid));
        }

        if self
            .store
            .track_scripthash(&scripthash, &origin, &ltx.detail.address)
        {
            updates.push(|| IndexUpdate::FirstFunding(scripthash));
        }

        if self
            .store
            .index_history_entry(&scripthash, HistoryEntry::new(txid, status))
        {
            updates.push(|| IndexUpdate::History(scripthash, txid));
        }

        self.watcher.mark_funded(&origin);
    }

    #[cfg_attr(not(feature = "track-spends"), allow(unused_variables, unused_mut))]
    fn process_outgoing(
        &mut self,
        txid: Txid,
        mut txentry: TxEntry,
        updates: &mut IndexUpdates,
    ) -> Result<()> {
        trace!(
            "processing outgoing tx txid={} status={:?}",
            txid,
            txentry.status
        );

        let tx = self.rpc.get_transaction(&txid, Some(true))?.transaction()?;

        for (vin, input) in tx.input.iter().enumerate() {
            if let Some(FundingInfo(scripthash, amount)) =
                self.store.lookup_txo_fund(&input.previous_output)
            {
                if self
                    .store
                    .index_history_entry(&scripthash, HistoryEntry::new(txid, txentry.status))
                {
                    updates.push(|| IndexUpdate::History(scripthash, txid));
                }

                #[cfg(feature = "track-spends")]
                {
                    // we could keep just the previous_output and lookup the scripthash and amount
                    // from the corrospanding FundingInfo, but we keep it here anyway for quick access
                    #[cfg(feature = "track-spends")]
                    txentry.spending.insert(
                        vin as u32,
                        SpendingInfo(scripthash, input.previous_output, amount),
                    );

                    if self
                        .store
                        .index_txo_spend(input.previous_output, TxInput::new(txid, vin as u32))
                    {
                        updates.push(|| IndexUpdate::TxoSpent(input.previous_output));
                    }
                }
            }
        }

        #[cfg(feature = "track-spends")]
        {
            if !txentry.spending.is_empty() {
                if self.store.index_tx_entry(&txid, txentry) {
                    updates.push(|| IndexUpdate::Transaction(txid));
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "category", content = "params")]
pub enum IndexUpdate {
    ChainTip(BlockId),
    Reorg(u32, BlockHash, BlockHash),

    Transaction(Txid),
    TransactionReplaced(Txid),

    History(ScriptHash, Txid),
    FirstFunding(ScriptHash),

    #[cfg(feature = "track-spends")]
    TxoSpent(OutPoint),
}

enum IndexUpdates {
    Stored(Vec<IndexUpdate>),
    Void,
}

impl IndexUpdates {
    fn new(stored: bool) -> Self {
        if stored {
            IndexUpdates::Stored(vec![])
        } else {
            IndexUpdates::Void
        }
    }
    fn push(&mut self, make_update: impl Fn() -> IndexUpdate) {
        match self {
            IndexUpdates::Stored(updates) => updates.push(make_update()),
            IndexUpdates::Void => (),
        }
    }
    fn into_vec(self) -> Vec<IndexUpdate> {
        match self {
            IndexUpdates::Stored(updates) => updates,
            IndexUpdates::Void => vec![],
        }
    }
}
impl IndexUpdate {
    // the scripthash affected by the update, if any
    pub fn scripthash(&self) -> Option<&ScriptHash> {
        match self {
            IndexUpdate::History(ref scripthash, _) | IndexUpdate::FirstFunding(ref scripthash) => {
                Some(scripthash)
            }
            _ => None,
        }
    }

    // the (previously) utxo spent by the update, if any
    pub fn outpoint(&self) -> Option<&OutPoint> {
        match self {
            #[cfg(feature = "track-spends")]
            IndexUpdate::TxoSpent(ref outpoint) => Some(outpoint),
            _ => None,
        }
    }

    pub fn category_str(&self) -> &str {
        match self {
            Self::ChainTip(_) => "ChainTip",
            Self::Reorg(..) => "Reorg",

            Self::Transaction(_) => "Transaction",
            Self::TransactionReplaced(_) => "TransactionReplaced",

            Self::History(..) => "History",
            Self::FirstFunding(_) => "FirstFunding",

            #[cfg(feature = "track-spends")]
            Self::TxoSpent(_) => "TxoSpent",
        }
    }
}

impl fmt::Display for IndexUpdate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// convert to a positive satoshi amount
fn parse_fee(fee: Option<SignedAmount>) -> Option<u64> {
    fee.map(|fee| fee.abs().as_sat() as u64)
}

const INIT_TX_PER_PAGE: usize = 150;
const DELTA_TX_PER_PAGE: usize = 25;
const MAX_TX_PER_PAGE: usize = 500;

// Fetch all unconfirmed transactions + transactions confirmed at or after start_height
fn load_transactions_since(
    rpc: &RpcClient,
    start_height: u32,
    init_per_page: Option<usize>,
    chunk_handler: &mut dyn FnMut(Vec<ListTransactionResult>, u32),
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

    // TODO: if the newest entry has the exact same (txid,address,height) as the previous newest,
    // skip processing the entries entirely

    if start_height <= tip_height {
        info!(
            "syncing transactions from block(s) {}..{} + mempool",
            start_height, tip_height,
        );
    } else {
        info!("syncing mempool transactions");
    }

    loop {
        trace!(
            "fetching {} transactions starting at index {}",
            per_page,
            start_index
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
            oldest_seen = Some((oldest.info.txid.clone(), oldest.detail.vout));
        } else {
            break;
        }

        let chunk: Vec<ListTransactionResult> = chunk
            .into_iter()
            .rev()
            .take_while(|ltx| ltx.info.confirmations <= max_confirmations)
            .collect();

        let exhausted = chunk.len() < per_page;

        chunk_handler(chunk, tip_height);

        if exhausted {
            break;
        }

        // -1 so we'll get the last entry of this page as the first of the next, as a marker for sanity check
        start_index = start_index + per_page - 1;
        per_page = MAX_TX_PER_PAGE.min(per_page * 2);
    }

    Ok(BlockId(tip_height, tip_hash))
}
