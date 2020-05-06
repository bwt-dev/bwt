use crate::error::{OptionExt, Result};
use crate::query::Query;

const VSIZE_BIN_WIDTH: u32 = 50_000; // vbytes

pub fn get_fee_histogram(query: &Query) -> Result<Vec<(f32, u32)>> {
    let rawmempool = query.get_raw_mempool()?;

    let mut entries = rawmempool
        .as_object()
        .or_err("invalid getrawmempool reply")?
        .values()
        .filter_map(|entry| {
            let size = entry["size"].as_u64()?;
            let fee = entry["fee"].as_f64()?;
            let feerate = fee as f32 / size as f32 * 100_000_000f32;
            Some((size as u32, feerate))
        })
        .collect::<Vec<(u32, f32)>>();

    // XXX we should take unconfirmed parents feerates into account

    entries.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    let mut histogram = vec![];
    let mut bin_size = 0;
    let mut last_feerate = None;

    for (size, feerate) in entries.into_iter().rev() {
        bin_size += size;
        if bin_size > VSIZE_BIN_WIDTH && last_feerate.map_or(true, |last| feerate > last) {
            // vsize of transactions paying >= e.fee_per_vbyte()
            histogram.push((feerate, bin_size));
            bin_size = 0;
        }
        last_feerate = Some(feerate);
    }

    if let Some(feerate) = last_feerate {
        histogram.push((feerate, bin_size));
    }

    Ok(histogram)
}
