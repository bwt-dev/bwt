use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use bitcoin::Address;
use bitcoin_hashes::{sha256, sha256d};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::Result;
use crate::json::{GetTransactionResult, ListTransactionsResult, TxCategory};
use crate::util::address_to_scripthash;

// pub type FullHash = [u8; 32];

pub struct AddrManager {
    rpc: Arc<RpcClient>,
    index: RwLock<Index>,
}

#[derive(Debug)]
struct Index {
    scripthashes: HashMap<sha256::Hash, BTreeSet<TxHist>>,
    transactions: HashMap<sha256d::Hash, TxEntry>,
    //tx_scripthashes: HashMap<sha256d::Hash, HashSet<sha256d::Hash>>,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub struct TxHist {
    pub height: u32,
    pub txid: sha256d::Hash,
}

#[derive(Debug)]
struct TxEntry {
    status: TxStatus,
    fee: Option<f64>,
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

    pub fn query(&self, scripthash: &sha256::Hash) -> BTreeSet<TxHist> {
        let index = self.index.read().unwrap();
        index
            .query(scripthash)
            .cloned()
            .unwrap_or_else(|| BTreeSet::new())
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

        let status = TxStatus::from_confirmations(ltx.confirmations, &ltx.blockhash, tip_height);

        if status == TxStatus::Conflicted {
            return self.purge_tx(&ltx.txid);
        }

        let height = status.sorting_height();

        let txentry = TxEntry {
            status,
            fee: ltx.fee,
        };
        self.index_tx_entry(&ltx.txid, txentry);

        let txhist = TxHist {
            height,
            txid: ltx.txid,
        };
        self.index_address_history(&ltx.address, txhist);
    }

    /// Process a transaction entry retrieved from "gettransaction"
    pub fn process_gtx(&mut self, gtx: GetTransactionResult, tip_height: u32) {
        let status = TxStatus::from_confirmations(gtx.confirmations, &gtx.blockhash, tip_height);

        if status == TxStatus::Conflicted {
            return self.purge_tx(&gtx.txid);
        }

        let height = status.sorting_height();

        let txentry = TxEntry {
            status,
            fee: gtx.fee,
        };
        self.index_tx_entry(&gtx.txid, txentry);

        let txhist = TxHist {
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
            txentry.status != TxStatus::Conflicted,
            "should not index conflicted tx entry"
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
    fn index_address_history(&mut self, address: &Address, txhist: TxHist) {
        let scripthash = address_to_scripthash(address);

        debug!(
            "index address history: address {:?} / scripthash {:?} --> {:?}",
            address, scripthash, txhist
        );

        self.scripthashes
            .entry(scripthash)
            .or_insert_with(|| BTreeSet::new())
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

        let old_txhist = TxHist {
            height: old_height,
            txid: *txid,
        };
        let new_txhist = TxHist {
            height: new_height,
            txid: *txid,
        };

        // TODO optimize, keep txid->scripthashes map
        for (_scripthash, txs) in &mut self.scripthashes {
            if txs.remove(&old_txhist) {
                txs.insert(new_txhist.clone());
            }
        }
    }

    fn purge_tx(&mut self, txid: &sha256d::Hash) {
        debug!("purge_tx {:?}", txid);
        if let Some(old_entry) = self.transactions.remove(txid) {
            let old_txhist = TxHist {
                height: old_entry.status.sorting_height(),
                txid: *txid,
            };
            for (_scripthash, txs) in &mut self.scripthashes {
                txs.remove(&old_txhist);
            }
        }
    }

    pub fn query(&self, scripthash: &sha256::Hash) -> Option<&BTreeSet<TxHist>> {
        self.scripthashes.get(scripthash)
    }
}

// TODO verify ordering
#[derive(Clone, PartialEq, Debug)]
enum TxStatus {
    Conflicted, // aka double spent
    Unconfirmed,
    Confirmed(u32, sha256d::Hash),
}

impl TxStatus {
    fn from_confirmations(
        confirmations: i32,
        blockhash: &Option<sha256d::Hash>,
        tip_height: u32,
    ) -> Self {
        if confirmations > 0 {
            TxStatus::Confirmed(
                tip_height - (confirmations as u32) + 1,
                blockhash.expect("missing blockhash for confirmed tx"),
            )
        } else if confirmations == 0 {
            TxStatus::Unconfirmed
        } else {
            // negative confirmations indicate the tx conflicts with the best chain (aka was double-spent)
            TxStatus::Conflicted
        }
    }

    fn sorting_height(&self) -> u32 {
        match self {
            TxStatus::Confirmed(height, _) => *height,
            TxStatus::Unconfirmed => std::u32::MAX,
            TxStatus::Conflicted => {
                panic!("sorting_height() should not be called on conflicted txs")
            }
        }
    }
}
