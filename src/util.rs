use bitcoin::Address;
use bitcoin_hashes::Hash;

use crate::types::ScriptHash;

pub fn address_to_scripthash(address: &Address) -> ScriptHash {
    ScriptHash::hash(&address.script_pubkey().into_bytes())
}
