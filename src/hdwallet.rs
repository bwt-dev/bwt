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
            if let Some(wallet) = self.wallets.get_mut(&parent_fingerprint) {
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
        for (_, wallet) in &mut self.wallets {
            wallet.do_imports(rpc)?
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct HDWallet {
    master: ExtendedPubKey,
    buffer_size: u32,
    initial_import_size: u32,

    max_used_index: Option<u32>,
    max_imported_index: Option<u32>,
}

// TODO figure out the imported indexes, either with listreceivedbyaddress (lots of data)
// or using getaddressesbylabel and a binary search (lots of requests)

impl HDWallet {
    pub fn new(master: ExtendedPubKey) -> Self {
        Self {
            master,
            buffer_size: 20,          // TODO configurable
            initial_import_size: 100, // TODO configurable
            max_used_index: None,
            max_imported_index: None,
        }
    }

    pub fn from_xpub(s: &str) -> Result<Vec<Self>> {
        let key = ExtendedPubKey::from_str(s)?;
        // XXX verify key network type

        Ok(vec![
            // receive account
            Self::new(key.derive_pub(&*EC, &[ChildNumber::from(0)])?),
            // change account
            Self::new(key.derive_pub(&*EC, &[ChildNumber::from(1)])?),
        ])
    }

    fn derive(&self, index: u32) -> ExtendedPubKey {
        self.master
            .derive_pub(&*EC, &[ChildNumber::from(index)])
            .unwrap()
    }

    fn do_imports(&mut self, rpc: &RpcClient) -> Result<()> {
        match (self.max_imported_index, self.max_used_index) {
            // nothing is imported yet, begin with initial_import_size
            (None, _) => self.import_range(rpc, 0, self.initial_import_size, KeyRescan::None),

            // we have imported and used addresses, extend the buffer as needed
            (Some(max_imported_index), Some(max_used_index))
                if max_imported_index < max_used_index + self.buffer_size =>
            {
                self.import_range(
                    rpc,
                    max_imported_index + 1,
                    max_used_index + self.buffer_size,
                    KeyRescan::None,
                )
            }

            // we're all good!
            _ => Ok(()),
        }
    }

    fn import_range(
        &mut self,
        rpc: &RpcClient,
        start_index: u32,
        end_index: u32,
        rescan: KeyRescan,
    ) -> Result<()> {
        assert!(end_index > start_index);

        info!(
            "importing hd key {} range {}-{}",
            self.master, start_index, end_index,
        );

        let import_reqs = (start_index..end_index)
            .map(|index| {
                let key = self.derive(index);
                let address = to_address(&key);
                let deriviation = DerivationInfo::Derived(key.parent_fingerprint, index);
                (address, rescan, deriviation)
            })
            .collect();

        batch_import(rpc, import_reqs)?;

        info!("done importing hd key {} up to {}", self.master, end_index);

        self.max_imported_index = Some(end_index);

        Ok(())
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
