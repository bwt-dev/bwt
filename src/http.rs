use std::sync::{mpsc, Arc, Mutex};
use std::{convert, net, thread};

use serde::{Deserialize, Deserializer};
use tokio::sync::{mpsc as tmpsc, oneshot};
use tokio_stream::{wrappers::UnboundedReceiverStream, Stream, StreamExt};
use warp::http::{header, StatusCode};
use warp::{self, reply, sse::Event, Filter, Reply};

use bitcoin::{Address, BlockHash, OutPoint, Txid};
use bitcoin_hashes::hex::{FromHex, ToHex};

use crate::error::{fmt_error_chain, BwtError, Error, OptionExt};
use crate::types::{BlockId, ScriptHash};
use crate::util::auth::http_basic_auth;
use crate::util::{banner, block_on_future, descriptor::Checksum, whitepaper};
use crate::{store, IndexChange, Query};

type SyncChanSender = Arc<Mutex<mpsc::Sender<()>>>;

fn setup(
    access_token: Option<String>,
    cors: Option<String>,
    query: Arc<Query>,
    sync_tx: SyncChanSender,
    listeners: Listeners,
) -> warp::Server<impl warp::Filter<Extract = impl warp::Reply> + Clone> {
    let query = warp::any().map(move || Arc::clone(&query));
    let sync_tx = warp::any().map(move || Arc::clone(&sync_tx));
    let listeners = warp::any().map(move || Arc::clone(&listeners));

    let mut headers = header::HeaderMap::new();
    if let Some(cors) = cors {
        // allow using "any" as an alias for "*", avoiding expansion when passing "*" can be tricky
        let cors = if cors == "any" { "*".into() } else { cors };
        headers.insert("Access-Control-Allow-Origin", cors.parse().unwrap());
    }

    // GET /wallets
    let wallets_handler = warp::get()
        .and(warp::path!("wallets"))
        .and(query.clone())
        .map(|query: Arc<Query>| {
            let wallets = query.get_wallets();
            reply::json(&wallets)
        });

    // GET /wallet/:checksum
    let wallet_handler = warp::get()
        .and(warp::path!("wallet" / Checksum))
        .and(query.clone())
        .map(|checksum: Checksum, query: Arc<Query>| {
            let wallet = query.get_wallet(&checksum).or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&wallet))
        })
        .map(handle_error);

    // GET /wallet/:checksum/:index
    let wallet_key_handler = warp::get()
        .and(warp::path!("wallet" / Checksum / u32))
        .and(query.clone())
        .map(|checksum: Checksum, index: u32, query: Arc<Query>| {
            let script_info = query
                .get_wallet_script_info(&checksum, index)
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&script_info))
        })
        .map(handle_error);

    // GET /wallet/:checksum/gap
    let wallet_gap_handler = warp::get()
        .and(warp::path!("wallet" / Checksum / "gap"))
        .and(query.clone())
        .map(|checksum: Checksum, query: Arc<Query>| {
            let gap = query
                .find_wallet_gap(&checksum)
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&gap))
        })
        .map(handle_error);

    // GET /wallet/:checksum/next
    let wallet_next_handler = warp::get()
        .and(warp::path!("wallet" / Checksum / "next"))
        .and(query.clone())
        .map(|checksum: Checksum, query: Arc<Query>| {
            let wallet = query.get_wallet(&checksum).or_err(StatusCode::NOT_FOUND)?;
            let next_index = wallet.get_next_index();
            let uri = format!("/wallet/{}/{}", checksum, next_index);
            // issue a 307 redirect to the wallet key resource uri, and also include the derivation
            // index in the response
            Ok(reply::with_header(
                reply::with_status(next_index.to_string(), StatusCode::TEMPORARY_REDIRECT),
                header::LOCATION,
                uri,
            ))
        })
        .map(handle_error);

    // GET /scripthash/:scripthash/*
    let scripthash_route = warp::path!("scripthash" / ScriptHash / ..);

    // GET /address/:address/*
    let address_route = warp::path!("address" / Address / ..).map(ScriptHash::from);
    // TODO check address version bytes matches the configured network

    // GET /wallet/:checksum/:index/*
    let wallet_key_route = warp::path!("wallet" / Checksum / u32 / ..)
        .and(query.clone())
        .map(|checksum: Checksum, index: u32, query: Arc<Query>| {
            let script_info = query
                .get_wallet_script_info(&checksum, index)
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(script_info.scripthash)
        })
        .and_then(reject_error);

    let spk_route = address_route
        .or(scripthash_route)
        .unify()
        .or(wallet_key_route)
        .unify();

    // GET /wallet/:checksum/:index
    // GET /address/:address
    // GET /scripthash/:scripthash
    let spk_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path::end())
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let script_info = query
                .get_script_info(&scripthash)
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&script_info))
        })
        .map(handle_error);

    // GET /wallet/:checksum/:index/stats
    // GET /address/:address/stats
    // GET /scripthash/:scripthash/stats
    let spk_stats_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path!("stats"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let script_stats = query
                .get_script_stats(&scripthash)?
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&script_stats))
        })
        .map(handle_error);

    // GET /wallet/:checksum/:index/utxos
    // GET /address/:address/utxos
    // GET /scripthash/:scripthash/utxos
    let spk_utxo_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path!("utxos"))
        .and(warp::query::<UtxoOptions>())
        .and(query.clone())
        .map(|scripthash, options: UtxoOptions, query: Arc<Query>| {
            let utxos =
                query.list_unspent(Some(&scripthash), options.min_conf, options.include_unsafe)?;
            Ok(reply::json(&utxos))
        })
        .map(handle_error);

    // GET /wallet/:checksum/:index/txs
    // GET /address/:address/txs
    // GET /scripthash/:scripthash/txs
    let spk_txs_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path!("txs"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let txs = query.map_history(&scripthash, |txhist| {
                query.get_tx_detail(&txhist.txid).unwrap()
            });
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // GET /wallet/:checksum/:index/txs/compact
    // GET /address/:address/txs/compact
    // GET /scripthash/:scripthash/txs/compact
    let spk_txs_compact_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path!("txs" / "compact"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let txs = query.map_history(&scripthash, compact_history);
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // GET /tx/:txid/*
    let tx_route = warp::path!("tx" / Txid / ..);

    // GET /tx/:txid
    let tx_handler = warp::get()
        .and(tx_route)
        .and(warp::path::end())
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let tx_info = query.get_tx_detail(&txid).or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&tx_info))
        })
        .map(handle_error);

    // GET /tx/:txid/verbose
    let tx_verbose_handler = warp::get()
        .and(tx_route)
        .and(warp::path!("verbose"))
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let tx_json = query.get_tx_json(&txid)?;
            Ok(reply::json(&tx_json))
        })
        .map(handle_error);

    // GET /tx/:txid/hex
    let tx_hex_handler = warp::get()
        .and(tx_route)
        .and(warp::path!("hex"))
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let tx_raw = query.get_tx_raw(&txid)?;
            Ok(tx_raw.to_hex())
        })
        .map(handle_error);

    // GET /tx/:txid/proof
    let tx_proof_handler = warp::get()
        .and(tx_route)
        .and(warp::path!("proof"))
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let proof = query.get_tx_proof(&txid)?;
            Ok(proof.to_hex())
        })
        .map(handle_error);

    // GET /txs
    // GET /txs/since/:block_height
    let txs_since_handler = warp::get()
        .and(warp::path("txs"))
        .and(
            warp::path!("since" / u32)
                .or(warp::path::end().map(|| 0))
                .unify(),
        )
        .and(query.clone())
        .map(|min_block_height: u32, query: Arc<Query>| {
            let txs = query.map_history_since(min_block_height, |txhist| {
                query.get_tx_detail(&txhist.txid).unwrap()
            });
            reply::json(&txs)
        });

    // GET /txs/since/:block_height/compact
    let txs_since_compact_handler = warp::get()
        .and(warp::path!("txs" / "since" / u32 / "compact"))
        .and(query.clone())
        .map(|min_block_height: u32, query: Arc<Query>| {
            let txs = query.map_history_since(min_block_height, compact_history);
            reply::json(&txs)
        });

    // POST /tx
    let tx_broadcast_handler = warp::post()
        .and(warp::body::json())
        .and(query.clone())
        .map(|body: BroadcastBody, query: Arc<Query>| {
            let txid = query.broadcast(&body.tx_hex)?;
            Ok(txid.to_string())
        })
        .map(handle_error);

    // GET /txo/:txid/:vout
    let txo_handler = warp::get()
        .and(warp::path!("txo" / Txid / u32))
        .and(query.clone())
        .map(|txid: Txid, vout: u32, query: Arc<Query>| {
            let txo = query
                .lookup_txo(&OutPoint::new(txid, vout))
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&txo))
        })
        .map(handle_error);

    // GET /utxos
    let utxos_handler = warp::get()
        .and(warp::path!("utxos"))
        .and(warp::query::<UtxoOptions>())
        .and(query.clone())
        .map(|options: UtxoOptions, query: Arc<Query>| {
            let utxos = query.list_unspent(None, options.min_conf, options.include_unsafe)?;
            Ok(reply::json(&utxos))
        })
        .map(handle_error);

    // GET /stream
    let sse_handler = warp::get()
        .and(warp::path!("stream"))
        .and(ChangelogFilter::param())
        .and(listeners.clone())
        .and(query.clone())
        .map(
            |filter: ChangelogFilter, listeners: Listeners, query: Arc<Query>| {
                let stream = make_sse_stream(filter, listeners, &query)?;
                Ok(warp::sse::reply(warp::sse::keep_alive().stream(stream)))
            },
        )
        .map(handle_error);

    // GET /wallet/:checksum/:index/stream
    // GET /scripthash/:scripthash/stream
    // GET /address/:address/stream
    let spk_sse_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("stream"))
        .and(ChangelogFilter::param())
        .and(listeners)
        .and(query.clone())
        .map(
            |scripthash: ScriptHash,
             mut filter: ChangelogFilter,
             listeners: Listeners,
             query: Arc<Query>| {
                filter.scripthash = Some(scripthash);
                let stream = make_sse_stream(filter, listeners, &query)?;
                Ok(warp::sse::reply(warp::sse::keep_alive().stream(stream)))
            },
        )
        .map(handle_error);

    // GET /block/tip
    let block_tip_handler = warp::get()
        .and(warp::path!("block" / "tip"))
        .and(query.clone())
        .map(|query: Arc<Query>| {
            // XXX currently returns the tip reported by bitcoind, return the indexer tip as well?
            let BlockId(height, blockhash) = query.get_tip()?;
            Ok(reply::json(&json!({ "hash": blockhash, "height": height })))
        })
        .map(handle_error);

    // GET /block/:hash
    let block_header_handler = warp::get()
        .and(warp::path!("block" / BlockHash))
        .and(query.clone())
        .map(|blockhash: BlockHash, query: Arc<Query>| {
            let header_info = query.get_header_info(&blockhash)?;
            Ok(reply::json(&header_info))
        })
        .map(handle_error);

    // GET /block/:hash/hex
    let block_hex_handler = warp::get()
        .and(warp::path!("block" / BlockHash / "hex"))
        .and(query.clone())
        .map(|blockhash: BlockHash, query: Arc<Query>| {
            let header_hex = query.get_header_hex(&blockhash)?;
            Ok(header_hex)
        })
        .map(handle_error);

    // GET /block/:block_height
    let block_height_handler = warp::get()
        .and(warp::path!("block" / u32))
        .and(query.clone())
        .map(|height: u32, query: Arc<Query>| {
            let blockhash = query.get_block_hash(height)?;
            let uri = format!("/block/{}", blockhash);
            // issue a 307 redirect to the block hash uri, and also include the hash in the body
            Ok(reply::with_header(
                reply::with_status(blockhash.to_string(), StatusCode::TEMPORARY_REDIRECT),
                header::LOCATION,
                uri,
            ))
        })
        .map(handle_error);

    // GET /mempool/histogram
    let mempool_histogram_handler = warp::get()
        .and(warp::path!("mempool" / "histogram"))
        .and(query.clone())
        .map(|query: Arc<Query>| {
            let histogram = query.fee_histogram()?;
            Ok(reply::json(&histogram))
        })
        .map(handle_error);

    // GET /fee-estimate/:confirmation-target
    let fee_estimate_handler = warp::get()
        .and(warp::path!("fee-estimate" / u16))
        .and(query.clone())
        .map(|confirmation_target: u16, query: Arc<Query>| {
            let feerate = query.estimate_fee(confirmation_target)?;
            Ok(reply::json(&feerate))
        })
        .map(handle_error);

    // GET /dump
    let dump_handler = warp::get()
        .and(warp::path!("dump"))
        .and(query.clone())
        .map(|query: Arc<Query>| reply::json(&query.dump_index()));

    // GET /debug
    let debug_handler = warp::get()
        .and(warp::path!("debug"))
        .and(query.clone())
        .map(|query: Arc<Query>| query.debug_index());

    // GET /banner.txt
    let banner_handler = warp::get()
        .and(warp::path!("banner.txt"))
        .and(query.clone())
        .map(|query: Arc<Query>| banner::get_welcome_banner(&query, true))
        .map(handle_error);

    // POST /sync
    let sync_handler = warp::post()
        .and(warp::path!("sync"))
        .and(sync_tx)
        .map(|sync_tx: SyncChanSender| {
            sync_tx.lock().unwrap().send(())?;
            Ok(reply::with_status("syncing queued", StatusCode::ACCEPTED))
        })
        .map(handle_error);

    // GET /bitcoin.pdf
    let whitepaper_handler = warp::get()
        .and(warp::path!("bitcoin.pdf"))
        .and(query)
        .map(|query: Arc<Query>| {
            let pdf_blob = whitepaper::get_whitepaper_pdf(query.rpc())?;
            Ok(reply::with_header(
                pdf_blob,
                "Content-Type",
                "application/pdf",
            ))
        })
        .map(handle_error);

    let handler = balanced_or_tree!(
        wallets_handler,
        wallet_handler,
        wallet_key_handler, // needs to be before spk_handler to work with keys that don't have any indexed history
        wallet_gap_handler,
        wallet_next_handler,
        spk_handler,
        spk_utxo_handler,
        spk_stats_handler,
        spk_txs_handler,
        spk_txs_compact_handler,
        tx_handler,
        tx_verbose_handler,
        tx_hex_handler,
        tx_proof_handler,
        txs_since_handler,
        txs_since_compact_handler,
        tx_broadcast_handler,
        txo_handler,
        utxos_handler,
        sse_handler,
        spk_sse_handler,
        block_tip_handler,
        block_header_handler,
        block_hex_handler,
        block_height_handler,
        mempool_histogram_handler,
        fee_estimate_handler,
        dump_handler,
        debug_handler,
        banner_handler,
        sync_handler,
        whitepaper_handler,
        warp::any().map(|| StatusCode::NOT_FOUND)
    )
    .with(warp::reply::with::headers(headers));

    // Wrap handler with (optional) authentication, logging and rejection handling
    let handler = http_basic_auth(access_token)
        .and_then(reject_error)
        .untuple_one()
        .and(handler)
        .with(warp::log("bwt::http"))
        .recover(handle_rejection);

    warp::serve(handler)
}

