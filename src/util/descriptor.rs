use std::iter::FromIterator;
use std::str::FromStr;

use bitcoin::Network;
use miniscript::descriptor::{Descriptor, DescriptorPublicKey};

use crate::error::{Error, OptionExt, Result};
use crate::util::xpub::{xpub_matches_network, Bip32Origin};

pub type ExtendedDescriptor = Descriptor<DescriptorPublicKey>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Checksum(String);

impl_string_serializer!(Checksum, c, c.0);

#[derive(Debug, Clone)]
pub struct DescKeyInfo {
    pub bip32_origin: Bip32Origin,
    pub is_ranged: bool,
}

impl From<&ExtendedDescriptor> for Checksum {
    fn from(desc: &ExtendedDescriptor) -> Self {
        get_checksum(desc)
    }
}

impl FromStr for Checksum {
    type Err = Error;

    fn from_str(inp: &str) -> Result<Self> {
        ensure!(inp.len() == 8, "Invalid descriptor checksum length");
        for ch in inp.chars() {
            CHECKSUM_CHARSET
                .find(ch)
                .or_err("Invalid descriptor checksum character")?;
        }
        Ok(Checksum(inp.into()))
    }
}

impl DescKeyInfo {
    pub fn extract(desc: &ExtendedDescriptor, network: Network) -> Result<Vec<DescKeyInfo>> {
        let mut valid_networks = true;
        let mut keys_info = vec![];

        tap_desc_pks(desc, |pk| match pk {
            DescriptorPublicKey::XPub(desc_xpub) => {
                // Get key origin information from the descriptor, fallback to extracting from the
                // xpub itself.
                let bip32_origin = desc_xpub
                    .origin
                    .as_ref()
                    .map_or_else(|| (&desc_xpub.xpub).into(), Into::<Bip32Origin>::into)
                    .extend(&desc_xpub.derivation_path);

                keys_info.push(DescKeyInfo {
                    bip32_origin,
                    is_ranged: desc_xpub.is_wildcard,
                });

                valid_networks = valid_networks && xpub_matches_network(&desc_xpub.xpub, network);
            }
            DescriptorPublicKey::SinglePub(desc_single) => {
                if let Some(bip32_origin) = &desc_single.origin {
                    keys_info.push(DescKeyInfo {
                        bip32_origin: bip32_origin.into(),
                        is_ranged: false,
                    });
                }
            }
        });

        ensure!(
            valid_networks,
            "Descriptor xpubs do not match the configured network {}",
            network
        );

        Ok(keys_info)
    }
}

pub trait DescriptorChecksum: Sized {
    /// Encode to string with the `#checksum` suffix
    fn to_string_with_checksum(&self) -> String;
    /// Parse a descriptor with an optional checksum suffix
    fn parse_with_checksum(s: &str) -> Result<Self>;
}

impl DescriptorChecksum for ExtendedDescriptor {
    fn to_string_with_checksum(&self) -> String {
        format!("{}#{}", self, get_checksum(&self))
    }

    fn parse_with_checksum(s: &str) -> Result<ExtendedDescriptor> {
        let parts: Vec<&str> = s.splitn(2, '#').collect();
        if parts.len() == 2 {
            let desc_str = parts[0];
            let desc = desc_str.parse::<ExtendedDescriptor>()?;
            let provided_checksum = parts[1].parse::<Checksum>()?;

            // FIXME using canonical encoding should not be required, but the current implementation
            // won't retain the checsum if the descriptor is encoded differently by rust-miniscript,
            // which would result in an unexpected behaviour.
            ensure!(
            desc.to_string() == desc_str,
            "Descriptors with explicit checksums must use canonical encoding. {} is expected to be encoded as `{}`",
            provided_checksum,
            desc.to_string()
        );

            let actual_checksum = get_checksum(&desc);
            ensure!(
                provided_checksum == actual_checksum,
                "Invalid descriptor checksum {}, expected {}",
                provided_checksum,
                actual_checksum,
            );
            Ok(desc)
        } else {
            Ok(s.parse()?)
        }
    }
}

fn tap_desc_pks<F>(desc: &ExtendedDescriptor, mut tap_fn: F)
where
    F: FnMut(&DescriptorPublicKey),
{
    // TODO this shouldn't call translate_pk() twice. this is tricky because both closure
    // arguments (Fpk/Fpkh) require a mutable borrow on `tap_fn`.

    desc.translate_pk(
        |pk| {
            tap_fn(pk);
            Ok::<DescriptorPublicKey, ()>(pk.clone())
        },
        |pk| Ok::<DescriptorPublicKey, ()>(pk.clone()),
    )
    .unwrap();

    desc.translate_pk(
        |pk| Ok::<DescriptorPublicKey, ()>(pk.clone()),
        |pk| {
            tap_fn(pk);
            Ok::<DescriptorPublicKey, ()>(pk.clone())
        },
    )
    .unwrap();
}

// Checksum code copied from https://github.com/bitcoindevkit/bdk/blob/master/src/descriptor/checksum.rs

const INPUT_CHARSET: &str =  "0123456789()[],'/*abcdefgh@:$%{}IJKLMNOPQRSTUVWXYZ&+-.;<=>?!^_|~ijklmnopqrstuvwxyzABCDEFGH`#\"\\ ";
const CHECKSUM_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// Compute the checksum of a descriptor
fn get_checksum(desc: &ExtendedDescriptor) -> Checksum {
    let desc_str = desc.to_string();
    let mut c = 1;
    let mut cls = 0;
    let mut clscount = 0;
    for ch in desc_str.chars() {
        let pos = INPUT_CHARSET
            .find(ch)
            .expect("ExtendedDescriptor's encoding cannot be invalid") as u64;
        c = poly_mod(c, pos & 31);
        cls = cls * 3 + (pos >> 5);
        clscount += 1;
        if clscount == 3 {
            c = poly_mod(c, cls);
            cls = 0;
            clscount = 0;
        }
    }
    if clscount > 0 {
        c = poly_mod(c, cls);
    }
    (0..8).for_each(|_| c = poly_mod(c, 0));
    c ^= 1;

    let mut chars = Vec::with_capacity(8);
    for j in 0..8 {
        chars.push(
            CHECKSUM_CHARSET
                .chars()
                .nth(((c >> (5 * (7 - j))) & 31) as usize)
                .unwrap(),
        );
    }

    Checksum(String::from_iter(chars))
}

fn poly_mod(mut c: u64, val: u64) -> u64 {
    let c0 = c >> 35;
    c = ((c & 0x7ffffffff) << 5) ^ val;
    if c0 & 1 > 0 {
        c ^= 0xf5dee51989
    };
    if c0 & 2 > 0 {
        c ^= 0xa9fdca3312
    };
    if c0 & 4 > 0 {
        c ^= 0x1bab10e32d
    };
    if c0 & 8 > 0 {
        c ^= 0x3706b1677a
    };
    if c0 & 16 > 0 {
        c ^= 0x644d626ffd
    };

    c
}
