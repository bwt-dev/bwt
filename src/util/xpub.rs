use std::result::Result as StdResult;
use std::str::FromStr;

use bitcoin::util::bip32::{DerivationPath, ExtendedPubKey, Fingerprint};
use bitcoin::{util::base58, Network};
use miniscript::descriptor::{Descriptor, DescriptorPublicKey, DescriptorXPub};

use crate::types::ScriptType;
use crate::util::descriptor::ExtendedDescriptor;
use crate::util::BoolThen;

pub fn xpub_matches_network(xpub: &ExtendedPubKey, network: Network) -> bool {
    // testnet and regtest share the same bip32 version bytes
    xpub.network == network || (xpub.network == Network::Testnet && network == Network::Regtest)
}

#[derive(Clone)]
pub struct XyzPubKey {
    pub network: Network,
    pub script_type: ScriptType,
    pub extended_pubkey: ExtendedPubKey,
}

impl_string_serializer!(XyzPubKey, xpub, xpub.extended_pubkey.to_string());
impl_debug_display!(XyzPubKey);

#[derive(Clone, Debug)]
pub struct Bip32Origin(pub Fingerprint, pub u32);
impl_string_serializer!(Bip32Origin, o, format!("{}/{}", o.0, o.1));

impl XyzPubKey {
    pub fn as_descriptor(&self, derivation_path: DerivationPath) -> ExtendedDescriptor {
        let bip32_origin = (self.extended_pubkey.depth > 0).do_then(|| {
            (
                self.extended_pubkey.parent_fingerprint,
                [self.extended_pubkey.child_number][..].into(),
            )
        });

        let desc_key = DescriptorPublicKey::XPub(DescriptorXPub {
            origin: bip32_origin,
            xpub: self.extended_pubkey,
            derivation_path,
            is_wildcard: true,
        });

        match self.script_type {
            ScriptType::P2pkh => Descriptor::Pkh(desc_key),
            ScriptType::P2wpkh => Descriptor::Wpkh(desc_key),
            ScriptType::P2shP2wpkh => Descriptor::ShWpkh(desc_key),
        }
    }
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
        let extended_pubkey = faux_xpub.parse()?;

        Ok(XyzPubKey {
            network,
            script_type,
            extended_pubkey,
        })
    }
}

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
