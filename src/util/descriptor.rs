use std::convert::TryFrom;
use std::iter::FromIterator;
use std::str::FromStr;

use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::util::bip32::Fingerprint;
use bitcoin::Network;
use miniscript::descriptor::{Descriptor, DescriptorPublicKey};

use crate::error::{Error, OptionExt, Result};
use crate::util::xpub::xpub_matches_network;

lazy_static! {
    static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
}

pub type ExtendedDescriptor = Descriptor<DescriptorPublicKey>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Checksum(String);

impl_string_serializer!(Checksum, c, c.0);

#[derive(Debug, Clone)]
pub struct DescXPubInfo {
    pub fingerprint: Fingerprint,
    pub ranged: bool,
}

impl TryFrom<&ExtendedDescriptor> for Checksum {
    type Error = Error;
    fn try_from(desc: &ExtendedDescriptor) -> Result<Self> {
        get_checksum(desc)
    }
}

impl FromStr for Checksum {
    type Err = ();

    fn from_str(inp: &str) -> Result<Self, ()> {
        Ok(Checksum(inp.into()))
    }
}

impl DescXPubInfo {
    pub fn extract(desc: &ExtendedDescriptor, network: Network) -> Result<Vec<DescXPubInfo>> {
        let mut valid_networks = true;
        let mut xpubs_info = vec![];
        tap_desc_pks(desc, |pk| {
            if let DescriptorPublicKey::XPub(desc_xpub) = pk {
                valid_networks = valid_networks && xpub_matches_network(&desc_xpub.xpub, network);
                let final_xpub = desc_xpub
                    .xpub
                    .derive_pub(&EC, &desc_xpub.derivation_path)
                    .unwrap();
                xpubs_info.push(DescXPubInfo {
                    fingerprint: final_xpub.fingerprint(),
                    ranged: desc_xpub.is_wildcard,
                });
            }
        });
        ensure!(
            valid_networks,
            "descriptor xpub does not match the configured network"
        );
        Ok(xpubs_info)
    }
}

fn tap_desc_pks<F>(desc: &ExtendedDescriptor, mut tap_fn: F)
where
    F: FnMut(&DescriptorPublicKey),
{
    // TODO don't call translate_pk() twice. this is tricky because both the closure arguments
    // require a mutable borrow on `tap_fn`

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
pub fn get_checksum(desc: &ExtendedDescriptor) -> Result<Checksum> {
    let desc_str = desc.to_string();
    let mut c = 1;
    let mut cls = 0;
    let mut clscount = 0;
    for ch in desc_str.chars() {
        let pos = INPUT_CHARSET
            .find(ch)
            .or_err("Invalid descriptor character")? as u64;
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

    Ok(Checksum(String::from_iter(chars)))
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
