use std::collections::HashMap;
use std::net;
use std::sync::{mpsc, Arc, Mutex};

use async_std::task;
use serde_derive::Deserialize;
use tokio::stream::{Stream, StreamExt};
use tokio::sync::mpsc as tmpsc;
use warp::http::StatusCode;
use warp::sse::ServerSentEvent;
use warp::{http::header, reply, Filter, Reply};

use bitcoin::util::bip32::Fingerprint;
use bitcoin::{Address, OutPoint, Txid};

use crate::error::{fmt_error_chain, Error, OptionExt};
use crate::indexer::IndexChange;
use crate::types::ScriptHash;
use crate::Query;

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
        headers.insert(
            "Access-Control-Allow-Origin",
            header::HeaderValue::from_str(&cors).unwrap(),
        );
    }

    // GET /hd
    let hd_wallets_handler =
        warp::get()
            .and(warp::path!("hd"))
            .and(query.clone())
            .map(|query: Arc<Query>| {
                let wallets = query.get_hd_wallets();
                reply::json(
                    &wallets
                        .iter()
                        .map(|(fp, wallet)| (fp, wallet.with_origin()))
                        .collect::<HashMap<_, _>>(),
                )
            });

    // GET /hd/:fingerprint
    let hd_wallet_handler = warp::get()
        .and(warp::path!("hd" / Fingerprint))
        .and(query.clone())
        .map(|fingerprint: Fingerprint, query: Arc<Query>| {
            let wallet = query.get_hd_wallet(&fingerprint).or_err("not found")?;
            Ok(reply::json(&wallet.with_origin()))
        })
        .map(handle_error);

    // GET /hd/:fingerprint/:index
    let hd_key_handler = warp::get()
        .and(warp::path!("hd" / Fingerprint / u32))
        .and(query.clone())
        .map(
            |fingerprint: Fingerprint, derivation_index: u32, query: Arc<Query>| {
                let script_info = query.get_hd_script_info(&fingerprint, derivation_index);
                reply::json(&script_info)
            },
        );

    // GET /hd/:fingerprint/gap
    let hd_gap_handler = warp::get()
        .and(warp::path!("hd" / Fingerprint / "gap"))
        .and(query.clone())
        .map(|fingerprint: Fingerprint, query: Arc<Query>| {
            let gap = query.find_hd_gap(&fingerprint);
            reply::json(&gap)
        });

    // Pre-processing
    // GET /address/:address/*
    // GET /scripthash/:scripthash/*
    let address_route = warp::path!("address" / Address / ..)
        // TODO ensure!(address.network == config.network);
        .map(|address: Address| ScriptHash::from(&address));
    let scripthash_route = warp::path!("scripthash" / ScriptHash / ..);
    let spk_route = address_route.or(scripthash_route).unify();

    // GET /address/:address
    // GET /scripthash/:scripthash
    let spk_handler = warp::get()
        .and(spk_route)
        .and(warp::path::end())
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let script_stats = query.get_script_stats(&scripthash)?;
            Ok(reply::json(&script_stats))
        })
        .map(handle_error);

    // GET /address/:address/info
    // GET /scripthash/:scripthash/info
    let spk_info_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("info"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let script_info = query.get_script_info(&scripthash).or_err("not found")?;
            Ok(reply::json(&script_info))
        })
        .map(handle_error);

    // GET /address/:address/utxos
    // GET /scripthash/:scripthash/utxos
    let spk_utxo_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("utxos"))
        .and(warp::query::<UtxoOptions>())
        .and(query.clone())
        .map(|scripthash, options: UtxoOptions, query: Arc<Query>| {
            let utxos =
                query.list_unspent(Some(&scripthash), options.min_conf, options.include_unsafe)?;
            Ok(reply::json(&utxos))
        })
        .map(handle_error);

    // GET /address/:address/history
    // GET /scripthash/:scripthash/history
    let spk_history_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("history"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let txs = query.map_history(&scripthash, |txhist| {
                query.get_tx_detail(&txhist.txid).unwrap()
            });
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // GET /address/:address/history/compact
    // GET /scripthash/:scripthash/history/compact
    let spk_history_compact_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("history" / "compact"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let txs = query.get_history(&scripthash);
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // Pre-processing
    // GET /tx/:txid/*
    let tx_route = warp::path!("tx" / Txid / ..);

    // GET /tx/:txid
    let tx_handler = warp::get()
        .and(tx_route)
        .and(warp::path::end())
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let tx_info = query.get_tx_detail(&txid).or_err("tx not found")?;
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
            let txs = query.get_history_since(min_block_height);
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
                .or_err("not found")?;
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
        .and(warp::query::<ChangelogFilter>())
        .and(listeners.clone())
        .map(|filter: ChangelogFilter, listeners: Listeners| {
            let stream = make_connection_sse_stream(listeners, filter);
            warp::sse::reply(warp::sse::keep_alive().stream(stream))
        });

    // GET /scripthash/:scripthash/stream
    // GET /address/:address/stream
    let spk_sse_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("stream"))
        .and(warp::query::<ChangelogFilter>())
        .and(listeners.clone())
        .map(
            |scripthash: ScriptHash, mut filter: ChangelogFilter, listeners: Listeners| {
                filter.scripthash = Some(scripthash);
                let stream = make_connection_sse_stream(listeners, filter);
                warp::sse::reply(warp::sse::keep_alive().stream(stream))
            },
        );

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

    let handlers = hd_wallets_handler
        .or(hd_wallet_handler)
        .or(hd_key_handler)
        .or(hd_gap_handler)
        .or(spk_handler)
        .or(spk_utxo_handler)
        .or(spk_info_handler)
        .or(spk_history_handler)
        .or(spk_history_compact_handler)
        .or(tx_handler)
        .or(tx_verbose_handler)
        .or(tx_hex_handler)
        .or(txs_since_handler)
        .or(txs_since_compact_handler)
        .or(tx_broadcast_handler)
        .or(txo_handler)
        .or(utxos_handler)
        .or(sse_handler)
        .or(spk_sse_handler)
        .or(mempool_histogram_handler)
        .or(fee_estimate_handler)
        .or(dump_handler)
        .or(debug_handler)
        .or(sync_handler)
        .with(warp::log("pxt::http"))
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

    pub fn send_updates(&self, changelog: &Vec<IndexChange>) {
        let mut listeners = self.listeners.lock().unwrap();
        if listeners.is_empty() {
            return;
        }
        info!(
            "sending {} updates to {} sse clients",
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
    filter: ChangelogFilter, // None means subscribing to everything
}

fn make_connection_sse_stream(
    listeners: Listeners,
    filter: ChangelogFilter,
) -> impl Stream<Item = Result<impl ServerSentEvent, warp::Error>> {
    debug!("subscribing sse client with {:?}", filter);
    let (tx, rx) = tmpsc::unbounded_channel();
    listeners.lock().unwrap().push(Listener { tx, filter });
    rx.map(|change: IndexChange| Ok(warp::sse::json(change)))
}

#[derive(Debug, Deserialize)]
struct ChangelogFilter {
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

fn handle_error<T>(result: Result<T, Error>) -> impl Reply
where
    T: Reply + Send,
{
    match result {
        Ok(x) => x.into_response(),
        Err(e) => {
            warn!("processing failed: {:#?}", e);
            let status = StatusCode::INTERNAL_SERVER_ERROR;
            let body = fmt_error_chain(&e);
            reply::with_status(body, status).into_response()
        }
    }
}
