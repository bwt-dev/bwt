use std::cmp;
use std::collections::HashMap;
use std::str::FromStr;

use bitcoin::util::bip32::{ChildNumber, ExtendedPubKey, Fingerprint, ScriptType};
use bitcoin::Address;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};
use hex;
use secp256k1::Secp256k1;
use serde_json::Value;

use crate::addrman::KeyRescan;
use crate::error::Result;

const LABEL_PREFIX: &str = "rust_eps";

lazy_static! {
    static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
}

pub struct HDWatcher {
    wallets: HashMap<Fingerprint, HDWallet>,
}

impl HDWatcher {
    pub fn new(wallets: Vec<HDWallet>) -> Self {
        let wallets = wallets
            .into_iter()
            .map(|wallet| (wallet.master.fingerprint(), wallet))
            .collect();
        HDWatcher { wallets }
    }

    pub fn mark_used(&mut self, derivation: &DerivationInfo) {
        if let DerivationInfo::Derived(parent_fingerprint, index) = derivation {
            if let Some(wallet) = self.wallets.get_mut(&parent_fingerprint) {
                wallet.max_used_index = cmp::max(wallet.max_used_index, *index);
                // if its used, its necessarily imported
                wallet.max_imported_index =
                    cmp::max(wallet.max_imported_index, wallet.max_used_index);
            }
        }
    }

    pub fn import_addresses(&mut self, rpc: &RpcClient) -> Result<()> {
        for (_, wallet) in &mut self.wallets {
            wallet.import_addresses(rpc)?
        }
        Ok(())
    }
}

pub struct HDWallet {
    master: ExtendedPubKey,
    buffer_size: u32,

    max_used_index: u32,
    max_imported_index: u32,
}

impl HDWallet {
    pub fn new(master: ExtendedPubKey) -> Self {
        Self {
            master,
            buffer_size: 50, // TODO configurable
            max_used_index: 0,
            // FIXME: index 0 is skipped, change to "next_import_index"
            max_imported_index: 0,
        }
    }

    pub fn from_xpub(s: &str) -> Result<Vec<Self>> {
        let key = ExtendedPubKey::from_str(s)?;
        // XXX verify key network type

        let receive = ChildNumber::from_normal_idx(0).unwrap();
        let change = ChildNumber::from_normal_idx(1).unwrap();

        Ok(vec![
            Self::new(key.derive_pub(&*EC, &[receive])?),
            Self::new(key.derive_pub(&*EC, &[change])?),
        ])
    }

    pub fn derive(&self, index: u32) -> ExtendedPubKey {
        let child = ChildNumber::from_normal_idx(index).expect("invalid derivation index");
        self.master
            .derive_pub(&*EC, &[child])
            .expect("failed key derivation")
    }

    fn import_addresses(&mut self, rpc: &RpcClient) -> Result<()> {
        if self.max_imported_index - self.max_used_index < self.buffer_size {
            // TODO set KeyRescan to the wallet's creation time during the initial sync,
            //      then to None for ongoing use
            self.import_range(
                rpc,
                self.max_imported_index + 1,
                self.buffer_size,
                KeyRescan::None,
            )
        } else {
            Ok(())
        }
    }

    fn import_range(
        &mut self,
        rpc: &RpcClient,
        start_index: u32,
        len: u32,
        rescan: KeyRescan,
    ) -> Result<()> {
        info!(
            "importing hd key {:?} range {} - {}",
            self.master,
            start_index,
            start_index + len
        );

        let import_reqs = (start_index..start_index + len)
            .map(|index| {
                let key = self.derive(index);
                let address = to_address(&key);
                let deriviation = DerivationInfo::Derived(key.parent_fingerprint, index);
                (address, rescan, deriviation)
            })
            .collect();

        batch_import(rpc, import_reqs)?;

        info!("done importing for {:?}", self.master);

        self.max_imported_index = cmp::max(self.max_imported_index, start_index + len);

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
    info!("importing to bitcoind: {:#?}", import_reqs);

    // TODO: parse result, detect errors
    Ok(rpc.call(
        "importmulti",
        &[json!(import_reqs
            .into_iter()
            .map(|(address, rescan, derivation)| {
                json!({
                  "scriptPubKey": { "address": address },
                  "timestamp": rescan.rpc_arg(),
                  "label": derivation.to_label(),
                })
            })
            .map(|x| {
                info!("importing: {:?}", x);
                x
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
            DerivationInfo::Derived(parent, index) => format!(
                "{}/{}/{}",
                LABEL_PREFIX,
                hex::encode(parent.as_bytes()),
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
                Fingerprint::from(&hex::decode(parent)?[0..4]),
                index.parse()?,
            ),
            _ => DerivationInfo::Standalone,
        })
    }
}
