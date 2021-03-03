use std::cmp::Ordering;

use serde::Serialize;

use bitcoin::{Address, BlockHash, Txid};
use bitcoin_hashes::{sha256, Hash};
use bitcoincore_rpc::json::GetMempoolEntryResult;

pub use crate::util::bitcoincore_ext::RescanSince;

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

impl BlockId {
    pub fn height(&self) -> u32 {
        self.0
    }
    pub fn hash(&self) -> &BlockHash {
        &self.1
    }
}

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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Deserialize)]
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

#[derive(Serialize, Clone, Debug)]
pub struct MempoolEntry {
    /// The vsize of this transaction
    pub vsize: u64,
    /// The direct fee paid by this transaction
    pub fee: u64,
    /// The total vsize of in-mempool ancestors (including this tx)
    pub ancestor_vsize: u64,
    /// The total fee paid by in-mempool ancestors (including this tx)
    pub ancestor_fee: u64,
    /// Whether this transaction could be replaced due to BIP125 (replace-by-fee)
    pub bip125_replaceable: bool,
}

impl MempoolEntry {
    /// Whether this transaction has unconfirmed ancestors as its inputs
    pub fn has_unconfirmed_parents(&self) -> bool {
        self.vsize != self.ancestor_vsize
    }

    /// The direct feerate paid by this transaction, in sat/vB
    pub fn own_feerate(&self) -> f64 {
        self.fee as f64 / self.vsize as f64
    }

    /// The effective feerate paid by this transaction, taking unconfirmed ancestors into account
    pub fn effective_feerate(&self) -> f64 {
        // ancestors can only contribute negatively to the effective rate
        self.own_feerate()
            .min(self.ancestor_fee as f64 / self.ancestor_vsize as f64)
    }
}

impl From<GetMempoolEntryResult> for MempoolEntry {
    fn from(entry: GetMempoolEntryResult) -> Self {
        Self {
            vsize: entry.vsize,
            fee: entry.fees.base.as_sat(),
            ancestor_vsize: entry.ancestor_size,
            ancestor_fee: entry.fees.ancestor.as_sat(),
            bip125_replaceable: entry.bip125_replaceable,
        }
    }
}
