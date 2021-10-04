use std::convert::TryInto;
use std::fmt;
use std::str::FromStr;

use serde::de;

use bitcoin::util::bip32::{ChildNumber, DerivationPath, ExtendedPubKey, Fingerprint};
use bitcoin::{util::base58, Network};
use miniscript::descriptor::{Descriptor, DescriptorPublicKey, DescriptorXKey, Wildcard};

use crate::types::ScriptType;
use crate::util::descriptor::ExtendedDescriptor;
use crate::util::BoolThen;

pub fn xpub_matches_network(xpub: &ExtendedPubKey, network: Network) -> bool {
    // testnet, signet and regtest share the same bip32 version bytes,
    // and therefore always identified as testnet.
    xpub.network == network
        || (xpub.network == Network::Testnet
            && matches!(network, Network::Regtest | Network::Signet))
}

/// An extended public key with an associated script type.
/// Used to represent SLIP 32 [xyz]pubs, as well as simple p2*pkh descriptors.
#[derive(Clone)]
pub struct XyzPubKey {
    script_type: ScriptType,
    xpub: ExtendedPubKey,
}

impl_string_serializer!(XyzPubKey, xyzpub, xyzpub.xpub.to_string());
impl_debug_display!(XyzPubKey);

#[derive(Clone, Debug)]
pub struct Bip32Origin(pub Fingerprint, pub DerivationPath);

impl XyzPubKey {
    pub fn as_descriptor(&self, derivation_path: DerivationPath) -> ExtendedDescriptor {
        let bip32_origin = (self.xpub.depth > 0).do_then(|| {
            (
                self.xpub.parent_fingerprint,
                [self.xpub.child_number][..].into(),
            )
        });

        let desc_key = DescriptorPublicKey::XPub(DescriptorXKey {
            origin: bip32_origin,
            xkey: self.xpub,
            derivation_path,
            wildcard: Wildcard::Unhardened,
        });

        match self.script_type {
            ScriptType::P2pkh => Descriptor::new_pkh(desc_key),
            ScriptType::P2wpkh => Descriptor::new_wpkh(desc_key).expect("no uncompressed"),
            ScriptType::P2shP2wpkh => Descriptor::new_sh_wpkh(desc_key).expect("no uncompressed"),
        }
    }
}

impl FromStr for XyzPubKey {
    type Err = crate::error::Error;

    fn from_str(inp: &str) -> Result<XyzPubKey, Self::Err> {
        let mut parts = inp.splitn(2, ":");
        let mut data = base58::from_check(parts.next().unwrap())?;

        if data.len() != 78 {
            bail!(base58::Error::InvalidLength(data.len()));
        }

        // rust-bitcoin's bip32 implementation does not support ypubs/zpubs.
        // instead, figure out the network and script type ourselves and feed rust-bitcoin with
        // a modified key that uses the version bytes it expects.

        let version = &data[0..4];
        let (network, mut script_type) = parse_xyz_version(version)?;
        data.splice(0..4, get_xpub_p2pkh_version(network).iter().cloned());

        let xpub = ExtendedPubKey::decode(&data)?;

        if let Some(explicit_type_str) = parts.next() {
            script_type = explicit_type_str.parse()?;
        }

        Ok(XyzPubKey { xpub, script_type })
    }
}

// Deserialize using the FromStr implementation
impl<'de> de::Deserialize<'de> for XyzPubKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl Bip32Origin {
    pub fn child(&self, cn: ChildNumber) -> Self {
        Self(self.0, self.1.child(cn))
    }

    pub fn extend<T: AsRef<[ChildNumber]>>(&self, path: T) -> Self {
        Self(self.0, self.1.extend(path))
    }
}
impl From<&(Fingerprint, DerivationPath)> for Bip32Origin {
    fn from(o: &(Fingerprint, DerivationPath)) -> Self {
        Self(o.0, o.1.clone())
    }
}
impl From<&ExtendedPubKey> for Bip32Origin {
    fn from(xpub: &ExtendedPubKey) -> Self {
        if xpub.depth > 0 {
            Self(xpub.parent_fingerprint, [xpub.child_number][..].into())
        } else {
            Self(xpub.fingerprint(), [][..].into())
        }
    }
}
impl fmt::Display for Bip32Origin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)?;
        for child in &self.1 {
            write!(f, "/{}", child)?;
        }
        Ok(())
    }
}
impl serde::Serialize for Bip32Origin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self)
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

        _ => {
            return Err(base58::Error::InvalidExtendedKeyVersion(
                version.try_into().unwrap(),
            ))
        }
    })
}

fn get_xpub_p2pkh_version(network: Network) -> [u8; 4] {
    match network {
        Network::Bitcoin => [0x04u8, 0x88, 0xB2, 0x1E],
        Network::Testnet | Network::Regtest | Network::Signet => [0x04u8, 0x35, 0x87, 0xCF],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xyzpub_to_desc() {
        let test_cases = [
            // Standard BIP32 xpub, uses p2pkh
            ("xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC",
             "pkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*)#l8pslygd"),

            // SLIP32 ypub, uses p2sh-p2wpkh
            ("ypub6QqdH2c5z7966e2a1ZAd7tpZRWNTu3xG7rNfHazDrjhAr9uT9iY9EPM6f4FyWceG9PWgHKPHd9JKu9BvAD5yJo1ajjVbxKB3dbCETvZ3Jzw",
             "sh(wpkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*))#s49cq0me"),

            // SLIP32 zpub, uses p2wpkh
            ("zpub6jftahH18ngZwwDgquxFKyv4bUWuqfwm2xtt4yt7Ek53uFigQNhhrT1EgGDZWXJBZ2dV2nyr5oesnRoUsuVz72hBc5C2YDzXuKFsrTu7JHp",
             "wpkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*)#pyf5ce6k"),
        ];
        for (xyz_str, expected_desc) in &test_cases {
            let xyzpub = xyz_str.parse::<XyzPubKey>().unwrap();
            let desc = xyzpub.as_descriptor([][..].into());

            assert_eq!(desc.to_string(), *expected_desc);
        }
    }
}
