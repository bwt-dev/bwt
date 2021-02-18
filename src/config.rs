use std::{fs, io, net, path, time};

use bitcoin::{Address, Network};
use bitcoincore_rpc::Auth as RpcAuth;

use crate::error::{Context, OptionExt, Result};
use crate::query::QueryConfig;
use crate::types::RescanSince;
use crate::util::auth::AuthMethod;
use crate::util::descriptor::ExtendedDescriptor;
use crate::util::xpub::XyzPubKey;
use crate::util::BoolThen;

#[cfg(any(feature = "pretty_env_logger", feature = "android_logger"))]
use log::Level;
#[cfg(all(feature = "pretty_env_logger", not(feature = "android_logger")))]
use pretty_env_logger::env_logger::Builder as LogBuilder;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "cli", derive(structopt::StructOpt))]
pub struct Config {
    //
    // General options
    //
    /// One of 'bitcoin', 'testnet', 'signet' or 'regtest'
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "n",
            long,
            default_value = "bitcoin",
            env,
            hide_env_values(true),
            display_order(1)
        )
    )]
    #[serde(default = "default_network")]
    pub network: Network,

    /// Increase verbosity level (up to 4 times) [env: VERBOSE]
    // cannot be set using an env var, it does not play nicely with from_occurrences
    #[cfg_attr(
        feature = "cli",
        structopt(short = "v", long, parse(from_occurrences), display_order(1000))
    )]
    #[serde(default)]
    pub verbose: usize,

    /// Show timestmaps in log messages
    #[cfg_attr(feature = "cli", structopt(short = "T", long, display_order(1001)))]
    #[serde(default)]
    pub timestamp: bool,

    //
    // Bitcoin Core options
    //
    /// Specify the bitcoind wallet to use (optional)
    #[cfg_attr(
        feature = "cli",
        structopt(short = "w", long, env, hide_env_values(true), display_order(30))
    )]
    pub bitcoind_wallet: Option<String>,

    /// Path to bitcoind directory (used for cookie file) [default: ~/.bitcoin]
    #[cfg_attr(
        feature = "cli",
        structopt(short = "r", long, env, hide_env_values(true), display_order(31))
    )]
    pub bitcoind_dir: Option<path::PathBuf>,

    /// URL for the bitcoind RPC server [default: http://localhost:<network-rpc-port>]
    #[cfg_attr(
        feature = "cli",
        structopt(short = "u", long, env, hide_env_values(true), display_order(32))
    )]
    pub bitcoind_url: Option<String>,

    /// Credentials for accessing the bitcoind RPC server (as <username>:<password>, used instead of the cookie file)
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "a",
            long,
            alias = "bitcoind-cred",
            env,
            hide_env_values(true),
            display_order(33)
        )
    )]
    pub bitcoind_auth: Option<String>,

    /// Cookie file for accessing the bitcoind RPC server [default: <bitcoind-dir>/.cookie]
    #[cfg_attr(
        feature = "cli",
        structopt(short = "c", long, env, hide_env_values(true), display_order(34))
    )]
    pub bitcoind_cookie: Option<path::PathBuf>,

    /// Create the specified bitcoind wallet if it's missing [env: CREATE_WALLET_IF_MISSING]
    #[cfg_attr(feature = "cli", structopt(long, short = "W", display_order(1002)))]
    #[serde(default)]
    pub create_wallet_if_missing: bool,

    //
    // Wallet tracking settings
    //
    /// Add a descriptor to track
    #[cfg_attr(feature = "cli", structopt(
        short = "d",
        long = "descriptor",
        parse(try_from_str = parse_desc),
        env, hide_env_values(true),
        use_delimiter(true), value_delimiter(";"),
        display_order(20)
    ))]
    #[serde(default)]
    pub descriptors: Vec<ExtendedDescriptor>,

    /// Add an extended public key to track (with separate internal/external chains)
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "x",
            long = "xpub",
            env,
            hide_env_values(true),
            use_delimiter(true),
            value_delimiter(";"),
            display_order(21)
        )
    )]
    #[serde(default)]
    pub xpubs: Vec<XyzPubKey>,

    /// Add an address to track
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "A",
            long = "address",
            env,
            hide_env_values(true),
            use_delimiter(true),
            value_delimiter(";"),
            display_order(23)
        )
    )]
    #[serde(default)]
    pub addresses: Vec<Address>,

    /// File with addresses to track
    #[cfg_attr(
        feature = "cli",
        structopt(short = "f", long, env, hide_env_values(true), display_order(24))
    )]
    #[serde(default)]
    pub addresses_file: Option<path::PathBuf>,

    /// Start date for wallet history rescan. Accepts YYYY-MM-DD formatted strings, unix timestamps, or 'now' to watch for new transactions only. Defaults to rescanning from genesis.
    // (defaults to scanning from genesis for structopt/cli use, or to 'now' for direct library use)
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "R",
            long,
            parse(try_from_str = parse_rescan),
            default_value = "0",
            env,
            hide_env_values(true),
            display_order(28)
        )
    )]
    #[serde(default = "default_rescan_since")]
    pub rescan_since: RescanSince,

    /// Force rescanning for historical transactions, even if the addresses were already previously imported [env: FORCE_RESCAN]
    #[cfg_attr(feature = "cli", structopt(short = "F", long, display_order(1003)))]
    #[serde(default)]
    pub force_rescan: bool,

    /// Gap limit for importing child addresses
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "g",
            long,
            default_value = "20",
            env,
            hide_env_values(true),
            display_order(51)
        )
    )]
    #[serde(default = "default_gap_limit")]
    pub gap_limit: u32,

    /// The batch size for importing addresses during the initial sync (set higher to reduce number of rescans)
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "G",
            long,
            default_value = "350",
            env,
            hide_env_values(true),
            display_order(52)
        )
    )]
    #[serde(default = "default_initial_import_size")]
    pub initial_import_size: u32,

    //
    // Auth settings
    //
    /// Generate an access token and persist it to the specified cookie file
    #[cfg_attr(
        feature = "cli",
        structopt(short = "C", long, env, hide_env_values(true), display_order(40))
    )]
    pub auth_cookie: Option<path::PathBuf>,

    /// Set access token for authentication
    #[cfg_attr(
        feature = "cli",
        structopt(short = "t", long, env, hide_env_values(true), display_order(41))
    )]
    pub auth_token: Option<String>,

    #[cfg_attr(feature = "cli", structopt(skip = false))]
    #[serde(default)]
    pub auth_ephemeral: bool,

    /// Print access token (useful with --auth-cookie) [env: PRINT_TOKEN]
    #[cfg_attr(feature = "cli", structopt(long, display_order(1006)))]
    #[serde(default)]
    pub print_token: bool,

    //
    // Electrum options
    //
    /// Address to bind the electrum rpc server [default: '127.0.0.1:50001' for mainnet, '127.0.0.1:60001' for testnet, '127.0.0.1:60601' for signet or '127.0.0.1:60401' for regtest]
    #[cfg(feature = "electrum")]
    #[cfg_attr(
        feature = "cli",
        structopt(short = "e", long, env, hide_env_values(true), display_order(43))
    )]
    pub electrum_addr: Option<net::SocketAddr>,

    /// Skip generating merkle proofs. Reduces resource usage, requires running Electrum with --skipmerklecheck. [env: ELECTRUM_SKIP_MERKLE]
    #[cfg(feature = "electrum")]
    #[cfg_attr(feature = "cli", structopt(long, short = "M", display_order(1004)))]
    #[serde(default)]
    pub electrum_skip_merkle: bool,

    /// Enable the Electrum SOCKS5-based authentication mechanism
    /// (see https://github.com/bwt-dev/bwt/blob/master/doc/auth.md) [env: ELECTRUM_SOCKS_AUTH]
    #[cfg(feature = "electrum")]
    #[cfg_attr(feature = "cli", structopt(long, short = "5", display_order(1004)))]
    #[serde(default)]
    pub electrum_socks_auth: bool,

    //
    // HTTP options
    //
    /// Address to bind the http api server [default: 127.0.0.1:3060]
    #[cfg(feature = "http")]
    #[cfg_attr(
        feature = "cli",
        structopt(short, long, env, hide_env_values(true), display_order(45))
    )]
    pub http_addr: Option<net::SocketAddr>,

    /// Allowed cross-origins for http api server (Access-Control-Allow-Origin)
    #[cfg(feature = "http")]
    #[cfg_attr(
        feature = "cli",
        structopt(short = "S", long, env, hide_env_values(true), display_order(46))
    )]
    pub http_cors: Option<String>,

    //
    // Miscellaneous options
    //
    /// Interval for checking for new blocks/seconds (in seconds)
    #[cfg_attr(feature = "cli", structopt(
        short = "i",
        long,
        default_value = "5",
        parse(try_from_str = parse_duration),
        env, hide_env_values(true),
        display_order(90)
    ))]
    #[serde(default = "default_poll_interval")]
    pub poll_interval: time::Duration,

    /// Custom command for broadcasting transactions. {tx_hex} is replaced with the transaction.
    #[cfg_attr(
        feature = "cli",
        structopt(
            short = "B",
            long = "tx-broadcast-cmd",
            env,
            hide_env_values(true),
            display_order(91)
        )
    )]
    pub broadcast_cmd: Option<String>,

    /// Disable the startup banner [env: NO_STARTUP_BANNER]
    #[cfg_attr(feature = "cli", structopt(
        long = "no-startup-banner",
        parse(from_flag = std::ops::Not::not),
        display_order(1005)
    ))]
    #[serde(default)]
    pub startup_banner: bool,

    /// Path to bind the sync notification unix socket
    #[cfg(unix)]
    #[cfg_attr(
        feature = "cli",
        structopt(long, short = "U", env, hide_env_values(true), display_order(101))
    )]
    pub unix_listener_path: Option<path::PathBuf>,

    /// Webhook url(s) to notify with index event updates
    #[cfg(feature = "webhooks")]
    #[cfg_attr(
        feature = "cli",
        structopt(
            long = "webhook-url",
            short = "H",
            env,
            hide_env_values(true),
            use_delimiter(true),
            value_delimiter(";"),
            display_order(102)
        )
    )]
    pub webhook_urls: Option<Vec<String>>,

    // Not exposed as a CLI option, always set to true for CLI use
    #[cfg_attr(feature = "cli", structopt(skip = true))]
    #[serde(default = "default_true")]
    pub require_addresses: bool,

    // Not exposed as a CLI option, always set to true for CLI use
    #[cfg_attr(feature = "cli", structopt(skip = true))]
    #[serde(default = "default_true")]
    pub setup_logger: bool,
}

