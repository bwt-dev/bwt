use std::str::FromStr;
use std::{net, path, time};

use chrono::{TimeZone, Utc};
use dirs::home_dir;
use log::Level;
use structopt::StructOpt;

use bitcoin::Network;
use bitcoincore_rpc::Auth as RpcAuth;

use crate::error::{OptionExt, Result, ResultExt};
use crate::hd::XyzPubKey;
use crate::types::RescanSince;

#[derive(StructOpt, Debug)]
pub struct Config {
    #[structopt(
        short = "n",
        long,
        help = "one of 'bitcoin', 'testnet' or 'regtest'",
        default_value = "bitcoin",
        env,
        hide_env_values(true),
        display_order(1)
    )]
    pub network: Network,

    // cannot be set using an env var, it does not play nicely with from_occurrences
    #[structopt(
        short = "v",
        long,
        help = "increase verbosity level (up to 4 times)",
        parse(from_occurrences),
        display_order(98)
    )]
    pub verbose: usize,

    #[structopt(
        short = "t",
        long,
        help = "show timestmaps in log messages",
        display_order(99)
    )]
    pub timestamp: bool,

    #[structopt(
        short = "w",
        long,
        help = "specify the bitcoind wallet to use (optional)",
        env,
        hide_env_values(true),
        display_order(30)
    )]
    pub bitcoind_wallet: Option<String>,

    #[structopt(
        short = "d",
        long,
        help = "path to bitcoind directory (used for cookie file) [default: ~/.bitcoin]",
        env,
        hide_env_values(true),
        display_order(31)
    )]
    pub bitcoind_dir: Option<path::PathBuf>,

    #[structopt(
        short = "u",
        long,
        help = "url for the bitcoind rpc server [default: http://localhost:<network-rpc-port>]",
        env,
        hide_env_values(true),
        display_order(32)
    )]
    pub bitcoind_url: Option<String>,

    #[structopt(
        short = "c",
        long,
        help = "credentials for accessing the bitcoind rpc server (as <username>:<password>, instead of reading the cookie file)",
        env,
        hide_env_values(true),
        display_order(33)
    )]
    pub bitcoind_cred: Option<String>,

    #[structopt(
        short = "C",
        long,
        help = "cookie file for accessing the bitcoind rpc server [default: <bitcoind-dir>/.cookie]",
        env,
        hide_env_values(true),
        display_order(34)
    )]
    pub bitcoind_cookie: Option<path::PathBuf>,

    #[structopt(
        short = "x",
        long = "xpub",
        help = "xpubs to track and since when (rescans from genesis by default, use <xpub>:<yyyy-mm-dd> or <xpub>:<unix-epoch> to specify a timestmap, or <xpub>:none to disable rescanning)",
        parse(try_from_str = parse_xpub),
        env, hide_env_values(true), use_delimiter(true),
        display_order(20)
    )]
    pub xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[structopt(
        short = "X",
        long = "bare-xpub",
        help = "bare xpubs to track (like --xpub but does not derive separate internal and external chains)",
        parse(try_from_str = parse_xpub),
        env, hide_env_values(true), use_delimiter(true),
        display_order(21)
    )]
    pub bare_xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[structopt(
        short = "g",
        long,
        help = "gap limit for importing hd addresses",
        default_value = "20",
        env,
        hide_env_values(true),
        display_order(51)
    )]
    pub gap_limit: u32,

    #[structopt(
        short = "G",
        long,
        help = "the batch size to use for importing addresses during the initial sync (set higher to reduce number of rescans)",
        default_value = "50",
        env,
        hide_env_values(true),
        display_order(52)
    )]
    pub initial_import_size: u32,

    //// TODO
    //#[structopt(
    //short,
    //long,
    //help = "addresses to track (address:yyyy-mm-dd)",
    //parse(try_from_str = "parse_address")
    //)]
    //addresses: Vec<(String, RescanSince)>,
    #[cfg(feature = "electrum")]
    #[structopt(
        short,
        long,
        help = "address to bind the electrum rpc server [default: '127.0.0.1:50001' for mainnet, '127.0.0.1:50001' for testnet or '127.0.0.2:60401' for regtest]",
        env,
        hide_env_values(true),
        display_order(40)
    )]
    pub electrum_rpc_addr: Option<net::SocketAddr>,

    #[cfg(feature = "http")]
    #[structopt(
        short,
        long,
        help = "address to bind the http api server",
        default_value = "127.0.0.1:3060",
        env,
        hide_env_values(true),
        display_order(41)
    )]
    pub http_server_addr: net::SocketAddr,

    #[cfg(feature = "http")]
    #[structopt(
        long,
        help = "allowed cross-origins for http api server (Access-Control-Allow-Origin)",
        env,
        hide_env_values(true),
        display_order(42)
    )]
    pub http_cors: Option<String>,

    #[structopt(
        short = "i",
        long,
        help = "interval for checking new blocks/txs (in seconds)",
        default_value = "5",
        parse(try_from_str = parse_duration),
        env, hide_env_values(true),
        display_order(90)
    )]
    pub poll_interval: time::Duration,

    #[cfg(unix)]
    #[structopt(
        long,
        short = "U",
        help = "path for binding sync notification unix socket",
        env,
        hide_env_values(true),
        display_order(91)
    )]
    pub unix_listener_path: Option<path::PathBuf>,

    #[cfg(feature = "webhooks")]
    #[structopt(
        long,
        short = "h",
        help = "webhook url(s) to notify with index event updates",
        env,
        hide_env_values(true),
        use_delimiter(true),
        display_order(92)
    )]
    pub webhook_urls: Option<Vec<String>>,
}