#[tokio::main]
async fn spawn<S>(
    warp_server: warp::Server<S>,
    addr: net::SocketAddr,
    addr_tx: oneshot::Sender<net::SocketAddr>,
    shutdown_rx: oneshot::Receiver<()>,
) where
    S: warp::Filter + Clone + Send + Sync + 'static,
    S::Extract: warp::Reply,
{
    let (bound_addr, server_ft) = warp_server.bind_with_graceful_shutdown(addr, async {
        shutdown_rx.await.ok();
    });

    // Send back the bound address, useful for binding on port 0
    addr_tx.send(bound_addr).unwrap();

    server_ft.await
}

pub struct HttpServer {
    addr: net::SocketAddr,
    listeners: Listeners,
    shutdown_tx: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HttpServer {
    pub fn start(
        addr: net::SocketAddr,
        access_token: Option<String>,
        cors: Option<String>,
        query: Arc<Query>,
        sync_tx: mpsc::Sender<()>,
    ) -> Self {
        let listeners = Arc::new(Mutex::new(Vec::new()));
        let sync_tx = Arc::new(Mutex::new(sync_tx));
        let warp_server = setup(access_token, cors, query, sync_tx, listeners.clone());

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (addr_tx, addr_rx) = oneshot::channel();

        let thread = thread::spawn(move || {
            spawn(warp_server, addr, addr_tx, shutdown_rx);
        });

        let bound_addr = block_on_future(addr_rx).expect("failed starting http server");
        info!("HTTP REST API server running on http://{}/", bound_addr);

        HttpServer {
            listeners,
            addr: bound_addr,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
        }
    }

    pub fn send_updates(&self, changelog: &[IndexChange]) {
        let mut listeners = self.listeners.lock().unwrap();
        if listeners.is_empty() {
            return;
        }
        info!(
            "sending {} update(s) to {} sse client(s)",
            changelog.len(),
            listeners.len()
        );
        // send updates while dropping unresponsive listeners
        listeners.retain(|listener| {
            changelog
                .iter()
                .filter(|change| listener.filter.matches(change))
                .all(|change| listener.tx.send(change.clone()).is_ok())
        })
    }

    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        trace!("HTTP server shutting down");
        self.shutdown_tx.take().unwrap().send(()).unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}

type Listeners = Arc<Mutex<Vec<Listener>>>;

struct Listener {
    tx: tmpsc::UnboundedSender<IndexChange>,
    filter: ChangelogFilter,
}

// Create a stream of real-time changelog events matching `filter`, optionally also including
// historical events occuring after `synced-tip`
fn make_sse_stream(
    filter: ChangelogFilter,
    listeners: Listeners,
    query: &Query,
) -> Result<impl Stream<Item = Result<Event, warp::Error>>, Error> {
    debug!("subscribing sse client with {:?}", filter);

    let (tx, rx) = tmpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);
    listeners.lock().unwrap().push(Listener {
        tx,
        filter: filter.clone(),
    });

