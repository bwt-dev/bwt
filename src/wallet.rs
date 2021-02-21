use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;

use bitcoin::{Address, Network};
use bitcoincore_rpc::json::{ImportMultiRequest, ImportMultiRequestScriptPubkey};
use bitcoincore_rpc::{self as rpc, Client as RpcClient, RpcApi};
use miniscript::DescriptorTrait;

use crate::error::{Context, Result};
use crate::store::MemoryStore;
use crate::types::RescanSince;
use crate::util::descriptor::{self, Checksum, DescKeyInfo, DescriptorExt, ExtendedDescriptor};
use crate::util::xpub::{Bip32Origin, XyzPubKey};
use crate::util::RpcApiExt;
use crate::Config;

const LABEL_PREFIX: &str = "bwt";

#[derive(Debug)]
pub struct WalletWatcher {
    network: Network,
    /// Descriptor wallets
    wallets: HashMap<Checksum, Wallet>,
    /// Standalone addresses pending import
    pending_standalone: Vec<AddressImport>,
    /// Force rescan on the first run
    force_rescan: bool,
}

type AddressImport = (Address, RescanSince);

impl WalletWatcher {
    pub fn new(
        network: Network,
        wallets: Vec<Wallet>,
        addresses: Vec<AddressImport>,
        force_rescan: bool,
    ) -> Result<Self> {
        let num_wallets = wallets.len();
        let wallets = wallets
            .into_iter()
            .map(|wallet| (wallet.checksum.clone(), wallet))
            .collect::<HashMap<_, _>>();
        ensure!(
            wallets.len() == num_wallets,
            "Descriptor checksum collision detected"
        );

        for (address, _) in &addresses {
            ensure!(
                address.network == network,
                "Invalid network for address {}",
                address
            );
        }

        Ok(Self {
            network,
            wallets,
            pending_standalone: addresses,
            force_rescan,
        })
    }

    pub fn from_config(config: &Config) -> Result<Self> {
        let mut wallets = vec![];
        for desc in &config.descriptors {
            wallets.push(
                Wallet::from_descriptor(
                    desc.clone(),
                    config.network,
                    config.gap_limit,
                    config.initial_import_size,
                    config.rescan_since,
                )
                .with_context(|| format!("invalid descriptor {}", desc))?,
            );
        }
        for xpub in &config.xpubs {
            // each xpub results in two wallets
            wallets.append(
                &mut Wallet::from_xpub(
                    xpub.clone(),
                    config.network,
                    config.gap_limit,
                    config.initial_import_size,
                    config.rescan_since,
                )
                .with_context(|| format!("invalid xpub {}", xpub))?,
            );
        }

        let addresses = config
            .addresses()?
            .into_iter()
            .map(|address| (address, config.rescan_since))
            .collect::<Vec<_>>();

        if wallets.is_empty() && addresses.is_empty() {
            if config.require_addresses {
                error!("Please provide at least one descriptors/xpubs/addresses to track (via --descriptor, --xpub or --address).");
                bail!("No descriptors/xpubs/addresses provided");
            } else {
                warn!("No descriptors, xpubs or addresses were provided. No wallet activity will be available.");
            }
        }

        Self::new(config.network, wallets, addresses, config.force_rescan)
    }

    pub fn wallets(&self) -> &HashMap<Checksum, Wallet> {
        &self.wallets
    }

    pub fn get(&self, checksum: &Checksum) -> Option<&Wallet> {
        self.wallets.get(checksum)
    }

    // Mark an address as funded
    pub fn mark_funded(&mut self, origin: &KeyOrigin) {
        if let KeyOrigin::Descriptor(checksum, index) = origin {
            if let Some(wallet) = self.wallets.get_mut(checksum) {
                if wallet.max_imported_index.map_or(true, |max| *index > max) {
                    wallet.max_imported_index = Some(*index);
                }

                if wallet.max_funded_index.map_or(true, |max| *index > max) {
                    wallet.max_funded_index = Some(*index);
                }
            }
        }
    }