impl Config {
    pub fn dotenv() {
        dirs::home_dir().map(|home| dotenv::from_path(home.join("bwt.env")).ok());
    }

    pub fn bitcoind_url(&self) -> String {
        format!(
            "{}/{}",
            self.bitcoind_url.clone().unwrap_or_else(|| {
                format!(
                    "http://localhost:{}",
                    match self.network {
                        Network::Bitcoin => 8332,
                        Network::Testnet => 18332,
                        Network::Regtest => 18443,
                    }
                )
            }),
            match self.bitcoind_wallet {
                Some(ref wallet) => format!("wallet/{}", wallet),
                None => "".into(),
            }
        )
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

    #[cfg(feature = "electrum")]
    pub fn electrum_rpc_addr(&self) -> net::SocketAddr {
        self.electrum_rpc_addr.clone().unwrap_or_else(|| {
            net::SocketAddr::new(
                "127.0.0.1".parse().unwrap(),
                match self.network {
                    Network::Bitcoin => 50001,
                    Network::Testnet => 60001,
                    Network::Regtest => 60401,
                },
            )
        })
    }

    pub fn setup_logger(&self) {
        if self.timestamp {
            pretty_env_logger::formatted_timed_builder()
        } else {
            pretty_env_logger::formatted_builder()
        }
        .filter_module(
            "bwt",
            match self.verbose {
                0 => Level::Info,
                1 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .filter_module(
            "bitcoincore_rpc",
            match self.verbose {
                0 | 1 => Level::Warn,
                2 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .filter_module(
            "warp",
            match self.verbose {
                0 | 1 => Level::Warn,
                2 => Level::Info,
                3 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .filter_module("hyper", Level::Warn.to_level_filter())
        .filter_level(
            match self.verbose {
                0 | 1 => Level::Warn,
                2 | 3 => Level::Info,
                4 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .init();
    }
}

fn parse_xpub(s: &str) -> Result<(XyzPubKey, RescanSince)> {
    let mut parts = s.splitn(2, ":");
    let xpub = XyzPubKey::from_str(parts.next().or_err("missing xpub")?)?;
    let rescan = parts
        .next()
        .map_or(Ok(RescanSince::Timestamp(0)), parse_rescan)?;
    Ok((xpub, rescan))
}

fn parse_rescan(s: &str) -> Result<RescanSince> {
    Ok(match s {
        "none" => RescanSince::Now,
        s => {
            // try as a unix timestamp first, then as a datetime string
            RescanSince::Timestamp(
                s.parse::<u64>()
                    .or_else(|_| parse_yyyymmdd(s))
                    .context("invalid rescan value")?,
            )
        }
    })
}

fn parse_yyyymmdd(s: &str) -> Result<u64> {
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
        .timestamp() as u64)
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
