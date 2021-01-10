use std::sync::{mpsc, Arc, RwLock};
use std::{net, thread, time};

use bitcoincore_rpc::{self as rpc, Client as RpcClient, RpcApi};

use crate::error::{BwtError, Result};
use crate::util::bitcoincore_ext::{Progress, RpcApiExt};
use crate::util::{banner, debounce_sender, on_oneshot_done};
use crate::{Config, Indexer, Query, WalletWatcher};

#[cfg(feature = "electrum")]
use crate::electrum::ElectrumServer;
#[cfg(feature = "http")]
use crate::http::HttpServer;
#[cfg(unix)]
use crate::listener;
#[cfg(feature = "webhooks")]
use crate::webhooks::WebHookNotifier;

const DEBOUNCE_SEC: u64 = 2;

pub struct App {
    config: Config,
    indexer: Arc<RwLock<Indexer>>,
    query: Arc<Query>,
    sync_chan: (mpsc::Sender<()>, mpsc::Receiver<()>),

    #[cfg(feature = "electrum")]
    electrum: Option<ElectrumServer>,
    #[cfg(feature = "http")]
    http: Option<HttpServer>,
    #[cfg(feature = "webhooks")]
    webhook: Option<WebHookNotifier>,
}

impl App {
    /// Start up bwt, run the initial sync and start the configured service(s)
    ///
    /// To abort during initialization, disconnect the progress_tx channel.
    /// To shutdown after initialization was completed, drop the App.
    pub fn boot(config: Config, progress_tx: Option<mpsc::Sender<Progress>>) -> Result<Self> {
        debug!("{}", scrub_config(&config));

        let watcher = WalletWatcher::from_config(&config)?;

        let rpc = Arc::new(RpcClient::new(
            config.bitcoind_url(),
            config.bitcoind_auth()?,
        )?);
        let indexer = Arc::new(RwLock::new(Indexer::new(rpc.clone(), watcher)));
        let query = Arc::new(Query::new((&config).into(), rpc.clone(), indexer.clone()));

        init_bitcoind(&rpc, &config, progress_tx.clone())?;

        if config.startup_banner {
            println!("{}", banner::get_welcome_banner(&query, false)?);
        }

        // do an initial sync without keeping track of updates
        indexer.write().unwrap().initial_sync(progress_tx.clone())?;

        if let Some(progress_tx) = progress_tx {
            if progress_tx.send(Progress::Done).is_err() {
                bail!(BwtError::Canceled);
            }
        }

        let (sync_tx, sync_rx) = mpsc::channel();
        // debounce sync message rate to avoid excessive indexing when bitcoind catches up
        let debounced_sync_tx = debounce_sender(sync_tx.clone(), DEBOUNCE_SEC);

        #[cfg(feature = "electrum")]
        let electrum = config
            .electrum_addr()
            .map(|addr| ElectrumServer::start(addr, config.electrum_skip_merkle, query.clone()));

        #[cfg(feature = "http")]
        let http = config.http_addr().map(|addr| {
            HttpServer::start(
                addr,
                config.http_cors.clone(),
                query.clone(),
                debounced_sync_tx.clone(),
            )
        });

        #[cfg(unix)]
        {
            if let Some(listener_path) = &config.unix_listener_path {
                listener::start(listener_path.clone(), debounced_sync_tx);
            }
        }

        #[cfg(feature = "webhooks")]
        let webhook = config.webhook_urls.clone().map(WebHookNotifier::start);

        Ok(App {
            config,
            indexer,
            query,
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
    pub fn sync(&self, shutdown_rx: Option<mpsc::Receiver<()>>) {
        debug!("starting sync loop");
        let shutdown_rx = shutdown_rx
            .map(|rx| self.bind_shutdown(rx))
            .or_else(|| self.default_shutdown_signal());

        loop {
            if let Some(shutdown_rx) = &shutdown_rx {
                if shutdown_rx.try_recv() != Err(mpsc::TryRecvError::Empty) {
                    break;
                }
            }

            #[allow(clippy::option_map_unit_fn)]
            match self.indexer.write().unwrap().sync() {
                Ok(updates) if !updates.is_empty() => {
                    #[cfg(feature = "electrum")]
                    self.electrum
                        .as_ref()
                        .map(|electrum| electrum.send_updates(&updates));

                    #[cfg(feature = "http")]
                    self.http.as_ref().map(|http| http.send_updates(&updates));

                    #[cfg(feature = "webhooks")]
                    self.webhook
                        .as_ref()
                        .map(|webhook| webhook.send_updates(&updates));
                }
                Ok(_) => (), // no updates
                Err(e) => warn!("error while updating index: {:?}", e),
            }

            // wait for poll_interval seconds or until we receive a sync notification message
            // (which can also get triggered through the shutdown signal)
            self.sync_chan
                .1
                .recv_timeout(self.config.poll_interval)
                .ok();
        }
    }

    /// Start a sync loop in a new background thread.
    pub fn sync_background(self) -> mpsc::SyncSender<()> {
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
        thread::spawn(move || self.sync(Some(shutdown_rx)));
        shutdown_tx
    }

    /// Get the `Query` instance
    pub fn query(&self) -> Arc<Query> {
        self.query.clone()
    }

    #[cfg(feature = "electrum")]
    pub fn electrum_addr(&self) -> Option<net::SocketAddr> {
        Some(self.electrum.as_ref()?.addr())
    }

    #[cfg(feature = "http")]
    pub fn http_addr(&self) -> Option<net::SocketAddr> {
        Some(self.http.as_ref()?.addr())
    }

    // Bind the shutdown receiver to also trigger `sync_tx`. This is needed to start the next
    // sync loop run immediately, which will then process the shutdown signal itself. Without
    // this, the shutdown signal will only be noticed after a delay.
    fn bind_shutdown(&self, shutdown_rx: mpsc::Receiver<()>) -> mpsc::Receiver<()> {
        let sync_tx = self.sync_chan.0.clone();
        on_oneshot_done(shutdown_rx, move || {
            sync_tx.send(()).unwrap();
        })
    }

    #[cfg(all(unix, feature = "signal-hook"))]
    fn default_shutdown_signal(&self) -> Option<mpsc::Receiver<()>> {
        use signal_hook::iterator::Signals;

        let signals = Signals::new(&[signal_hook::SIGINT, signal_hook::SIGTERM]).unwrap();
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
        let sync_tx = self.sync_chan.0.clone();

        thread::spawn(move || {
            let signal = signals.into_iter().next().unwrap();
            trace!("received shutdown signal {}", signal);
            shutdown_tx.send(()).unwrap();
            // Need to also trigger `sync_tx`, see rational above
            sync_tx.send(()).unwrap();
        });

        Some(shutdown_rx)
    }

    #[cfg(not(all(unix, feature = "signal-hook")))]
    fn default_shutdown_signal(&self) -> Option<mpsc::Receiver<()>> {
        None
    }

    pub fn test_rpc(config: &Config) -> Result<()> {
        let rpc = RpcClient::new(config.bitcoind_url(), config.bitcoind_auth()?)?;
        rpc.get_wallet_info()?;
        Ok(())
    }
}

// Load the specified wallet, ignore "wallet is already loaded" errors
fn load_wallet(rpc: &RpcClient, name: &str) -> Result<()> {
    use crate::util::bitcoincore_ext::RPC_WALLET_ERROR;
    const MSG_ALREADY_LOADED_SUFF: &str = "Duplicate -wallet filename specified.";
    match rpc.load_wallet(name) {
        Ok(_) => Ok(()),
        Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)))
            if e.code == RPC_WALLET_ERROR && e.message.ends_with(MSG_ALREADY_LOADED_SUFF) =>
        {
            Ok(())
        }
        Err(e) => bail!(e),
    }
}

