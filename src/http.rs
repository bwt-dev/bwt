use std::net;
use std::sync::{mpsc, Arc, Mutex};

use async_std::task;
use serde::{Deserialize, Deserializer};
use tokio::stream::{self, Stream, StreamExt};
use tokio::sync::mpsc as tmpsc;
use warp::http::{header, StatusCode};
use warp::sse::ServerSentEvent;
use warp::{reply, Filter, Reply};

use bitcoin::{Address, BlockHash, OutPoint, Txid};
use bitcoin_hashes::hex::FromHex;

use crate::error::{fmt_error_chain, BwtError, Error, OptionExt};
use crate::types::{BlockId, DescrChecksum, ScriptHash};
use crate::{store, IndexChange, Query};

type SyncChanSender = Arc<Mutex<mpsc::Sender<()>>>;

#[tokio::main]
async fn run(
    addr: net::SocketAddr,
    cors: Option<String>,
    query: Arc<Query>,
    sync_tx: SyncChanSender,
    listeners: Listeners,
) {
    let query = warp::any().map(move || Arc::clone(&query));
    let sync_tx = warp::any().map(move || Arc::clone(&sync_tx));
    let listeners = warp::any().map(move || Arc::clone(&listeners));

    let mut headers = header::HeaderMap::new();
    if let Some(cors) = cors {
        // allow using "any" as an alias for "*", avoiding expansion when passing "*" can be tricky
        let cors = if cors == "any" { "*".into() } else { cors };
        headers.insert(
            "Access-Control-Allow-Origin",
            header::HeaderValue::from_str(&cors).unwrap(),
        );
    }

    // GET /wallet
    let hd_wallets_handler = warp::get()
        .and(warp::path!("wallet"))
        .and(query.clone())
        .map(|query: Arc<Query>| {
            let wallets = query.get_hd_wallets();
            reply::json(&wallets)
        });

    // GET /wallet/:fingerprint
    let hd_wallet_handler = warp::get()
        .and(warp::path!("wallet" / DescrChecksum))
        .and(query.clone())
        .map(|descr_cs: DescrChecksum, query: Arc<Query>| {
            let wallet = query
                .get_hd_wallet(descr_cs)
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&wallet))
        })
        .map(handle_error);

    // GET /wallet/:fingerprint/:index
    let hd_key_handler = warp::get()
        .and(warp::path!("wallet" / DescrChecksum / u32))
        .and(query.clone())
        .map(|descr_cs: DescrChecksum, index: u32, query: Arc<Query>| {
            let script_info = query
                .get_hd_script_info(descr_cs, index)?
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&script_info))
        })
        .map(handle_error);

    // GET /wallet/:fingerprint/gap
    let hd_gap_handler = warp::get()
        .and(warp::path!("wallet" / DescrChecksum / "gap"))
        .and(query.clone())
        .map(|descr_cs: DescrChecksum, query: Arc<Query>| {
            let gap = query.find_hd_gap(descr_cs)?.or_err(StatusCode::NOT_FOUND)?;
            Ok(reply::json(&gap))
        })
        .map(handle_error);

    // GET /wallet/:fingerprint/next
    let hd_next_handler = warp::get()
        .and(warp::path!("wallet" / DescrChecksum / "next"))
        .and(query.clone())
        .map(|descr_cs: DescrChecksum, query: Arc<Query>| {
            let wallet = query
                .get_hd_wallet(descr_cs.clone())
                .or_err(StatusCode::NOT_FOUND)?;
            let next_index = wallet.get_next_index();
            let uri = format!("/wallet/{}/{}", descr_cs, next_index);
            // issue a 307 redirect to the hdkey resource uri, and also include the derivation
            // index in the response
            Ok(reply::with_header(
                reply::with_status(next_index.to_string(), StatusCode::TEMPORARY_REDIRECT),
                header::LOCATION,
                header::HeaderValue::from_str(&uri)?,
            ))
        })
        .map(handle_error);

    // GET /scripthash/:scripthash/*
    let scripthash_route = warp::path!("scripthash" / ScriptHash / ..);

    // GET /address/:address/*
    let address_route = warp::path!("address" / Address / ..).map(ScriptHash::from);
    // TODO check address version bytes matches the configured network

    // GET /wallet/:fingerprint/:index/*
    let hd_key_route = warp::path!("wallet" / DescrChecksum / u32 / ..)
        .and(query.clone())
        .map(|descr_cs: DescrChecksum, index: u32, query: Arc<Query>| {
            let script_info = query
                .get_hd_script_info(descr_cs, index)?
                .or_err(StatusCode::NOT_FOUND)?;
            Ok(script_info.scripthash)
        })
        .and_then(reject_error);

    let spk_route = address_route
        .or(scripthash_route)
        .unify()
        .or(hd_key_route)
        .unify();

    // GET /wallet/:fingerprint/:index
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

    // GET /wallet/:fingerprint/:index/stats
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

    // GET /wallet/:fingerprint/:index/utxos
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

    // GET /wallet/:fingerprint/:index/txs
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

    // GET /wallet/:fingerprint/:index/txs/compact
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
            Ok(hex::encode(tx_raw))
        })
        .map(handle_error);

    // GET /tx/:txid/proof
    let tx_proof_handler = warp::get()
        .and(tx_route)
        .and(warp::path!("proof"))
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let proof = query.get_tx_proof(&txid)?;
            Ok(hex::encode(proof))
        })
        .map(handle_error);

    // GET /txs/since/:block_height
    let txs_since_handler = warp::get()
        .and(warp::path!("txs" / "since" / u32))
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

    // GET /wallet/:fingerprint/:index/stream
    // GET /scripthash/:scripthash/stream
    // GET /address/:address/stream
    let spk_sse_handler = warp::get()
        .and(spk_route.clone())
        .and(warp::path!("stream"))
        .and(ChangelogFilter::param())
        .and(listeners.clone())
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
            let BlockId(blockhash, height) = query.get_tip()?;
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
            let url = format!("/block/{}", blockhash);
            // issue a 307 redirect to the block hash uri, and also include the hash in the body
            Ok(reply::with_header(
                reply::with_status(blockhash.to_string(), StatusCode::TEMPORARY_REDIRECT),
                header::LOCATION,
                header::HeaderValue::from_str(&url)?,
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

    // POST /sync
    let sync_handler = warp::post()
        .and(warp::path!("sync"))
        .and(sync_tx.clone())
        .map(|sync_tx: SyncChanSender| {
            info!("received sync notification");
            sync_tx.lock().unwrap().send(())?;
            Ok(reply::with_status("syncing queued", StatusCode::ACCEPTED))
        })
        .map(handle_error);

    // hd_key_handler needs to be oredered before spk_handler, so it'll work with keys that don't have any indexed history

    let hd_handlers = hd_wallets_handler
        .or(hd_wallet_handler.or(hd_key_handler))
        .or(hd_gap_handler.or(hd_next_handler));
    let spk_handlers = spk_handler
        .or(spk_utxo_handler.or(spk_stats_handler))
        .or(spk_txs_handler.or(spk_txs_compact_handler));
    let tx_handlers = tx_handler
        .or(tx_verbose_handler.or(tx_hex_handler))
        .or(tx_proof_handler.or(txs_since_handler))
        .or(txs_since_compact_handler.or(tx_broadcast_handler));
    let txo_handlers = txo_handler.or(utxos_handler);
    let sse_handlers = sse_handler.or(spk_sse_handler);
    let block_handlers = block_tip_handler
        .or(block_header_handler.or(block_hex_handler))
        .or(block_height_handler);
    let mempool_handlers = mempool_histogram_handler.or(fee_estimate_handler);
    let other_handlers = dump_handler.or(debug_handler.or(sync_handler));

    let handlers = hd_handlers
        .or(spk_handlers.or(tx_handlers))
        .or(txo_handlers.or(sse_handlers))
        .or(block_handlers.or(mempool_handlers))
        .or(other_handlers)
        .or(warp::any().map(|| StatusCode::NOT_FOUND))
        .with(warp::log("bwt::http"))
        .with(warp::reply::with::headers(headers));

    info!("HTTP REST API server starting on http://{}/", addr);

    warp::serve(handlers).run(addr).await
}

pub struct HttpServer {
    _thread: task::JoinHandle<()>,
    listeners: Listeners,
}

impl HttpServer {
    pub fn start(
        addr: net::SocketAddr,
        cors: Option<String>,
        query: Arc<Query>,
        sync_tx: mpsc::Sender<()>,
    ) -> Self {
        let sync_tx = Arc::new(Mutex::new(sync_tx));

        let listeners: Listeners = Arc::new(Mutex::new(Vec::new()));
        let thr_listeners = Arc::clone(&listeners);

        HttpServer {
            _thread: task::spawn(async move {
                run(addr, cors, query, sync_tx, thr_listeners);
            }),
            listeners,
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
) -> Result<impl Stream<Item = Result<impl ServerSentEvent, warp::Error>>, Error> {
    debug!("subscribing sse client with {:?}", filter);

    let (tx, rx) = tmpsc::unbounded_channel();
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

    Ok(stream::iter(changelog).chain(rx).map(make_sse_msg).map(Ok))
}

fn make_sse_msg(change: IndexChange) -> impl ServerSentEvent {
    match &change {
        IndexChange::ChainTip(blockid) => {
            // set the synced tip as the sse identifier field, so the client will send it back to
            // us on reconnection via the Last-Event-Id header.
            (warp::sse::id(blockid.to_string()), warp::sse::json(change)).into_a()
        }
        _ => warp::sse::json(change).into_b(),
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
    let height: u32 = parts.next().req()?.parse()?;
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

fn handle_error<T>(result: Result<T, Error>) -> impl Reply
where
    T: Reply + Send,
{
    match result {
        Ok(x) => x.into_response(),
        Err(e) => {
            warn!("processing failed: {:#?}", e);
            let status = get_error_status(&e);
            let body = fmt_error_chain(&e);
            reply::with_status(body, status).into_response()
        }
    }
}

async fn reject_error<T>(result: Result<T, Error>) -> Result<T, warp::Rejection> {
    result.map_err(|err| {
        warn!("pre-processing failed: {:?}", err);
        warp::reject::custom(WarpError(err))
    })
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
