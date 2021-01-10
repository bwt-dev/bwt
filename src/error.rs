use core::fmt::Display;

pub use anyhow::{Context, Error, Result};

use bitcoin::{BlockHash, Txid};
use bitcoincore_rpc as rpc;

use crate::types::ScriptHash;

#[cfg(feature = "http")]
use warp::http::StatusCode;

#[derive(thiserror::Error, Debug)]
pub enum BwtError {
    #[error("Reorg detected at height {0} (previous={1} current={2})")]
    ReorgDetected(u32, BlockHash, BlockHash),

    #[error("Transaction not found: {0}")]
    TxNotFound(Txid),

    #[error("Address or script hash not found: {0}")]
    ScriptHashNotFound(ScriptHash),

    #[error("Blocks unavailable due to pruning")]
    PrunedBlocks,

    #[error("The operation was canceled")]
    Canceled,

    #[error("Custom broadcast command failed with {0}")]
    BroadcastCmdFailed(std::process::ExitStatus),

    #[error("Error communicating with the Bitcoin RPC: {0}")]
    RpcProtocol(rpc::Error),

    #[error("Bitcoin RPC error code {}: {}", .0.code, .0.message)]
    Rpc(rpc::jsonrpc::error::RpcError),
}

impl BwtError {
    #[cfg(feature = "http")]
    pub fn status_code(&self) -> StatusCode {
        match self {
            BwtError::ReorgDetected(..) => StatusCode::GONE,
            BwtError::PrunedBlocks => StatusCode::GONE,
            BwtError::TxNotFound(_) => StatusCode::NOT_FOUND,
            BwtError::ScriptHashNotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
impl From<rpc::Error> for BwtError {
    fn from(err: rpc::Error) -> Self {
        use crate::util::bitcoincore_ext::RPC_MISC_ERROR;
        if let rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(e)) = err {
            match (e.code, e.message.as_str()) {
                (RPC_MISC_ERROR, "Block not available (pruned data)") => BwtError::PrunedBlocks,
                _ => BwtError::Rpc(e),
            }
        } else {
            BwtError::RpcProtocol(err)
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
