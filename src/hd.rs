use std::collections::HashMap;
use std::result::Result as StdResult;
use std::str::FromStr;
use std::sync::Arc;

use serde::Serialize;

use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::{util::base58, Address, Network};
use bitcoincore_rpc::json::ImportMultiRequest;
use bitcoincore_rpc::{self as rpc, Client as RpcClient, RpcApi};

use crate::error::{Context, Result};
use crate::types::{DescrChecksum, Descriptor, RescanSince, ScriptType};

const LABEL_PREFIX: &str = "bwt";

lazy_static! {
    static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
}

#[derive(Debug)]
pub struct HDWatcher {
    wallets: HashMap<DescrChecksum, HDWallet>,
}

impl HDWatcher {
    pub fn new(wallets: Vec<HDWallet>) -> Self {
        HDWatcher {
            wallets: wallets
                .into_iter()
                .map(|wallet| (wallet.descriptor.checksum(), wallet))
                .collect(),
        }
    }

    pub fn wallets(&self) -> &HashMap<DescrChecksum, HDWallet> {
        &self.wallets
    }

    pub fn get(&self, descr_cs: DescrChecksum) -> Option<&HDWallet> {
        self.wallets.get(&descr_cs)
    }

    // Mark an address as funded
    pub fn mark_funded(&mut self, origin: &KeyOrigin) {
        if let KeyOrigin::Derived(descr_cs, index) = origin {
            if let Some(wallet) = self.wallets.get_mut(descr_cs) {
                if wallet.max_imported_index.map_or(true, |max| *index > max) {
                    wallet.max_imported_index = Some(*index);
                }

                if wallet.max_funded_index.map_or(true, |max| *index > max) {
                    wallet.max_funded_index = Some(*index);
                }
            }
        }
    }

    // check previous imports and update max_imported_index
    pub fn check_imports(&mut self, rpc: &RpcClient) -> Result<()> {
        debug!("checking previous imports");
        let labels: Vec<String> = rpc.call("listlabels", &[]).map_err(labels_error)?;
        let mut imported_indexes: HashMap<DescrChecksum, u32> = HashMap::new();
        for label in labels {
            if let Some(KeyOrigin::Derived(descr_cs, index)) = KeyOrigin::from_label(&label) {
                if self.wallets.contains_key(&descr_cs) {
                    imported_indexes
                        .entry(descr_cs)
                        .and_modify(|current| *current = (*current).max(index))
                        .or_insert(index);
                }
            }
        }

        for (descr_cs, max_imported_index) in imported_indexes {
            trace!(
                "wallet {} was imported up to index {}",
                descr_cs,
                max_imported_index
            );
            let wallet = self.wallets.get_mut(&descr_cs).unwrap();
            wallet.max_imported_index = Some(max_imported_index);

            // if anything was imported at all, assume we've finished the initial sync. this might
            // not hold true if bwt shuts down while syncing, but this only means that we'll use
            // the smaller gap_limit instead of the initial_import_size, which is acceptable.
            wallet.done_initial_import = true;
        }
        Ok(())
    }

    pub fn do_imports(&mut self, rpc: &RpcClient, rescan: bool) -> Result<bool> {
        let mut import_reqs = vec![];
        let mut pending_updates = vec![];

        for (descr_cs, wallet) in self.wallets.iter_mut() {
            let watch_index = wallet.watch_index();
            if wallet.max_imported_index.map_or(true, |i| watch_index > i) {
                let start_index = wallet
                    .max_imported_index
                    .map_or(0, |max_imported| max_imported + 1);

                debug!(
                    "importing {} range {}-{} with rescan={}",
                    descr_cs, start_index, watch_index, rescan,
                );

                import_reqs.append(&mut wallet.make_imports(start_index, watch_index, rescan));

                pending_updates.push((wallet, descr_cs, watch_index));
            } else if !wallet.done_initial_import {
                debug!(
                    "done initial import for {} up to index {}",
                    descr_cs,
                    wallet.max_imported_index.unwrap()
                );
                wallet.done_initial_import = true;
            } else {
                trace!("no imports needed for {}", descr_cs);
            }
        }

        let has_imports = !import_reqs.is_empty();

        if has_imports {
            // TODO report syncing progress
            info!(
                "importing batch of {} addresses... (this may take awhile)",
                import_reqs.len()
            );
            batch_import(rpc, import_reqs)?;
            info!("done importing batch");
        }

        for (wallet, descr_cs, imported_index) in pending_updates {
            debug!("imported {} up to index {}", descr_cs, imported_index);
            wallet.max_imported_index = Some(imported_index);
        }

        Ok(has_imports)
    }
}

