use std::collections::HashMap;
use std::str::FromStr;

use serde_json::Value;

use bitcoin::util::bip32::{ChildNumber, ExtendedPubKey, Fingerprint};
use bitcoin::{Address, Network};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use secp256k1::Secp256k1;

use crate::error::{Result, ResultExt};
use crate::types::{KeyRescan, ScriptType};
use crate::util::XyzPubKey;

const LABEL_PREFIX: &str = "pxt";

lazy_static! {
    static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
}

#[derive(Debug)]
pub struct HDWatcher {
    wallets: HashMap<Fingerprint, HDWallet>,
}

impl HDWatcher {
    pub fn new(wallets: Vec<HDWallet>) -> Self {
        HDWatcher {
            wallets: wallets
                .into_iter()
                .map(|wallet| (wallet.master.fingerprint(), wallet))
                .collect(),
        }
    }

    /// Mark an address as imported and optionally used
    pub fn mark_funded(&mut self, origin: &KeyOrigin) {
        if let KeyOrigin::Derived(parent_fingerprint, index) = origin {
            if let Some(wallet) = self.wallets.get_mut(parent_fingerprint) {
                if wallet.max_imported_index.map_or(true, |max| *index > max) {
                    wallet.max_imported_index = Some(*index);
                }

                if wallet.max_used_index.map_or(true, |max| *index > max) {
                    wallet.max_used_index = Some(*index);
                }
            }
        }
    }

