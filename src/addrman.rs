use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use bitcoin::Address;
use bitcoin_hashes::{sha256, sha256d};
use bitcoincore_rpc::{json::ListUnspentResult, Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::json::{GetTransactionResult, ListTransactionsResult, TxCategory};
use crate::util::address_to_scripthash;

#[cfg(feature = "electrum")]
use crate::electrum::get_status_hash;

pub struct AddrManager {
    rpc: Arc<RpcClient>,
    index: RwLock<Index>,
}

#[derive(Debug)]
struct Index {
    scripthashes: HashMap<sha256::Hash, ScriptEntry>,
    transactions: HashMap<sha256d::Hash, TxEntry>,
}

#[derive(Debug)]
struct ScriptEntry {
    address: String,
    history: BTreeSet<HistoryEntry>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct HistoryEntry {
    pub txid: sha256d::Hash,
    pub status: TxStatus,
}

#[derive(Debug, Clone)]
pub struct TxEntry {
    pub status: TxStatus,
    pub fee: Option<u64>,
}

pub struct Tx {
    pub txid: sha256d::Hash,
    pub entry: TxEntry,
}

#[derive(Debug)]
pub struct Utxo {
    pub status: TxStatus,
    pub txid: sha256d::Hash,
    pub vout: u32,
    pub value: u64,
}

impl Utxo {
    fn from_unspent(unspent: ListUnspentResult, tip_height: u32) -> Self {
        Self {
            status: TxStatus::new(unspent.confirmations as i32, tip_height),
            txid: unspent.txid,
            vout: unspent.vout,
            value: unspent.amount.into_inner() as u64,
        }
    }
}

impl AddrManager {
    pub fn new(rpc: Arc<RpcClient>) -> Self {
        AddrManager {
            rpc,
            index: RwLock::new(Index::new()),
        }
    }

    pub fn import(&self, address: &str, rescan: bool) -> Result<()> {
        let address = Address::from_str(address)?;
        self.rpc
            .import_address(&address, None, Some(rescan), None)?;
        // if rescan {self.update()?;}
        Ok(())
    }

    pub fn update(&self) -> Result<()> {
        self.update_listtransactions()?;

        Ok(())
    }

    fn update_listtransactions(&self) -> Result<()> {
        let tip_height = self.rpc.get_block_count()? as u32;
        let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

        let ltxs: Vec<ListTransactionsResult> = self.rpc.call(
            "listtransactions",
            &["*".into(), 100_000_000.into(), 0.into(), true.into()],
        )?;

        if tip_hash != self.rpc.get_best_block_hash()? {
            warn!("tip changed while fetching transactions, retrying...");
            return self.update();
        }

        let mut index = self.index.write().unwrap();
        for ltx in ltxs {
            index.process_ltx(ltx, tip_height);
        }

        // TODO: remove confliced txids from index

        Ok(())
    }

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Result<Vec<Tx>> {
        let index = self.index.read().unwrap();
        index
            .get_history(scripthash)
            .cloned()
            .unwrap_or_else(|| BTreeSet::new())
            .into_iter()
            .map(|hist| {
                Ok(Tx {
                    txid: hist.txid,
                    entry: index.get_tx(&hist.txid).or_err("missing tx")?.clone(),
                })
            })
            .collect::<Result<Vec<Tx>>>()
    }

    #[cfg(feature = "electrum")]
    pub fn status_hash(&self, scripthash: &sha256::Hash) -> Option<sha256::Hash> {
        let index = self.index.read().unwrap();
        index.get_history(scripthash).map(get_status_hash)
    }

    /// Get the unspent utxos owned by scripthash
    pub fn list_unspent(&self, scripthash: &sha256::Hash, min_conf: u32) -> Result<Vec<Utxo>> {
        let index = self.index.read().unwrap();
        let address = index.get_address(scripthash).or_err("unknown scripthash")?;

        let tip_height = self.rpc.get_block_count()? as u32;
        let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

        let unspents: Vec<ListUnspentResult> = self.rpc.call(
            "listunspent",
            &[
                min_conf.into(),
                9999999.into(),
                vec![address].into(),
                false.into(),
            ],
        )?;

        if tip_hash != self.rpc.get_best_block_hash()? {
            warn!("tip changed while fetching unspents, retrying...");
            return self.list_unspent(scripthash, min_conf);
        }

        Ok(unspents
            .into_iter()
            .map(|unspent| Utxo::from_unspent(unspent, tip_height))
            .filter(|utxo| utxo.status.is_viable())
            .collect())
    }

    /// Get the scripthash balance as a tuple of (confirmed_balance, unconfirmed_balance)
    pub fn get_balance(&self, scripthash: &sha256::Hash) -> Result<(u64, u64)> {
        let utxos = self.list_unspent(scripthash, 0)?;
        let (confirmed, unconfirmed): (Vec<Utxo>, Vec<Utxo>) = utxos
            .into_iter()
            .filter(|utxo| utxo.status.is_viable())
            .partition(|utxo| utxo.status.is_confirmed());

        Ok((
            confirmed.iter().map(|u| u.value).sum(),
            unconfirmed.iter().map(|u| u.value).sum(),
        ))
    }
}

impl Index {
    fn new() -> Self {
        Index {
            scripthashes: HashMap::new(),
            transactions: HashMap::new(),
        }
    }

    /// Process a transaction entry retrieved from "listtransactions"
    pub fn process_ltx(&mut self, ltx: ListTransactionsResult, tip_height: u32) {
        if !ltx.category.should_process() {
            return;
        }

        let status = TxStatus::new(ltx.confirmations, tip_height);

        if !status.is_viable() {
            return self.purge_tx(&ltx.txid);
        }

        let txentry = TxEntry {
            status: status,
            fee: parse_fee(ltx.fee),
        };
        self.index_tx_entry(&ltx.txid, txentry);

        let txhist = HistoryEntry {
            status,
            txid: ltx.txid,
        };
        self.index_address_history(&ltx.address, txhist);
    }

    /// Process a transaction entry retrieved from "gettransaction"
    pub fn process_gtx(&mut self, gtx: GetTransactionResult, tip_height: u32) {
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

            self.index_address_history(&detail.address, txhist.clone());
        }
    }

    /// Index transaction entry
    fn index_tx_entry(&mut self, txid: &sha256d::Hash, txentry: TxEntry) {
        assert!(
            txentry.status.is_viable(),
            "should not index non-viable tx entries"
        );

        // XXX re-read out from self.transactions if needed instead of cloning anyway?
        let new_status = txentry.status.clone();

        match self.transactions.insert(*txid, txentry) {
            Some(old_entry) => self.update_tx_status(txid, old_entry.status, new_status),
            None => info!("new tx: {:?}", txid),
        }
    }

    /// Index address history entry
    fn index_address_history(&mut self, address: &Address, txhist: HistoryEntry) {
        let scripthash = address_to_scripthash(address);

        let added = self.scripthashes
            .entry(&scripthash)
            .or_insert_with(|| ScriptEntry {
                address: address.to_string(),
                history: BTreeSet::new(),
            })
            .history
            .insert(txhist);

        if added {
            info!("new history entry for {:?}", scripthash)
        }
    }

    /// Update the scripthash history index to reflect the new tx status
    fn update_tx_status(
        &mut self,
        txid: &sha256d::Hash,
        old_status: TxStatus,
        new_status: TxStatus,
    ) {
        if old_status == new_status {
            return;
        }

        info!("transition tx {:?} status: {:?} -> {:?}", txid, old_status, new_status);

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

    fn purge_tx(&mut self, txid: &sha256d::Hash) {
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
    pub fn get_address(&self, scripthash: &sha256::Hash) -> Option<&str> {
        self.scripthashes
            .get(scripthash)
            .map(|x| x.address.as_str())
    }

    pub fn get_tx(&self, txid: &sha256d::Hash) -> Option<&TxEntry> {
        self.transactions.get(txid)
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Copy)]
pub enum TxStatus {
    Conflicted,     // aka double spent
    Unconfirmed,    // (fee)
    Confirmed(u32), // (height)
}

impl Ord for TxStatus {
    fn cmp(&self, other: &TxStatus) -> Ordering {
        match self {
            TxStatus::Confirmed(height) => match other {
                TxStatus::Confirmed(other_height) => height.cmp(other_height),
                TxStatus::Unconfirmed | TxStatus::Conflicted => Ordering::Greater,
            },
            TxStatus::Unconfirmed => match other {
                TxStatus::Confirmed(_) => Ordering::Less,
                TxStatus::Unconfirmed => Ordering::Equal,
                TxStatus::Conflicted => Ordering::Greater,
            },
            TxStatus::Conflicted => match other {
                TxStatus::Confirmed(_) | TxStatus::Unconfirmed => Ordering::Less,
                TxStatus::Conflicted => Ordering::Equal,
            },
        }
    }
}

impl PartialOrd for TxStatus {
    fn partial_cmp(&self, other: &TxStatus) -> Option<Ordering> {
        Some(self.cmp(other))
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

impl TxStatus {
    fn new(confirmations: i32, tip_height: u32) -> Self {
        if confirmations > 0 {
            TxStatus::Confirmed(tip_height - (confirmations as u32) + 1)
        } else if confirmations == 0 {
            TxStatus::Unconfirmed
        } else {
            // negative confirmations indicate the tx conflicts with the best chain (aka was double-spent)
            TxStatus::Conflicted
        }
    }

    // height suitable for the electrum protocol
    // TODO -1 to indicate unconfirmed tx with unconfirmed parents
    pub fn electrum_height(&self) -> u32 {
        match self {
            TxStatus::Confirmed(height) => *height,
            TxStatus::Unconfirmed => 0,
            TxStatus::Conflicted => {
                unreachable!("electrum_height() should not be called on conflicted txs")
            }
        }
    }

    fn is_viable(&self) -> bool {
        match self {
            TxStatus::Confirmed(_) | TxStatus::Unconfirmed => true,
            TxStatus::Conflicted => false,
        }
    }

    pub fn is_confirmed(&self) -> bool {
        match self {
            TxStatus::Confirmed(_) => true,
            TxStatus::Unconfirmed | TxStatus::Conflicted => false,
        }
    }

    pub fn is_unconfirmed(&self) -> bool {
        match self {
            TxStatus::Unconfirmed => true,
            TxStatus::Confirmed(_) | TxStatus::Conflicted => false,
        }
    }
}

// convert from a negative float to a positive satoshi amount
fn parse_fee(fee: Option<f64>) -> Option<u64> {
    fee.map(|fee| (fee * -1.0 * 100_000_000.0) as u64)
}
