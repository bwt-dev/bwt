use std::collections::hash_map::{Entry, HashMap};
use std::str::FromStr;

use serde_json::Value;

use bitcoin::util::{base58, bip32::ExtendedPubKey};
use bitcoin::{Network, Txid};

use crate::types::ScriptType;

const VSIZE_BIN_WIDTH: u32 = 50_000; // vbytes

// Make the fee histogram our of a list of `getrawmempool true` entries
pub fn make_fee_histogram(mempool_entries: HashMap<Txid, Value>) -> Vec<(f32, u32)> {
    let mut entries: Vec<(u32, f32)> = mempool_entries
        .values()
        .map(|entry| {
            let size = entry["size"].as_u64().unwrap(); // bitcoind is borked if this fails
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

pub fn remove_if<K, V>(hm: &mut HashMap<K, V>, key: K, predicate: impl Fn(&V) -> bool)
where
    K: Eq + std::hash::Hash,
{
    if let Entry::Occupied(entry) = hm.entry(key) {
        if predicate(entry.get()) {
            entry.remove_entry();
        }
    }
}

pub struct XyzPubKey {
    pub network: Network,
    pub script_type: ScriptType,
    pub extended_pubkey: ExtendedPubKey,
}

impl FromStr for XyzPubKey {
    type Err = base58::Error;

    fn from_str(inp: &str) -> Result<XyzPubKey, base58::Error> {
        let mut data = base58::from_check(inp)?;

        if data.len() != 78 {
            return Err(base58::Error::InvalidLength(data.len()));
        }

        // rust-bitcoin's bip32 implementation does not support ypubs/zpubs.
        // instead, figure out the network and script type ourselves and feed rust-bitcoin with a
        // modified faux xpub string that uses the regular p2pkh xpub version bytes it expects.
        //
        // NOTE: this does mean that the fingerprints will be computed using the fauxed version
        // bytes instead of the real ones. that's okay as long as the fingerprints as consistent
        // within pxt, but does mean that they will mismatch the fingerprints reported by other
        // software.

        let version = &data[0..4];
        let (network, script_type) = parse_xyz_version(version)?;
        data.splice(0..4, get_xpub_p2pkh_version(network).iter().cloned());

        let faux_xpub = base58::check_encode_slice(&data);
        let extended_pubkey = ExtendedPubKey::from_str(&faux_xpub)?;

        Ok(XyzPubKey {
            network,
            script_type,
            extended_pubkey,
        })
    }
}

impl XyzPubKey {
    pub fn matches_network(&self, network: Network) -> bool {
        self.network == network || (self.network == Network::Testnet && network == Network::Regtest)
    }
}

fn parse_xyz_version(version: &[u8]) -> Result<(Network, ScriptType), base58::Error> {
    Ok(match version {
        [0x04u8, 0x88, 0xB2, 0x1E] => (Network::Bitcoin, ScriptType::P2pkh),
        [0x04u8, 0xB2, 0x47, 0x46] => (Network::Bitcoin, ScriptType::P2wpkh),
        [0x04u8, 0x9D, 0x7C, 0xB2] => (Network::Bitcoin, ScriptType::P2shP2wpkh),

        [0x04u8, 0x35, 0x87, 0xCF] => (Network::Testnet, ScriptType::P2pkh),
        [0x04u8, 0x5F, 0x1C, 0xF6] => (Network::Testnet, ScriptType::P2wpkh),
        [0x04u8, 0x4A, 0x52, 0x62] => (Network::Testnet, ScriptType::P2shP2wpkh),

        _ => return Err(base58::Error::InvalidVersion(version.to_vec())),
    })
}

fn get_xpub_p2pkh_version(network: Network) -> [u8; 4] {
    match network {
        Network::Bitcoin => [0x04u8, 0x88, 0xB2, 0x1E],
        Network::Testnet | Network::Regtest => [0x04u8, 0x35, 0x87, 0xCF],
    }
}

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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
                    trace!("debouncing sync for {:?}", duration);
                    match debounce_rx.recv_timeout(duration) {
                        // if we receive another message within the `duration`, debounce and start over again
                        Ok(()) => continue,
                        // if we timed-out, we're good!
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => panic!(),
                    }
                }
            }
            info!("unix socket triggering index sync");
            forward_tx.send(()).unwrap();
        }
    });

    debounce_tx
}
