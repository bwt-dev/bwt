use std::{net, path, time};

use bitcoin::Network;
use bitcoincore_rpc::Auth as RpcAuth;
use chrono::{TimeZone, Utc};
use dirs::home_dir;
use structopt::StructOpt;

use crate::error::{OptionExt, Result, ResultExt};

#[derive(Debug)]
pub struct Config {
    pub network: Network,
    pub xpubs: Vec<(String, Option<u32>)>,
    pub verbose: usize,
    pub poll_interval: time::Duration,

    pub bitcoind_url: String,
    pub bitcoind_auth: RpcAuth,

    #[cfg(feature = "electrum")]
    pub electrum_rpc_addr: Option<net::SocketAddr>,
    #[cfg(feature = "http")]
    pub http_server_addr: Option<net::SocketAddr>,
}

#[derive(StructOpt)]
#[structopt(
    name = "server config",
    about = "server config for personal-xpub-tracker"
)]
pub struct CliConfig {
    #[structopt(
        short,
        long,
        help = "one of 'bitcoin', 'testnet' or 'regtest'",
        default_value = "bitcoin"
    )]
    network: Network,

    #[structopt(
        short,
        long,
        help = "increase verbosity level (up to 3 times)",
        parse(from_occurrences)
    )]
    verbose: usize,

    #[structopt(
        short = "i",
        long = "poll-interval",
        help = "interval for checking new blocks/txs (in seconds)",
        default_value = "5",
        parse(try_from_str = "parse_duration")
    )]
    poll_interval: time::Duration,

    // bitcoind related configuration
    #[structopt(
        short = "d",
        long = "bitcoind-dir",
        help = "path to bitcoind directory (used for cookie file, defaults to ~/.bitcoin/<network>)"
    )]
    bitcoind_dir: Option<path::PathBuf>,

    #[structopt(
        short = "u",
        long = "bitcoind-url",
        help = "url for the bitcoind rpc server (defaults to http://localhost:<network-rpc-port>)"
    )]
    bitcoind_url: Option<String>,

    #[structopt(
        short = "C",
        long = "bitcoind-cred",
        help = "credentials for accessing the bitcoind rpc server (as <username>:<password>, instead of reading the cookie file)"
    )]
    bitcoind_cred: Option<String>,

    #[structopt(
        short = "c",
        long = "bitcoind-cookie",
        help = "cookie file for accessing the bitcoind rpc server (defaults to <bitcoind-dir>/.cookie)"
    )]
    bitcoind_cookie: Option<path::PathBuf>,

    // wallets to watch
    #[structopt(
        short,
        long = "xpub",
        help = "xpubs to scan and since when (<xpub>, <xpub>:now, <xpub>:<yyyy-mm-dd> or <xpub>:<unix-epoch>)",
        parse(try_from_str = "parse_xpub")
    )]
    xpubs: Vec<(String, Option<u32>)>,

    //// TODO
    //#[structopt(
    //short,
    //long = "address",
    //help = "addresses to track (address:yyyy-mm-dd)",
    //parse(try_from_str = "parse_address")
    //)]
    //addresses: Vec<(String, Option<u32>)>,

    // pxt server configuration
    #[cfg(feature = "electrum")]
    #[structopt(
        short,
        long = "electrum-rpc-addr",
        help = "address to bind the electrum rpc server (host:port)"
    )]
    electrum_rpc_addr: Option<net::SocketAddr>,

    #[cfg(feature = "http")]
    #[structopt(
        short,
        long = "http-server-addr",
        help = "address to bind the http rest server (host:port)"
    )]
    http_server_addr: Option<net::SocketAddr>,
}

impl Config {
    pub fn from_args() -> Self {
        // use structopt to parse args into CliConfig first, then do some more processing
        // and convert it convert it intoa Config
        Self::from_cli(CliConfig::from_args()).unwrap()
    }

    fn from_cli(config: CliConfig) -> Result<Self> {
        #[cfg(feature = "electrum")]
        let electrum_rpc_addr = config.electrum_rpc_addr;

        #[cfg(feature = "http")]
        let http_server_addr = config.http_server_addr;

        let CliConfig {
            network,
            verbose,
            poll_interval,
            bitcoind_url,
            bitcoind_dir,
            bitcoind_cred,
            bitcoind_cookie,
            xpubs,
            ..
        } = config;

        let bitcoind_url = bitcoind_url.unwrap_or_else(|| {
            format!(
                "http://localhost:{}/",
                match network {
                    Network::Bitcoin => 8332,
                    Network::Testnet => 18332,
                    Network::Regtest => 18443,
                }
            )
        });

        // might be a None if there's no known home directory
        let bitcoind_dir = bitcoind_dir.or_else(|| {
            let mut dir = home_dir()?.join(".bitcoin");
            match network {
                Network::Bitcoin => (),
                Network::Testnet => dir.push("testnet3"),
                Network::Regtest => dir.push("regtest"),
            }
            Some(dir)
        });

        let bitcoind_auth = bitcoind_cred
            .and_then(|cred| {
                let mut parts = cred.splitn(2, ":");
                Some(RpcAuth::UserPass(parts.next()?.into(), parts.next()?.into()))
            })
            .or_else(|| {
                let cookie = bitcoind_cookie.or_else(|| {
                    let cookie = bitcoind_dir?.join(".cookie");
                    if cookie.exists() {
                        Some(cookie)
                    } else {
                        None
                    }
                })?;
                Some(RpcAuth::CookieFile(cookie))
            })
            .or_err("no available authentication for bitcoind rpc, please specify credentials or a cookie file")?;

        Ok(Config {
            network,
            verbose,
            poll_interval,
            bitcoind_url,
            bitcoind_auth,
            xpubs,

            #[cfg(feature = "electrum")]
            electrum_rpc_addr,
            #[cfg(feature = "http")]
            http_server_addr,
        })
    }
}

fn parse_xpub(s: &str) -> Result<(String, Option<u32>)> {
    let mut parts = s.splitn(2, ":");
    let xpub = parts.next().or_err("missing xpub")?;
    let creation_time = parts.next().map_or(Ok(Some(0)), parse_rescan)?;
    Ok((xpub.into(), creation_time))
}

fn parse_rescan(s: &str) -> Result<Option<u32>> {
    Ok(if s == "now" {
        None
    } else {
        // try as a unix timestamp first, then as a datetime string
        Some(
            s.parse::<u32>()
                .or_else(|_| parse_yyyymmdd(s))
                .context("invalid rescan value")?,
        )
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
