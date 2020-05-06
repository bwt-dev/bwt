extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
extern crate structopt;
#[macro_use]
pub extern crate bitcoin_hashes;

pub mod config;
pub mod error;
pub mod hd;
pub mod indexer;
pub mod mempool;
pub mod query;
pub mod types;
pub mod util;

pub use config::Config;
pub use error::{Error, Result};
pub use hd::{HDWallet, HDWatcher};
pub use indexer::Indexer;
pub use query::Query;

#[cfg(feature = "electrum")]
pub mod electrum;
#[cfg(feature = "electrum")]
pub mod merkle;

#[cfg(feature = "electrum")]
pub use electrum::ElectrumServer;