    // fetch historical changelog since the requested start point (if requesed)
    let changelog = match &filter.synced_tip {
        Some(synced_tip) => query.get_changelog_after(synced_tip)?,
        None => vec![],
    }
    .into_iter()
    .filter(move |change| filter.matches(change));
    // TODO don't produce unwanted events to begin with instead of filtering them

    Ok(tokio_stream::iter(changelog)
        .chain(rx)
        .map(make_sse_msg)
        .map(Ok))
}

fn make_sse_msg(change: IndexChange) -> Event {
    match &change {
        IndexChange::ChainTip(blockid) => {
            // set the synced tip as the sse identifier field, so the client will send it back to
            // us on reconnection via the Last-Event-Id header.
            Event::default()
                .id(blockid.to_string())
                .json_data(change)
                .unwrap()
        }
        _ => Event::default().json_data(change).unwrap(),
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "kebab-case")]
struct ChangelogFilter {
    #[serde(default, deserialize_with = "deser_synced_tip")]
    synced_tip: Option<BlockId>,

    scripthash: Option<ScriptHash>,
    outpoint: Option<OutPoint>,
    category: Option<String>,
    // warp::query() does not support nested arrays
    //pub scripthash: Option<Vec<ScriptHash>>,
    //pub category: Option<Vec<String>>,
}