#[derive(Debug, Clone)]
pub struct HDWallet {
    descriptor: Descriptor,
    network: Network,
    gap_limit: u32,
    initial_import_size: u32,
    rescan_policy: RescanSince,

    pub max_funded_index: Option<u32>,
    pub max_imported_index: Option<u32>,
    pub done_initial_import: bool,
}

impl HDWallet {
    pub fn new(
        descriptor: Descriptor,
        network: Network,
        gap_limit: u32,
        initial_import_size: u32,
        rescan_policy: RescanSince,
    ) -> Self {
        Self {
            descriptor,
            network,
            gap_limit,
            // setting initial_import_size < gap_limit makes no sense, the user probably meant to increase both
            initial_import_size: initial_import_size.max(gap_limit),
            rescan_policy,
            done_initial_import: false,
            max_funded_index: None,
            max_imported_index: None,
        }
    }

    pub fn from_config(
        descriptors: &[(String, RescanSince)],
        xpubs: &[(XyzPubKey, RescanSince)],
        bare_xpubs: &[(XyzPubKey, RescanSince)],
        network: Network,
        gap_limit: u32,
        initial_import_size: u32,
        rpc: Arc<RpcClient>,
    ) -> Result<Vec<Self>> {
        // Descriptors
        let mut wallets = vec![];
        for (descriptor, rescan) in descriptors {
            wallets.push(
                Self::from_descriptor(
                    descriptor.clone(),
                    network,
                    gap_limit,
                    initial_import_size,
                    *rescan,
                    rpc.clone(),
                )
                .with_context(|| format!("invalid descriptor {}", descriptor))?
                .clone(),
            );
        }

        // Xpubs
        for (xyz, rescan) in xpubs {
            // Change and receiving output descriptors
            for change in 0..2 {
                let descriptor = match xyz.script_type {
                    ScriptType::P2pkh => {
                        format!("pkh({}/{}/*)", xyz.extended_pubkey.to_string(), change)
                    }
                    ScriptType::P2wpkh => {
                        format!("wpkh({}/{}/*)", xyz.extended_pubkey.to_string(), change)
                    }
                    ScriptType::P2shP2wpkh => {
                        format!("sh(wpkh({}/{}/*))", xyz.extended_pubkey.to_string(), change)
                    }
                };
                wallets.push(
                    Self::from_descriptor(
                        descriptor.clone(),
                        network,
                        gap_limit,
                        initial_import_size,
                        *rescan,
                        rpc.clone(),
                    )
                    .with_context(|| format!("Invalid xpub-derived descriptor {}", descriptor))?
                    .clone(),
                );
            }
        }

        // Bare xpubs
        for (xyz, rescan) in bare_xpubs {
            let descriptor = match xyz.script_type {
                ScriptType::P2pkh => format!("pkh({})", xyz.extended_pubkey.to_string()),
                ScriptType::P2wpkh => format!("wpkh({})", xyz.extended_pubkey.to_string()),
                ScriptType::P2shP2wpkh => format!("sh(wpkh({}))", xyz.extended_pubkey.to_string()),
            };
            wallets.push(
                Self::from_descriptor(
                    descriptor.clone(),
                    network,
                    gap_limit,
                    initial_import_size,
                    *rescan,
                    rpc.clone(),
                )
                .with_context(|| format!("Invalid bare-xpub-derived descriptor {}", descriptor))?
                .clone(),
            );
        }

        if wallets.is_empty() {
            warn!("Please provide at least one descriptor to track (via --descriptors).");
            bail!("No descriptors provided");
        }
        Ok(wallets)
    }

    pub fn from_descriptor(
        descriptor: String,
        network: Network,
        gap_limit: u32,
        initial_import_size: u32,
        rescan_policy: RescanSince,
        rpc: Arc<RpcClient>,
    ) -> Result<Self> {
        let descriptor = Descriptor::new(&descriptor, rpc)?;
        Ok(Self::new(
            descriptor,
            network,
            gap_limit,
            initial_import_size,
            rescan_policy,
        ))
    }

    /// Returns the maximum index that needs to be watched
    fn watch_index(&self) -> u32 {
        let gap_limit = if self.done_initial_import {
            self.gap_limit
        } else {
            self.initial_import_size
        };

        self.max_funded_index
            .map_or(gap_limit - 1, |max| max + gap_limit)
    }

