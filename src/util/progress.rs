use std::{fmt::Write, sync::mpsc, thread, time};

use bitcoincore_rpc::json::{self, ScanningDetails};
use bitcoincore_rpc::{self as rpc, Client, RpcApi};

use crate::error::{BwtError, Result};
use crate::util::bitcoincore_ext::RPC_IN_WARMUP;
use crate::util::{fmt_date, fmt_duration, BoolThen};

const LT: &str = "bwt::progress";

#[derive(Debug, Clone)]
pub enum Progress {
    Sync { progress_f: f32, tip: u64 },
    Scan { progress_f: f32, eta: u64 },
    Done,
}

/// Wait for the bitcoind rpc to warm up. When `wait_block_sync` is true, also wauts
/// for bitcoind to finish syncing blocks.
pub fn wait_bitcoind_ready(
    rpc: &Client,
    progress_tx: Option<mpsc::Sender<Progress>>,
    interval: time::Duration,
    wait_block_sync: bool,
) -> Result<json::GetBlockchainInfoResult> {
    let mut sync_start: Option<(time::Instant, u64)> = None;
    Ok(loop {
        match rpc.get_blockchain_info() {
            Ok(info) => {
                if is_synced(&info) {
                    if let Some(ref progress_tx) = progress_tx {
                        let progress = Progress::Sync {
                            progress_f: 1.0,
                            tip: info.median_time,
                        };
                        ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                    }
                    break info;
                } else if !wait_block_sync {
                    break info;
                }

                if let Some(ref progress_tx) = progress_tx {
                    let progress = Progress::Sync {
                        progress_f: info.verification_progress as f32,
                        tip: info.median_time,
                        // TODO expose sync rate and eta info to library consumers
                    };
                    ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                } else if info.blocks == 0 {
                    info!(
                        target: LT,
                        "bitcoind syncing up... [fetched {} headers]", info.headers
                    );
                } else if info.headers != info.blocks {
                    // blocks can be > headers in some cases, like after `reconsiderblock`
                    let blocks_left = info.headers.saturating_sub(info.blocks);

                    let mut est_info = String::new();
                    if let Some((start_time, start_height)) = sync_start {
                        let blocks_synced = info.blocks - start_height;
                        if blocks_synced > 3 {
                            let rate = blocks_synced as f32 / start_time.elapsed().as_secs_f32();
                            write!(est_info, ", {:.1} blocks/s", rate)?;
                            if info.verification_progress > 0.85 {
                                let eta = time::Duration::from_secs_f32(blocks_left as f32 / rate);
                                write!(est_info, ", ETA {}", fmt_duration(&eta))?;
                            }
                        };
                    }

                    info!(target: LT,
                        "bitcoind syncing up... [{} blocks remaining of {}, {:.2}% completed, tip at {}{}]",
                        blocks_left, info.headers, info.verification_progress * 100.0, fmt_date(info.median_time), est_info,
                    );
                }

                // Keep track of the start time and height to calculate the block processing rate,
                // and reset it the occasionally to account for changes in the average block sizes.
                if sync_start.map_or(true, |(_, start_height)| info.blocks - start_height >= 2016) {
                    sync_start = Some((time::Instant::now(), info.blocks));
                }
            }
            Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)))
                if e.code == RPC_IN_WARMUP =>
            {
                info!(target: LT, "waiting for bitcoind to warm up: {}", e.message);
            }
            Err(e) => bail!(e),
        }
        thread::sleep(interval);
    })
}

pub fn is_synced(info: &json::GetBlockchainInfoResult) -> bool {
    info.blocks == info.headers && (!info.initial_block_download || info.chain == "regtest")
}

/// Wait for bitcoind to finish rescanning for wallet activity.
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
                let eta = (progress_f > 0.1)
                    .do_then(|| (duration as f32 / progress_f) as u64 - duration as u64);

                if let Some(ref progress_tx) = progress_tx {
                    let eta = eta.unwrap_or(0);
                    let progress = Progress::Scan { progress_f, eta };
                    ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                } else {
                    let est_info = eta.map_or("".into(), |eta| {
                        format!(", ETA {}", fmt_duration(&time::Duration::from_secs(eta)))
                    });
                    info!(
                        target: LT,
                        "bitcoind scanning history... [{:.1}% completed{}]",
                        progress_f * 100.0,
                        est_info
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
