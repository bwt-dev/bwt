use std::{net, path, time};

use bitcoin::Network;
use bitcoincore_rpc::Auth as RpcAuth;

use crate::error::{OptionExt, Result};
use crate::query::QueryConfig;
use crate::types::RescanSince;
use crate::util::descriptor::ExtendedDescriptor;
use crate::util::xpub::XyzPubKey;

#[cfg(feature = "pretty_env_logger")]
use {log::Level, pretty_env_logger::env_logger::Builder as LogBuilder};

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "cli", derive(structopt::StructOpt))]
pub struct Config {
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "n",
            long,
            help = "One of 'bitcoin', 'testnet' or 'regtest'",
            default_value = "bitcoin",
            env,
            hide_env_values(true),
            display_order(1)
        )
    )]
    #[serde(default = "default_network")]
    pub network: Network,

    // cannot be set using an env var, it does not play nicely with from_occurrences
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "v",
            long,
            help = "Increase verbosity level (up to 4 times)",
            parse(from_occurrences),
            display_order(98)
        )
    )]
    #[serde(default = "default_verbose")]
    pub verbose: usize,

    // XXX not settable as an env var due to https://github.com/TeXitoi/structopt/issues/305
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "t",
            long,
            help = "Show timestmaps in log messages",
            display_order(99)
        )
    )]
    #[serde(default = "default_false")]
    pub timestamp: bool,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "w",
            long,
            help = "Specify the bitcoind wallet to use (optional)",
            env,
            hide_env_values(true),
            display_order(30)
        )
    )]
    pub bitcoind_wallet: Option<String>,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "r",
            long,
            help = "Path to bitcoind directory (used for cookie file) [default: ~/.bitcoin)",
            env,
            hide_env_values(true),
            display_order(31)
        )
    )]
    pub bitcoind_dir: Option<path::PathBuf>,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "u",
            long,
            help = "URL for the bitcoind RPC server [default: http://localhost:<network-rpc-port>)",
            env,
            hide_env_values(true),
            display_order(32)
        )
    )]
    pub bitcoind_url: Option<String>,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "a",
            long,
            help = "Credentials for accessing the bitcoind RPC server (as <username>:<password>, used instead of the cookie file)",
            alias = "bitcoind-cred",
            env,
            hide_env_values(true),
            display_order(33)
        )
    )]
    pub bitcoind_auth: Option<String>,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "c",
            long,
            help = "Cookie file for accessing the bitcoind RPC server [default: <bitcoind-dir>/.cookie)",
            env,
            hide_env_values(true),
            display_order(34)
        )
    )]
    pub bitcoind_cookie: Option<path::PathBuf>,

    #[cfg_attr(feature = "cli", structopt(
        short = "d",
        long = "descriptor",
        help = "Descriptors to track (scans for history from the genesis by default, use <desc>@<yyyy-mm-dd> or <desc>@<unix-epoch> to specify a rescan timestmap, or <desc>@none to disable rescan)",
        parse(try_from_str = parse_desc),
        env, hide_env_values(true),
        use_delimiter(true), value_delimiter(";"),
        display_order(20)
    ))]
    #[serde(default = "default_empty_vec")]
    pub descriptors: Vec<(ExtendedDescriptor, RescanSince)>,

    #[cfg_attr(feature = "cli", structopt(
        short = "x",
        long = "xpub",
        help = "xpubs to track (represented as two separate descriptors for the internal/external chains, supports <xpub>@<rescan-time>)",
        parse(try_from_str = parse_xpub),
        env, hide_env_values(true),
        use_delimiter(true), value_delimiter(";"),
        display_order(21)
    ))]
    #[serde(default = "default_empty_vec")]
    pub xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[cfg_attr(feature = "cli", structopt(
        short = "X",
        long = "bare-xpub",
        help = "Bare xpubs to track (like --xpub, but does not derive separate internal/external chains)",
        parse(try_from_str = parse_xpub),
        env, hide_env_values(true), use_delimiter(true),
        display_order(22)
    ))]
    #[serde(default = "default_empty_vec")]
    pub bare_xpubs: Vec<(XyzPubKey, RescanSince)>,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "g",
            long,
            help = "Gap limit for importing child addresses",
            default_value = "20",
            env,
            hide_env_values(true),
            display_order(51)
        )
    )]
    #[serde(default = "default_gap_limit")]
    pub gap_limit: u32,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "G",
            long,
            help = "The batch size for importing addresses during the initial sync (set higher to reduce number of rescans)",
            default_value = "350",
            env,
            hide_env_values(true),
            display_order(52)
        )
    )]
    #[serde(default = "default_initial_import_size")]
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
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "e",
            long,
            help = "Address to bind the electrum rpc server [default: '127.0.0.1:50001' for mainnet, '127.0.0.1:60001' for testnet or '127.0.0.1:60401' for regtest]",
            env,
            hide_env_values(true),
            display_order(40)
        )
    )]
    pub electrum_rpc_addr: Option<net::SocketAddr>,

    // XXX not settable as an env var due to https://github.com/TeXitoi/structopt/issues/305
    #[cfg(feature = "electrum")]
    #[cfg_attr(
        feature = "cli",
        structopt(
            long,
            help = "Skip generating merkle proofs. Reduces resource usage, requires running Electrum with --skipmerklecheck",
            display_order(41)
        )
    )]
    #[serde(default = "default_false")]
    pub electrum_skip_merkle: bool,

    #[cfg(feature = "http")]
    #[cfg_attr(
        feature = "cli",
        structopt(
            short,
            long,
            help = "Address to bind the http api server",
            default_value = "127.0.0.1:3060",
            env,
            hide_env_values(true),
            display_order(45)
        )
    )]
    #[serde(default = "default_http_server_addr")]
    pub http_server_addr: net::SocketAddr,

    #[cfg(feature = "http")]
    #[cfg_attr(
        feature = "cli",
        structopt(
            long,
            help = "Allowed cross-origins for http api server (Access-Control-Allow-Origin)",
            env,
            hide_env_values(true),
            display_order(46)
        )
    )]
    pub http_cors: Option<String>,

    #[cfg_attr(feature = "cli", structopt(
        short = "i",
        long,
        help = "Interval for checking for new blocks/seconds (in seconds)",
        default_value = "5",
        parse(try_from_str = parse_duration),
        env, hide_env_values(true),
        display_order(90)
    ))]
    #[serde(default = "default_poll_interval")]
    pub poll_interval: time::Duration,

    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "B",
            long = "tx-broadcast-cmd",
            help = "Custom command for broadcasting transactions. {tx_hex} is replaced with the transaction.",
            env,
            hide_env_values(true),
            display_order(91)
        )
    )]
    pub broadcast_cmd: Option<String>,

    // XXX this is not settable as an env var due to https://github.com/clap-rs/clap/issues/1476
    #[cfg_attr(feature = "cli", structopt(
        long = "no-startup-banner",
        help = "Disable the startup banner",
        parse(from_flag = std::ops::Not::not),
        display_order(92)
    ))]
    #[serde(default = "default_false")]
    pub startup_banner: bool,

    #[cfg(unix)]
    #[cfg_attr(
        feature = "cli",
        structopt(
            long,
            short = "U",
            help = "Path to bind the sync notification unix socket",
            env,
            hide_env_values(true),
            display_order(101)
        )
    )]
    pub unix_listener_path: Option<path::PathBuf>,

    #[cfg(feature = "webhooks")]
    #[cfg_attr(
        feature = "cli",
        structopt(
            long = "webhook-url",
            short = "H",
            help = "Webhook url(s) to notify with index event updates",
            env,
            hide_env_values(true),
            use_delimiter(true),
            value_delimiter(";"),
            display_order(102)
        )
    )]
    pub webhook_urls: Option<Vec<String>>,
}

