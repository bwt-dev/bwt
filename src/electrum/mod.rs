use bitcoin::Txid;
use bitcoin_hashes::{sha256d, Hash, HashEngine};

use crate::error::{OptionExt, Result};
use crate::query::Query;
use crate::types::{MempoolEntry, ScriptHash, StatusHash, TxStatus};
use crate::util::BoolThen;

mod server;
pub use server::ElectrumServer;

pub fn electrum_height(status: TxStatus, has_unconfirmed_parents: Option<bool>) -> i32 {
    match status {
        TxStatus::Confirmed(height) => height as i32,
        TxStatus::Unconfirmed => match has_unconfirmed_parents {
            Some(false) => 0, // a height of 0 indicates an unconfirmed tx where all the parents are confirmed
            Some(true) => -1, // a height of -1 indicates an unconfirmed tx with unconfirmed parents used as its inputs
            None => -1,       // if has_unconfirmed_parents is unknown, error on the side of caution
        },
        TxStatus::Conflicted => {
            unreachable!("electrum_height() should not be called on conflicted txs")
        }
    }
}

trait QueryExt {
    fn get_status_hash(&self, scripthash: &ScriptHash) -> Option<StatusHash>;

    fn electrum_merkle_proof(
        &self,
        txid: &Txid,
        height: u32,
    ) -> Result<(Vec<sha256d::Hash>, usize)>;

    fn electrum_header_merkle_proof(
        &self,
        height: u32,
        cp_height: u32,
    ) -> Result<(Vec<sha256d::Hash>, sha256d::Hash)>;

    fn electrum_id_from_pos(
        &self,
        height: u32,
        tx_pos: usize,
        want_merkle: bool,
    ) -> Result<(Txid, Vec<sha256d::Hash>)>;
}

impl QueryExt for Query {
    fn get_status_hash(&self, scripthash: &ScriptHash) -> Option<StatusHash> {
        let mut engine = StatusHash::engine();
        let has_history = self.for_each_history(scripthash, |hist| {
            let has_unconfirmed_parents = hist.status.is_unconfirmed().and_then(|| {
                self.with_mempool_entry(&hist.txid, MempoolEntry::has_unconfirmed_parents)
            });
            let p = format!(
                "{}:{}:",
                hist.txid,
                electrum_height(hist.status, has_unconfirmed_parents)
            );
            engine.input(&p.into_bytes());
        });

        if has_history {
            Some(StatusHash::from_engine(engine))
        } else {
            // empty history needs to be represented as a `null` in json
            None
        }
    }

    fn electrum_merkle_proof(
        &self,
        txid: &Txid,
        height: u32,
    ) -> Result<(Vec<sha256d::Hash>, usize)> {
        let block_hash = self.get_block_hash(height)?;
        let txids = self.get_block_txids(&block_hash)?;
        let pos = txids
            .iter()
            .position(|c_txid| c_txid == txid)
            .or_err("missing tx")?;

        let hashes = txids.into_iter().map(sha256d::Hash::from).collect();
        let (branch, _root) = create_merkle_branch_and_root(hashes, pos);
        Ok((branch, pos))
    }

    fn electrum_header_merkle_proof(
        &self,
        height: u32,
        cp_height: u32,
    ) -> Result<(Vec<sha256d::Hash>, sha256d::Hash)> {
        if cp_height < height {
            bail!("cp_height #{} < height #{}", cp_height, height);
        }

        let best_height = self.get_tip_height()?;
        if best_height < cp_height {
            bail!(
                "cp_height #{} above best block height #{}",
                cp_height,
                best_height
            );
        }

        let heights: Vec<u32> = (0..=cp_height).collect();
        let header_hashes = heights
            .into_iter()
            .map(|height| self.get_block_hash(height).map(sha256d::Hash::from))
            .collect::<Result<Vec<sha256d::Hash>>>()?;

        Ok(create_merkle_branch_and_root(
            header_hashes,
            height as usize,
        ))
    }

    fn electrum_id_from_pos(
        &self,
        height: u32,
        tx_pos: usize,
        want_merkle: bool,
    ) -> Result<(Txid, Vec<sha256d::Hash>)> {
        let block_hash = self.get_block_hash(height)?;
        let txids = self.get_block_txids(&block_hash)?;
        let txid = *txids.get(tx_pos).or_err(format!(
            "No tx in position #{} in block #{}",
            tx_pos, height
        ))?;

        let branch = if want_merkle {
            let hashes = txids.into_iter().map(sha256d::Hash::from).collect();
            create_merkle_branch_and_root(hashes, tx_pos).0
        } else {
            vec![]
        };
        Ok((txid, branch))
    }
}

fn merklize(left: sha256d::Hash, right: sha256d::Hash) -> sha256d::Hash {
    let data = [&left[..], &right[..]].concat();
    sha256d::Hash::hash(&data)
}

fn create_merkle_branch_and_root(
    mut hashes: Vec<sha256d::Hash>,
    mut index: usize,
) -> (Vec<sha256d::Hash>, sha256d::Hash) {
    let mut merkle = vec![];
    while hashes.len() > 1 {
        if hashes.len() % 2 != 0 {
            let last = *hashes.last().unwrap();
            hashes.push(last);
        }
        index = if index % 2 == 0 { index + 1 } else { index - 1 };
        merkle.push(hashes[index]);
        index /= 2;
        hashes = hashes
            .chunks(2)
            .map(|pair| merklize(pair[0], pair[1]))
            .collect()
    }
    (merkle, hashes[0])
}
