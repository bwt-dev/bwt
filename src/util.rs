use bitcoin::Address;
use bitcoin_hashes::{sha256, Hash};

pub fn address_to_scripthash(address: &Address) -> sha256::Hash {
    sha256::Hash::hash(&address.script_pubkey().into_bytes())
}