    /// Check previous imports and update our state
    pub fn check_imports(&mut self, rpc: &RpcClient) -> Result<()> {
        debug!("checking previous imports");

        // Lookup descriptor wallet imports and update their max_imported_index
        let mut imported_indexes: HashMap<Checksum, u32> = HashMap::new();
        let labels = rpc.list_labels().map_err(labels_error)?;
        for label in labels {
            if let Some(KeyOrigin::Descriptor(checksum, index)) = KeyOrigin::from_label(&label) {
                if self.wallets.contains_key(&checksum) {
                    imported_indexes
                        .entry(checksum)
                        .and_modify(|current| *current = (*current).max(index))
                        .or_insert(index);
                }
            }
        }
        for (checksum, max_imported_index) in imported_indexes {
            trace!(
                "wallet {} was imported up to index {}",
                checksum,
                max_imported_index
            );
            let wallet = self.wallets.get_mut(&checksum).unwrap();
            wallet.max_imported_index = Some(max_imported_index);
        }

        // Lookup previously imported standalone addresses and remove them from the pending import queue
        if !self.force_rescan {
            let standalones = rpc
                .get_addresses_by_label(KeyOrigin::standalone_label())?
                .into_iter()
                .map(|(k, _)| k)
                .collect::<HashSet<_>>();
            trace!(
                "found {} previously imported standalone addresses",
                standalones.len()
            );
            self.pending_standalone
                .retain(|(address, _)| !standalones.contains(address));
        }

        Ok(())
    }

    pub fn do_imports(&mut self, rpc: &RpcClient, rescan: bool) -> Result<bool> {
        let mut import_reqs = vec![];
        let mut pending_updates = vec![];

        for (_, wallet) in self.wallets.iter_mut() {
            if self.force_rescan || wallet.needs_imports() {
                let start_index = iif!(self.force_rescan, 0, wallet.import_start_index());
                let end_index = wallet.import_end_index(rescan);

                import_reqs.append(&mut wallet.make_imports(start_index, end_index, rescan));

                pending_updates.push((wallet, end_index));
            }
        }

        if !self.pending_standalone.is_empty() {
            let label = KeyOrigin::standalone_label();
            import_reqs.extend(
                self.pending_standalone
                    .iter()
                    .cloned()
                    .map(|(address, rescan)| (address, rescan, label.into())),
            );
        }

        let has_imports = !import_reqs.is_empty();

        if has_imports {
            info!(
                "importing batch of {} addresses... (this may take awhile)",
                import_reqs.len()
            );
            batch_import(rpc, import_reqs)?;
            debug!("done importing batch");

            for (wallet, imported_index) in pending_updates {
                wallet.max_imported_index = Some(imported_index);
            }

            // the force_rescan flag applies to the first import batch only
            self.force_rescan = false;

            // we don't need to keep standalone addresses around once they get imported
            self.pending_standalone.clear();
        }

        Ok(has_imports)
    }

