use std::cmp::Ordering;

use serde::Serialize;

use bitcoin::{Address, BlockHash, Txid};
use bitcoin_hashes::{sha256, Hash};
pub use bitcoincore_rpc::json::ImportMultiRescanSince as RescanSince;

hash_newtype!(
    ScriptHash,
    sha256::Hash,
    32,
    doc = "The hash of an spk.",
    true
);

impl From<&Address> for ScriptHash {
    fn from(address: &Address) -> Self {
        ScriptHash::hash(&address.script_pubkey().into_bytes())
    }
}

impl From<Address> for ScriptHash {
    fn from(address: Address) -> Self {
        ScriptHash::from(&address)
    }
}

#[cfg(feature = "electrum")]
hash_newtype!(StatusHash, sha256::Hash, 32, doc = "The status hash.");

#[derive(Serialize, Debug, PartialEq, Clone, Copy)]
pub struct BlockId(pub u32, pub BlockHash);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.0, self.1)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct InPoint {
    pub txid: Txid,
    pub vin: u32,
}

impl_string_serializer!(InPoint, input, format!("{}:{}", input.txid, input.vin));

impl InPoint {
    pub fn new(txid: Txid, vin: u32) -> Self {
        InPoint { txid, vin }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptType {
    P2pkh,
    P2wpkh,
    P2shP2wpkh,
}

#[derive(Clone, Eq, PartialEq, Debug, Copy, Hash)]
pub enum TxStatus {
    Conflicted, // aka double spent
    Unconfirmed,
    Confirmed(u32), // (height)
}

impl TxStatus {
    pub fn from_confirmations(confirmations: i32, tip_height: u32) -> Self {
        match confirmations.cmp(&0) {
            Ordering::Greater => TxStatus::Confirmed(tip_height - (confirmations as u32) + 1),
            Ordering::Equal => TxStatus::Unconfirmed,
            Ordering::Less => TxStatus::Conflicted,
        }
    }

    pub fn is_viable(self) -> bool {
        match self {
            TxStatus::Confirmed(_) | TxStatus::Unconfirmed => true,
            TxStatus::Conflicted => false,
        }
    }

    pub fn is_confirmed(self) -> bool {
        match self {
            TxStatus::Confirmed(_) => true,
            TxStatus::Unconfirmed | TxStatus::Conflicted => false,
        }
    }

    pub fn is_unconfirmed(self) -> bool {
        match self {
            TxStatus::Unconfirmed => true,
            TxStatus::Confirmed(_) | TxStatus::Conflicted => false,
        }
    }
}

// Serialize confirmed transactions as the block height, unconfirmed as null and confliced as -1
impl serde::Serialize for TxStatus {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ::serde::Serializer,
    {
        match self {
            TxStatus::Confirmed(height) => serializer.serialize_u32(*height),
            TxStatus::Unconfirmed => serializer.serialize_none(),
            TxStatus::Conflicted => serializer.serialize_i8(-1),
        }
    }
}

impl Ord for TxStatus {
    fn cmp(&self, other: &TxStatus) -> Ordering {
        match (self, other) {
            (TxStatus::Confirmed(my_height), TxStatus::Confirmed(other_height)) => {
                my_height.cmp(other_height)
            }
            (TxStatus::Confirmed(_), TxStatus::Unconfirmed) => Ordering::Less,
            (TxStatus::Unconfirmed, TxStatus::Confirmed(_)) => Ordering::Greater,
            (TxStatus::Unconfirmed, TxStatus::Unconfirmed) => Ordering::Equal,
            (TxStatus::Conflicted, _) | (_, TxStatus::Conflicted) => {
                unreachable!("confliced txs should not be ordered")
            }
        }
    }
}

impl PartialOrd for TxStatus {
    fn partial_cmp(&self, other: &TxStatus) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
