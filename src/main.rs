#[macro_use]
extern crate log;

use std::sync::{Arc, RwLock};
use std::thread;

use bitcoincore_rpc::Client as RpcClient;

use pxt::{AddrManager, Config, HDWallet, HDWatcher, Query, Result};

#[cfg(feature = "electrum")]
use pxt::ElectrumServer;

fn main() -> Result<()> {
    let config = Config::from_args();

    stderrlog::new().verbosity(2 + config.verbose).init()?;

    let wallets = HDWallet::from_xpubs(&config.xpubs[..])?;
    let watcher = HDWatcher::new(wallets);

    let rpc = Arc::new(RpcClient::new(config.bitcoind_url, config.bitcoind_auth)?);
    let manager = Arc::new(RwLock::new(AddrManager::new(Arc::clone(&rpc), watcher)));
    let query = Arc::new(Query::new(Arc::clone(&rpc), Arc::clone(&manager)));

    manager.write().unwrap().update()?;

    #[cfg(feature = "electrum")]
    let electrum = ElectrumServer::start(config.electrum_rpc_addr, Arc::clone(&query));

    loop {
        manager
            .write()
            .unwrap()
            .update()
            .map_err(|err| warn!("error while updating addrman: {:#?}", err))
            .ok();

        #[cfg(feature = "electrum")]
        electrum.notify();

        thread::sleep(config.poll_interval);
    }

    Ok(())
}
