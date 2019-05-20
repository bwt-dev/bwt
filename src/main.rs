use std::sync::Arc;

use bitcoincore_rpc::{Auth as RpcAuth, Client as RpcClient};

use rust_eps::addrman::AddrManager;
use rust_eps::error::Result;
use rust_eps::query::Query;

#[cfg(feature = "electrum")]
use rust_eps::electrum::ElectrumServer;

fn main() -> Result<()> {
    stderrlog::new().verbosity(3).init()?;

    let rpc_url = "http://localhost:18888/".into();
    let rpc_auth = RpcAuth::UserPass("user3".into(), "password3".into());

    let rpc = Arc::new(RpcClient::new(rpc_url, rpc_auth)?);
    let manager = Arc::new(AddrManager::new(Arc::clone(&rpc)));
    let query = Arc::new(Query::new(Arc::clone(&rpc), Arc::clone(&manager)));

    manager.update()?;

    #[cfg(feature = "electrum")]
    {
        let electrum = ElectrumServer::new(Arc::clone(&query));
        // XXX in separate thread
        electrum.start()?;
    }

    Ok(())
}
