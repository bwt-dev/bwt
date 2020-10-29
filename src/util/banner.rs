use std::time::{Duration as StdDuration, UNIX_EPOCH};

use chrono::Duration;

use bitcoin::{blockdata::constants, Amount};
use bitcoincore_rpc::RpcApi;

use crate::util::bitcoincore_ext::RpcApiExt;
use crate::{Query, Result};

const DIFFCHANGE_INTERVAL: u64 = constants::DIFFCHANGE_INTERVAL as u64;
const TARGET_BLOCK_SPACING: u64 = constants::TARGET_BLOCK_SPACING as u64;
const INITIAL_REWARD: u64 = 50 * constants::COIN_VALUE;
const HALVING_INTERVAL: u64 = 210_000;

pub fn get_welcome_banner(query: &Query, omit_donation: bool) -> Result<String> {
    let rpc = query.rpc();

    let net_info = rpc.get_network_info()?;
    let chain_info = rpc.get_blockchain_info()?;
    let mempool_info = rpc.get_mempool_info()?;
    let net_totals = rpc.get_net_totals()?;
    let peers = rpc.get_peer_info()?;
    let hash_rate_7d = rpc.get_network_hash_ps(Some(1008), None)?;
    let uptime = dur_from_secs(rpc.uptime()?);
    let tip = rpc.get_block_stats(&rpc.get_best_block_hash()?)?;

    let est_fee = |target| {
        query
            .estimate_fee(target)
            .ok()
            .flatten()
            .map_or("ɴ/ᴀ".into(), |rate| format!("{:.1}", rate))
    };
    let est_20m = est_fee(2u16);
    let est_4h = est_fee(24u16);
    let est_1d = est_fee(144u16);

    let mut chain_name = chain_info.chain;
    if chain_name == "main" || chain_name == "test" {
        chain_name = format!("{}net", chain_name)
    };

    // 24 hour average bandwidth usage
    let num_days = uptime.num_seconds() as f64 / 86400f64;
    let bandwidth_up = (net_totals.total_bytes_sent as f64 / num_days) as u64;
    let bandwidth_down = (net_totals.total_bytes_recv as f64 / num_days) as u64;

    // Time until the next difficulty adjustment
    let retarget_blocks = DIFFCHANGE_INTERVAL - (tip.height % DIFFCHANGE_INTERVAL);
    let retarget_dur = dur_from_secs(retarget_blocks * TARGET_BLOCK_SPACING);

    // Current reward era and time until next halving
    let reward_era = tip.height / HALVING_INTERVAL;
    let block_reward = Amount::from_sat(INITIAL_REWARD / 2u64.pow(reward_era as u32));
    let halving_blocks = HALVING_INTERVAL - (tip.height % HALVING_INTERVAL);
    let halving_dur = dur_from_secs(halving_blocks * TARGET_BLOCK_SPACING);

    // Time since last block
    let tip_ago = match (UNIX_EPOCH + StdDuration::from_secs(tip.time as u64)).elapsed() {
        Ok(elapsed) => format!("{} ago", format_dur(&Duration::from_std(elapsed).unwrap())),
        Err(_) => "just now".to_string(), // account for blocks with a timestamp slightly in the future
    };

    // sat/kb -> sat/vB
    let mempool_min_fee = mempool_info.mempool_min_fee.as_sat() as f64 / 1000f64;

    let has_inbound = peers.iter().any(|p| p.inbound);

    let modes = [
        if chain_info.pruned {
            "✂️ ᴘʀᴜɴᴇᴅ"
        } else {
            "🗄️ ᴀʀᴄʜɪᴠᴀʟ"
        },
        if net_info.local_relay {
            "🗣️ ᴍᴇᴍᴘᴏᴏʟ ʀᴇʟᴀʏ"
        } else {
            "📦 ʙʟᴏᴄᴋsᴏɴʟʏ"
        },
        if has_inbound {
            "👂 ʟɪsᴛᴇɴs"
        } else {
            "🙉 ɴᴏʟɪsᴛᴇɴ"
        },
    ];

    let ver_lines = big_numbers(crate::BWT_VERSION);

    Ok(format!(
        r#"
   ██████  ██     ██ ████████ 
   ██   ██ ██     ██    ██    
   ██████  ██  █  ██    ██        {ver_line1}
   ██   ██ ██ ███ ██    ██    █ █ {ver_line2}
   ██████   ███ ███     ██    ▀▄▀ {ver_line3}

   {client_name}

   {modes}

     NETWORK: 🌐  {chain_name}
   CONNECTED: 💻  {connected_peers} ᴘᴇᴇʀs
      UPTIME: ⏱️  {uptime}

   BANDWIDTH: 📶  {bandwidth_up} 🔼  {bandwidth_down} 🔽 (24ʜ ᴀᴠɢ)
  CHAIN SIZE: 💾  {chain_size}

    HASHRATE: ⛏️  {hash_rate} (7ᴅ ᴀᴠɢ)
  DIFFICULTY: 🏋️  {difficulty} (ʀᴇ-🎯  ɪɴ {retarget_dur} ⏳)
  REWARD ERA: 🎁  {block_reward:.2} ʙᴛᴄ (½ ɪɴ {halving_dur} ⏳)

  LAST BLOCK: ⛓️  {tip_height} ／ {tip_ago} ／ {tip_size} ／ {tip_weight} ／ {tip_n_tx}
                 Fᴇᴇ ʀᴀᴛᴇ {tip_fee_per10}-{tip_fee_per90} sᴀᴛ/ᴠʙ ／ ᴀᴠɢ {tip_fee_avg} sᴀᴛ/ᴠʙ ／ ᴛᴏᴛᴀʟ {tip_fee_total:.3} ʙᴛᴄ
     MEMPOOL: 💭  {mempool_size} ／ {mempool_n_tx} ／ ᴍɪɴ {mempool_min_fee:.1} sᴀᴛ/ᴠʙ
    FEES EST: 🏷️  20 ᴍɪɴᴜᴛᴇs: {est_20m} ／ 4 ʜᴏᴜʀs: {est_4h} ／ 1 ᴅᴀʏ: {est_1d} (sᴀᴛ/ᴠʙ)

{donation_frag}"#,
        modes = modes.join(" "),
        client_name = to_widetext(&net_info.subversion),
        chain_name = to_smallcaps(&chain_name),
        connected_peers = net_info.connections,
        uptime = to_smallcaps(&format_dur(&uptime).to_uppercase()),
        bandwidth_up = to_smallcaps(&format_bytes(bandwidth_up)),
        bandwidth_down = to_smallcaps(&format_bytes(bandwidth_down)),
        chain_size = to_smallcaps(&format_bytes(chain_info.size_on_disk)),
        hash_rate = to_smallcaps(&format_metric(hash_rate_7d, " ", "H/s")),
        difficulty = to_smallcaps(&format_metric(chain_info.difficulty as f64, " ", "")),
        retarget_dur = to_smallcaps(&format_dur(&retarget_dur).to_uppercase()),
        halving_dur = to_smallcaps(&format_dur(&halving_dur).to_uppercase()),
        block_reward = block_reward.as_btc(),
        tip_height = tip.height,
        tip_ago = to_smallcaps(&tip_ago),
        tip_size = to_smallcaps(&format_bytes(tip.total_size)),
        tip_weight = to_smallcaps(&format_metric(tip.total_weight as f64, " ", "WU")),
        tip_n_tx = to_smallcaps(&format_metric(tip.txs as f64, "", " txs")),
        tip_fee_per10 = tip.feerate_percentiles.0,
        tip_fee_per90 = tip.feerate_percentiles.4,
        tip_fee_avg = tip.avg_fee_rate,
        tip_fee_total = tip.total_fee.as_btc(),
        mempool_size = to_smallcaps(&format_metric(mempool_info.bytes as f64, " V", "B")),
        mempool_n_tx = to_smallcaps(&format_metric(mempool_info.size as f64, "", " txs")),
        mempool_min_fee = mempool_min_fee,
        est_20m = est_20m,
        est_4h = est_4h,
        est_1d = est_1d,
        ver_line1 = ver_lines.0,
        ver_line2 = ver_lines.1,
        ver_line3 = ver_lines.2,
        donation_frag = if !omit_donation {
            " SUPPORT DEV: 🚀  bc1qmuagsjvq0lh3admnafk0qnlql0vvxv08au9l2d ／ https://btcpay.shesek.info\n"
        } else {
            ""
        },
    ))
}

