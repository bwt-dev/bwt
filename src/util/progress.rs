use std::{fmt::Write, sync::mpsc, thread, time};

use bitcoincore_rpc::json::{self, ScanningDetails};
use bitcoincore_rpc::{self as rpc, Client, RpcApi};

use crate::error::{BwtError, Result};
use crate::util::bitcoincore_ext::RPC_IN_WARMUP;
use crate::util::{fmt_date, fmt_duration};

const LT: &str = "bwt::progress";

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
    let mut sync_start: Option<(time::Instant, u64)> = None;
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

                // Keep track of the start time and height to calculate the block processing rate
                let (start_time, start_height) =
                    sync_start.get_or_insert_with(|| (time::Instant::now(), info.blocks));

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
                    let blocks_synced = info.blocks - *start_height;

                    let mut est_info = String::new();
                    if blocks_synced > 3 {
                        let rate = blocks_synced as f32 / start_time.elapsed().as_secs_f32();
                        write!(est_info, ", {:.1} blocks/s", rate)?;
                        if info.verification_progress > 0.85 {
                            let eta = time::Duration::from_secs_f32(blocks_left as f32 / rate);
                            write!(est_info, ", ETA {}", fmt_duration(&eta))?;
                        }
                    };

                    info!(target: LT,
                        "bitcoind syncing up... [{} blocks remaining of {}, {:.2}% completed, tip at {}{}]",
                        blocks_left, info.headers, info.verification_progress * 100.0, fmt_date(info.median_time), est_info,
                    );
                }

                // Reset the counters occasionally to account for changes in the average block sizes
                if info.blocks - *start_height >= 288 {
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
                    progress_f > 0.1,
                    (duration as f32 / progress_f) as u64 - duration as u64,
                    0
                );

                if let Some(ref progress_tx) = progress_tx {
                    let progress = Progress::Scan { progress_f, eta };
                    ensure!(progress_tx.send(progress).is_ok(), BwtError::Canceled);
                } else {
                    let est_info = iif!(
                        eta > 0,
                        format!(", ETA {}", fmt_duration(&time::Duration::from_secs(eta))),
                        "".into()
                    );
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
