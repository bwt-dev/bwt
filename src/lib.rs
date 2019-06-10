#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate structopt;

pub mod config;
pub mod error;
pub mod hd;
pub mod indexer;
pub mod json;
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
