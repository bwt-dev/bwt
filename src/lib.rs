#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
extern crate stderrlog;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;

pub mod addrman;
pub mod error;
pub mod hd;
pub mod json;
pub mod mempool;
pub mod query;
pub mod util;

pub use addrman::AddrManager;
pub use error::{Error, Result};
pub use hd::{HDWallet, HDWatcher, KeyRescan};
pub use query::Query;

#[cfg(feature = "electrum")]
pub mod electrum;
#[cfg(feature = "electrum")]
pub mod merkle;

#[cfg(feature = "electrum")]
pub use electrum::ElectrumServer;
