#[macro_use]
extern crate log;

use std::net;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bitcoincore_rpc::{Auth as RpcAuth, Client as RpcClient};

use pxt::{AddrManager, HDWallet, HDWatcher, Query, Result};

#[cfg(feature = "electrum")]
use pxt::ElectrumServer;

fn main() -> Result<()> {
    stderrlog::new().verbosity(2).init()?;

    let wallets = HDWallet::from_xpub("tpubD6NzVbkrYhZ4WmV7Mum4qn9JbyDfjEjAcBUq5ETGd6yrumH8EwgwLhuWbKT1YAcSX4iZr4cY9BgNDHfo8oxfhHssBA3YV6uB1KgTSd9vDcM", Some(0))?;

    let watcher = HDWatcher::new(wallets);

    let rpc_url = "http://localhost:18888/".into();
    let rpc_auth = RpcAuth::UserPass("user3".into(), "password3".into());

    let rpc = Arc::new(RpcClient::new(rpc_url, rpc_auth)?);
    let manager = Arc::new(AddrManager::new(Arc::clone(&rpc), watcher));
    let query = Arc::new(Query::new(Arc::clone(&rpc), Arc::clone(&manager)));

    manager.update()?;

    #[cfg(feature = "electrum")]
    let electrum = {
        let rpc_addr = net::SocketAddr::from_str("127.0.0.1:3005")?;
        ElectrumServer::start(rpc_addr, Arc::clone(&query))
    };

    loop {
        // TODO cpature errors
        manager.update()?;

        #[cfg(feature = "electrum")]
        electrum.notify();

        thread::sleep(Duration::from_secs(5));
    }

    Ok(())
}
