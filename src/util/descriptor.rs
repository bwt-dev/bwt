use std::str::FromStr;

use bitcoin::secp256k1::{self, Secp256k1};
use bitcoin::{Address, Network};
use miniscript::descriptor::{Descriptor, DescriptorPublicKey, DescriptorTrait, Wildcard};
use miniscript::{ForEachKey, TranslatePk2};

use crate::error::{Error, OptionExt, Result};
use crate::util::xpub::{xpub_matches_network, Bip32Origin};

pub type ExtendedDescriptor = Descriptor<DescriptorPublicKey>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Checksum(String);

impl_string_serializer!(Checksum, c, c.0);

/// Derive the address at `index`
pub fn derive_address(desc: &ExtendedDescriptor, index: u32, network: Network) -> Option<Address> {
    lazy_static! {
        pub static ref EC: Secp256k1<secp256k1::VerifyOnly> = Secp256k1::verification_only();
    }
    desc.derive(index)
        .translate_pk2(|xpk| xpk.derive_public_key(&EC))
        .ok()?
        .address(network)
        .ok()
}

#[derive(Debug, Clone)]
pub struct DescKeyInfo {
    pub bip32_origin: Bip32Origin,
    pub is_wildcard: bool,
}

const CHECKSUM_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

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
        let mut keys_info = vec![];

        let is_valid = desc.for_each_key(|fe| {
            match fe.as_key() {
                DescriptorPublicKey::XPub(desc_xpub) => {
                    // Get key origin information from the descriptor, fallback to extracting from the
                    // xpub itself.
                    let bip32_origin = desc_xpub
                        .origin
                        .as_ref()
                        .map_or_else(|| (&desc_xpub.xkey).into(), Into::<Bip32Origin>::into)
                        .extend(&desc_xpub.derivation_path);

                    keys_info.push(DescKeyInfo {
                        bip32_origin,
                        is_wildcard: desc_xpub.wildcard != Wildcard::None,
                    });

                    xpub_matches_network(&desc_xpub.xkey, network)
                }
                DescriptorPublicKey::SinglePub(desc_single) => {
                    if let Some(bip32_origin) = &desc_single.origin {
                        keys_info.push(DescKeyInfo {
                            bip32_origin: bip32_origin.into(),
                            is_wildcard: false,
                        });
                    }
                    true
                }
            }
        });

        ensure!(
            is_valid,
            "xpubs do not match the configured network {}",
            network
        );

        Ok(keys_info)
    }
}

pub trait DescriptorExt: Sized {
    /// Encode to string without the `#checksum` suffix
    fn to_string_no_checksum(&self) -> String;
    /// Get just the checksum
    fn checksum(&self) -> Checksum;
    /// Parse a descriptor, ensuring that it uses canonical encoding if the checksum is explicit
    fn parse_canonical(s: &str) -> Result<Self>;
}

impl DescriptorExt for ExtendedDescriptor {
    fn to_string_no_checksum(&self) -> String {
        self.to_string().splitn(2, '#').next().unwrap().into()
    }

    fn checksum(&self) -> Checksum {
        let desc_str = self.to_string();
        let checksum = desc_str.splitn(2, '#').skip(1).next().unwrap();
        Checksum(checksum.into())
    }

    fn parse_canonical(desc_str: &str) -> Result<ExtendedDescriptor> {
        let provided_desc_str = desc_str.splitn(2, '#').next().unwrap();
        let desc: ExtendedDescriptor = desc_str.parse()?;

        ensure!(
            desc.to_string_no_checksum() == provided_desc_str,
            "Descriptors with explicit checksums must use canonical encoding. `{}` is expected to be encoded as `{}`",
            provided_desc_str,
            desc.to_string_no_checksum()
        );
        Ok(desc)
    }
}
