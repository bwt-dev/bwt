use std::sync::{Arc, RwLock};

use bitcoin_hashes::{hex::ToHex, sha256, sha256d};
use bitcoincore_rpc::json::EstimateSmartFeeResult;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use serde_json::Value;

use crate::addrman::AddrManager;
use crate::error::{OptionExt, Result};
use crate::types::{Tx, Utxo};

pub struct Query {
    rpc: Arc<RpcClient>,
    addrman: Arc<RwLock<AddrManager>>,
}

impl Query {
    pub fn new(rpc: Arc<RpcClient>, addrman: Arc<RwLock<AddrManager>>) -> Self {
        Query { rpc, addrman }
    }

    pub fn get_tip(&self) -> Result<(u32, sha256d::Hash)> {
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

    pub fn get_header_by_hash(&self, blockhash: &sha256d::Hash) -> Result<String> {
        Ok(self
            .rpc
            .call("getblockheader", &[json!(blockhash), false.into()])?)
    }

    pub fn get_block_hash(&self, height: u32) -> Result<sha256d::Hash> {
        Ok(self.rpc.get_block_hash(height as u64)?)
    }

    pub fn get_block_txids(&self, blockhash: &sha256d::Hash) -> Result<Vec<sha256d::Hash>> {
        let info = self.rpc.get_block_info(blockhash)?;
        Ok(info.tx)
    }

    pub fn estimate_fee(&self, target: u16) -> Result<Option<f32>> {
        let feerate = self
            .rpc
            .call::<EstimateSmartFeeResult>("estimatesmartfee", &[target.into()])?
            .feerate
            .and_then(|rate| rate.as_f64())
            // from BTC/kB to sat/b
            .map(|rate| (rate * 100_000f64) as f32);
        Ok(feerate)
    }

    pub fn relay_fee(&self) -> Result<f32> {
        let feerate = self.rpc.call::<Value>("getmempoolinfo", &[])?["minrelaytxfee"]
            .as_f64()
            .or_err("invalid getmempoolinfo reply")?;

        // from BTC/kB to sat/b
        Ok((feerate * 100_000f64) as f32)
    }

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Result<Vec<Tx>> {
        Ok(self.addrman.read().unwrap().get_history(scripthash)?)
    }

    pub fn list_unspent(&self, scripthash: &sha256::Hash, min_conf: u32) -> Result<Vec<Utxo>> {
        Ok(self
            .addrman
            .read()
            .unwrap()
            .list_unspent(scripthash, min_conf)?)
    }

    #[cfg(feature = "electrum")]
    pub fn status_hash(&self, scripthash: &sha256::Hash) -> Option<sha256::Hash> {
        self.addrman.read().unwrap().status_hash(scripthash)
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

    pub fn get_transaction_hex(&self, txid: &sha256d::Hash) -> Result<String> {
        Ok(self
            .rpc
            .call("getrawtransaction", &[txid.to_hex().into(), false.into()])?)
    }

    pub fn get_transaction_decoded(&self, txid: &sha256d::Hash) -> Result<Value> {
        Ok(self
            .rpc
            .call("getrawtransaction", &[txid.to_hex().into(), true.into()])?)
    }

    pub fn broadcast(&self, tx_hex: &str) -> Result<sha256d::Hash> {
        Ok(self.rpc.send_raw_transaction(tx_hex)?)
    }

    pub fn get_raw_mempool(&self) -> Result<Value> {
        Ok(self.rpc.call("getrawmempool", &[json!(true)])?)
    }
}
