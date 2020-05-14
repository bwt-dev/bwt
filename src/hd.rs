use std::collections::HashMap;
use std::result::Result as StdResult;
use std::str::FromStr;

use serde::Serialize;
use serde_json::Value;

use bitcoin::util::bip32::{ChildNumber, ExtendedPubKey, Fingerprint};
use bitcoin::{util::base58, Address, Network};
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use secp256k1::Secp256k1;

use crate::error::{Result, ResultExt};
use crate::types::{KeyRescan, ScriptType};

const LABEL_PREFIX: &str = "bwt";

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

    pub fn wallets(&self) -> &HashMap<Fingerprint, HDWallet> {
        &self.wallets
    }

    pub fn get(&self, fingerprint: &Fingerprint) -> Option<&HDWallet> {
        self.wallets.get(fingerprint)
    }

    // Mark an address as funded
    pub fn mark_funded(&mut self, origin: &KeyOrigin) {
        if let KeyOrigin::Derived(parent_fingerprint, index) = origin {
            if let Some(wallet) = self.wallets.get_mut(parent_fingerprint) {
                if wallet.max_imported_index.map_or(true, |max| *index > max) {
                    wallet.max_imported_index = Some(*index);
                }

                if wallet.max_funded_index.map_or(true, |max| *index > max) {
                    wallet.max_funded_index = Some(*index);
                }
            }
        }
    }

    pub fn watch(&mut self, rpc: &RpcClient) -> Result<bool> {
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
                    wallet.rescan_policy
                };

                debug!(
                    "importing range {}-{} of xpub {} rescan={:?}",
                    start_index, watch_index, wallet.master, rescan,
                );

                import_reqs.append(&mut wallet.make_imports(start_index, watch_index, rescan));
                pending_updates.push((wallet, watch_index));
            } else if !wallet.done_initial_import {
                debug!(
                    "done initial import for xpub {} (up to index {:?})",
                    wallet.master,
                    wallet.max_imported_index.unwrap_or(0)
                );
                wallet.done_initial_import = true;
            }
        }

        let has_imports = !import_reqs.is_empty();

        if has_imports {
            info!("importing batch of {} addresses", import_reqs.len());
            batch_import(rpc, import_reqs)?;
            info!("done importing batch");
        }

        for (wallet, watched_index) in pending_updates {
            debug!(
                "imported xpub {} up to index {}",
                wallet.master, watched_index
            );
            wallet.max_imported_index = Some(watched_index);
        }

        Ok(has_imports)
    }
}

#[derive(Debug, Clone)]
pub struct HDWallet {
    master: ExtendedPubKey,
    network: Network,
    script_type: ScriptType,
    gap_limit: u32,
    initial_gap_limit: u32,
    rescan_policy: KeyRescan,

    pub max_funded_index: Option<u32>,
    pub max_imported_index: Option<u32>,
    pub done_initial_import: bool,
}

// TODO figure out the imported indexes, either with listreceivedbyaddress (lots of data)
// or using getaddressesbylabel and a binary search (lots of requests)

impl HDWallet {
    pub fn new(
        master: ExtendedPubKey,
        network: Network,
        script_type: ScriptType,
        gap_limit: u32,
        initial_gap_limit: u32,
        rescan_policy: KeyRescan,
    ) -> Self {
        Self {
            master,
            network,
            script_type,
            gap_limit,
            // setting initial_gap_limit < gap_limit makes no sense, the user probably meant to increase both
            initial_gap_limit: initial_gap_limit.max(gap_limit),
            rescan_policy,
            done_initial_import: false,
            max_funded_index: None,
            max_imported_index: None,
        }
    }

    pub fn from_bare_xpub(
        xpub: XyzPubKey,
        network: Network,
        gap_limit: u32,
        initial_gap_limit: u32,
        rescan_policy: KeyRescan,
    ) -> Result<Self> {
        ensure!(
            xpub.matches_network(network),
            "xpub network mismatch, {} is {} and not {}",
            xpub,
            xpub.network,
            network
        );
        Ok(Self::new(
            xpub.extended_pubkey,
            network,
            xpub.script_type,
            gap_limit,
            initial_gap_limit,
            rescan_policy,
        ))
    }

    pub fn from_xpub(
        xpub: XyzPubKey,
        network: Network,
        gap_limit: u32,
        initial_gap_limit: u32,
        rescan_policy: KeyRescan,
    ) -> Result<Vec<Self>> {
        ensure!(
            xpub.matches_network(network),
            "xpub network mismatch, {} is {} and not {}",
            xpub,
            xpub.network,
            network
        );
        Ok(vec![
            // external chain (receive)
            Self::new(
                xpub.extended_pubkey
                    .derive_pub(&*EC, &[ChildNumber::from(0)])?,
                network,
                xpub.script_type,
                gap_limit,
                initial_gap_limit,
                rescan_policy,
            ),
            // internal chain (change)
            Self::new(
                xpub.extended_pubkey
                    .derive_pub(&*EC, &[ChildNumber::from(1)])?,
                network,
                xpub.script_type,
                gap_limit,
                initial_gap_limit,
                rescan_policy,
            ),
        ])
    }

