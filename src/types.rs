use std::cmp::Ordering;

use serde::Serialize;
use serde_json::Value;

use bitcoin::{Address, BlockHash, Txid};
use bitcoin_hashes::{sha256, Hash};

hash_newtype!(ScriptHash, sha256::Hash, 32, doc = "The hash of an spk.");

impl From<&Address> for ScriptHash {
    fn from(address: &Address) -> Self {
        ScriptHash::hash(&address.script_pubkey().into_bytes())
    }
}

#[cfg(feature = "electrum")]
hash_newtype!(StatusHash, sha256::Hash, 32, doc = "The status hash.");

#[derive(Debug, PartialEq)]
pub struct BlockId(pub u32, pub BlockHash);

#[derive(Debug, Copy, Clone)]
pub struct TxInput {
    pub txid: Txid,
    pub vin: u32,
}

serde_string_serializer_impl!(TxInput, input, format!("{}:{}", input.txid, input.vin));

impl TxInput {
    pub fn new(txid: Txid, vin: u32) -> Self {
        TxInput { txid, vin }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ScriptType {
    P2pkh,
    P2wpkh,
    P2shP2wpkh,
}

#[derive(Clone, Eq, PartialEq, Debug, Copy, Hash, Serialize)]
#[serde(tag = "status", content = "block_height", rename_all = "lowercase")]
pub enum TxStatus {
    Conflicted, // aka double spent
    Unconfirmed,
    Confirmed(u32), // (height)
}

impl TxStatus {
    pub fn new(confirmations: i32, tip_height: u32) -> Self {
        if confirmations > 0 {
            TxStatus::Confirmed(tip_height - (confirmations as u32) + 1)
        } else if confirmations == 0 {
            TxStatus::Unconfirmed
        } else {
            // negative confirmations indicate the tx conflicts with the best chain (aka was double-spent)
            TxStatus::Conflicted
        }
    }

    pub fn is_viable(&self) -> bool {
        match self {
            TxStatus::Confirmed(_) | TxStatus::Unconfirmed => true,
            TxStatus::Conflicted => false,
        }
    }

    pub fn is_confirmed(&self) -> bool {
        match self {
            TxStatus::Confirmed(_) => true,
            TxStatus::Unconfirmed | TxStatus::Conflicted => false,
        }
    }

    pub fn is_unconfirmed(&self) -> bool {
        match self {
            TxStatus::Unconfirmed => true,
            TxStatus::Confirmed(_) | TxStatus::Conflicted => false,
        }
    }
}

impl Ord for TxStatus {
    fn cmp(&self, other: &TxStatus) -> Ordering {
        match self {
            TxStatus::Confirmed(height) => match other {
                TxStatus::Confirmed(other_height) => height.cmp(other_height),
                TxStatus::Unconfirmed | TxStatus::Conflicted => Ordering::Greater,
            },
            TxStatus::Unconfirmed => match other {
                TxStatus::Confirmed(_) => Ordering::Less,
                TxStatus::Unconfirmed => Ordering::Equal,
                TxStatus::Conflicted => Ordering::Greater,
            },
            TxStatus::Conflicted => match other {
                TxStatus::Confirmed(_) | TxStatus::Unconfirmed => Ordering::Less,
                TxStatus::Conflicted => Ordering::Equal,
            },
        }
    }
}

impl PartialOrd for TxStatus {
    fn partial_cmp(&self, other: &TxStatus) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Copy, Clone, Debug)]
pub enum KeyRescan {
    None,
    All,
    Since(u32),
}

impl KeyRescan {
    pub fn as_rpc_timestamp(&self) -> Value {
        match self {
            KeyRescan::None => json!("now"),
            KeyRescan::All => json!(0),
            KeyRescan::Since(epoch) => json!(epoch),
        }
    }
}