/* Disabled because this takes too long

  let utxo_info = query.rpc().get_tx_out_set_info()?;
  let total_supply = utxo_info.total_amount.to_string();

   𝚄𝚃𝚇𝙾 𝚂𝙸𝚉𝙴: 🗃️ {utxo_size}

   ✔️ 𝚅𝙴𝚁𝙸𝙵𝙸𝙴𝙳 ✔️
   𝙲𝙸𝚁𝙲𝚄𝙻𝙰𝚃𝙸𝙽𝙶   {total_supply}
      𝚂𝚄𝙿𝙿𝙻𝚈     {total_supply_line}

        utxo_size = to_smallcaps(&format_bytes(utxo_info.disk_size)),
        total_supply = to_smallcaps(&total_supply),
        total_supply_line = "‾".repeat(total_supply.len()),
        height = utxo_info.height,
*/

fn dur_from_secs(seconds: u64) -> Duration {
    Duration::from_std(StdDuration::from_secs(seconds)).unwrap()
}

fn format_dur(dur: &Duration) -> String {
    let days = dur.num_days();
    if days > 90 {
        return format!("{} months", days / 30);
    }
    if days > 21 {
        return format!("{} weeks", days / 7);
    }
    if days > 3 {
        return format!("{} days", days);
    }
    let hours = dur.num_hours();
    if hours > 3 {
        return format!("{} hours", hours);
    }
    let minutes = dur.num_minutes();
    if minutes > 3 {
        return format!("{} minutes", minutes);
    }
    format!("{} seconds", dur.num_seconds())
}

