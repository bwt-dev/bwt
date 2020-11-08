use serde::{de, Serialize};
use std::fmt::{self, Formatter};

use bitcoincore_rpc::json::ImportMultiRescanSince;
use bitcoincore_rpc::{Client, Result as RpcResult, RpcApi};

// Extensions for rust-bitcoincore-rpc

pub trait RpcApiExt: RpcApi {
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

    fn get_mempool_info(&self) -> RpcResult<GetMempoolInfoResult> {
        self.call("getmempoolinfo", &[])
    }
}

impl RpcApiExt for Client {}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
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

#[derive(Clone, PartialEq, Eq, Copy, Debug, Serialize)]
pub enum RescanSince {
    Now,
    Timestamp(u64),
}

impl Into<ImportMultiRescanSince> for &RescanSince {
    fn into(self) -> ImportMultiRescanSince {
        match self {
            RescanSince::Now => ImportMultiRescanSince::Now,
            RescanSince::Timestamp(t) => ImportMultiRescanSince::Timestamp(*t),
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
                write!(formatter, "unix timestamp or 'now'")
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
                    Err(de::Error::custom(format!(
                        "invalid str '{}', expecting 'now' or unix timestamp",
                        value
                    )))
                }
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
