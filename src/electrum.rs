use std::sync::Arc;

use bitcoin_hashes::{hex::FromHex, sha256};
use jsonrpc_tcp_server::jsonrpc_core::{
    Error as RpcServerError, IoHandler, Params, Result as RpcResult,
};
use jsonrpc_tcp_server::ServerBuilder;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::query::Query;

pub struct ElectrumServer {
    query: Arc<Query>,
}

impl ElectrumServer {
    pub fn new(query: Arc<Query>) -> Self {
        Self { query }
    }

    pub fn start(self) -> Result<()> {
        let server = Arc::new(self);
        let mut io = IoHandler::default();

        {
            let server = Arc::clone(&server);
            io.add_method("blockchain.block.header", move |params| {
                wrap(server.blockchain_block_header(params))
            });
        }

        {
            let server = Arc::clone(&server);
            io.add_method("blockchain.scripthash.get_history", move |params| {
                wrap(server.blockchain_scripthash_get_history(params))
            });
        }

        let server = ServerBuilder::new(io)
            .start(&"127.0.0.1:9009".parse().unwrap())
            .expect("failed starting server");

        server.wait();

        Ok(())
    }

    fn blockchain_block_header(&self, p: Params) -> Result<String> {
        let (height, cp_height): (u32, Option<u32>) = pad_params(p, 2).parse()?;

        Ok(self.query.get_header(height)?)
    }

    fn blockchain_scripthash_get_history(&self, p: Params) -> Result<Vec<Value>> {
        let (scripthash,): (String,) = p.parse()?;
        let scripthash = sha256::Hash::from_hex(&scripthash)?;

        Ok(self
            .query
            .get_history(&scripthash)?
            .iter()
            .map(|tx| json!({ "height": tx.height, "tx_hash": tx.txid }))
            .collect())
    }
}

fn wrap<T: Serialize>(res: Result<T>) -> RpcResult<Value> {
    res.and_then(|val| serde_json::to_value(val).map_err(Error::from))
        .map_err(|e| {
            warn!("request failed: {:?}", e);
            RpcServerError::invalid_params(e.to_string())
        })
}

/*
fn parse<T: DeserializeOwned>(p: Params) -> RpcResult<T> {
    p.parse().map_err(|e| {
        warn!("parse failed: {:?}", e);
        RpcServerError::invalid_params(e.to_string())
    })
}
*/

fn pad_params(mut params: Params, n: usize) -> Params {
    if let Params::Array(ref mut values) = params {
        while values.len() < n {
            values.push(Value::Null);
        }
    } // passing non-array is a noop
    params
}

/*
impl From<RpcServerError> for Error {
    fn from(err: RpcServerError) -> Self {
        Error::from(err.to_string())
    }
}

impl From<Error> for RpcServerError {
    fn from(err: Error) -> Self {
        RpcServerError::from(err.to_string())
    }
}*/
