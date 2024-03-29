use serde::{de, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Formatter};

use bitcoin::Address;
use bitcoincore_rpc::json::{self, ImportMultiRescanSince};
use bitcoincore_rpc::{self as rpc, jsonrpc, Client, Result as RpcResult, RpcApi};

#[cfg(feature = "proxy")]
use super::jsonrpc_proxy::SimpleHttpTransport;
#[cfg(not(feature = "proxy"))]
use jsonrpc::simple_http::SimpleHttpTransport;

// Extensions for rust-bitcoincore-rpc

pub const RPC_MISC_ERROR: i32 = -1;
pub const RPC_INVALID_PARAMETER: i32 = -8;
pub const RPC_WALLET_ERROR: i32 = -4;
pub const RPC_INVALID_ADDRESS_OR_KEY: i32 = -5;
pub const RPC_WALLET_INVALID_LABEL_NAME: i32 = -11;
pub const RPC_WALLET_NOT_FOUND: i32 = -18;
pub const RPC_IN_WARMUP: i32 = -28;
pub const RPC_METHOD_NOT_FOUND: i32 = -32601;

pub trait RpcApiExt: RpcApi {
    fn list_labels(&self) -> RpcResult<Vec<String>> {
        self.call("listlabels", &[])
    }

    fn get_addresses_by_label(&self, label: &str) -> RpcResult<HashMap<Address, AddressEntry>> {
        match self.call("getaddressesbylabel", &[json!(label)]) {
            Ok(x) => Ok(x),
            // "No addresses with label ..."
            Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(e)))
                if e.code == RPC_WALLET_INVALID_LABEL_NAME =>
            {
                Ok(HashMap::new())
            }
            Err(e) => Err(e),
        }
    }

    // Only supports the fields we're interested in (so not currently upstremable)
    fn get_block_stats(&self, blockhash: &bitcoin::BlockHash) -> RpcResult<GetBlockStatsResult> {
        let fields = (
            "height",
            "time",
            "total_size",
            "total_weight",
            "txs",
            "totalfee",
            "avgfeerate",
            "feerate_percentiles",
        );
        self.call("getblockstats", &[json!(blockhash), json!(fields)])
    }

    // Retrieve a mempool entry, returning an Ok(None) if it doesn't exists
    fn get_mempool_entry_opt(
        &self,
        txid: &bitcoin::Txid,
    ) -> RpcResult<Option<json::GetMempoolEntryResult>> {
        match self.get_mempool_entry(txid) {
            Ok(entry) => Ok(Some(entry)),
            // "Transaction not in mempool" error. Not sure why it uses that code..
            Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(e)))
                if e.code == RPC_INVALID_ADDRESS_OR_KEY =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    // Only supports the fields we're interested in (so not currently upstremable)
    fn get_mempool_info(&self) -> RpcResult<GetMempoolInfoResult> {
        self.call("getmempoolinfo", &[])
    }

    // listsinceblock with the 'wallet_conflicts' field, pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/161
    fn list_since_block_(
        &self,
        blockhash: Option<&bitcoin::BlockHash>,
    ) -> RpcResult<ListSinceBlockResult> {
        self.call(
            "listsinceblock",
            &[json!(blockhash), 1.into(), true.into(), true.into()],
        )
    }

    // Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/174
    fn create_wallet_(&self, wallet_name: &str) -> RpcResult<json::LoadWalletResult> {
        // Create with disable_private_keys=true, blank=true, descriptors=false
        self.call(
            "createwallet",
            &[
                json!(wallet_name),
                true.into(),
                true.into(),
                json!(""),
                false.into(),
                false.into(),
                json!(null),
            ],
        )
    }

    fn prune_blockchain(&self, until: u64) -> RpcResult<u64> {
        self.call("pruneblockchain", &[json!(until)])
    }
}

impl RpcApiExt for Client {}

pub fn create_rpc_client(config: &crate::Config) -> Result<Client, crate::Error> {
    let mut builder = SimpleHttpTransport::builder().url(&config.bitcoind_url())?;

    if let (Some(user), pass) = get_user_pass(config.bitcoind_auth()?)? {
        builder = builder.auth(user, pass);
    }

    if let Some(timeout) = config.bitcoind_timeout {
        builder = builder.timeout(timeout);
    }

    #[cfg(feature = "proxy")]
    if let Some(proxy_addr) = &config.bitcoind_proxy {
        builder = builder.proxy(proxy_addr)?;
    }

    Ok(Client::from_jsonrpc(jsonrpc::Client::with_transport(
        builder.build(),
    )))
}

