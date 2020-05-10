use std::net;
use std::str::FromStr;
use std::sync::{mpsc, Arc, Mutex};

use async_std::task;
use serde_derive::Deserialize;
use warp::http::StatusCode;
use warp::Filter;
use warp::{reply, Reply};

use bitcoin::{Address, Txid};

use crate::error::{Error, OptionExt};
use crate::Query;
use crate::types::ScriptHash;

type SyncChanSender = Arc<Mutex<mpsc::Sender<()>>>;

#[tokio::main]
async fn run(addr: net::SocketAddr, query: Arc<Query>, sync_tx: SyncChanSender) {
    let query = warp::any().map(move || Arc::clone(&query));
    let sync_tx = warp::any().map(move || Arc::clone(&sync_tx));

    // Pre-processing
    // GET /address/:address/*
    // GET /scripthash/:scripthash/*
    let address_route = warp::path!("address" / String / ..)
        .map(|address: String| {
            let address = Address::from_str(&address)?;
            // TODO ensure!(address.network == config.network);
            let scripthash = ScriptHash::from(&address);
            Ok(scripthash)
        })
        .and_then(reject_error);
    let scripthash_route = warp::path!("scripthash" / String / ..)
        .map(|scripthash: String| Ok(ScriptHash::from_str(&scripthash)?))
        .and_then(reject_error);
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

    // GET /address/:address/utxo
    // GET /scripthash/:scripthash/utxo
    let spk_utxo_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("utxo"))
        .and(warp::query::<UtxoOptions>())
        .and(query.clone())
        .map(|scripthash, options: UtxoOptions, query: Arc<Query>| {
            let utxos = query.list_unspent(&scripthash, options.min_conf)?;
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
                query.get_tx_info(&txhist.txid).unwrap()
            });
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // GET /address/:address/history/minimal
    // GET /scripthash/:scripthash/history/minimal
    let spk_minimal_history_handler = warp::get()
        .and(spk_route)
        .and(warp::path!("history" / "minimal"))
        .and(query.clone())
        .map(|scripthash, query: Arc<Query>| {
            let txs = query.get_history(&scripthash);
            Ok(reply::json(&txs))
        })
        .map(handle_error);

    // Pre-processing
    // GET /tx/:txid/*
    let tx_route = warp::path("tx").and(
        warp::path::param()
            .map(|txid: String| Ok(Txid::from_str(&txid)?))
            .and_then(reject_error),
    );

    // GET /tx/:txid
    let tx_handler = warp::get()
        .and(tx_route)
        .and(warp::path::end())
        .and(query.clone())
        .map(|txid: Txid, query: Arc<Query>| {
            let tx_info = query.get_tx_info(&txid).or_err("tx not found")?;
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
                query.get_tx_info(&txhist.txid).unwrap()
            });
            reply::json(&txs)
        });

    // GET /mempool/histogram
    let mempool_histogram_handler = warp::get()
        .and(warp::path!("mempool" / "histogram"))
        .and(query.clone())
        .map(|query: Arc<Query>| {
            let histogram = query.fee_histogram()?;
            Ok(reply::json(&histogram))
        })
        .map(handle_error);

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
            info!("received sync notification via http server");
            sync_tx.lock().unwrap().send(())?;
            Ok("syncing in progress")
        })
        .map(handle_error);

    let handlers = spk_handler
        .or(spk_utxo_handler)
        .or(spk_history_handler)
        .or(spk_minimal_history_handler)
        .or(tx_handler)
        .or(tx_verbose_handler)
        .or(tx_hex_handler)
        .or(txs_since_handler)
        .or(mempool_histogram_handler)
        .or(debug_handler)
        .or(sync_handler)
        .with(warp::log("pxt"));

    info!("starting http server on {}", addr);

    warp::serve(handlers).run(addr).await
}

pub struct HttpServer(task::JoinHandle<()>);

impl HttpServer {
    pub fn start(addr: net::SocketAddr, query: Arc<Query>, sync_tx: mpsc::Sender<()>) -> Self {
        HttpServer(task::spawn(async move {
            let sync_tx = Arc::new(Mutex::new(sync_tx));
            run(addr, query, sync_tx);
        }))
    }
}

#[derive(Deserialize, Debug)]
struct UtxoOptions {
    #[serde(default)]
    min_conf: usize,
}

async fn reject_error<T>(result: Result<T, Error>) -> Result<T, warp::Rejection> {
    result.map_err(|err| {
        warn!("filter rejected: {:?}", err);
        warp::reject::custom(WarpError::Error(err))
    })
}

fn handle_error<T>(result: Result<T, Error>) -> impl Reply
where
    T: Reply + Send,
{
    match result {
        Ok(x) => x.into_response(),
        Err(e) => {
            warn!("request failed with: {:#?}", e);
            let status = StatusCode::INTERNAL_SERVER_ERROR;
            reply::with_status(e.to_string(), status).into_response()
        }
    }
}

#[derive(Debug)]
enum WarpError {
    Error(Error),
}

impl warp::reject::Reject for WarpError {}
