use std::sync::Arc;

use bitcoin_hashes::{hex::ToHex, sha256, sha256d};
use bitcoincore_rpc::json::EstimateSmartFeeResult;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use serde_json::Value;

use crate::addrman::{AddrManager, TxVal, Utxo};
use crate::error::Result;

pub struct Query {
    rpc: Arc<RpcClient>,
    addrman: Arc<AddrManager>,
}

impl Query {
    pub fn new(rpc: Arc<RpcClient>, addrman: Arc<AddrManager>) -> Self {
        Query { rpc, addrman }
    }

    pub fn get_tip(&self) -> Result<(u32, sha256d::Hash)> {
        let tip_height = self.rpc.get_block_count()? as u32;
        let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;
        Ok((tip_height, tip_hash))
    }

    pub fn get_header(&self, height: u32) -> Result<String> {
        let blockhash = self.rpc.get_block_hash(height as u64)?;
        self.get_header_by_hash(blockhash)
    }

    pub fn get_headers(&self, heights: &[u32]) -> Result<Vec<String>> {
        Ok(heights
            .iter()
            .map(|h| self.get_header(*h))
            .collect::<Result<Vec<String>>>()?)
    }

    pub fn get_header_by_hash(&self, blockhash: sha256d::Hash) -> Result<String> {
        Ok(self
            .rpc
            .call("getblockheader", &[json!(blockhash), false.into()])?)
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

    /*
        // XXX sat/byte or btc/kb?
        pub fn estimate_fee(&self, target: u32) -> Result<f64> {
        }

        pub fn relay_fee(&self) -> Result<f64> {
        }

        pub fn get_balance(&self, scripthash: &sha256::Hash) -> Result<(f64, f64)> {
        }
    */

    pub fn get_history(&self, scripthash: &sha256::Hash) -> Result<Vec<TxVal>> {
        Ok(self.addrman.get_history(scripthash))
    }

    pub fn list_unspent(&self, scripthash: &sha256::Hash, min_conf: u32) -> Result<Vec<Utxo>> {
        Ok(self.addrman.list_unspent(scripthash, min_conf)?)
    }

    pub fn get_balance(&self, scripthash: &sha256::Hash) -> Result<(u64, u64)> {
        Ok(self.addrman.get_balance(scripthash)?)
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

    /*

    pub fn get_transaction_merkle_proof(&self, txid: &sha256d::Hash) -> Result<MerkleProof> {
    }

    pub fn get_transaction_from_pos(&self, height: u32, position: u32) -> Result<sha256d::Hash> {
    }

    pub fn get_fee_histogram(&self) -> Result<Vec<(f32, u32)>> {
    }*/
}
