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

    #[structopt(
        short = "x",
        long = "xpub",
        help = "xpubs to track and since when (<xpub>, <xpub>:all, <xpub>:none, <xpub>:<yyyy-mm-dd> or <xpub>:<unix-epoch>)",
        parse(try_from_str = parse_xpub)
    )]
    pub xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[structopt(
        short = "X",
        long = "bare-xpub",
        help = "bare xpubs to track; like --xpub but does not derive separate internal and external chains",
        parse(try_from_str = parse_xpub)
    )]
    pub bare_xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[structopt(
        short = "g",
        long = "gap-limit",
        help = "gap limit for importing hd addresses",
        default_value = "20"
    )]
    pub gap_limit: u32,

    #[structopt(
        short = "G",
        long = "initial-gap-limit",
        help = "gap limit to be used during the initial sync (higher to reduce number of rescans)",
        default_value = "50"
    )]
    pub initial_gap_limit: u32,

    //// TODO
    //#[structopt(
    //short,
    //long = "address",
    //help = "addresses to track (address:yyyy-mm-dd)",
    //parse(try_from_str = "parse_address")
    //)]
    //addresses: Vec<(String, RescanSince)>,
    #[cfg(feature = "electrum")]
    #[structopt(
        short,
        long = "electrum-rpc-addr",
        help = "address to bind the electrum rpc server (host:port)"
    )]
    pub electrum_rpc_addr: Option<net::SocketAddr>,

    #[cfg(feature = "http")]
    #[structopt(
        short,
        long = "http-server-addr",
        help = "address to bind the http api server (host:port)",
        default_value = "127.0.0.1:3060"
    )]
    pub http_server_addr: net::SocketAddr,

    #[cfg(feature = "http")]
    #[structopt(
        long = "cors",
        help = "allowed cross-origins for http api server (Access-Control-Allow-Origin)"
    )]
    pub cors: Option<String>,

    #[cfg(unix)]
    #[structopt(
        long = "unix-listener-path",
        help = "path for binding sync notification unix socket"
    )]
    pub unix_listener_path: Option<path::PathBuf>,

    #[cfg(feature = "webhooks")]
    #[structopt(
        long = "webhook-url",
        help = "webhook url to notify with index event updates"
    )]
    pub webhook_urls: Option<Vec<String>>,
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
        pretty_env_logger::formatted_builder()
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
        "all" => RescanSince::Timestamp(0),
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