fn format_bytes(bytes: u64) -> String {
    format_metric(bytes as f64, " ", "B")
}

fn format_metric(num: f64, space: &str, suf: &str) -> String {
    if num >= 1000000000000000000f64 {
        format!("{:.1}{}E{}", num / 1000000000000000000f64, space, suf)
    } else if num >= 1000000000000000f64 {
        format!("{:.1}{}P{}", num / 1000000000000000f64, space, suf)
    } else if num >= 1000000000000f64 {
        format!("{:.1}{}T{}", num / 1000000000000f64, space, suf)
    } else if num >= 1000000000f64 {
        format!("{:.1}{}G{}", num / 1000000000f64, space, suf)
    } else if num >= 1000000f64 {
        format!("{:.1}{}M{}", num / 1000000f64, space, suf)
    } else if num >= 1000f64 {
        format!("{:.1}{}K{}", num / 1000f64, space, suf)
    } else {
        format!("{}{}{}", num, space, suf)
    }
}

lazy_static! {
    static ref SMALLCAPS_ALPHABET: Vec<char> =
        "ᴀʙᴄᴅᴇFɢʜɪᴊᴋʟᴍɴᴏᴘQʀsᴛᴜᴠᴡxʏᴢᴀʙᴄᴅᴇFɢʜɪᴊᴋʟᴍɴᴏᴘQʀsᴛᴜᴠᴡxʏᴢ01234567890./:".chars().collect::<Vec<_>>();
    static ref WIDETEXT_ALPHABET: Vec<char> =
        "ａｂｃｄｅｆｇｈｉｊｋｌｍｎｏｐｑｒｓｔｕｖｗｘｙｚＡＢＣＤＥＦＧＨＩＪＫＬＭＮＯＰＱＲＳＴＵＶＷＸＹＺ０１２３４５６７８９０．／："
            .chars()
            .collect::<Vec<_>>();
}

fn convert_alphabet(s: &str, alphabet: &[char]) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => alphabet[c as usize - 97],
            'A'..='Z' => alphabet[c as usize - 65 + 26],
            '0'..='9' => alphabet[c as usize - 48 + 26 * 2],
            '.' => alphabet[63],
            '/' => alphabet[64],
            ':' => alphabet[65],
            c => c,
        })
        .collect()
}

fn to_smallcaps(s: &str) -> String {
    convert_alphabet(s, &SMALLCAPS_ALPHABET[..])
}
fn to_widetext(s: &str) -> String {
    convert_alphabet(s, &WIDETEXT_ALPHABET[..])
}

fn big_numbers(s: &str) -> (String, String, String) {
    let mut lines = ("".to_string(), "".to_string(), "".to_string());
    for c in s.chars() {
        let char_lines = match c {
            '0' => ("█▀▀█", "█  █", "█▄▄█"),
            '1' => ("▄█ ", " █ ", "▄█▄"),
            '2' => ("█▀█", " ▄▀", "█▄▄"),
            '3' => ("█▀▀█", "  ▀▄", "█▄▄█"),
            '4' => (" █▀█ ", "█▄▄█▄", "   █ "),
            '5' => ("█▀▀", "▀▀▄", "▄▄▀"),
            '6' => ("▄▀▀▄", "█▄▄ ", "▀▄▄▀"),
            '7' => ("▀▀▀█", "  █ ", " ▐▌ "),
            '8' => ("▄▀▀▄", "▄▀▀▄", "▀▄▄▀"),
            '9' => ("▄▀▀▄", "▀▄▄█", " ▄▄▀"),
            '.' => (" ", " ", "█"),
            _ => continue,
        };
        lines.0.push_str(char_lines.0);
        lines.1.push_str(char_lines.1);
        lines.2.push_str(char_lines.2);
        lines.0.push_str(" ");
        lines.1.push_str(" ");
        lines.2.push_str(" ");
    }
    lines
}
