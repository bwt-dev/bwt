use std::sync::{mpsc, Arc, RwLock};
use std::{thread, time};

use bitcoincore_rpc::{Client as RpcClient, RpcApi};

use crate::util::debounce_sender;
use crate::{Config, HDWallet, HDWatcher, Indexer, Query, Result};

#[cfg(feature = "electrum")]
use crate::electrum::ElectrumServer;
#[cfg(feature = "http")]
use crate::http::HttpServer;
#[cfg(unix)]
use crate::listener;
#[cfg(feature = "webhooks")]
use crate::webhooks::WebHookNotifier;

const DEBOUNCE_SEC: u64 = 7;

pub struct App {
    config: Config,
    indexer: Arc<RwLock<Indexer>>,
    sync_chan: (mpsc::Sender<()>, mpsc::Receiver<()>),

    #[cfg(feature = "electrum")]
    electrum: ElectrumServer,
    #[cfg(feature = "http")]
    http: HttpServer,
    #[cfg(feature = "webhooks")]
    webhook: Option<WebHookNotifier>,
}

impl App {
    pub fn boot(config: Config) -> Result<Self> {
        info!("{:?}", config);

        let wallets = HDWallet::from_xpubs(
            &config.xpubs[..],
            &config.bare_xpubs[..],
            config.network,
            config.gap_limit,
            config.initial_gap_limit,
        )?;
        let watcher = HDWatcher::new(wallets);

        let rpc = Arc::new(RpcClient::new(
            config.bitcoind_url(),
            config.bitcoind_auth()?,
        )?);
        let indexer = Arc::new(RwLock::new(Indexer::new(rpc.clone(), watcher)));
        let query = Arc::new(Query::new(rpc.clone(), indexer.clone()));

        wait_ibd(&rpc)?;

        // do an initial sync without keeping track of updates
        indexer.write().unwrap().initial_sync()?;

        let (sync_tx, sync_rx) = mpsc::channel();
        // debounce sync message rate to avoid excessive indexing when bitcoind catches up
        let sync_tx = debounce_sender(sync_tx, DEBOUNCE_SEC);

        #[cfg(feature = "electrum")]
        let electrum = ElectrumServer::start(config.electrum_rpc_addr(), query.clone());

        #[cfg(feature = "http")]
        let http = HttpServer::start(
            config.http_server_addr,
            config.cors.clone(),
            query.clone(),
            sync_tx.clone(),
        );

        #[cfg(unix)]
        {
            if let Some(listener_path) = &config.unix_listener_path {
                listener::start(listener_path.clone(), sync_tx.clone());
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
            sync_chan: (sync_tx, sync_rx),
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
            match self.indexer.write().unwrap().sync() {
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
            self.sync_chan
                .1
                .recv_timeout(self.config.poll_interval)
                .ok();
        }
    }
}

fn wait_ibd(rpc: &RpcClient) -> Result<()> {
    let netinfo = rpc.get_network_info()?;
    let mut bcinfo = rpc.get_blockchain_info()?;
    info!(
        "connected to {} on {}, protocolversion={}, pruned={}, bestblock={}",
        netinfo.subversion,
        bcinfo.chain,
        netinfo.protocol_version,
        bcinfo.pruned,
        bcinfo.best_block_hash
    );

    trace!("{:?}", netinfo);
    trace!("{:?}", bcinfo);

    let dur = time::Duration::from_secs(15);
    while bcinfo.initial_block_download {
        /* || bcinfo.blocks < bcinfo.headers */
        info!(
            "waiting for bitcoind to sync [{}/{} blocks, ibd={}]",
            bcinfo.blocks, bcinfo.headers, bcinfo.initial_block_download
        );
        thread::sleep(dur);
        bcinfo = rpc.get_blockchain_info()?;
    }

    Ok(())
}
