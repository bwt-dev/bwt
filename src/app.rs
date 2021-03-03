use std::sync::{mpsc, Arc, RwLock};
use std::{net, thread, time};

use bitcoincore_rpc::{self as rpc, Client as RpcClient, RpcApi};

use crate::error::{BwtError, Result};
use crate::util::progress::Progress;
use crate::util::{banner, debounce_sender, fd_readiness_notification, on_oneshot_done};
use crate::{Config, IndexChange, Indexer, Query, WalletWatcher};

#[cfg(feature = "electrum")]
use crate::electrum::ElectrumServer;
#[cfg(feature = "http")]
use crate::http::HttpServer;
#[cfg(unix)]
use crate::listener;
#[cfg(feature = "webhooks")]
use crate::webhooks::WebHookNotifier;

const DEBOUNCE_SEC: u64 = 2;
const LT: &str = "bwt";

pub struct App {
    config: Config,
    indexer: Arc<RwLock<Indexer>>,
    query: Arc<Query>,
    access_token: Option<String>,
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
        debug!(target: LT, "{}", scrub_config(&config));

        let watcher = WalletWatcher::from_config(&config)?;
        let rpc = Arc::new(RpcClient::new(
            config.bitcoind_url(),
            config.bitcoind_auth()?,
        )?);
        let indexer = Arc::new(RwLock::new(Indexer::new(rpc.clone(), watcher)?));
        let query = Arc::new(Query::new((&config).into(), rpc.clone(), indexer.clone()));

        // wait for bitcoind to load up and initialize the wallet
        init_bitcoind(&rpc, &config, progress_tx.clone())?;

        if config.startup_banner {
            println!("{}", banner::get_welcome_banner(&query, false)?);
        }

        // do an initial sync without keeping track of updates
        indexer.write().unwrap().initial_sync(progress_tx.clone())?;

        // abort if the progress channel was shutdown
        if let Some(progress_tx) = progress_tx {
            if progress_tx.send(Progress::Done).is_err() {
                bail!(BwtError::Canceled);
            }
        }

        // prepare access token (user-provided, cookie file or ephemeral)
        let access_token = config.auth_method()?.get_token()?;
        if config.print_token {
            if let Some(token) = &access_token {
                info!(target: "bwt::auth", "Your SECRET access token is: {}", token);
            }
        }

        // channel for triggering real-time index sync
        // debounced to avoid excessive indexing when bitcoind catches up
        let (sync_tx, sync_rx) = mpsc::channel();
        #[cfg(any(feature = "http", unix))]
        let debounced_sync_tx = debounce_sender(sync_tx.clone(), DEBOUNCE_SEC);

        #[cfg(feature = "electrum")]
        let electrum = config.electrum_addr().map(|addr| {
            ElectrumServer::start(
                addr,
                iif!(config.electrum_socks_auth, access_token.clone(), None),
                config.electrum_skip_merkle,
                query.clone(),
            )
        });