impl Config {
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
                            Network::Signet => 38332,
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

    pub fn addresses(&self) -> Result<Vec<Address>> {
        let mut addresses = self.addresses.clone();

        if let Some(addresses_file) = &self.addresses_file {
            let file = fs::File::open(addresses_file).context("failed opening addresses file")?;
            let reader = io::BufReader::new(file);

            addresses.append(
                &mut io::BufRead::lines(reader)
                    .filter_map(|l| {
                        let l = l.ok()?;
                        let l = l.trim();
                        (!l.is_empty()).do_then(|| l.parse())
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }

        Ok(addresses)
    }

    pub fn auth_method(&self) -> Result<AuthMethod> {
        Ok(
            match (&self.auth_cookie, &self.auth_token, self.auth_ephemeral) {
                (Some(cookie), None, false) => AuthMethod::Cookie(cookie.clone()),
                (None, Some(token), false) => AuthMethod::UserProvided(token.clone()),
                (None, None, true) => AuthMethod::Ephemeral,
                (None, None, false) => AuthMethod::None,
                _ => bail!("Invalid combination of authentication options"),
            },
        )
    }

    #[cfg(feature = "electrum")]
    pub fn electrum_addr(&self) -> Option<net::SocketAddr> {
        self.electrum_addr.clone().or_else(|| {
            // Use a default value when used as CLI, require explicitly setting it for library use
            #[cfg(feature = "cli")]
            return Some(net::SocketAddr::new(
                [127, 0, 0, 1].into(),
                match self.network {
                    Network::Bitcoin => 50001,
                    Network::Testnet => 60001,
                    Network::Regtest => 60401,
                    Network::Signet => 60601,
                },
            ));
            #[cfg(not(feature = "cli"))]
            return None;
        })
    }

    #[cfg(feature = "http")]
    pub fn http_addr(&self) -> Option<net::SocketAddr> {
        self.http_addr.clone().or_else(|| {
            // Use a default value when used as CLI, require explicitly setting it for library use
            #[cfg(feature = "cli")]
            return Some(([127, 0, 0, 1], 3060).into());

            #[cfg(not(feature = "cli"))]
            return None;
        })
    }

    #[cfg(feature = "cli")]
    pub fn from_args_env() -> Result<Config> {
        use std::env;
        use structopt::StructOpt;

        let mut config = Self::from_args();

        // Setting boolean options as env vars is not supported by clap/structopt
        // https://github.com/TeXitoi/structopt/issues/305
        let bool_env = |key| env::var(key).map_or(false, |val| !val.is_empty() && val != "0");
        if bool_env("FORCE_RESCAN") {
            config.force_rescan = true;
        }
        if bool_env("CREATE_WALLET_IF_MISSING") {
            config.create_wallet_if_missing = true;
        }
        if bool_env("NO_STARTUP_BANNER") {
            config.startup_banner = false;
        }
        if bool_env("NO_REQUIRE_ADDRESSES") {
            config.require_addresses = false;
        }
        if bool_env("PRINT_TOKEN") {
            config.print_token = true;
        }
        #[cfg(feature = "electrum")]
        if bool_env("ELECTRUM_SKIP_MERKLE") {
            config.electrum_skip_merkle = true;
        }
        #[cfg(feature = "electrum")]
        if bool_env("ELECTRUM_SOCKS_AUTH") {
            config.electrum_socks_auth = true;
        }
        if let Ok(verbose) = env::var("VERBOSE") {
            config.verbose = iif!(verbose.is_empty(), 0, verbose.parse().unwrap_or(1));
        }

        // Support configuring xpubs/descriptors using wildcard env vars
        // (XPUB_* and DESCRIPTOR_* / DESC_*)
        for (key, val) in env::vars() {
            if key.starts_with("XPUB_") || key == "XPUB" {
                config.xpubs.push(val.parse()?);
            }
            if key.starts_with("DESC_") || key.starts_with("DESCRIPTOR_") || key == "DESCRIPTOR" {
                config.descriptors.push(val.parse()?);
            }
            if key.starts_with("ADDRESS_") {
                config.addresses.push(val.parse()?);
            }
        }
        Ok(config)
    }

    #[cfg(feature = "cli")]
    pub fn dotenv() {
        dirs::home_dir().map(|home| dotenv::from_path(home.join("bwt.env")).ok());
    }

    #[cfg(all(not(feature = "pretty_env_logger"), not(feature = "android_logger")))]
    pub fn setup_logger(&self) {}

    #[cfg(any(feature = "pretty_env_logger", feature = "android_logger"))]
    pub fn setup_logger(&self) {
        if !self.setup_logger {
            return;
        }

        // If both pretty_env_logger and android_logger are enabled, android_logger takes priority

        #[cfg(all(feature = "pretty_env_logger", not(feature = "android_logger")))]
        let mut builder = apply_log_env(if self.timestamp {
            pretty_env_logger::formatted_timed_builder()
        } else {
            pretty_env_logger::formatted_builder()
        });

        #[cfg(feature = "android_logger")]
        let mut builder = android_logger::FilterBuilder::from_env("RUST_LOG");

        builder
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
            );

        #[cfg(all(feature = "pretty_env_logger", not(feature = "android_logger")))]
        builder.init();

        #[cfg(feature = "android_logger")]
        android_logger::init_once(
            android_logger::Config::default()
                .with_min_level(match self.verbose {
                    0 => Level::Info,
                    1 => Level::Debug,
                    _ => Level::Trace,
                })
                .with_filter(builder.build()),
        );
    }
}

#[cfg(all(feature = "pretty_env_logger", not(feature = "android_logger")))]
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
fn parse_desc(s: &str) -> Result<ExtendedDescriptor> {
    use crate::util::descriptor::DescriptorExt;
    Ok(ExtendedDescriptor::parse_canonical(s)?)
}

#[cfg(feature = "cli")]
fn parse_rescan(s: &str) -> Result<RescanSince> {
    Ok(match s {
        "all" | "genesis" => RescanSince::Timestamp(0),
        "now" | "none" => RescanSince::Now,
        s => {
            // try as a unix timestamp first, then as a datetime string
            RescanSince::Timestamp(
                s.parse::<u64>()
                    .or_else(|_| parse_yyyymmdd(s))
                    .context("invalid rescan-since value")?,
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
            parts.next().required()?.parse()?,
            parts.next().required()?.parse()?,
            parts.next().required()?.parse()?,
        )
        .single()
        .required()?
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
        Network::Signet => dir.push("signet"),
    }
    let cookie = dir.join(".cookie");
    iif!(cookie.exists(), Some(cookie), None)
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
    verbose, timestamp, broadcast_cmd, startup_banner,
    descriptors, xpubs, addresses, addresses_file, force_rescan,
    bitcoind_wallet, bitcoind_dir, bitcoind_url, bitcoind_auth, bitcoind_cookie, create_wallet_if_missing,
    auth_cookie, auth_token, auth_ephemeral, print_token,
    #[cfg(feature = "electrum")] electrum_addr,
    #[cfg(feature = "electrum")] electrum_skip_merkle,
    #[cfg(feature = "electrum")] electrum_socks_auth,
    #[cfg(feature = "http")] http_addr,
    #[cfg(feature = "http")] http_cors,
    #[cfg(feature = "webhooks")] webhook_urls,
    #[cfg(unix)] unix_listener_path,
  )
  @custom(
    network=Network::Bitcoin,
    rescan_since=RescanSince::Now,
    gap_limit=20,
    initial_import_size=350,
    poll_interval=time::Duration::from_secs(5),
    require_addresses=true,
    setup_logger=true,
  )
);

// Used for serde's default attributes, which must be provided as functions

fn default_network() -> Network {
    Network::Bitcoin
}
fn default_rescan_since() -> RescanSince {
    RescanSince::Now
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
fn default_true() -> bool {
    true
}
