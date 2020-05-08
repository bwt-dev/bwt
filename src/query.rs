use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use bitcoin::{BlockHash, OutPoint, Txid};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::indexer::{FundingInfo, HistoryEntry, Indexer, ScriptInfo, TxEntry};
use crate::types::{BlockId, ScriptHash, TxStatus, Utxo};

#[cfg(feature = "track-spends")]
use crate::{indexer::SpendingInfo, types::TxInput};

pub struct Query {
    rpc: Arc<RpcClient>,
    indexer: Arc<RwLock<Indexer>>,
}

impl Query {
    pub fn new(rpc: Arc<RpcClient>, indexer: Arc<RwLock<Indexer>>) -> Self {
        Query { rpc, indexer }
    }

    pub fn get_tip(&self) -> Result<BlockId> {
        let tip_height = self.get_tip_height()?;
        let tip_hash = self.get_block_hash(tip_height)?;
        Ok(BlockId(tip_height, tip_hash))
    }

    pub fn get_tip_height(&self) -> Result<u32> {
        Ok(self.rpc.get_block_count()? as u32)
    }

    pub fn get_header(&self, height: u32) -> Result<String> {
        self.get_header_by_hash(&self.get_block_hash(height)?)
    }

    pub fn get_headers(&self, heights: &[u32]) -> Result<Vec<String>> {
        Ok(heights
            .iter()
            .map(|h| self.get_header(*h))
            .collect::<Result<Vec<String>>>()?)
    }

    pub fn get_header_by_hash(&self, blockhash: &BlockHash) -> Result<String> {
        Ok(self
            .rpc
            .call("getblockheader", &[json!(blockhash), false.into()])?)
    }

    pub fn get_block_hash(&self, height: u32) -> Result<BlockHash> {
        Ok(self.rpc.get_block_hash(height as u64)?)
    }

    pub fn get_block_txids(&self, blockhash: &BlockHash) -> Result<Vec<Txid>> {
        let info = self.rpc.get_block_info(blockhash)?;
        Ok(info.tx)
    }

    pub fn estimate_fee(&self, target: u16) -> Result<Option<f64>> {
        let feerate = self
            .rpc
            .estimate_smart_fee(target, None)?
            .fee_rate
            // from sat/kB to sat/b
            .map(|rate| (rate.as_sat() as f64 / 1000f64) as f64);
        Ok(feerate)
    }

    pub fn relay_fee(&self) -> Result<f64> {
        let feerate = self.rpc.call::<Value>("getmempoolinfo", &[])?["minrelaytxfee"]
            .as_f64()
            .or_err("invalid getmempoolinfo reply")?;

        // from BTC/kB to sat/b
        Ok((feerate * 100_000f64) as f64)
    }

    pub fn get_history(&self, scripthash: &ScriptHash) -> Vec<HistoryEntry> {
        self.indexer.read().unwrap().get_history(scripthash)
    }

    pub fn get_history_info(&self, scripthash: &ScriptHash) -> Vec<TxInfo> {
        self.with_history(scripthash, |txhist| self.get_tx_info(&txhist.txid).unwrap())
            .unwrap_or_else(|| vec![])
    }

    pub fn list_unspent(&self, scripthash: &ScriptHash, min_conf: usize) -> Result<Vec<Utxo>> {
        Ok(self
            .indexer
            .read()
            .unwrap()
            .list_unspent(scripthash, min_conf)?)
    }

    pub fn with_history<T>(
        &self,
        scripthash: &ScriptHash,
        f: impl Fn(&HistoryEntry) -> T,
    ) -> Option<Vec<T>> {
        self.indexer.read().unwrap().with_history(scripthash, f)
    }

    pub fn with_tx_entry<T>(&self, txid: &Txid, f: fn(&TxEntry) -> T) -> Option<T> {
        self.indexer.read().unwrap().with_tx_entry(txid, f)
    }

