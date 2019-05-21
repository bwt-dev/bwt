use std::collections::BTreeSet;
use std::sync::Arc;

use bitcoin_hashes::sha256;
use bitcoincore_rpc::{json::EstimateSmartFeeResult, Client as RpcClient, RpcApi};

use crate::addrman::{AddrManager, TxHist};
use crate::error::Result;

pub struct Query {
    rpc: Arc<RpcClient>,
    addrman: Arc<AddrManager>,
}

impl Query {
    pub fn new(rpc: Arc<RpcClient>, addrman: Arc<AddrManager>) -> Self {
        Query { rpc, addrman }
    }

    pub fn get_header(&self, height: u32) -> Result<String> {
        let blockhash = self.rpc.get_block_hash(height as u64)?;
        let blockhex = self
            .rpc
            .call("getblockheader", &[json!(blockhash), false.into()])?;
        Ok(blockhex)
    }

    pub fn get_headers(&self, heights: &[u32]) -> Result<Vec<String>> {
        Ok(heights
            .iter()
            .map(|h| self.get_header(*h))
            .collect::<Result<Vec<String>>>()?)
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

    pub fn query(&self, scripthash: &sha256::Hash) -> Result<BTreeSet<TxHist>> {
        Ok(self.addrman.query(scripthash))
    }

    /*
    pub fn list_unspent(&self, scripthash: &sha256::Hash) -> Result<Vec<Utxo>> {
    }

    // XXX broadcast here?

    pub fn get_transaction&self, (txid: &sha256d::Hash) -> Result<String> {
    }

    pub fn get_transaction_decoded(&self, txid: &sha256d::Hash) -> Result<GetRawTransactionResult> {
    }

    pub fn get_transaction_merkle_proof(&self, txid: &sha256d::Hash) -> Result<MerkleProof> {
    }

    pub fn get_transaction_from_pos(&self, height: u32, position: u32) -> Result<sha256d::Hash> {
    }

    pub fn get_fee_histogram(&self) -> Result<Vec<(f32, u32)>> {
    }*/
}