    pub fn from_xpubs(
        xpubs: &[(XyzPubKey, KeyRescan)],
        bare_xpubs: &[(XyzPubKey, KeyRescan)],
        network: Network,
        gap_limit: u32,
        initial_gap_limit: u32,
    ) -> Result<Vec<Self>> {
        let mut wallets = vec![];
        for (xpub, rescan) in xpubs {
            wallets.append(
                &mut Self::from_xpub(xpub.clone(), network, gap_limit, initial_gap_limit, *rescan)
                    .with_context(|e| format!("invalid xpub {}: {:?}", xpub, e))?,
            );
        }
        for (xpub, rescan) in bare_xpubs {
            wallets.push(
                Self::from_bare_xpub(xpub.clone(), network, gap_limit, initial_gap_limit, *rescan)
                    .with_context(|e| format!("invalid xpub {}: {:?}", xpub, e))?,
            );
        }
        Ok(wallets)
    }

    pub fn derive(&self, index: u32) -> ExtendedPubKey {
        self.master
            .derive_pub(&*EC, &[ChildNumber::from(index)])
            .unwrap()
    }

    /// Returns the maximum index that needs to be watched
    fn watch_index(&self) -> u32 {
        let gap_limit = if self.done_initial_import {
            self.gap_limit
        } else {
            self.initial_gap_limit
        };

        self.max_funded_index
            .map_or(gap_limit - 1, |max| max + gap_limit)
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
                let origin = KeyOrigin::Derived(key.parent_fingerprint, index);
                (address, rescan, origin)
            })
            .collect()
    }

    pub fn to_address(&self, key: &ExtendedPubKey) -> Address {
        match self.script_type {
            ScriptType::P2pkh => Address::p2pkh(&key.public_key, self.network),
            ScriptType::P2wpkh => Address::p2wpkh(&key.public_key, self.network),
            ScriptType::P2shP2wpkh => Address::p2shwpkh(&key.public_key, self.network),
        }
    }

    pub fn derive_address(&self, index: u32) -> Address {
        self.to_address(&self.derive(index))
    }

    pub fn get_next_index(&self) -> u32 {
        self.max_funded_index
            .map_or(0, |max_funded_index| max_funded_index + 1)
    }
}
fn batch_import(
    rpc: &RpcClient,
    import_reqs: Vec<(Address, KeyRescan, KeyOrigin)>,
) -> Result<Vec<Value>> {
    // TODO parse result, detect errors
    // TODO use importmulti with ranged descriptors? the key derivation info won't be
    // directly available on `listtransactions` and would require an additional rpc all.
    Ok(rpc.call(
        "importmulti",
        &[json!(import_reqs
            .into_iter()
            .map(|(address, rescan, origin)| {
                let label = origin.to_label();

                trace!(
                    "importing {} as {} with rescan {:?}",
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

impl_string_serializer!(
    KeyOrigin,
    origin,
    match origin {
        KeyOrigin::Standalone => "standalone".into(),
        KeyOrigin::Derived(parent_fingerprint, index) => {
            format!("{}/{}", parent_fingerprint, index)
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

    pub fn from_extkey(key: &ExtendedPubKey) -> Self {
        let derivation_index = match key.child_number {
            ChildNumber::Normal { index } => index,
            ChildNumber::Hardened { .. } => unreachable!("unexpected hardened derivation"),
        };
        KeyOrigin::Derived(key.parent_fingerprint, derivation_index)
    }

    pub fn is_standalone(origin: &KeyOrigin) -> bool {
        match origin {
            KeyOrigin::Standalone => true,
            KeyOrigin::Derived(..) => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct XyzPubKey {
    pub network: Network,
    pub script_type: ScriptType,
    pub extended_pubkey: ExtendedPubKey,
}

impl FromStr for XyzPubKey {
    type Err = base58::Error;

    fn from_str(inp: &str) -> StdResult<XyzPubKey, base58::Error> {
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
        // within bwt, but does mean that they will mismatch the fingerprints reported by other software.
        // This also means that it is impossible to export the same key chain using different script types.

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

impl_string_serializer!(XyzPubKey, xpub, xpub.extended_pubkey.to_string());

fn parse_xyz_version(version: &[u8]) -> StdResult<(Network, ScriptType), base58::Error> {
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

use serde::ser::SerializeStruct;

// Serialize the HDWallet struct with an additional virtual "origin" field
impl Serialize for HDWallet {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut rgb = serializer.serialize_struct("HDWallet", 3)?;
        rgb.serialize_field("xpub", &self.master)?;
        rgb.serialize_field("origin", &KeyOrigin::from_extkey(&self.master))?;
        rgb.serialize_field("network", &self.network)?;
        rgb.serialize_field("script_type", &self.script_type)?;
        rgb.serialize_field("gap_limit", &self.gap_limit)?;
        rgb.serialize_field("initial_gap_limit", &self.initial_gap_limit)?;
        rgb.serialize_field("rescan_policy", &self.rescan_policy)?;
        rgb.serialize_field("max_funded_index", &self.max_funded_index)?;
        rgb.serialize_field("max_imported_index", &self.max_imported_index)?;
        rgb.serialize_field("done_initial_import", &self.done_initial_import)?;
        rgb.end()
    }
}
