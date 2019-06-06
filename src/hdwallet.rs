use std::collections::HashMap;
use std::str::FromStr;

use bitcoin::util::bip32::{ChildNumber, ExtendedPubKey, Fingerprint, ScriptType};
use bitcoin::Address;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use hex;
use secp256k1::Secp256k1;
use serde_json::Value;

use crate::error::Result;

const LABEL_PREFIX: &str = "rust_eps";

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
    pub fn mark_address(&mut self, derivation: &DerivationInfo, is_used: bool) {
        if let DerivationInfo::Derived(parent_fingerprint, index) = derivation {
            if let Some(wallet) = self.wallets.get_mut(parent_fingerprint) {
                if wallet.max_imported_index.map_or(true, |max| *index > max) {
                    wallet.max_imported_index = Some(*index);
                }

                if is_used && wallet.max_used_index.map_or(true, |max| *index > max) {
                    wallet.max_used_index = Some(*index);
                }
            }
        }
    }

    pub fn do_imports(&mut self, rpc: &RpcClient) -> Result<()> {
        let mut import_reqs = vec![];
        let mut pending_updates = vec![];

        for (_, wallet) in self.wallets.iter_mut() {
            let watch_index = wallet.watch_index();
            if wallet
                .max_imported_index
                .map_or(true, |max_imported| watch_index > max_imported)
            {
                let start_index = wallet
                    .max_imported_index
                    .map_or(0, |max_imported| max_imported + 1);

                let rescan = if wallet.done_initial_import {
                    KeyRescan::None
                } else {
                    wallet.initial_rescan
                };

                info!(
                    "importing hd wallet {} range {}-{} with rescan policy {:?}",
                    wallet.master, start_index, watch_index, rescan,
                );

                import_reqs.append(&mut wallet.make_imports(start_index, watch_index, rescan));
                pending_updates.push((wallet, watch_index));
            } else if !wallet.done_initial_import {
                info!("done initial import for {}", wallet.master);
                // XXX figure out done_initial_import logic (following restart etc)
                wallet.done_initial_import = true;
            }
        }

        batch_import(rpc, import_reqs)?;

        for (wallet, watched_index) in pending_updates {
            info!("imported hd key {} up to {}", wallet.master, watched_index,);
            wallet.max_imported_index = Some(watched_index);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct HDWallet {
    master: ExtendedPubKey,
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
    pub fn new(master: ExtendedPubKey, creation_time: Option<u32>) -> Self {
        let initial_rescan = creation_time.map_or(KeyRescan::All, KeyRescan::Since);

        Self {
            master,
            initial_rescan,
            buffer_size: 20,          // TODO configurable
            initial_buffer_size: 100, // TODO configurable
            done_initial_import: false,
            max_used_index: None,
            max_imported_index: None,
        }
    }

    pub fn from_xpub(s: &str, creation_time: Option<u32>) -> Result<Vec<Self>> {
        let key = ExtendedPubKey::from_str(s)?;
        // XXX verify key network type

        Ok(vec![
            // receive account
            Self::new(
                key.derive_pub(&*EC, &[ChildNumber::from(0)])?,
                creation_time,
            ),
            // change account
            Self::new(
                key.derive_pub(&*EC, &[ChildNumber::from(1)])?,
                creation_time,
            ),
        ])
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
    ) -> Vec<(Address, KeyRescan, DerivationInfo)> {
        (start_index..end_index)
            .map(|index| {
                let key = self.derive(index);
                let address = to_address(&key);
                let deriviation = DerivationInfo::Derived(key.parent_fingerprint, index);
                (address, rescan, deriviation)
            })
            .collect()
    }
}

fn to_address(key: &ExtendedPubKey) -> Address {
    match key.script_type {
        ScriptType::P2pkh => Address::p2pkh(&key.public_key, key.network),
        ScriptType::P2wpkh => Address::p2wpkh(&key.public_key, key.network),
        ScriptType::P2shP2wpkh => Address::p2shwpkh(&key.public_key, key.network),
    }
}

fn batch_import(
    rpc: &RpcClient,
    import_reqs: Vec<(Address, KeyRescan, DerivationInfo)>,
) -> Result<Vec<Value>> {
    debug!("importing {} addresses", import_reqs.len());

    // TODO: parse result, detect errors
    Ok(rpc.call(
        "importmulti",
        &[json!(import_reqs
            .into_iter()
            .map(|(address, rescan, derivation)| {
                let label = derivation.to_label();

                info!(
                    "importing {} as {} with rescan {:?}",
                    address, label, rescan
                );

                json!({
                  "scriptPubKey": { "address": address },
                  "timestamp": rescan.rpc_arg(),
                  "label": label,
                })
            })
            .collect::<Vec<Value>>())],
    )?)
}

#[derive(Debug)]
pub enum DerivationInfo {
    Derived(Fingerprint, u32),
    Standalone,
}

impl DerivationInfo {
    pub fn to_label(&self) -> String {
        match self {
            DerivationInfo::Derived(parent_fingerprint, index) => format!(
                "{}/{}/{}",
                LABEL_PREFIX,
                hex::encode(parent_fingerprint.as_bytes()),
                index
            ),
            DerivationInfo::Standalone => LABEL_PREFIX.into(),
        }
    }

    pub fn from_label(s: &str) -> Self {
        Self::try_from_label(s).unwrap_or(DerivationInfo::Standalone)
    }

    fn try_from_label(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.splitn(3, "/").collect();
        Ok(match (parts.get(0), parts.get(1), parts.get(2)) {
            (Some(&LABEL_PREFIX), Some(parent), Some(index)) => DerivationInfo::Derived(
                Fingerprint::from(&hex::decode(parent)?[..]),
                index.parse()?,
            ),
            _ => DerivationInfo::Standalone,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum KeyRescan {
    None,
    All,
    Since(u32),
}

impl KeyRescan {
    pub fn rpc_arg(&self) -> Value {
        match self {
            KeyRescan::None => json!("now"),
            KeyRescan::All => json!(0),
            KeyRescan::Since(epoch) => json!(epoch),
        }
    }
}
