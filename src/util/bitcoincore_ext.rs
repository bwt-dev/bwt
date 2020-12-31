use serde::{de, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Formatter};
use std::{sync::mpsc, thread, time};

use bitcoin::Address;
use bitcoincore_rpc::json::{self, ImportMultiRescanSince, ScanningDetails};
use bitcoincore_rpc::{self as rpc, Client, Result as RpcResult, RpcApi};

// Extensions for rust-bitcoincore-rpc

pub trait RpcApiExt: RpcApi {
    fn list_labels(&self) -> RpcResult<Vec<String>> {
        self.call("listlabels", &[])
    }

    fn get_addresses_by_label(&self, label: &str) -> RpcResult<HashMap<Address, AddressEntry>> {
        match self.call("getaddressesbylabel", &[json!(label)]) {
            Ok(x) => Ok(x),
            // "No addresses with label ..."
            Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(e))) if e.code == -11 => {
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

    // Only supports the fields we're interested in (so not currently upstremable)
    fn get_mempool_info(&self) -> RpcResult<GetMempoolInfoResult> {
        self.call("getmempoolinfo", &[])
    }

    fn wait_blockchain_sync(
        &self,
        progress_tx: Option<mpsc::Sender<Progress>>,
        interval: time::Duration,
    ) -> RpcResult<json::GetBlockchainInfoResult> {
        Ok(loop {
            let info = self.get_blockchain_info()?;

            if info.blocks == info.headers
                && (!info.initial_block_download || info.chain == "regtest")
            {
                if let Some(ref progress_tx) = progress_tx {
                    let progress = Progress::Sync {
                        progress_n: 1.0,
                        tip: info.median_time,
                    };
                    progress_tx.send(progress).ok();
                }
                break info;
            }

            if let Some(ref progress_tx) = progress_tx {
                let progress = Progress::Sync {
                    progress_n: info.verification_progress as f32,
                    tip: info.median_time,
                };
                if progress_tx.send(progress).is_err() {
                    break info;
                }
            } else {
                info!(target: "bwt",
                    "waiting for bitcoind to sync [{}/{} blocks, progress={:.1}%]",
                    info.blocks, info.headers, info.verification_progress * 100.0
                );
            }
            thread::sleep(interval);
        })
    }

    fn wait_wallet_scan(
        &self,
        progress_tx: Option<mpsc::Sender<Progress>>,
        shutdown_rx: Option<mpsc::Receiver<()>>,
        interval: time::Duration,
    ) -> RpcResult<json::GetWalletInfoResult> {
        // Stop if the shutdown signal was received or if the channel was disconnected
        let should_shutdown = || {
            shutdown_rx
                .as_ref()
                .map_or(false, |rx| rx.try_recv() != Err(mpsc::TryRecvError::Empty))
        };

        Ok(loop {
            let info = self.get_wallet_info()?;
            if should_shutdown() {
                break info;
            }
            match info.scanning {
                None => {
                    warn!("Your bitcoin node does not report the `scanning` status in `getwalletinfo`. It is recommended to upgrade to Bitcoin Core v0.19+ to enable this.");
                    warn!("This is needed for bwt to wait for scanning to finish before starting up. Starting bwt while the node is scanning may lead to unexpected results. Continuing anyway...");
                    break info;
                }
                Some(ScanningDetails::NotScanning(_)) => {
                    if let Some(ref progress_tx) = progress_tx {
                        let progress = Progress::Scan {
                            progress_n: 1.0,
                            eta: 0,
                        };
                        if progress_tx.send(progress).is_err() {
                            break info;
                        }
                    }
                    // Stop as soon as scanning is completed if no explicit shutdown_rx was given,
                    // or continue until the shutdown signal is received if it was.
                    if shutdown_rx.is_none() {
                        break info;
                    }
                }
                Some(ScanningDetails::Scanning {
                    progress: progress_n,
                    duration,
                }) => {
                    let eta = if progress_n > 0.0 {
                        (duration as f32 / progress_n) as u64 - duration as u64
                    } else {
                        0
                    };

                    if let Some(ref progress_tx) = progress_tx {
                        let progress = Progress::Scan { progress_n, eta };
                        if progress_tx.send(progress).is_err() {
                            break info;
                        }
                    } else {
                        info!(target: "bwt",
                            "waiting for bitcoind to finish scanning [done {:.1}%, running for {}m, eta {}m]",
                            progress_n * 100.0, duration / 60, eta / 60
                        );
                    }
                }
            }
            thread::sleep(interval);
            if should_shutdown() {
                break info;
            }
        })
    }
}

impl RpcApiExt for Client {}

#[derive(Debug, Copy, Clone)]
pub enum Progress {
    Sync { progress_n: f32, tip: u64 },
    Scan { progress_n: f32, eta: u64 },
}

#[derive(Debug, Deserialize)]
pub struct AddressEntry {
    pub purpose: json::GetAddressInfoResultLabelPurpose,
}

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
