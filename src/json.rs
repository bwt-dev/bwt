use bitcoin::Address;
use bitcoin_hashes::sha256d;
use bitcoincore_rpc::json::{
    serde_hex, GetTransactionResultDetail, GetTransactionResultDetailCategory as RpcCategory,
};

// not available in bitcoincore_json
#[derive(Deserialize, Debug)]
pub struct ListTransactionsResult {
    pub address: Address,
    pub txid: sha256d::Hash,
    pub category: TxCategory,
    pub confirmations: i32,
    pub blockhash: Option<sha256d::Hash>,
    pub fee: Option<f64>,

    #[serde(default)]
    pub label: String,
}

// available in bitcoincore_json, but blockhash is mandatory -- TODO upstream a fix
#[derive(Deserialize, Debug)]
pub struct GetTransactionResult {
    pub fee: Option<f64>,
    pub confirmations: i32,
    pub blockhash: Option<sha256d::Hash>,
    pub txid: sha256d::Hash,
    pub details: Vec<GetTransactionResultDetail>,
    #[serde(with = "serde_hex")]
    pub hex: Vec<u8>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TxCategory {
    Send,
    Receive,
    Generate,
    Immature,
    Orphan,
}

impl TxCategory {
    // we don't deal with mining related transactions
    pub fn should_process(&self) -> bool {
        match self {
            TxCategory::Send | TxCategory::Receive => true,
            TxCategory::Generate | TxCategory::Immature | TxCategory::Orphan => false,
        }
    }
}

impl From<RpcCategory> for TxCategory {
    fn from(category: RpcCategory) -> Self {
        match category {
            RpcCategory::Send => TxCategory::Send,
            RpcCategory::Receive => TxCategory::Receive,
        }
    }
}
