use std::sync::mpsc;
use std::sync::{Arc, RwLock};

use bitcoincore_rpc::Client as RpcClient;

use crate::{Config, HDWallet, HDWatcher, Indexer, Query, Result};

#[cfg(feature = "electrum")]
use crate::electrum::ElectrumServer;
#[cfg(feature = "http")]
use crate::http::HttpServer;
#[cfg(unix)]
use crate::listener;
#[cfg(feature = "webhooks")]
use crate::webhooks::WebHookNotifier;

pub struct App {
    config: Config,
    indexer: Arc<RwLock<Indexer>>,
    query: Arc<Query>,

    sync_channel: (mpsc::Sender<()>, mpsc::Receiver<()>),

    #[cfg(feature = "electrum")]
    electrum: ElectrumServer,
    #[cfg(feature = "http")]
    http: HttpServer,
    #[cfg(feature = "webhooks")]
    webhook: Option<WebHookNotifier>,
}

impl App {
    pub fn boot(config: Config) -> Result<Self> {
        info!("[app] {:?}", config);

        let wallets = HDWallet::from_xpubs(&config.xpubs[..], config.network)?;
        let watcher = HDWatcher::new(wallets);

        let rpc = Arc::new(RpcClient::new(
            config.bitcoind_url(),
            config.bitcoind_auth()?,
        )?);
        let indexer = Arc::new(RwLock::new(Indexer::new(Arc::clone(&rpc), watcher)));
        let query = Arc::new(Query::new(Arc::clone(&rpc), Arc::clone(&indexer)));
        let (tx, rx) = mpsc::channel();

        // do an initial sync without keeping track of updates
        indexer.write().unwrap().sync(false)?;

        #[cfg(feature = "electrum")]
        let electrum = ElectrumServer::start(config.electrum_rpc_addr(), Arc::clone(&query));

        #[cfg(feature = "http")]
        let http = HttpServer::start(config.http_server_addr, Arc::clone(&query), tx.clone());

        #[cfg(unix)]
        {
            if let Some(listener_path) = &config.unix_listener_path {
                listener::start(listener_path.clone(), tx.clone());
            }
        }

        #[cfg(feature = "webhooks")]
        let webhook = config
            .webhook_urls
            .clone()
            .map(|urls| WebHookNotifier::start(urls));

        Ok(App {
            config,
            indexer,
            query,
            sync_channel: (tx, rx),
            #[cfg(feature = "electrum")]
            electrum,
            #[cfg(feature = "http")]
            http,
            #[cfg(feature = "webhooks")]
            webhook,
        })
    }

    /// Start a sync loop blocking the current thread
    pub fn sync(self) {
        loop {
            match self.indexer.write().unwrap().sync(true) {
                Ok(updates) if updates.len() > 0 => {
                    #[cfg(feature = "electrum")]
                    self.electrum.send_updates(&updates);

                    #[cfg(feature = "http")]
                    self.http.send_updates(&updates);

                    #[cfg(feature = "webhooks")]
                    self.webhook
                        .as_ref()
                        .map(|webhook| webhook.send_updates(&updates));
                }
                Ok(_) => (), // no updates
                Err(e) => warn!("error while updating index: {:#?}", e),
            }

            // wait for poll_interval seconds, or until we receive a sync notification message
            // TODO debounce messages to avoid excessive indexing
            self.sync_channel
                .1
                .recv_timeout(self.config.poll_interval)
                .ok();
        }
    }
}