impl ChangelogFilter {
    fn matches(&self, change: &IndexChange) -> bool {
        self.scripthash_matches(change)
            && self.category_matches(change)
            && self.outpoint_matches(change)
    }
    fn scripthash_matches(&self, change: &IndexChange) -> bool {
        self.scripthash.as_ref().map_or(true, |filter_sh| {
            change
                .scripthash()
                .map_or(false, |change_sh| filter_sh == change_sh)
            //.map_or(false, |change_sh| filter_sh.contains(change_sh))
        })
    }
    fn category_matches(&self, change: &IndexChange) -> bool {
        self.category.as_ref().map_or(true, |filter_cat| {
            change.category_str() == filter_cat
            //let change_cat = change.category_str();
            //filter_cat.iter().any(|filter_cat| filter_cat == change_cat)
        })
    }
    fn outpoint_matches(&self, change: &IndexChange) -> bool {
        self.outpoint.as_ref().map_or(true, |filter_outpoint| {
            change
                .outpoint()
                .map_or(false, |change_outpoint| filter_outpoint == change_outpoint)
        })
    }

    fn param() -> impl Filter<Extract = (ChangelogFilter,), Error = warp::Rejection> + Clone {
        warp::query::<ChangelogFilter>()
            .and(warp::sse::last_event_id::<String>())
            .map(
                |mut filter: ChangelogFilter, last_event_id: Option<String>| {
                    // When available, use the Server-Sent-Events Last-Event-Id header as the synced tip
                    if let Some(last_event_id) = last_event_id {
                        if let Ok(synced_tip) = parse_synced_tip(&last_event_id) {
                            filter.synced_tip = Some(synced_tip);
                        }
                    }
                    filter
                },
            )
    }
}