    /// Add an address to be tracked
    ///
    /// The address will be added to the list of pending imports and will get imported on the next sync run.
    pub fn track_address(&mut self, address: Address, rescan_since: RescanSince) -> Result<()> {
        ensure!(
            address.network == self.network,
            "Invalid network for address {}",
            address
        );
        self.pending_standalone.push((address, rescan_since));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Wallet {
    desc: ExtendedDescriptor,
    is_wildcard: bool,
    checksum: Checksum,
    keys_info: Vec<DescKeyInfo>,
    network: Network,
    rescan_since: RescanSince,
    gap_limit: u32,
    initial_import_size: u32,

    max_funded_index: Option<u32>,
    max_imported_index: Option<u32>,
}

impl Wallet {
    pub fn from_descriptor(
        desc: ExtendedDescriptor,
        network: Network,
        gap_limit: u32,
        initial_import_size: u32,
        rescan_since: RescanSince,
    ) -> Result<Self> {
        ensure!(
            descriptor::derive_address(&desc, 0, network).is_some(),
            "Invalid descriptor, hardended derivation or no address representation: `{}`",
            desc
        );

        let checksum = desc.checksum();
        let keys_info = DescKeyInfo::extract(&desc, network)?;
        let is_wildcard = keys_info.iter().any(|x| x.is_wildcard);

        Ok(Self {
            desc,
            checksum,
            keys_info,
            is_wildcard,
            network,
            gap_limit,
            // setting initial_import_size < gap_limit makes no sense, the user probably meant to increase both
            initial_import_size: initial_import_size.max(gap_limit),
            rescan_since,
            max_funded_index: None,
            max_imported_index: None,
        })
    }

    pub fn from_xpub(
        xpub: XyzPubKey,
        network: Network,
        gap_limit: u32,
        initial_import_size: u32,
        rescan_since: RescanSince,
    ) -> Result<Vec<Self>> {
        Ok(vec![
            // external chain (receive)
            Self::from_descriptor(
                xpub.as_descriptor([0.into()][..].into()),
                network,
                gap_limit,
                initial_import_size,
                rescan_since,
            )?,
            // internal chain (change)
            Self::from_descriptor(
                xpub.as_descriptor([1.into()][..].into()),
                network,
                gap_limit,
                initial_import_size,
                rescan_since,
            )?,
        ])
    }

    fn needs_imports(&self) -> bool {
        if !self.is_wildcard {
            return self.max_imported_index.is_none();
        }

        self.max_imported_index.map_or(true, |imported| {
            self.max_funded_index
                .map_or(false, |funded| imported - funded < self.gap_limit)
        })
    }

    /// Returns the start index that needs to be imported
    fn import_start_index(&self) -> u32 {
        self.max_imported_index
            .map_or(0, |max_imported| max_imported + 1)
    }

    /// Returns the maximum index to be imported
    fn import_end_index(&self, is_rescan: bool) -> u32 {
        if !self.is_wildcard {
            return 0;
        }

        // use larger chunk size during the initial rescan
        let chunk_size = iif!(is_rescan, self.initial_import_size, self.gap_limit);

        self.max_funded_index
            .map_or(chunk_size - 1, |max| max + chunk_size)
            // the current max_imported_index may be larger due to a previously larger chunk size
            .max(self.max_imported_index.unwrap_or(0))
    }

    fn make_imports(
        &self,
        start_index: u32,
        end_index: u32,
        rescan: bool,
    ) -> Vec<(Address, RescanSince, String)> {
        let rescan_since = iif!(rescan, self.rescan_since, RescanSince::Now);

        debug!(
            "importing {} range {}-{} ({} rescan)",
            self.checksum,
            start_index,
            end_index,
            iif!(rescan_since == RescanSince::Now, "without", "with"),
        );

        (start_index..=end_index)
            .map(|index| {
                let address = self.derive_address(index);
                let origin = KeyOrigin::Descriptor(self.checksum.clone(), index);
                (address, rescan_since, origin.to_label())
            })
            .collect()
    }

    /// Derive an address using the given non-hardended child derivation index.
    /// Assumes the index is valid and panics if not. See is_valid_index().
    pub fn derive_address(&self, index: u32) -> Address {
        descriptor::derive_address(&self.desc, index, self.network)
            .expect("constructed Wallet must have address representation")
    }

    pub fn derive_desc(&self, index: u32) -> ExtendedDescriptor {
        self.desc.derive(index)
    }

    pub fn get_next_index(&self) -> u32 {
        if self.is_wildcard {
            self.max_funded_index
                .map_or(0, |max_funded_index| max_funded_index + 1)
        } else {
            0
        }
    }

    pub fn is_valid_index(&self, index: u32) -> bool {
        if self.is_wildcard {
            // non-hardended derivation only
            index & (1 << 31) == 0
        } else {
            index == 0
        }
    }

    pub fn find_gap(&self, store: &MemoryStore) -> Option<usize> {
        // return None if this wallet has no history at all
        let max_funded_index = self.max_funded_index?;

        Some(if self.is_wildcard {
            (0..=max_funded_index)
                .map(|derivation_index| self.derive_address(derivation_index))
                .fold((0, 0), |(curr_gap, max_gap), address| {
                    if store.has_history(&address.into()) {
                        (0, curr_gap.max(max_gap))
                    } else {
                        (curr_gap + 1, max_gap)
                    }
                })
                .1
        } else {
            0
        })
    }

    /// Get the bip32 origins of the public keys used at the provided index
    pub fn bip32_origins(&self, index: u32) -> Vec<Bip32Origin> {
        self.keys_info
            .iter()
            .map(|i| {
                if i.is_wildcard {
                    i.bip32_origin.child(index.into())
                } else {
                    i.bip32_origin.clone()
                }
            })
            .collect()
    }
}

fn batch_import(rpc: &RpcClient, import_reqs: Vec<(Address, RescanSince, String)>) -> Result<()> {
    // XXX use importmulti with ranged descriptors? the key derivation info won't be
    //     directly available on `listtransactions` and would require an additional rpc all.

    let results = rpc.import_multi(
        &import_reqs
            .iter()
            .map(|(address, rescan, label)| ImportMultiRequest {
                label: Some(&label),
                watchonly: Some(true),
                timestamp: (*rescan).into(),
                script_pubkey: Some(ImportMultiRequestScriptPubkey::Address(&address)),
                ..Default::default()
            })
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
    Descriptor(Checksum, u32),
    Standalone,
}

impl_string_serializer!(
    KeyOrigin,
    origin,
    match origin {
        KeyOrigin::Standalone => "standalone".into(),
        KeyOrigin::Descriptor(checksum, index) => {
            format!("{}/{}", checksum, index)
        }
    }
);

impl KeyOrigin {
    pub fn to_label(&self) -> String {
        match self {
            KeyOrigin::Descriptor(checksum, index) => {
                format!("{}/{}/{}", LABEL_PREFIX, checksum, index)
            }
            KeyOrigin::Standalone => LABEL_PREFIX.into(),
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.splitn(3, '/').collect();
        match (parts.get(0), parts.get(1), parts.get(2)) {
            (Some(&LABEL_PREFIX), Some(parent), Some(index)) => Some(KeyOrigin::Descriptor(
                parent.parse().ok()?,
                index.parse().ok()?,
            )),
            (Some(&LABEL_PREFIX), None, None) => Some(KeyOrigin::Standalone),
            _ => None,
        }
    }

    pub fn is_standalone(origin: &KeyOrigin) -> bool {
        match origin {
            KeyOrigin::Standalone => true,
            KeyOrigin::Descriptor(..) => false,
        }
    }

    pub fn standalone_label() -> &'static str {
        LABEL_PREFIX
    }
}

// show a specialzied error message for unsupported `listlabels` (added in Bitcoin Core 0.17.0)
fn labels_error(error: rpc::Error) -> bitcoincore_rpc::Error {
    use crate::util::bitcoincore_ext::RPC_METHOD_NOT_FOUND;
    if let rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)) = error {
        if e.code == RPC_METHOD_NOT_FOUND {
            warn!("Your bitcoind node appears to be too old to support the labels API, which bwt relies on. \
                  Please upgrade your node. v0.19.0 is highly recommended, v0.17.0 is sufficient.");
        }
    }
    error
}

use serde::ser::SerializeStruct;

impl Serialize for Wallet {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let bip32_origins: Vec<_> = self.keys_info.iter().map(|i| &i.bip32_origin).collect();

        let mut rgb = serializer.serialize_struct("Wallet", 3)?;

        rgb.serialize_field("desc", &self.desc)?;
        rgb.serialize_field("network", &self.network)?;
        rgb.serialize_field("is_wildcard", &self.is_wildcard)?;
        rgb.serialize_field("bip32_origins", &bip32_origins)?;
        rgb.serialize_field("rescan_since", &self.rescan_since)?;
        rgb.serialize_field("max_funded_index", &self.max_funded_index)?;
        rgb.serialize_field("max_imported_index", &self.max_imported_index)?;
        rgb.serialize_field(
            "satisfaction_weight",
            &self.desc.max_satisfaction_weight().ok(),
        )?;

        if self.is_wildcard {
            rgb.serialize_field("gap_limit", &self.gap_limit)?;
            rgb.serialize_field("initial_import_size", &self.initial_import_size)?;
        }

        rgb.end()
    }
}
