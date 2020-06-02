use bitcoin::Txid;
use bitcoin_hashes::{sha256d, Hash};
use bitcoincore_rpc as rpc;

use crate::error::{Error, OptionExt, Result};
use crate::query::Query;

pub fn get_merkle_proof(
    query: &Query,
    txid: &Txid,
    height: u32,
) -> Result<(Vec<sha256d::Hash>, usize)> {
    let block_hash = query.get_block_hash(height)?;
    let txids = match query.get_block_txids(&block_hash) {
        Ok(txids) => txids,
        // if we can't generate the spv proof due to pruning, return a fauxed proof instead of an
        // error, which electrum will accept when run with --skipmerklecheck.
        Err(e) if is_pruned_error(&e) => vec![*txid],
        Err(e) => bail!(e),
    };
    let pos = txids
        .iter()
        .position(|c_txid| c_txid == txid)
        .or_err("missing tx")?;

    let hashes = txids.into_iter().map(sha256d::Hash::from).collect();
    let (branch, _root) = create_merkle_branch_and_root(hashes, pos);
    Ok((branch, pos))
}

pub fn get_header_merkle_proof(
    query: &Query,
    height: u32,
    cp_height: u32,
) -> Result<(Vec<sha256d::Hash>, sha256d::Hash)> {
    if cp_height < height {
        bail!("cp_height #{} < height #{}", cp_height, height);
    }

    let best_height = query.get_tip_height()?;
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
        .map(|height| query.get_block_hash(height).map(sha256d::Hash::from))
        .collect::<Result<Vec<sha256d::Hash>>>()?;

    Ok(create_merkle_branch_and_root(
        header_hashes,
        height as usize,
    ))
}

pub fn get_id_from_pos(
    query: &Query,
    height: u32,
    tx_pos: usize,
    want_merkle: bool,
) -> Result<(Txid, Vec<sha256d::Hash>)> {
    let block_hash = query.get_block_hash(height)?;
    let txids = query.get_block_txids(&block_hash)?;
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

fn is_pruned_error(e: &Error) -> bool {
    if let Some(e) = e.downcast_ref::<rpc::Error>() {
        if let rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)) = e {
            return e.code == -1 && e.message == "Block not available (pruned data)";
        }
    }
    false
}
