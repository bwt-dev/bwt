#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitcoin_hashes;
#[macro_use]
extern crate serde;

#[macro_use]
pub mod util;

pub mod app;
pub mod config;
pub mod error;
pub mod indexer;
pub mod query;
pub mod store;
pub mod types;
pub mod wallet;

#[cfg(unix)]
pub mod listener;

#[cfg(feature = "electrum")]
pub mod electrum;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "webhooks")]
pub mod webhooks;

pub use app::App;
pub use config::Config;
pub use error::{Error, Result};
pub use indexer::{IndexChange, Indexer};
pub use query::Query;
pub use types::RescanSince;
pub use util::progress::Progress;
pub use wallet::{Wallet, WalletWatcher};

pub const BWT_VERSION: &str = env!("CARGO_PKG_VERSION");
