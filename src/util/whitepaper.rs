use crate::error::{OptionExt, Result};
use bitcoin::blockdata::script::Instruction;
use bitcoincore_rpc::{Client as RpcClient, RpcApi};

const WP_HEIGHT: u64 = 230009;
const WP_TX_POS: usize = 113;

/// Extract the Bitcoin whitepaper
/// For some historical context, see https://twitter.com/search?q=(%23BitcoinWhitepaper%20OR%20%23BitcoinPdf)%20until%3A2021-01-23%20since%3A2021-01-20
pub fn get_whitepaper_pdf(client: &RpcClient) -> Result<Vec<u8>> {
    let blockhash = client.get_block_hash(WP_HEIGHT)?;
    let txids = client.get_block_info(&blockhash)?.tx;
    let txid = txids.get(WP_TX_POS).required()?;
    let tx = client.get_raw_transaction(&txid, Some(&blockhash))?;

    let mut blob = vec![];
    for out in &tx.output {
        for instruction in out.script_pubkey.instructions() {
            if let Ok(Instruction::PushBytes(data)) = instruction {
                if data.len() >= 33 {
                    blob.extend_from_slice(data);
                }
            }
        }
    }
    blob.drain(0..8); // remove size and crc32 checksum
    blob.drain(blob.len() - 8..); // drop null bytes at the end
    Ok(blob)
}