fn parse_synced_tip(s: &str) -> Result<BlockId, Error> {
    let mut parts = s.splitn(2, ':');
    let height: u32 = parts.next().required()?.parse()?;
    Ok(match parts.next() {
        Some(block_hash) => BlockId(height, BlockHash::from_hex(block_hash)?),
        None => BlockId(height, BlockHash::default()),
    })
}

fn deser_synced_tip<'de, D>(deserializer: D) -> std::result::Result<Option<BlockId>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let blockid = parse_synced_tip(&s).map_err(|err| serde::de::Error::custom(err.to_string()))?;
    Ok(Some(blockid))
}

#[derive(Deserialize, Debug)]
struct UtxoOptions {
    #[serde(default)]
    min_conf: usize,
    include_unsafe: Option<bool>,
}

#[derive(Deserialize, Debug)]
struct BroadcastBody {
    tx_hex: String,
}

fn compact_history(tx_hist: &store::HistoryEntry) -> serde_json::Value {
    json!([tx_hist.txid, tx_hist.status])
}

// Handle errors produced by route handlers
fn handle_error<T>(result: Result<T, Error>) -> impl Reply
where
    T: Reply + Send,
{
    match result {
        Ok(x) => x.into_response(),
        Err(e) => {
            warn!("handler failed: {:#?}", e);
            let status = get_error_status(&e);
            let body = fmt_error_chain(&e);
            reply::with_status(body, status).into_response()
        }
    }
}

// Transform an anyhow::Error into a Rejection
async fn reject_error<T>(result: Result<T, Error>) -> Result<T, warp::Rejection> {
    result.map_err(|err| warp::reject::custom(WarpError(err)))
}

// Format rejections into HTTP responses
async fn handle_rejection(err: warp::Rejection) -> Result<impl Reply, convert::Infallible> {
    warn!("request rejected: {:?}", err);
    let (status, body) = if let Some(WarpError(err)) = err.find() {
        (get_error_status(&err), fmt_error_chain(&err))
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", err))
    };
    let reply = reply::with_status(body, status);
    Ok(iif!(
        status == StatusCode::UNAUTHORIZED,
        reply::with_header(reply, "WWW-Authenticate", "Basic realm=\"\"").into_response(),
        reply.into_response()
    ))
}

fn get_error_status(e: &Error) -> StatusCode {
    if let Some(status_code) = e.downcast_ref::<StatusCode>() {
        *status_code
    } else if let Some(bwt_err) = e.downcast_ref::<BwtError>() {
        bwt_err.status_code()
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(Debug)]
struct WarpError(Error);
impl warp::reject::Reject for WarpError {}
