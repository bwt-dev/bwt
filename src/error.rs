use core::fmt::Display;

pub use anyhow::{Context, Error, Result};

use bitcoin::{BlockHash, Txid};

use crate::types::ScriptHash;

#[cfg(feature = "http")]
use warp::http::StatusCode;

#[derive(thiserror::Error, Debug)]
pub enum BwtError {
    #[error("Reorg detected at height {0} (previous={1} current={2})")]
    ReorgDetected(u32, BlockHash, BlockHash),

    #[error("Transaction not found: {0}")]
    TxNotFound(Txid),

    #[error("Script hash not found: {0}")]
    ScriptHashNotFound(ScriptHash),
}

impl BwtError {
    #[cfg(feature = "http")]
    pub fn status_code(&self) -> StatusCode {
        match self {
            BwtError::ReorgDetected(..) => StatusCode::GONE,
            BwtError::TxNotFound(_) | BwtError::ScriptHashNotFound(_) => StatusCode::NOT_FOUND,
        }
    }
}

// or_err() is somewhat redundant now that anyhow provides Option::context() (this was originally
// implemented for failure), but the or_err() name better expresses the intention so its kept around.

pub trait OptionExt<T> {
    fn or_err<D>(self, context: D) -> Result<T>
    where
        D: Display + Send + Sync + 'static;

    fn req(self) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_err<D>(self, context: D) -> Result<T>
    where
        D: Display + Send + Sync + 'static,
    {
        self.context(context)
    }

    fn req(self) -> Result<T> {
        self.context("missing required option")
    }
}

pub fn fmt_error_chain(err: &Error) -> String {
    err.chain()
        .map(|e| e.to_string())
        .collect::<Vec<String>>()
        .join(": ")
}
