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
mod macros;

pub mod app;
pub mod config;
pub mod error;
pub mod hd;
pub mod indexer;
pub mod mempool;
pub mod query;
pub mod store;
pub mod types;
pub mod util;

pub use app::App;
pub use config::Config;
pub use error::{Error, Result};
pub use hd::{HDWallet, HDWatcher};
pub use indexer::Indexer;
pub use query::Query;

#[cfg(feature = "electrum")]
pub mod electrum;
#[cfg(feature = "electrum")]
pub mod merkle;

#[cfg(feature = "http")]
pub mod http;
