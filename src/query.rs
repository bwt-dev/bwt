use bitcoincore_rpc::Client as RpcClient;

use crate::error::Result;
use crate::addrmaan::AddrManager;

pub struct Query {
    addrman: Arc<AddrManager>,
    rpc: Arc<RpcClient>,
}

impl Query {
    fn new(addrman: Arc<AddrManager>, rpc: Arc<RpcClient>) -> Self {
        Query { addrman, rpc }
    }

    pub fn get_header(&self, height: u32) -> Result<String> {
        let blockhash = self.rpc.get_block_hash(height)?;
        let blockhex = self.call("getblockheader", &[json!(blockhash)?, false.into()])?;
        Ok(blockhex)
    }

    pub fn get_headers(&self, heights: &[u32]) -> Result<Vec<String>> {
        Ok(heights.iter().map(self.get_header).collect::<Result<Vec<String>>()?)
    }

    // XXX sat/byte or btc/kb?
    pub fn estimate_fee(&self, target: u32) -> Result<f64> {
    }

    pub fn relay_fee(&self) -> Result<f64> {
    }

    pub fn get_balance(&self, scripthash: &sha256::Hash) -> Result<(f64, f64)> {
    }

    // XXX confirmed, unconfirmed, or both
    pub fn get_history(&self, scripthash: &sha256::Hash) -> Result<Vec<TxHist>> {
    }

    pub fn list_unspent(&self, scripthash: &sha256::Hash) -> Result<Vec<Utxo>> {
    }

    // XXX broadcast here?

    pub fn get_transaction&self, (txid: &sha256d::Hash) -> Result<String> {
    }

    pub fn get_transaction_decoded(&self, txid: &sha256d::Hash) -> Result<GetRawTransactionResult> {
    }

    pub fn get_transaction_merkle(&self, txid: &sha256d::Hash) -> Result<MerkleProof> {
    }

    pub fn get_transaction_from_pos(&self, height: u32, position: u32) -> Result<sha256d::Hash> {
    }

    pub fn get_fee_histogram(&self) -> Result<Vec<(f32, u32)>> {
    }
}
