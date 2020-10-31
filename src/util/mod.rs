use std::collections::hash_map::{Entry, HashMap};
use std::time::{Duration, Instant};
use std::{sync::mpsc, thread};

use bitcoin::secp256k1::{self, Secp256k1};
use serde_json::Value;

use bitcoin::Txid;

#[macro_use]
mod macros;

pub mod banner;
pub mod bitcoincore_ext;
pub mod descriptor;
pub mod xpub;

lazy_static! {
    pub static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
}

const VSIZE_BIN_WIDTH: u32 = 50_000; // vbytes

// Make the fee histogram our of a list of `getrawmempool true` entries
pub fn make_fee_histogram(mempool_entries: HashMap<Txid, Value>) -> Vec<(f32, u32)> {
    let mut entries: Vec<(u32, f32)> = mempool_entries
        .values()
        .map(|entry| {
            let size = entry["vsize"]
                .as_u64()
                .or_else(|| entry["size"].as_u64())
                .unwrap(); // bitcoind is borked if this fails
            let fee = entry["fee"].as_f64().unwrap();
            let feerate = fee as f32 / size as f32 * 100_000_000f32;
            (size as u32, feerate)
        })
        .collect();

    // XXX we should take unconfirmed parents feerates into account

    entries.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    let mut histogram = vec![];
    let mut bin_size = 0;
    let mut last_feerate = None;

    for (size, feerate) in entries.into_iter().rev() {
        bin_size += size;
        if bin_size > VSIZE_BIN_WIDTH && last_feerate.map_or(true, |last| feerate > last) {
            // vsize of transactions paying >= e.fee_per_vbyte()
            histogram.push((feerate, bin_size));
            bin_size = 0;
        }
        last_feerate = Some(feerate);
    }

    if let Some(feerate) = last_feerate {
        histogram.push((feerate, bin_size));
    }

    histogram
}

pub fn remove_if<K, V>(hm: &mut HashMap<K, V>, key: K, predicate: impl Fn(&mut V) -> bool) -> bool
where
    K: Eq + std::hash::Hash,
{
    if let Entry::Occupied(mut entry) = hm.entry(key) {
        if predicate(entry.get_mut()) {
            entry.remove_entry();
        }
        true
    } else {
        false
    }
}

// debounce a Sender to only emit events sent when `duration` seconds has passed since
// the previous event, or after `duration` seconds elapses without new events coming in.
pub fn debounce_sender(forward_tx: mpsc::Sender<()>, duration: u64) -> mpsc::Sender<()> {
    let duration = Duration::from_secs(duration);
    let (debounce_tx, debounce_rx) = mpsc::channel();

    thread::spawn(move || {
        loop {
            let tick_start = Instant::now();
            // always wait for the first sync message to arrive first
            debounce_rx.recv().unwrap();
            if tick_start.elapsed() < duration {
                // if duration hasn't passed, debounce for another `duration` seconds
                loop {
                    trace!(target: "bwt::real-time", "debouncing sync for {:?}", duration);
                    match debounce_rx.recv_timeout(duration) {
                        // if we receive another message within the `duration`, debounce and start over again
                        Ok(()) => continue,
                        // if we timed-out, we're good!
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => panic!(),
                    }
                }
            }
            info!(target: "bwt::real-time", "triggering real-time index sync");
            forward_tx.send(()).unwrap();
        }
    });

    debounce_tx
}

pub trait BoolThen {
    // Similar to https://doc.rust-lang.org/std/primitive.bool.html#method.then (nightly only)
    fn do_then<T>(self, f: impl FnOnce() -> T) -> Option<T>;

    // Alternative version where the closure returns an Option<T>
    fn and_then<T>(self, f: impl FnOnce() -> Option<T>) -> Option<T>;
}

impl BoolThen for bool {
    fn do_then<T>(self, f: impl FnOnce() -> T) -> Option<T> {
        if self {
            Some(f())
        } else {
            None
        }
    }
    fn and_then<T>(self, f: impl FnOnce() -> Option<T>) -> Option<T> {
        if self {
            f()
        } else {
            None
        }
    }
}
