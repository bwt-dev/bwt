use std::sync::{Arc, RwLock};
use std::thread;

use bitcoincore_rpc::Client as RpcClient;

use crate::{Config, HDWallet, HDWatcher, Indexer, Query, Result};

#[cfg(feature = "electrum")]
use crate::electrum::ElectrumServer;
#[cfg(feature = "http")]
use crate::http::HttpServer;

pub struct App {
    config: Config,
    indexer: Arc<RwLock<Indexer>>,
    query: Arc<Query>,

    #[cfg(feature = "electrum")]
    electrum: ElectrumServer,
    #[cfg(feature = "http")]
    http: HttpServer,
}

impl App {
    pub fn boot(config: Config) -> Result<Self> {
        info!("booting with config: #{:?}", config);

        let wallets = HDWallet::from_xpubs(&config.xpubs[..], config.network)?;
        let watcher = HDWatcher::new(wallets);

        let rpc = Arc::new(RpcClient::new(
            config.bitcoind_url.clone(),
            config.bitcoind_auth.clone(),
        )?);
        let indexer = Arc::new(RwLock::new(Indexer::new(Arc::clone(&rpc), watcher)));
        let query = Arc::new(Query::new(Arc::clone(&rpc), Arc::clone(&indexer)));

        indexer.write().unwrap().sync()?;

        #[cfg(feature = "electrum")]
        let electrum = ElectrumServer::start(config.electrum_rpc_addr, Arc::clone(&query));

        #[cfg(feature = "http")]
        let http = HttpServer::start(config.http_server_addr, Arc::clone(&query));

        Ok(App {
            config,
            indexer,
            query,
            #[cfg(feature = "electrum")]
            electrum,
            #[cfg(feature = "http")]
            http,
        })
    }

    /// Start a sync loop blocking the current thread
    pub fn sync(self) {
        loop {
            self.indexer
                .write()
                .unwrap()
                .sync()
                .map_err(|err| warn!("error while updating index: {:#?}", err))
                .ok();
            // XXX fatal?

            //indexer.read().unwrap().dump();

            #[cfg(feature = "electrum")]
            self.electrum.notify();

            thread::sleep(self.config.poll_interval);
        }
    }
}