    pub fn get_script_info(&self, scripthash: &ScriptHash) -> Option<ScriptInfo> {
        self.indexer.read().unwrap().get_script_info(scripthash)
    }

    /// Get the scripthash balance as a tuple of (confirmed_balance, unconfirmed_balance)
    pub fn get_balance(&self, scripthash: &ScriptHash) -> Result<(u64, u64)> {
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

    pub fn get_tx_raw(&self, txid: &Txid) -> Result<Vec<u8>> {
        Ok(self.rpc.get_transaction(txid, Some(true))?.hex)
    }

    pub fn get_tx_json(&self, txid: &Txid) -> Result<Value> {
        let blockhash = self.indexer.read().unwrap().find_tx_blockhash(txid)?;

        Ok(self.rpc.call(
            "getrawtransaction",
            &[json!(txid), true.into(), json!(blockhash)],
        )?)
    }

    pub fn get_tx_entry(&self, txid: &Txid) -> Result<TxEntry> {
        Ok(self
            .indexer
            .read()
            .unwrap()
            .get_tx_entry(txid)
            .or_err("tx not found")?)
    }

    pub fn broadcast(&self, tx_hex: &str) -> Result<Txid> {
        Ok(self.rpc.send_raw_transaction(tx_hex)?)
    }

    pub fn get_raw_mempool(&self) -> Result<Value> {
        Ok(self.rpc.call("getrawmempool", &[json!(true)])?)
    }

    pub fn get_tx_info(&self, txid: &Txid) -> Option<TxInfo> {
        let index = self.indexer.read().unwrap();
        let tx_entry = index.get_tx_entry(txid)?;

        let funding = tx_entry
            .funding
            .iter()
            .map(|(vout, FundingInfo(scripthash, amount))| {
                TxInfoFunding {
                    vout: *vout,
                    script_info: index.get_script_info(scripthash).unwrap(), // must exists
                    #[cfg(feature = "track-spends")]
                    spent_by: index.lookup_txo_spend(&OutPoint::new(*txid, *vout)),
                    amount: *amount,
                }
            })
            .collect::<Vec<TxInfoFunding>>();

        #[cfg(feature = "track-spends")]
        let spending = tx_entry
            .spending
            .iter()
            .map(|(vin, SpendingInfo(scripthash, prevout, amount))| {
                TxInfoSpending {
                    vin: *vin,
                    script_info: index.get_script_info(scripthash).unwrap(), // must exists
                    amount: *amount,
                    prevout: *prevout,
                }
            })
            .collect::<Vec<TxInfoSpending>>();

        #[cfg(feature = "track-spends")]
        let balance = {
            let funding_sum = funding.iter().map(|f| f.amount).sum::<u64>();
            let spending_sum = spending.iter().map(|s| s.amount).sum::<u64>();
            funding_sum as i64 - spending_sum as i64
        };

        Some(TxInfo {
            txid: *txid,
            status: tx_entry.status,
            fee: tx_entry.fee,
            funding: funding,
            #[cfg(feature = "track-spends")]
            spending: spending,
            #[cfg(feature = "track-spends")]
            balance: balance,
        })
    }
}

#[derive(Serialize, Debug)]
pub struct TxInfo {
    txid: Txid,
    #[serde(flatten)]
    status: TxStatus,
    fee: Option<u64>,
    funding: Vec<TxInfoFunding>,
    #[cfg(feature = "track-spends")]
    spending: Vec<TxInfoSpending>,
    #[cfg(feature = "track-spends")]
    balance: i64,
}

#[derive(Serialize, Debug)]
struct TxInfoFunding {
    vout: u32,
    #[serde(flatten)]
    script_info: ScriptInfo, // scripthash, address & origin
    amount: u64,
    #[cfg(feature = "track-spends")]
    spent_by: Option<TxInput>,
}

#[derive(Serialize, Debug)]
struct TxInfoSpending {
    vin: u32,
    #[serde(flatten)]
    script_info: ScriptInfo, // scripthash, address & origin
    amount: u64,
    prevout: OutPoint,
}
