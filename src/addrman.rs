use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use bitcoin::Address;
use bitcoin_hashes::{sha256, sha256d};
use bitcoincore_rpc::{json::ListUnspentResult, Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::json::{GetTransactionResult, ListTransactionsResult, TxCategory};
use crate::util::address_to_scripthash;

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
    history: BTreeSet<ScriptHist>,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
struct ScriptHist {
    height: u32,
    txid: sha256d::Hash,
}

#[derive(Debug, Clone)]
pub struct TxEntry {
    pub status: TxStatus,
    pub fee: Option<u64>,
}

#[derive(Debug)]
pub struct TxVal(pub sha256d::Hash, pub TxEntry);

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
            status: TxStatus::from_confirmations(unspent.confirmations as i32, tip_height),
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

        debug!(
            "indexed: {:#?} {:#?}",
            index.scripthashes, index.transactions
        );

        // TODO: remove missing txids from index

        Ok(())
    }

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Vec<TxVal> {
        let index = self.index.read().unwrap();
        index
            .get_history(scripthash)
            .cloned()
            .unwrap_or_else(|| BTreeSet::new())
            .into_iter()
            .filter_map(|hist| {
                let entry = index.get_tx(&hist.txid)?.clone();
                Some(TxVal(hist.txid, entry))
            })
            .collect()
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
        debug!("index ltx: {:?}", ltx);

        if !ltx.category.should_process() {
            return;
        }

        let status = TxStatus::from_confirmations(ltx.confirmations, tip_height);

        if !status.is_viable() {
            return self.purge_tx(&ltx.txid);
        }

        let height = status.sorting_height();

        let txentry = TxEntry {
            status,
            fee: parse_fee(ltx.fee),
        };
        self.index_tx_entry(&ltx.txid, txentry);

        let txhist = ScriptHist {
            height,
            txid: ltx.txid,
        };
        self.index_address_history(&ltx.address, txhist);
    }

    /// Process a transaction entry retrieved from "gettransaction"
    pub fn process_gtx(&mut self, gtx: GetTransactionResult, tip_height: u32) {
        let status = TxStatus::from_confirmations(gtx.confirmations, tip_height);

        if !status.is_viable() {
            return self.purge_tx(&gtx.txid);
        }

        let height = status.sorting_height();

        let txentry = TxEntry {
            status,
            fee: parse_fee(gtx.fee),
        };
        self.index_tx_entry(&gtx.txid, txentry);

        let txhist = ScriptHist {
            height,
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
        debug!("index_tx_entry: {:?} {:?}", txid, txentry);

        assert!(
            txentry.status.is_viable(),
            "should not index non-viable tx entries"
        );

        let new_height = txentry.status.sorting_height();

        match self.transactions.insert(*txid, txentry) {
            Some(old_entry) => {
                let old_height = old_entry.status.sorting_height();
                self.update_tx_height(txid, old_height, new_height)
            }
            None => (),
        }
    }

    /// Index address history entry
    fn index_address_history(&mut self, address: &Address, txhist: ScriptHist) {
        let scripthash = address_to_scripthash(address);

        debug!(
            "index address history: address {:?} / scripthash {:?} --> {:?}",
            address, scripthash, txhist
        );

        self.scripthashes
            .entry(scripthash)
            .or_insert_with(|| ScriptEntry {
                address: address.to_string(),
                history: BTreeSet::new(),
            })
            .history
            .insert(txhist);
    }

    /*
    pub fn update_tx_status(&mut self, txid: &sha256d::Hash, new_status: TxStatus) -> Result<()> {
        let txentry = self.transactions.get_mut(txid).or_err("tx not found")?;
        if txentry.status != new_status {
            let old_status = txentry.status.clone();
            txentry.status = new_status.clone();
            self.update_tx_height(txid, &old_status, &new_status);
        }
        Ok(())
    }
    */

    /// Update the scripthash history index to reflect the new tx status
    fn update_tx_height(&mut self, txid: &sha256d::Hash, old_height: u32, new_height: u32) {
        if old_height == new_height {
            return;
        }

        let old_txhist = ScriptHist {
            height: old_height,
            txid: *txid,
        };
        let new_txhist = ScriptHist {
            height: new_height,
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
        debug!("purge_tx {:?}", txid);
        if let Some(old_entry) = self.transactions.remove(txid) {
            let old_txhist = ScriptHist {
                height: old_entry.status.sorting_height(),
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

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Option<&BTreeSet<ScriptHist>> {
        self.scripthashes.get(scripthash).map(|x| &x.history)
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

#[derive(Clone, PartialEq, Debug)]
pub enum TxStatus {
    Conflicted, // aka double spent
    Unconfirmed,
    Confirmed(u32 /*, sha256d::Hash */),
}

impl TxStatus {
    fn from_confirmations(confirmations: i32, tip_height: u32) -> Self {
        if confirmations > 0 {
            TxStatus::Confirmed(tip_height - (confirmations as u32) + 1)
        } else if confirmations == 0 {
            TxStatus::Unconfirmed
        } else {
            // negative confirmations indicate the tx conflicts with the best chain (aka was double-spent)
            TxStatus::Conflicted
        }
    }

    // height representation for index sorting
    fn sorting_height(&self) -> u32 {
        match self {
            TxStatus::Confirmed(height) => *height,
            TxStatus::Unconfirmed => std::u32::MAX,
            TxStatus::Conflicted => {
                panic!("sorting_height() should not be called on conflicted txs")
            }
        }
    }

    // height suitable for the electrum protocol
    pub fn elc_height(&self) -> u32 {
        match self {
            TxStatus::Confirmed(height) => *height,
            TxStatus::Unconfirmed => 0,
            TxStatus::Conflicted => panic!("elc_height() should not be called on conflicted txs"),
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
}

// convert from a negative float to a positive satoshi amount
fn parse_fee(fee: Option<f64>) -> Option<u64> {
    fee.map(|fee| (fee * -1.0 * 100_000_000.0) as u64)
}
