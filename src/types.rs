use std::cmp::Ordering;

use bitcoin::Txid;
use bitcoin_hashes::{sha256, Hash};
use bitcoincore_rpc::json::ListUnspentResultEntry;
use serde_json::Value;

hash_newtype!(ScriptHash, sha256::Hash, 32, doc = "The hash of an spk.");
#[cfg(feature = "electrum")]
hash_newtype!(StatusHash, sha256::Hash, 32, doc = "The status hash.");

#[derive(Debug)]
pub struct Utxo {
    pub status: TxStatus,
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
}

impl Utxo {
    pub fn from_unspent(unspent: ListUnspentResultEntry, tip_height: u32) -> Self {
        Self {
            status: TxStatus::new(unspent.confirmations as i32, tip_height),
            txid: unspent.txid,
            vout: unspent.vout,
            value: unspent.amount.as_sat(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
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

    // height suitable for the electrum protocol
    // TODO -1 to indicate unconfirmed tx with unconfirmed parents
    #[cfg(feature = "electrum")]
    pub fn electrum_height(&self) -> u32 {
        match self {
            TxStatus::Confirmed(height) => *height,
            TxStatus::Unconfirmed => 0,
            TxStatus::Conflicted => {
                unreachable!("electrum_height() should not be called on conflicted txs")
            }
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
    pub fn rpc_arg(&self) -> Value {
        match self {
            KeyRescan::None => json!("now"),
            KeyRescan::All => json!(0),
            KeyRescan::Since(epoch) => json!(epoch),
        }
    }
}