// Copied from rust-bitcoincore-rpc where it is private, pending
// https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/205
fn get_user_pass(auth: rpc::Auth) -> rpc::Result<(Option<String>, Option<String>)> {
    use rpc::{Auth, Error};
    use std::fs::File;
    use std::io::Read;
    match auth {
        Auth::None => Ok((None, None)),
        Auth::UserPass(u, p) => Ok((Some(u), Some(p))),
        Auth::CookieFile(path) => {
            let mut file = File::open(path)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            let mut split = contents.splitn(2, ":");
            Ok((
                Some(split.next().ok_or(Error::InvalidCookieFile)?.into()),
                Some(split.next().ok_or(Error::InvalidCookieFile)?.into()),
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AddressEntry {
    pub purpose: json::GetAddressInfoResultLabelPurpose,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize, Default)]
pub struct GetBlockStatsResult {
    pub height: u64,
    pub time: u64,
    pub txs: u64,
    pub total_weight: u64,
    pub total_size: u64,
    #[serde(rename = "totalfee", with = "bitcoin::util::amount::serde::as_sat")]
    pub total_fee: bitcoin::Amount,
    #[serde(rename = "avgfeerate")]
    pub avg_fee_rate: u64,
    pub feerate_percentiles: (u64, u64, u64, u64, u64),
}

// Used to make a GetBlockStatsResult representation of the genesis block
// (getblockstats on it fails with "Can't read undo data from disk")
impl From<json::GetBlockHeaderResult> for GetBlockStatsResult {
    fn from(header: json::GetBlockHeaderResult) -> Self {
        Self {
            height: header.height as u64,
            time: header.time as u64,
            txs: header.n_tx as u64,
            ..Default::default()
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetMempoolInfoResult {
    pub size: u64,
    pub bytes: u64,
    #[serde(
        rename = "mempoolminfee",
        with = "bitcoin::util::amount::serde::as_btc"
    )]
    pub mempool_min_fee: bitcoin::Amount,
}

// Wrap rust-bitcoincore-rpc's RescanSince to enable deserialization
// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/150
// XXX The PR does not include null handling

#[derive(Clone, PartialEq, Eq, Copy, Debug, Serialize)]
#[serde(into = "ImportMultiRescanSince")]
pub enum RescanSince {
    Now,
    Timestamp(u64),
}

impl Into<ImportMultiRescanSince> for RescanSince {
    fn into(self) -> ImportMultiRescanSince {
        match self {
            RescanSince::Now => ImportMultiRescanSince::Now,
            RescanSince::Timestamp(t) => ImportMultiRescanSince::Timestamp(t),
        }
    }
}

impl<'de> serde::Deserialize<'de> for RescanSince {
    fn deserialize<D>(deserializer: D) -> Result<RescanSince, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = RescanSince;

            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                write!(formatter, "unix timestamp or 'now'/null")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RescanSince::Timestamp(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value == "now" {
                    Ok(RescanSince::Now)
                } else {
                    Err(de::Error::invalid_value(de::Unexpected::Str(value), &self))
                }
            }

            // handle nulls
            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RescanSince::Now)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/161
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ListTransactionResult {
    #[serde(flatten)]
    pub info: WalletTxInfo,
    #[serde(flatten)]
    pub detail: json::GetTransactionResultDetail,

    pub trusted: Option<bool>,
    pub comment: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ListSinceBlockResult {
    pub transactions: Vec<ListTransactionResult>,
    #[serde(default)]
    pub removed: Vec<ListTransactionResult>,
    pub lastblock: bitcoin::BlockHash,
}
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct WalletTxInfo {
    pub confirmations: i32,
    pub blockhash: Option<bitcoin::BlockHash>,
    pub blockindex: Option<usize>,
    pub blocktime: Option<u64>,
    pub blockheight: Option<u32>,
    pub txid: bitcoin::Txid,
    pub time: u64,
    pub timereceived: u64,
    #[serde(rename = "bip125-replaceable")]
    pub bip125_replaceable: json::Bip125Replaceable,
    #[serde(rename = "walletconflicts")]
    pub wallet_conflicts: Vec<bitcoin::Txid>,
}
