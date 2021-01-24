use std::{sync::mpsc, thread, time};

use bitcoincore_rpc::json::{self, ScanningDetails};
use bitcoincore_rpc::{self as rpc, Client, RpcApi};

use crate::error::{BwtError, Result};
use crate::util::bitcoincore_ext::RPC_IN_WARMUP;
use crate::util::fmt_date;

#[derive(Debug, Clone)]
pub enum Progress {
    Sync { progress_f: f32, tip: u64 },
    Scan { progress_f: f32, eta: u64 },
    Done,
}

pub fn wait_blockchain_sync(
    rpc: &Client,
    progress_tx: Option<mpsc::Sender<Progress>>,
    interval: time::Duration,
) -> Result<json::GetBlockchainInfoResult> {
    Ok(loop {
        match rpc.get_blockchain_info() {
            Ok(info) => {
                if info.blocks == info.headers
                    && (!info.initial_block_download || info.chain == "regtest")
                {
                    if let Some(ref progress_tx) = progress_tx {
                        let progress = Progress::Sync {
                            progress_f: 1.0,
                            tip: info.median_time,
                        };
                        ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                    }
                    break info;
                }

                if let Some(ref progress_tx) = progress_tx {
                    let progress = Progress::Sync {
                        progress_f: info.verification_progress as f32,
                        tip: info.median_time,
                    };
                    ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                } else {
                    info!(target: "bwt",
                        "waiting for bitcoind to sync [{}/{} blocks, progress={:.1}%, tip at {}]",
                        info.blocks, info.headers, info.verification_progress * 100.0, fmt_date(info.median_time),
                    );
                }
            }
            Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)))
                if e.code == RPC_IN_WARMUP =>
            {
                info!("waiting for bitcoind to warm up: {}", e.message);
            }
            Err(e) => bail!(e),
        }
        thread::sleep(interval);
    })
}

pub fn wait_wallet_scan(
    rpc: &Client,
    progress_tx: Option<mpsc::Sender<Progress>>,
    shutdown_rx: Option<mpsc::Receiver<()>>,
    interval: time::Duration,
) -> Result<json::GetWalletInfoResult> {
    // Stop if the shutdown signal was received or if the channel was disconnected
    let should_stop = || {
        shutdown_rx
            .as_ref()
            .map_or(false, |rx| rx.try_recv() != Err(mpsc::TryRecvError::Empty))
    };

    let info = loop {
        let info = rpc.get_wallet_info()?;
        if should_stop() {
            break info;
        }
        match info.scanning {
            None => {
                warn!("Your bitcoin node does not report the `scanning` status in `getwalletinfo`. It is recommended to upgrade to Bitcoin Core v0.19+ to enable this.");
                warn!("This is needed for bwt to wait for scanning to finish before starting up. Starting bwt while the node is scanning may lead to unexpected results. Continuing anyway...");
                break info;
            }
            Some(ScanningDetails::NotScanning(_)) => {
                // Stop as soon as scanning is completed if no explicit shutdown_rx was given,
                // or continue until the shutdown signal is received if it was. There might be
                // additional rounds of import.
                if shutdown_rx.is_none() || should_stop() {
                    break info;
                }
            }
            Some(ScanningDetails::Scanning {
                progress: progress_f,
                duration,
            }) => {
                let eta = iif!(
                    progress_f > 0.0,
                    (duration as f32 / progress_f) as u64 - duration as u64,
                    0
                );

                if let Some(ref progress_tx) = progress_tx {
                    let progress = Progress::Scan { progress_f, eta };
                    ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                } else {
                    info!(target: "bwt",
                        "waiting for bitcoind to finish scanning [done {:.1}%, running for {}m, eta {}m]",
                        progress_f * 100.0, duration / 60, eta / 60
                    );
                }
            }
        }
        thread::sleep(interval);
        if should_stop() {
            break info;
        }
    };

    if let Some(progress_tx) = progress_tx {
        let progress = Progress::Scan {
            progress_f: 1.0,
            eta: 0,
        };
        ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
    }

    Ok(info)
}