// Initialize bitcoind and wait for it to finish syncing and rescanning
// Aborted if the progress channel gets disconnected.
fn init_bitcoind(
    rpc: &RpcClient,
    config: &Config,
    progress_tx: Option<mpsc::Sender<Progress>>,
) -> Result<()> {
    const INTERVAL: time::Duration = time::Duration::from_secs(7);

    let bcinfo = rpc.wait_blockchain_sync(progress_tx.clone(), INTERVAL)?;

    if let Some(bitcoind_wallet) = &config.bitcoind_wallet {
        load_wallet(&rpc, bitcoind_wallet)?;
    }
    let walletinfo = rpc.wait_wallet_scan(progress_tx, None, INTERVAL)?;

    let netinfo = rpc.get_network_info()?;
    info!(
        "bwt v{} connected to {} on {} at height {}",
        crate::BWT_VERSION,
        netinfo.subversion,
        bcinfo.chain,
        bcinfo.headers
    );

    trace!("{:?}", netinfo);
    trace!("{:?}", bcinfo);
    trace!("{:?}", walletinfo);

    Ok(())
}

fn scrub_config(config: &Config) -> String {
    let mut s = format!("{:?}", config);
    if let Some(auth) = config.bitcoind_auth.as_deref() {
        s = s.replace(auth, "**SCRUBBED**")
    }
    s
}
