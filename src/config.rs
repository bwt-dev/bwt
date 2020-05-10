use std::{net, path, time};

use bitcoin::Network;
use bitcoincore_rpc::Auth as RpcAuth;
use chrono::{TimeZone, Utc};
use dirs::home_dir;
use structopt::StructOpt;

use crate::error::{OptionExt, Result, ResultExt};
use crate::types::KeyRescan;

#[derive(StructOpt, Debug)]
pub struct Config {
    #[structopt(
        short,
        long,
        help = "one of 'bitcoin', 'testnet' or 'regtest'",
        default_value = "bitcoin"
    )]
    pub network: Network,

    #[structopt(
        short,
        long,
        help = "increase verbosity level (up to 3 times)",
        parse(from_occurrences)
    )]
    pub verbose: usize,

    #[structopt(
        short = "i",
        long = "poll-interval",
        help = "interval for checking new blocks/txs (in seconds)",
        default_value = "5",
        parse(try_from_str = parse_duration)
    )]
    pub poll_interval: time::Duration,

    #[structopt(
        short = "d",
        long = "bitcoind-dir",
        help = "path to bitcoind directory (used for cookie file, defaults to ~/.bitcoin/<network>)"
    )]
    pub bitcoind_dir: Option<path::PathBuf>,

    #[structopt(
        short = "u",
        long = "bitcoind-url",
        help = "url for the bitcoind rpc server (defaults to http://localhost:<network-rpc-port>)"
    )]
    pub bitcoind_url: Option<String>,

    #[structopt(
        short = "C",
        long = "bitcoind-cred",
        help = "credentials for accessing the bitcoind rpc server (as <username>:<password>, instead of reading the cookie file)"
    )]
    pub bitcoind_cred: Option<String>,

    #[structopt(
        short = "c",
        long = "bitcoind-cookie",
        help = "cookie file for accessing the bitcoind rpc server (defaults to <bitcoind-dir>/.cookie)"
    )]
    pub bitcoind_cookie: Option<path::PathBuf>,

    // wallets to watch
    #[structopt(
        short,
        long = "xpub",
        help = "xpubs to scan and since when (<xpub>, <xpub>:all, <xpub>:none, <xpub>:<yyyy-mm-dd> or <xpub>:<unix-epoch>)",
        parse(try_from_str = parse_xpub)
    )]
    pub xpubs: Vec<(String, KeyRescan)>,

    //// TODO
    //#[structopt(
    //short,
    //long = "address",
    //help = "addresses to track (address:yyyy-mm-dd)",
    //parse(try_from_str = "parse_address")
    //)]
    //addresses: Vec<(String, KeyRescan)>,
    #[cfg(feature = "electrum")]
    #[structopt(
        short,
        long = "electrum-rpc-addr",
        help = "address to bind the electrum rpc server (host:port)"
    )]
    pub electrum_rpc_addr: net::SocketAddr,

    #[cfg(feature = "http")]
    #[structopt(
        short,
        long = "http-server-addr",
        help = "address to bind the http rest server (host:port)"
    )]
    pub http_server_addr: net::SocketAddr,
}

impl Config {
    pub fn bitcoind_url(&self) -> String {
        self.bitcoind_url.clone().unwrap_or_else(|| {
            format!(
                "http://localhost:{}/",
                match self.network {
                    Network::Bitcoin => 8332,
                    Network::Testnet => 18332,
                    Network::Regtest => 18443,
                }
            )
        })
    }

    pub fn bitcoind_auth(&self) -> Result<RpcAuth> {
        Ok(self.bitcoind_cred
            .as_ref()
            .and_then(|cred| {
                let mut parts = cred.splitn(2, ":");
                Some(RpcAuth::UserPass(parts.next()?.into(), parts.next()?.into()))
            })
            .or_else(|| {
                let cookie = self.bitcoind_cookie.clone().or_else(|| get_cookie(self))?;
                Some(RpcAuth::CookieFile(cookie))
            })
            .or_err("no available authentication for bitcoind rpc, please specify credentials or a cookie file")?)
    }
}

fn parse_xpub(s: &str) -> Result<(String, KeyRescan)> {
    let mut parts = s.splitn(2, ":");
    let xpub = parts.next().or_err("missing xpub")?;
    let rescan = parts.next().map_or(Ok(KeyRescan::Since(0)), parse_rescan)?;
    Ok((xpub.into(), rescan))
}

fn parse_rescan(s: &str) -> Result<KeyRescan> {
    Ok(match s {
        "none" => KeyRescan::None,
        "all" => KeyRescan::All,
        s => {
            // try as a unix timestamp first, then as a datetime string
            KeyRescan::Since(
                s.parse::<u32>()
                    .or_else(|_| parse_yyyymmdd(s))
                    .context("invalid rescan value")?,
            )
        }
    })
}

fn parse_yyyymmdd(s: &str) -> Result<u32> {
    let mut parts = s.splitn(3, "-");
    Ok(Utc
        .ymd_opt(
            parts.next().req()?.parse()?,
            parts.next().req()?.parse()?,
            parts.next().req()?.parse()?,
        )
        .single()
        .req()?
        .and_hms(0, 0, 0)
        .timestamp() as u32)
}

fn parse_duration(s: &str) -> Result<time::Duration> {
    Ok(time::Duration::from_secs(s.parse()?))
}

fn get_cookie(config: &Config) -> Option<path::PathBuf> {
    let mut dir = config
        .bitcoind_dir
        .clone()
        .or_else(|| Some(home_dir()?.join(".bitcoin")))?;
    match config.network {
        Network::Bitcoin => (),
        Network::Testnet => dir.push("testnet3"),
        Network::Regtest => dir.push("regtest"),
    }
    let cookie = dir.join(".cookie");
    if cookie.exists() {
        Some(cookie)
    } else {
        println!("cookie file not found in {:?}", cookie);
        None
    }
}
