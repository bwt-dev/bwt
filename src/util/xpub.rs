use std::fmt;
use std::result::Result as StdResult;
use std::str::FromStr;

use bitcoin::util::bip32::{ChildNumber, DerivationPath, ExtendedPubKey, Fingerprint};
use bitcoin::{util::base58, Address, Network};
use miniscript::descriptor::{Descriptor, DescriptorPublicKey, DescriptorXPub};

use crate::types::ScriptType;
use crate::util::descriptor::ExtendedDescriptor;
use crate::util::{BoolThen, EC};

pub fn xpub_matches_network(xpub: &ExtendedPubKey, network: Network) -> bool {
    // testnet and regtest share the same bip32 version bytes
    xpub.network == network || (xpub.network == Network::Testnet && network == Network::Regtest)
}

/// An extended public key with an associated script type.
/// Used to represent SLIP 32 [xyz]pubs, as well as simple p2*pkh descriptors.
#[derive(Clone)]
pub struct XyzPubKey {
    pub network: Network,
    pub script_type: ScriptType,
    pub extended_pubkey: ExtendedPubKey,
}

impl_string_serializer!(XyzPubKey, xpub, xpub.extended_pubkey.to_string());
impl_debug_display!(XyzPubKey);

#[derive(Clone, Debug)]
pub struct Bip32Origin(pub Fingerprint, pub DerivationPath);

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

    /// Convert simple p2*pkh ranged descriptors to their XyzPubKey representation
    pub fn try_from_desc(desc: &ExtendedDescriptor) -> Option<Self> {
        let (script_type, desc_xpub) = match desc {
            Descriptor::Pkh(DescriptorPublicKey::XPub(xpub)) => (ScriptType::P2pkh, xpub),
            Descriptor::Wpkh(DescriptorPublicKey::XPub(xpub)) => (ScriptType::P2wpkh, xpub),
            Descriptor::ShWpkh(DescriptorPublicKey::XPub(xpub)) => (ScriptType::P2shP2wpkh, xpub),
            _ => return None,
        };

        if !desc_xpub.is_wildcard {
            return None;
        }

        Some(XyzPubKey {
            network: desc_xpub.xpub.network,
            script_type,
            extended_pubkey: desc_xpub
                .xpub
                .derive_pub(&*EC, &desc_xpub.derivation_path)
                .unwrap(),
        })
    }

    /// Get the address of the key at the specified derivation index
    /// Panics if given a hardened child number
    pub fn derive_address(&self, index: u32, network: Network) -> Address {
        let key = self.extended_pubkey.ckd_pub(&*EC, index.into()).unwrap();
        match self.script_type {
            ScriptType::P2pkh => Address::p2pkh(&key.public_key, network),
            ScriptType::P2wpkh => Address::p2wpkh(&key.public_key, network).unwrap(),
            ScriptType::P2shP2wpkh => Address::p2shwpkh(&key.public_key, network).unwrap(),
            // the two unwraps above can only fail if the public key is non-compressed, which it cannot be.
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
    fn from(ext_key: &ExtendedPubKey) -> Self {
        if ext_key.depth > 0 {
            Self(
                ext_key.parent_fingerprint,
                [ext_key.child_number][..].into(),
            )
        } else {
            Self(ext_key.fingerprint(), [][..].into())
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
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self)
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

#[cfg(test)]
mod tests {
    use super::*;

    // Test xyzpub -> descriptor -> xyzpub roundtrip
    #[test]
    fn test_xpub_to_desc_conversion() {
        let net = Network::Bitcoin;
        let test_cases = [
            // Standard BIP32 xpub, uses p2pkh
            ("xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC",
             "pkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*)"),

            // SLIP32 ypub, uses p2sh-p2wpkh
            ("ypub6QqdH2c5z7966e2a1ZAd7tpZRWNTu3xG7rNfHazDrjhAr9uT9iY9EPM6f4FyWceG9PWgHKPHd9JKu9BvAD5yJo1ajjVbxKB3dbCETvZ3Jzw",
             "sh(wpkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*))"),

            // SLIP32 zpub, uses p2wpkh
            ("zpub6jftahH18ngZwwDgquxFKyv4bUWuqfwm2xtt4yt7Ek53uFigQNhhrT1EgGDZWXJBZ2dV2nyr5oesnRoUsuVz72hBc5C2YDzXuKFsrTu7JHp",
             "wpkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*)"),
        ];
        for (xyz_str, expected_desc) in &test_cases {
            let xyzpub = xyz_str.parse::<XyzPubKey>().unwrap();
            let desc = xyzpub.as_descriptor([][..].into());
            let xyzpub_rt = XyzPubKey::try_from_desc(&desc).unwrap();

            assert_eq!(desc.to_string(), *expected_desc);
            assert_eq!(xyzpub_rt.extended_pubkey, xyzpub.extended_pubkey);
            assert_eq!(xyzpub_rt.script_type, xyzpub.script_type);

            let address = xyzpub.derive_address(9, net);
            assert_eq!(desc.derive(9.into()).address(net).unwrap(), address);
            assert_eq!(xyzpub_rt.derive_address(9, net), address);
        }
    }

    // Test descriptor -> xyzpub -> descriptor roundtrip
    #[test]
    fn test_desc_to_xpub_conversion() {
        let net = Network::Bitcoin;
        // Simple ranged p2*pkh descriptors that can be represented as as optimized XyzPubKey
        let test_cases =[
            // p2pkh
            ("pkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/*)",
             "xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC",
             ScriptType::P2pkh),

            // p2wpkh with xpub child derivation
            ("wpkh(xpub661MyMwAqRbcFLqTBCNzuoj4FYE1xRxmCjrSWC6LUjKHo46Du4NacKgxdrJPWhzLjkPsXqnjAUwn1raMSWfxWZKysPoBNQMZMs8b5JM8egC/0/*)",
             "xpub68VHDuZRhKBTDwzEiVPAL8gfPvkLQiUYsZ4W7PAT6LxPYchGuSXh7NQBL418maAsf89gZsDTntQVzPC37qmxd3qKvJMbAGCSV5eBjUwiPZk",
             ScriptType::P2wpkh),
        ];
        for (desc_str, expected_xpub, expected_type) in &test_cases {
            let desc = desc_str.parse::<ExtendedDescriptor>().unwrap();
            let xyzpub = XyzPubKey::try_from_desc(&desc).unwrap();
            let desc_rt = xyzpub.as_descriptor([][..].into());

            assert_eq!(xyzpub.extended_pubkey.to_string(), *expected_xpub);
            assert_eq!(xyzpub.script_type, *expected_type);

            let address = desc.derive(9.into()).address(net).unwrap();
            assert_eq!(xyzpub.derive_address(9, net), address);
            assert_eq!(desc_rt.derive(9.into()).address(net).unwrap(), address);
        }

        // Descriptors without an XyzPubKey representation
        let unoptimizable_descs = [
          "wsh(multi(1,tpubD6NzVbkrYhZ4XmWGpWP6vdR1uS1NVvgUgM3wFUzCywE8nupMQpmvBGBYzjcZfHX46xSCpBxmFSswJzE98vsL48hW5HsampQhRBnKUHin36y/*))",
          // non-ranged, no child derivation to optimize
          "pkh(tpubD6NzVbkrYhZ4XmWGpWP6vdR1uS1NVvgUgM3wFUzCywE8nupMQpmvBGBYzjcZfHX46xSCpBxmFSswJzE98vsL48hW5HsampQhRBnKUHin36y)",
          "pkh(021ebb0d349ccd72d3648c944c84e38345cf8d200dcf216cb624a0b869bbf974f0)",
        ];
        for desc_str in &unoptimizable_descs {
            assert!(XyzPubKey::try_from_desc(&desc_str.parse().unwrap()).is_none());
        }
    }
}