    fn make_imports(
        &self,
        start_index: u32,
        end_index: u32,
        rescan: bool,
    ) -> Vec<(Descriptor, u32, String, RescanSince)> {
        let rescan_since = if rescan {
            self.rescan_policy
        } else {
            RescanSince::Now
        };

        (start_index..=end_index)
            .map(|index| {
                let label = KeyOrigin::Derived(self.descriptor.checksum(), index).to_label();
                (self.descriptor.clone(), index, label, rescan_since)
            })
            .collect()
    }

    pub fn derive_address(&self, index: u32, rpc: &RpcClient) -> Result<Address> {
        let res = rpc.derive_addresses(&self.descriptor.0, Some([index, index]))?;
        Ok(res[0].to_owned())
    }

    pub fn get_next_index(&self) -> u32 {
        self.max_funded_index
            .map_or(0, |max_funded_index| max_funded_index + 1)
    }
}
fn batch_import(
    rpc: &RpcClient,
    import_reqs: Vec<(Descriptor, u32, String, RescanSince)>,
) -> Result<()> {
    let results = rpc.import_multi(
        &import_reqs
            .iter()
            .map(
                |(descriptor, index, label, rescan_since)| ImportMultiRequest {
                    label: Some(&label),
                    watchonly: Some(true),
                    timestamp: *rescan_since,
                    range: Some((*index as usize, *index as usize)),
                    descriptor: Some(&descriptor.0),
                    ..Default::default()
                },
            )
            .collect::<Vec<_>>(),
        None,
    )?;

    for (i, result) in results.iter().enumerate() {
        if !result.success {
            let req = import_reqs.get(i).unwrap(); // should not fail unless bitcoind is messing with us
            bail!("import for {:?} failed: {:?}", req, result);
        } else if !result.warnings.is_empty() {
            debug!("import succeed with warnings: {:?}", result);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeyOrigin {
    Derived(DescrChecksum, u32),
    Standalone,
}

impl_string_serializer!(
    KeyOrigin,
    origin,
    match origin {
        KeyOrigin::Standalone => "standalone".into(),
        KeyOrigin::Derived(parent_descr_cs, index) => {
            format!("{}/{}", parent_descr_cs, index)
        }
    }
);

impl KeyOrigin {
    pub fn to_label(&self) -> String {
        match self {
            KeyOrigin::Derived(descr_cs, index) => {
                format!("{}/{}/{}", LABEL_PREFIX, descr_cs, index)
            }
            KeyOrigin::Standalone => LABEL_PREFIX.into(),
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(3, '/').collect();
        match (parts.get(0), parts.get(1), parts.get(2)) {
            (Some(&LABEL_PREFIX), Some(parent), Some(index)) => Some(KeyOrigin::Derived(
                DescrChecksum(parent.to_string()),
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

#[derive(Clone)]
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
        // TODO make extkeys seralize back to a string using their original version bytes

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
impl_debug_display!(XyzPubKey);

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

// show a specialzied error message for unsupported `listlabels` (added in Bitcoin Core 0.17.0)
fn labels_error(error: bitcoincore_rpc::Error) -> bitcoincore_rpc::Error {
    if let rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)) = error {
        // Method not found
        if e.code == -32601 {
            warn!("Your bitcoind node appears to be too old to support the labels API, which bwt relies on. \
                  Please upgrade your node. v0.19.0 is highly recommended, v0.17.0 is sufficient.");
        }
    }
    error
}

use serde::ser::SerializeStruct;

// Serialize the HDWallet struct with an additional virtual "origin" field
impl Serialize for HDWallet {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut rgb = serializer.serialize_struct("HDWallet", 3)?;
        rgb.serialize_field("descriptor", &self.descriptor.0)?;
        rgb.serialize_field("network", &self.network)?;
        rgb.serialize_field("gap_limit", &self.gap_limit)?;
        rgb.serialize_field("initial_import_size", &self.initial_import_size)?;
        rgb.serialize_field("rescan_policy", &self.rescan_policy)?;
        rgb.serialize_field("max_funded_index", &self.max_funded_index)?;
        rgb.serialize_field("max_imported_index", &self.max_imported_index)?;
        rgb.serialize_field("done_initial_import", &self.done_initial_import)?;
        rgb.end()
    }
}
