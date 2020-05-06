use std::sync::{Arc, RwLock};

use bitcoin::{BlockHash, Txid};
use bitcoin_hashes::hex::ToHex;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use serde_json::Value;

use crate::error::{OptionExt, Result};
use crate::indexer::{Indexer, Tx};
use crate::types::{ScriptHash, Utxo};

#[cfg(feature = "electrum")]
use crate::types::StatusHash;

pub struct Query {
    rpc: Arc<RpcClient>,
    indexer: Arc<RwLock<Indexer>>,
}

impl Query {
    pub fn new(rpc: Arc<RpcClient>, indexer: Arc<RwLock<Indexer>>) -> Self {
        Query { rpc, indexer }
    }

    pub fn get_tip(&self) -> Result<(u32, BlockHash)> {
        let tip_height = self.get_tip_height()?;
        let tip_hash = self.get_block_hash(tip_height)?;
        Ok((tip_height, tip_hash))
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

    pub fn get_history(&self, scripthash: &ScriptHash) -> Result<Vec<Tx>> {
        Ok(self.indexer.read().unwrap().get_history(scripthash)?)
    }

    pub fn list_unspent(&self, scripthash: &ScriptHash, min_conf: usize) -> Result<Vec<Utxo>> {
        Ok(self
            .indexer
            .read()
            .unwrap()
            .list_unspent(scripthash, min_conf)?)
    }

    #[cfg(feature = "electrum")]
    pub fn status_hash(&self, scripthash: &ScriptHash) -> Option<StatusHash> {
        self.indexer.read().unwrap().status_hash(scripthash)
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

    pub fn get_transaction_hex(&self, txid: &Txid) -> Result<String> {
        Ok(self
            .rpc
            .call("getrawtransaction", &[txid.to_hex().into(), false.into()])?)
    }

    pub fn get_transaction_decoded(&self, txid: &Txid) -> Result<Value> {
        Ok(self
            .rpc
            .call("getrawtransaction", &[txid.to_hex().into(), true.into()])?)
    }

    pub fn broadcast(&self, tx_hex: &str) -> Result<Txid> {
        Ok(self.rpc.send_raw_transaction(tx_hex)?)
    }

    pub fn get_raw_mempool(&self) -> Result<Value> {
        Ok(self.rpc.call("getrawmempool", &[json!(true)])?)
    }
}