impl Config {
    pub fn dotenv() {
        #[cfg(feature = "cli")]
        dirs::home_dir().map(|home| dotenv::from_path(home.join("bwt.env")).ok());
    }

    pub fn bitcoind_url(&self) -> String {
        format!(
            "{}/{}",
            self.bitcoind_url.as_ref().map_or_else(
                || {
                    format!(
                        "http://localhost:{}",
                        match self.network {
                            Network::Bitcoin => 8332,
                            Network::Testnet => 18332,
                            Network::Regtest => 18443,
                        }
                    )
                },
                |url| url.trim_end_matches('/').into()
            ),
            match self.bitcoind_wallet {
                Some(ref wallet) => format!("wallet/{}", wallet),
                None => "".into(),
            }
        )
    }

    pub fn bitcoind_auth(&self) -> Result<RpcAuth> {
        Ok(self.bitcoind_auth
            .as_ref()
            .and_then(|auth| {
                let mut parts = auth.splitn(2, ':');
                Some(RpcAuth::UserPass(parts.next()?.into(), parts.next()?.into()))
            })
            .or_else(|| {
                let cookie = self.bitcoind_cookie.clone().or_else(|| get_cookie(self))?;
                Some(RpcAuth::CookieFile(cookie))
            })
            .or_err("no valid authentication found for bitcoind rpc, specify user/pass or a cookie file")?)
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
        #[cfg(feature = "pretty_env_logger")]
        apply_log_env(if self.timestamp {
            pretty_env_logger::formatted_timed_builder()
        } else {
            pretty_env_logger::formatted_builder()
        })
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

#[cfg(feature = "pretty_env_logger")]
fn apply_log_env(mut builder: LogBuilder) -> LogBuilder {
    use std::env;
    if let Ok(s) = env::var("RUST_LOG") {
        builder.parse_filters(&s);
    }
    if let Ok(s) = env::var("RUST_LOG_STYLE") {
        builder.parse_write_style(&s);
    }
    builder
}

#[cfg(feature = "cli")]
fn parse_desc(s: &str) -> Result<(ExtendedDescriptor, RescanSince)> {
    use crate::util::descriptor::DescriptorChecksum;
    let mut parts = s.trim().splitn(2, '@');
    let desc = ExtendedDescriptor::parse_with_checksum(parts.next().req()?)?;
    let rescan = parse_rescan(parts.next())?;
    Ok((desc, rescan))
}

#[cfg(feature = "cli")]
fn parse_xpub(s: &str) -> Result<(XyzPubKey, RescanSince)> {
    let mut parts = s.trim().splitn(2, '@');
    let xpub = parts.next().req()?.parse()?;
    let rescan = parse_rescan(parts.next())?;
    Ok((xpub, rescan))
}

#[cfg(feature = "cli")]
fn parse_rescan(s: Option<&str>) -> Result<RescanSince> {
    use crate::error::Context;
    Ok(match s {
        None | Some("all") => RescanSince::Timestamp(0),
        Some("now") | Some("none") => RescanSince::Now,
        Some(s) => {
            // try as a unix timestamp first, then as a datetime string
            RescanSince::Timestamp(
                s.parse::<u64>()
                    .or_else(|_| parse_yyyymmdd(s))
                    .context("invalid rescan value")?,
            )
        }
    })
}

#[cfg(feature = "cli")]
fn parse_yyyymmdd(s: &str) -> Result<u64> {
    use chrono::{TimeZone, Utc};
    let mut parts = s.splitn(3, '-');
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

#[cfg(feature = "cli")]
fn parse_duration(s: &str) -> Result<time::Duration> {
    Ok(time::Duration::from_secs(s.parse()?))
}

fn get_cookie(config: &Config) -> Option<path::PathBuf> {
    let mut dir = config.bitcoind_dir.clone().or_else(bitcoind_default_dir)?;
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

#[cfg(feature = "dirs")]
fn bitcoind_default_dir() -> Option<path::PathBuf> {
    // Windows: C:\Users\Satoshi\Appdata\Roaming\Bitcoin
    #[cfg(target_os = "windows")]
    return Some(dirs::data_dir()?.join("Bitcoin"));

    // macOS: ~/Library/Application Support/Bitcoin
    #[cfg(target_os = "macos")]
    return Some(dirs::config_dir()?.join("Bitcoin"));

    // Linux and others: ~/.bitcoin
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    return Some(dirs::home_dir()?.join(".bitcoin"));
}
#[cfg(not(feature = "dirs"))]
fn bitcoind_default_dir() -> Option<path::PathBuf> {
    None
}

impl From<&Config> for QueryConfig {
    fn from(config: &Config) -> QueryConfig {
        QueryConfig {
            network: config.network,
            broadcast_cmd: config.broadcast_cmd.clone(),
        }
    }
}

// NOTE: the default values below are also duplicated in structopt's attributes

// Create a Default implementation
defaultable!(Config,
  @default(
    verbose, timestamp, descriptors, xpubs, bare_xpubs, broadcast_cmd, startup_banner,
    bitcoind_wallet, bitcoind_dir, bitcoind_url, bitcoind_auth, bitcoind_cookie,
    #[cfg(feature = "electrum")] electrum_rpc_addr,
    #[cfg(feature = "electrum")] electrum_skip_merkle,
    #[cfg(feature = "http")] http_cors,
    #[cfg(feature = "webhooks")] webhook_urls,
    #[cfg(unix)] unix_listener_path,
  )
  @custom(
    network=Network::Bitcoin, gap_limit=20, initial_import_size=350, poll_interval=time::Duration::from_secs(5),
    #[cfg(feature = "http")] http_server_addr=([127,0,0,1],3060).into(),
  )
);

// Used for serde's default attributes, which must be provided as functions

fn default_false() -> bool {
    false
}
fn default_network() -> Network {
    Network::Bitcoin
}
fn default_verbose() -> usize {
    0
}
fn default_gap_limit() -> u32 {
    20
}
fn default_initial_import_size() -> u32 {
    350
}
fn default_poll_interval() -> time::Duration {
    time::Duration::from_secs(5)
}
fn default_empty_vec<T>() -> Vec<T> {
    vec![]
}
#[cfg(feature = "http")]
fn default_http_server_addr() -> net::SocketAddr {
    ([127, 0, 0, 1], 3060).into()
}