    pub fn watch(&mut self, rpc: &RpcClient) -> Result<()> {
        let mut import_reqs = vec![];
        let mut pending_updates = vec![];

        for (_, wallet) in self.wallets.iter_mut() {
            let watch_index = wallet.watch_index();
            if watch_index > wallet.max_imported_index.unwrap_or(0) {
                let start_index = wallet
                    .max_imported_index
                    .map_or(0, |max_imported| max_imported + 1);

                let rescan = if wallet.done_initial_import {
                    KeyRescan::None
                } else {
                    wallet.initial_rescan
                };

                debug!(
                    "[hd] importing range {}-{} of xpub {} rescan={:?}",
                    start_index, watch_index, wallet.master, rescan,
                );

                import_reqs.append(&mut wallet.make_imports(start_index, watch_index, rescan));
                pending_updates.push((wallet, watch_index));
            } else if !wallet.done_initial_import {
                debug!(
                    "[hd] done initial import for xpub {} (up to index {:?})",
                    wallet.master, wallet.max_imported_index
                );
                wallet.done_initial_import = true;
            }
        }

        if !import_reqs.is_empty() {
            info!("[hd] importing batch of {} addresses", import_reqs.len());
            batch_import(rpc, import_reqs)?;
            info!("[hd] done importing batch");
        }

        for (wallet, watched_index) in pending_updates {
            debug!(
                "[hd] imported xpub {} up to index {}",
                wallet.master, watched_index
            );
            wallet.max_imported_index = Some(watched_index);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct HDWallet {
    master: ExtendedPubKey,
    network: Network,
    script_type: ScriptType,
    initial_rescan: KeyRescan,
    buffer_size: u32,
    initial_buffer_size: u32,

    done_initial_import: bool,
    max_used_index: Option<u32>,
    max_imported_index: Option<u32>,
}

// TODO figure out the imported indexes, either with listreceivedbyaddress (lots of data)
// or using getaddressesbylabel and a binary search (lots of requests)

impl HDWallet {
    pub fn new(
        master: ExtendedPubKey,
        network: Network,
        script_type: ScriptType,
        initial_rescan: KeyRescan,
    ) -> Self {
        Self {
            master,
            network,
            script_type,
            initial_rescan,
            buffer_size: 5,          // TODO configurable
            initial_buffer_size: 10, // TODO configurable
            done_initial_import: false,
            max_used_index: None,
            max_imported_index: None,
        }
    }

    pub fn from_xpub(s: &str, network: Network, initial_rescan: KeyRescan) -> Result<Vec<Self>> {
        let xyzpub = XyzPubKey::from_str(s)?;

        ensure!(
            xyzpub.matches_network(network),
            "xpub network mismatch, {} is {} and not {}",
            s,
            xyzpub.network,
            network
        );

        let XyzPubKey {
            extended_pubkey: key,
            script_type,
            ..
        } = xyzpub;

        Ok(vec![
            // external chain (receive)
            Self::new(
                key.derive_pub(&*EC, &[ChildNumber::from(0)])?,
                network,
                script_type,
                initial_rescan,
            ),
            // internal chain (change)
            Self::new(
                key.derive_pub(&*EC, &[ChildNumber::from(1)])?,
                network,
                script_type,
                initial_rescan,
            ),
        ])
    }

    pub fn from_xpubs(xpubs: &[(String, KeyRescan)], network: Network) -> Result<Vec<Self>> {
        let mut wallets = vec![];
        for (xpub, rescan) in xpubs {
            wallets.append(
                &mut Self::from_xpub(xpub, network, *rescan)
                    .with_context(|e| format!("invalid xpub {}: {:?}", xpub, e))?,
            );
        }
        Ok(wallets)
    }

    fn derive(&self, index: u32) -> ExtendedPubKey {
        self.master
            .derive_pub(&*EC, &[ChildNumber::from(index)])
            .unwrap()
    }

    /// Returns the maximum index that needs to be watched
    fn watch_index(&self) -> u32 {
        let buffer_size = if self.done_initial_import {
            self.buffer_size
        } else {
            self.initial_buffer_size
        };

        self.max_used_index
            .map_or(buffer_size - 1, |max| max + buffer_size)
    }

    fn make_imports(
        &self,
        start_index: u32,
        end_index: u32,
        rescan: KeyRescan,
    ) -> Vec<(Address, KeyRescan, KeyOrigin)> {
        (start_index..=end_index)
            .map(|index| {
                let key = self.derive(index);
                let address = self.to_address(&key);
                let deriviation = KeyOrigin::Derived(key.parent_fingerprint, index);
                (address, rescan, deriviation)
            })
            .collect()
    }

    fn to_address(&self, key: &ExtendedPubKey) -> Address {
        match self.script_type {
            ScriptType::P2pkh => Address::p2pkh(&key.public_key, self.network),
            ScriptType::P2wpkh => Address::p2wpkh(&key.public_key, self.network),
            ScriptType::P2shP2wpkh => Address::p2shwpkh(&key.public_key, self.network),
        }
    }
}
fn batch_import(
    rpc: &RpcClient,
    import_reqs: Vec<(Address, KeyRescan, KeyOrigin)>,
) -> Result<Vec<Value>> {
    // TODO: parse result, detect errors
    Ok(rpc.call(
        "importmulti",
        &[json!(import_reqs
            .into_iter()
            .map(|(address, rescan, origin)| {
                let label = origin.to_label();

                trace!(
                    "[hd] importing {} as {} with rescan {:?}",
                    address,
                    label,
                    rescan
                );

                json!({
                  "scriptPubKey": { "address": address },
                  "timestamp": rescan.as_rpc_timestamp(),
                  "label": label,
                })
            })
            .collect::<Vec<Value>>())],
    )?)
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeyOrigin {
    Derived(Fingerprint, u32),
    Standalone,
}

serde_string_serializer_impl!(
    KeyOrigin,
    origin,
    match origin {
        KeyOrigin::Standalone => "standalone".into(),
        KeyOrigin::Derived(parent_fingerprint, index) => {
            format!("{}:{}", parent_fingerprint, index)
        }
    }
);

impl KeyOrigin {
    pub fn to_label(&self) -> String {
        match self {
            KeyOrigin::Derived(parent_fingerprint, index) => format!(
                "{}/{}/{}",
                LABEL_PREFIX,
                hex::encode(parent_fingerprint.as_bytes()),
                index
            ),
            KeyOrigin::Standalone => LABEL_PREFIX.into(),
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(3, "/").collect();
        match (parts.get(0), parts.get(1), parts.get(2)) {
            (Some(&LABEL_PREFIX), Some(parent), Some(index)) => Some(KeyOrigin::Derived(
                Fingerprint::from(&hex::decode(parent).ok()?[..]),
                index.parse().ok()?,
            )),
            (Some(&LABEL_PREFIX), None, None) => Some(KeyOrigin::Standalone),
            _ => None,
        }
    }

    pub fn is_standalone(origin: &KeyOrigin) -> bool {
        match origin {
            KeyOrigin::Standalone => true,
            KeyOrigin::Derived(..) => false,
        }
    }
}
