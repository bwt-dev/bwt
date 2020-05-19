#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitcoin_hashes;
#[macro_use]
extern crate serde;

#[macro_use]
mod macros;

extern crate shesek_bitcoincore_rpc as bitcoincore_rpc;

pub mod app;
pub mod config;
pub mod error;
pub mod hd;
pub mod indexer;
pub mod query;
pub mod store;
pub mod types;
pub mod util;

#[cfg(unix)]
pub mod listener;

#[cfg(feature = "electrum")]
pub mod electrum;
#[cfg(feature = "electrum")]
pub mod merkle;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "webhooks")]
pub mod webhooks;

pub use app::App;
pub use config::Config;
pub use error::{Error, Result};
pub use hd::{HDWallet, HDWatcher};
pub use indexer::{IndexChange, Indexer};
pub use query::Query;
