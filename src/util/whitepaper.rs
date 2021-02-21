use crate::error::{OptionExt, Result};
use bitcoin::blockdata::script::Instruction;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

/// Extract the Bitcoin whitepaper
/// For some historical context, see https://twitter.com/search?q=(%23BitcoinWhitepaper%20OR%20%23BitcoinPdf)%20until%3A2021-01-23%20since%3A2021-01-20
pub fn get_whitepaper_pdf(client: &RpcClient) -> Result<Vec<u8>> {
    let txid = "54e48e5f5c656b26c3bca14a8c95aa583d07ebe84dde3b7dd4a78f4e4186e713"
        .parse()
        .unwrap();
    let mut blob = vec![];
    for vout in 0..=945 {
        let out = client.get_tx_out(&txid, vout, None)?.required()?;
        for instruction in out.script_pub_key.script()?.instructions() {
            if let Ok(Instruction::PushBytes(data)) = instruction {
                blob.extend_from_slice(data);
            }
        }
    }
    blob.drain(0..8); // remove size and crc32 checksum
    blob.drain(blob.len() - 8..); // drop null bytes at the end
    Ok(blob)
}