        #[cfg(feature = "http")]
        let http = config.http_addr().map(|addr| {
            HttpServer::start(
                addr,
                access_token.clone(),
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

        fd_readiness_notification();

        Ok(App {
            config,
            indexer,
            query,
            access_token,
            sync_chan: (sync_tx, sync_rx),
            #[cfg(feature = "electrum")]
            electrum,
            #[cfg(feature = "http")]
            http,
            #[cfg(feature = "webhooks")]
            webhook,
        })
    }

    /// Synchronize new blocks/transactions and emit updates to the running services
    #[allow(clippy::option_map_unit_fn)]
    pub fn sync(&self) -> Result<Vec<IndexChange>> {
        let updates = self.indexer.write().unwrap().sync()?;

        if !updates.is_empty() {
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
        Ok(updates)
    }

    /// Start a sync loop blocking the current thread
    pub fn sync_loop(&self, shutdown_rx: Option<mpsc::Receiver<()>>) {
        let shutdown_rx = shutdown_rx
            .map(|rx| self.bind_shutdown(rx))
            .or_else(|| self.default_shutdown_signal());

        debug!(target: LT, "starting sync loop");
        loop {
            if let Some(shutdown_rx) = &shutdown_rx {
                if shutdown_rx.try_recv() != Err(mpsc::TryRecvError::Empty) {
                    break;
                }
            }

            if let Err(e) = self.sync() {
                warn!(target: LT, "failed syncing with bitcoind: {:?}", e);
                // Report the error and try again on the next run, this might be
                // a temporary connectivity issue.
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
    /// Takes ownership over the app. You can retain a Query instance before calling this.
    pub fn sync_background(self) -> mpsc::SyncSender<()> {
        let (shutdown_tx, shutdown_rx) = mpsc::sync_channel(1);
        thread::spawn(move || self.sync_loop(Some(shutdown_rx)));
        shutdown_tx
    }

    /// Get the sender for triggering a real-time index sync
    pub fn sync_sender(&self) -> mpsc::Sender<()> {
        self.sync_chan.0.clone()
    }

    /// Get the `Query` instance
    pub fn query(&self) -> Arc<Query> {
        self.query.clone()
    }

    /// Get the access token
    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
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
            trace!(target: LT, "received shutdown signal {}", signal);
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

// Load the specified wallet, ignore "wallet is already loaded" errors,
// and create the wallet if the `create_wallet_if_missing` option was set
fn load_wallet(rpc: &RpcClient, name: &str, create_if_missing: bool) -> Result<()> {
    use crate::util::bitcoincore_ext::{RPC_WALLET_ERROR, RPC_WALLET_NOT_FOUND};
    // Needs to be identified by the error message string because it uses a generic wallet error code.
    // The error message changed in Bitcoin Core v0.21.
    const MSG_ALREADY_LOADED_OLD: &str = "Duplicate -wallet filename specified.";
    const MSG_ALREADY_LOADED_NEW: &str = "is already loaded.";

    match rpc.load_wallet(name) {
        Ok(_) => Ok(()),
        // Wallet already loaded
        Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)))
            if e.code == RPC_WALLET_ERROR
                && (e.message.ends_with(MSG_ALREADY_LOADED_OLD)
                    || e.message.ends_with(MSG_ALREADY_LOADED_NEW)) =>
        {
            Ok(())
        }
        // Wallet missing, create it if the `create_wallet_if_missing` option was set
        Err(rpc::Error::JsonRpc(rpc::jsonrpc::Error::Rpc(ref e)))
            if create_if_missing && e.code == RPC_WALLET_NOT_FOUND =>
        {
            rpc.create_wallet(name, Some(true), Some(true), None, None)?;
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
    use crate::util::progress::{wait_blockchain_sync, wait_wallet_scan};

    const INTERVAL_SLOW: time::Duration = time::Duration::from_secs(6);
    const INTERVAL_FAST: time::Duration = time::Duration::from_millis(1500);
    // use the fast interval if we're reporting progress to a channel, or the slow one if its only for CLI
    let interval = iif!(progress_tx.is_some(), INTERVAL_FAST, INTERVAL_SLOW);

    let bcinfo = wait_blockchain_sync(rpc, progress_tx.clone(), interval)?;

    if let Some(bitcoind_wallet) = &config.bitcoind_wallet {
        load_wallet(&rpc, bitcoind_wallet, config.create_wallet_if_missing)?;
    }
    let walletinfo = wait_wallet_scan(rpc, progress_tx, None, interval)?;

    let netinfo = rpc.get_network_info()?;
    info!(
        target: LT,
        "bwt v{} connected to {} on {} at height {}",
        crate::BWT_VERSION,
        netinfo.subversion,
        bcinfo.chain,
        bcinfo.blocks
    );

    trace!(target: LT, "{:?}", netinfo);
    trace!(target: LT, "{:?}", bcinfo);
    trace!(target: LT, "{:?}", walletinfo);

    Ok(())
}

fn scrub_config(config: &Config) -> String {
    let mut s = format!("{:?}", config);
    if let Some(auth) = config.bitcoind_auth.as_deref() {
        s = s.replace(auth, "**SCRUBBED**")
    }
    if let Some(token) = config.auth_token.as_deref() {
        s = s.replace(token, "**SCRUBBED**")
    }
    s
}
