use std::cmp;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use bitcoin::Txid;
use bitcoin_hashes::{hex::ToHex, Hash};
use serde_json::{from_str, from_value, Value};

use crate::error::{fmt_error_chain, Result, ResultExt};
use crate::indexer::IndexChange;
use crate::merkle::{get_header_merkle_proof, get_id_from_pos, get_merkle_proof};
use crate::query::Query;
use crate::types::{BlockId, ScriptHash, StatusHash, TxStatus};

// Heavily based on the RPC server implementation written by Roman Zeyde for electrs,
// released under the MIT license. https://github.com/romanz/electrs

const PXT_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "1.4";
const MAX_HEADERS: u32 = 2016;

struct Connection {
    query: Arc<Query>,
    stream: TcpStream,
    addr: SocketAddr,
    chan: SyncChannel<Message>,
    subman: Arc<Mutex<SubscriptionManager>>,
    subscriber_id: usize,
}

impl Connection {
    pub fn new(
        query: Arc<Query>,
        stream: TcpStream,
        addr: SocketAddr,
        subman: Arc<Mutex<SubscriptionManager>>,
    ) -> Connection {
        let chan = SyncChannel::new(10);
        let subscriber_id = subman.lock().unwrap().register(chan.sender());
        Connection {
            query,
            subman,
            subscriber_id,
            stream,
            addr,
            chan,
        }
    }

    fn blockchain_headers_subscribe(&mut self) -> Result<Value> {
        self.subman
            .lock()
            .unwrap()
            .subscribe_blocks(self.subscriber_id);

        let BlockId(tip_height, tip_hash) = self.query.get_tip()?;
        let tip_hex = self.query.get_header_hex(&tip_hash)?;
        Ok(json!({ "height": tip_height, "hex": tip_hex }))
    }

    fn server_version(&self) -> Result<Value> {
        Ok(json!([format!("bwt {}", PXT_VERSION), PROTOCOL_VERSION]))
    }

    fn server_banner(&self) -> Result<Value> {
        Ok(json!("Welcome to bwt 🚀🌑"))
    }

    fn server_donation_address(&self) -> Result<Value> {
        Ok(Value::Null)
    }

    fn server_peers_subscribe(&self) -> Result<Value> {
        Ok(json!([]))
    }

    fn mempool_get_fee_histogram(&self) -> Result<Value> {
        let histogram = &self.query.fee_histogram()?;
        Ok(json!(histogram))
    }

    fn blockchain_block_header(&self, params: Value) -> Result<Value> {
        let (height, cp_height): (u32, Option<u32>) = from_value(pad_params(params, 2))?;

        let blockhash = self.query.get_block_hash(height)?;
        let header_hex = self.query.get_header_hex(&blockhash)?;

        Ok(match cp_height {
            Some(cp_height) => {
                let (branch, root) = get_header_merkle_proof(&self.query, height, cp_height)?;

                json!({
                    "header": header_hex,
                    "root": root.to_hex(),
                    "branch": map_str(branch),
                })
            }
            None => json!(header_hex),
        })
    }

    fn blockchain_block_headers(&self, params: Value) -> Result<Value> {
        let (start_height, count, cp_height): (u32, u32, Option<u32>) =
            from_value(pad_params(params, 3))?;

        let count = cmp::min(count, MAX_HEADERS);

        // drop unknown heights (from the specs: "If the chain has not extended sufficiently far,
        // only the available headers will be returned. If more headers than max were requested at
        // most max will be returned.")
        let max_height = cmp::min(start_height + count, self.query.get_tip_height()?);

        // TODO use batch rpc when available in rust-bitcoincore-rpc
        let headers: Vec<String> = (start_height..max_height)
            .map(|height| {
                let blockhash = self.query.get_block_hash(height)?;
                self.query.get_header_hex(&blockhash)
            })
            .collect::<Result<Vec<_>>>()?;

        let mut result = json!({
            "count": headers.len(),
            "hex": headers.join(""),
            "max": MAX_HEADERS,
        });

        if count > 0 {
            if let Some(cp_height) = cp_height {
                let (branch, root) =
                    get_header_merkle_proof(&self.query, start_height + (count - 1), cp_height)?;

                result["root"] = json!(root.to_hex());
                result["branch"] = json!(map_str(branch));
            }
        }

        Ok(result)
    }

    fn blockchain_estimatefee(&self, params: Value) -> Result<Value> {
        let (target,): (u16,) = from_value(params)?;
        let fee_rate = self.query.estimate_fee(target)?;

        // format for electrum: from sat/b to BTC/kB, -1 to indicate no estimate is available
        Ok(json!(fee_rate.map_or(-1.0, |rate| rate / 100_000f64)))
    }

    fn blockchain_relayfee(&self) -> Result<Value> {
        let fee_rate = self.query.relay_fee()?;
        // sat/b to BTC/kB
        Ok(json!(fee_rate / 100_000f64))
    }

    fn blockchain_scripthash_subscribe(&mut self, params: Value) -> Result<Value> {
        let (script_hash,): (ScriptHash,) = from_value(params)?;

        self.subman
            .lock()
            .unwrap()
            .subscribe_scripthash(self.subscriber_id, script_hash);

        let status_hash = get_status_hash(&self.query, &script_hash);
        Ok(json!(status_hash))
    }

    fn blockchain_scripthash_get_balance(&self, params: Value) -> Result<Value> {
        let (script_hash,): (ScriptHash,) = from_value(params)?;

        let (confirmed_balance, mempool_balance) = self.query.get_script_balance(&script_hash)?;

        Ok(json!({
            "confirmed": confirmed_balance,
            "unconfirmed": mempool_balance,
        }))
    }

    fn blockchain_scripthash_get_history(&self, params: Value) -> Result<Value> {
        let (script_hash,): (ScriptHash,) = from_value(params)?;

        let txs: Vec<Value> = self.query.map_history(&script_hash, |txhist| {
            let fee = self.query.with_tx_entry(&txhist.txid, |e| e.fee);
            json!({
                "height": electrum_height(&txhist.status),
                "tx_hash": txhist.txid,
                "fee": fee,
            })
        });
        Ok(json!(txs))
    }

    fn blockchain_scripthash_listunspent(&self, params: Value) -> Result<Value> {
        let (script_hash,): (ScriptHash,) = from_value(params)?;

        let utxos: Vec<Value> = self
            .query
            .list_unspent(Some(&script_hash), 0, None)?
            .into_iter()
            .map(|utxo| {
                json!({
                    "height": electrum_height(&utxo.status),
                    "tx_hash": utxo.txid,
                    "tx_pos": utxo.vout,
                    "value": utxo.amount,
                })
            })
            .collect();
        Ok(json!(utxos))
    }

    fn blockchain_transaction_broadcast(&self, params: Value) -> Result<Value> {
        let (tx_hex,): (String,) = from_value(params)?;

        let txid = self.query.broadcast(&tx_hex)?;
        Ok(json!(txid.to_hex()))
    }

    fn blockchain_transaction_get(&self, params: Value) -> Result<Value> {
        let (txid, verbose): (Txid, Option<bool>) = from_value(pad_params(params, 2))?;
        let verbose = verbose.unwrap_or(false);

        Ok(if verbose {
            json!(self.query.get_tx_json(&txid)?)
        } else {
            let raw = self.query.get_tx_raw(&txid)?;
            json!(hex::encode(&raw))
        })
    }

    fn blockchain_transaction_get_merkle(&self, params: Value) -> Result<Value> {
        let (txid, height): (Txid, u32) = from_value(params)?;

        let (merkle, pos) = get_merkle_proof(&self.query, &txid, height)?;

        Ok(json!({
            "block_height": height,
            "merkle": map_str(merkle),
            "pos": pos,
        }))
    }

    fn blockchain_transaction_id_from_pos(&self, params: Value) -> Result<Value> {
        let (height, tx_pos, want_merkle): (u32, usize, Option<bool>) =
            from_value(pad_params(params, 3))?;
        let want_merkle = want_merkle.unwrap_or(false);

        let (txid, merkle) = get_id_from_pos(&self.query, height, tx_pos, want_merkle)?;

        Ok(if !want_merkle {
            json!(txid.to_hex())
        } else {
            json!({
                "tx_hash": txid,
                "merkle": map_str(merkle),
            })
        })
    }

    fn handle_command(&mut self, method: &str, params: Value, id: Value) -> Result<Value> {
        match method {
            "server.ping"
            | "blockchain.scripthash.subscribe"
            | "blockchain.estimatefee"
            | "mempool.get_fee_histogram" => {
                trace!("rpc #{} <--- {} {}", id, method, params);
            }
            _ => {
                debug!("rpc #{} <--- {} {}", id, method, params);
            }
        }

        let result = match method {
            "blockchain.block.header" => self.blockchain_block_header(params),
            "blockchain.block.headers" => self.blockchain_block_headers(params),
            "blockchain.estimatefee" => self.blockchain_estimatefee(params),
            "blockchain.headers.subscribe" => self.blockchain_headers_subscribe(),
            "blockchain.relayfee" => self.blockchain_relayfee(),
            "blockchain.scripthash.get_balance" => self.blockchain_scripthash_get_balance(params),
            "blockchain.scripthash.get_history" => self.blockchain_scripthash_get_history(params),
            "blockchain.scripthash.listunspent" => self.blockchain_scripthash_listunspent(params),
            "blockchain.scripthash.subscribe" => self.blockchain_scripthash_subscribe(params),
            "blockchain.transaction.broadcast" => self.blockchain_transaction_broadcast(params),
            "blockchain.transaction.get" => self.blockchain_transaction_get(params),
            "blockchain.transaction.get_merkle" => self.blockchain_transaction_get_merkle(params),
            "blockchain.transaction.id_from_pos" => self.blockchain_transaction_id_from_pos(params),
            "mempool.get_fee_histogram" => self.mempool_get_fee_histogram(),
            "server.banner" => self.server_banner(),
            "server.donation_address" => self.server_donation_address(),
            "server.peers.subscribe" => self.server_peers_subscribe(),
            "server.ping" => Ok(Value::Null),
            "server.version" => self.server_version(),
            &_ => bail!("unknown method {} {:?}", method, params),
        };

        Ok(match result {
            Ok(result) => {
                trace!("rpc #{} ---> {} {}", id, method, result);
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            }
            Err(e) => {
                warn!("rpc #{} {} failed: {:?}", id, method, e,);
                json!({"jsonrpc": "2.0", "id": id, "error": fmt_error_chain(&e)})
            }
        })
    }

    fn jsonrpc_notification(&mut self, change: IndexChange) -> Result<Value> {
        Ok(match change {
            IndexChange::ChainTip(BlockId(tip_height, tip_hash)) => {
                let hex_header = self.query.get_header_hex(&tip_hash)?;
                let header = json!({"hex": hex_header, "height": tip_height });
                json!({
                    "jsonrpc": "2.0",
                    "method": "blockchain.headers.subscribe",
                    "params": [header]})
            }
            IndexChange::History(scripthash, ..) => {
                let new_status_hash = get_status_hash(&self.query, &scripthash);
                json!({
                    "jsonrpc": "2.0",
                    "method": "blockchain.scripthash.subscribe",
                    "params": [scripthash, new_status_hash]
                })
            }
            _ => unreachable!(), // we're not supposed to receive anything else
        })
    }

    fn send_values(&mut self, values: &[Value]) -> Result<()> {
        for value in values {
            let line = value.to_string() + "\n";
            self.stream
                .write_all(line.as_bytes())
                .context(format!("failed to send {}", value))?;
        }
        Ok(())
    }

    fn handle_replies(&mut self) -> Result<()> {
        loop {
            let msg = self.chan.receiver().recv().context("channel closed")?;
            match msg {
                Message::Request(line) => {
                    let mut cmd: Value = from_str(&line).context("invalid JSON format")?;
                    let reply = match (cmd["method"].take(), cmd["params"].take(), cmd["id"].take())
                    {
                        (Value::String(method), params, id) => {
                            self.handle_command(&method, params, id)?
                        }
                        _ => bail!("invalid command: {}", line),
                    };
                    self.send_values(&[reply])?
                }
                Message::IndexChange(change) => {
                    let value = self.jsonrpc_notification(change)?;
                    debug!(
                        "sending notification {} {}",
                        value["method"], value["params"]
                    );
                    self.send_values(&[value])?;
                }
                Message::Done => return Ok(()),
            }
        }
    }

    fn handle_requests(mut reader: BufReader<TcpStream>, tx: SyncSender<Message>) -> Result<()> {
        loop {
            let mut line = Vec::<u8>::new();
            reader
                .read_until(b'\n', &mut line)
                .context("failed to read a request")?;
            if line.is_empty() {
                tx.send(Message::Done).context("channel closed")?;
                return Ok(());
            } else {
                if line.starts_with(&[22, 3, 1]) {
                    // (very) naive SSL handshake detection
                    let _ = tx.send(Message::Done);
                    bail!("invalid request - maybe SSL-encrypted data?: {:?}", line)
                }
                match String::from_utf8(line) {
                    Ok(req) => tx.send(Message::Request(req)).context("channel closed")?,
                    Err(err) => {
                        let _ = tx.send(Message::Done);
                        bail!("invalid UTF8: {}", err)
                    }
                }
            }
        }
    }

    pub fn run(mut self) {
        let reader = BufReader::new(self.stream.try_clone().expect("failed to clone TcpStream"));
        let tx = self.chan.sender();
        let child = spawn_thread("reader", || Connection::handle_requests(reader, tx));
        if let Err(e) = self.handle_replies() {
            error!("[{}] connection handling failed: {:#?}", self.addr, e,)
        }
        trace!("[{}] shutting down connection", self.addr);
        self.subman.lock().unwrap().remove(self.subscriber_id);
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Err(err) = child.join().expect("receiver panicked") {
            error!("[{}] receiver failed: {:?}", self.addr, err);
        }
    }
}

fn pad_params(mut params: Value, n: usize) -> Value {
    if let Value::Array(ref mut values) = params {
        while values.len() < n {
            values.push(Value::Null);
        }
    } // passing a non-array is a noop
    params
}

fn map_str<T>(items: Vec<T>) -> Vec<String>
where
    T: ToString,
{
    items.into_iter().map(|item| item.to_string()).collect()
}

fn get_status_hash(query: &Query, scripthash: &ScriptHash) -> Option<StatusHash> {
    let p = query.map_history(scripthash, |hist| {
        format!("{}:{}:", hist.txid, electrum_height(&hist.status))
    });

    if !p.is_empty() {
        Some(StatusHash::hash(&p.join("").into_bytes()))
    } else {
        None
    }
}

// TODO -1 to indicate unconfirmed tx with unconfirmed parents
fn electrum_height(status: &TxStatus) -> u32 {
    match status {
        TxStatus::Confirmed(height) => *height,
        TxStatus::Unconfirmed => 0,
        TxStatus::Conflicted => {
            unreachable!("electrum_height() should not be called on conflicted txs")
        }
    }
}

#[derive(Debug)]
pub enum Message {
    Request(String),
    IndexChange(IndexChange),
    Done,
}

pub enum Notification {
    IndexChangelog(Vec<IndexChange>),
    Exit,
}

pub struct ElectrumServer {
    notification: Sender<Notification>,
    server: Option<thread::JoinHandle<()>>, // so we can join the server while dropping this ojbect
}

impl ElectrumServer {
    fn start_notifier(
        notification: Channel<Notification>,
        subman: Arc<Mutex<SubscriptionManager>>,
        acceptor: Sender<Option<(TcpStream, SocketAddr)>>,
    ) {
        spawn_thread("notification", move || {
            for msg in notification.receiver().iter() {
                match msg {
                    Notification::IndexChangelog(changelog) => {
                        subman.lock().unwrap().dispatch(changelog);
                    }
                    Notification::Exit => acceptor.send(None).unwrap(),
                }
            }
        });
    }

    fn start_acceptor(addr: SocketAddr) -> Channel<Option<(TcpStream, SocketAddr)>> {
        let chan = Channel::unbounded();
        let acceptor = chan.sender();
        spawn_thread("acceptor", move || {
            let listener =
                TcpListener::bind(addr).unwrap_or_else(|e| panic!("bind({}) failed: {}", addr, e));
            info!(
                "Electrum RPC server running on {} (protocol {})",
                addr, PROTOCOL_VERSION
            );
            loop {
                let (stream, addr) = listener.accept().expect("accept failed");
                stream
                    .set_nonblocking(false)
                    .expect("failed to set connection as blocking");
                acceptor.send(Some((stream, addr))).expect("send failed");
            }
        });
        chan
    }

    pub fn start(addr: SocketAddr, query: Arc<Query>) -> Self {
        let notification = Channel::unbounded();
        Self {
            notification: notification.sender(),
            server: Some(spawn_thread("rpc", move || {
                let subman = Arc::new(Mutex::new(SubscriptionManager {
                    next_id: 0,
                    subscribers: HashMap::new(),
                }));
                let acceptor = Self::start_acceptor(addr);
                Self::start_notifier(notification, subman.clone(), acceptor.sender());
                let mut children = vec![];
                while let Some((stream, addr)) = acceptor.receiver().recv().unwrap() {
                    let query = query.clone();
                    let subman = subman.clone();
                    children.push(spawn_thread("peer", move || {
                        info!("[{}] connected peer", addr);
                        let conn = Connection::new(query, stream, addr, subman);
                        conn.run();
                        info!("[{}] disconnected peer", addr);
                    }));
                }
                let subman = subman.lock().unwrap();
                trace!("closing {} RPC connections", subman.subscribers.len());
                for (_, subscriber) in subman.subscribers.iter() {
                    let _ = subscriber.sender.send(Message::Done);
                }
                trace!("waiting for {} RPC handling threads", children.len());
                for child in children {
                    let _ = child.join();
                }
                trace!("RPC connections are closed");
            })),
        }
    }

    pub fn send_updates(&self, changelog: &Vec<IndexChange>) {
        let changelog: Vec<IndexChange> = changelog
            .iter()
            .filter(|change| match change {
                IndexChange::ChainTip(..) | IndexChange::History(..) => true,
                _ => false,
            })
            .cloned()
            .collect();

        if !changelog.is_empty() {
            self.notification
                .send(Notification::IndexChangelog(changelog))
                .unwrap();
        }
    }

    pub fn join(mut self) {
        if let Some(server) = self.server.take() {
            server.join().unwrap()
        }
    }
}

impl Drop for ElectrumServer {
    fn drop(&mut self) {
        trace!("stop accepting new RPCs");
        self.notification.send(Notification::Exit).unwrap();
        if let Some(handle) = self.server.take() {
            handle.join().unwrap();
        }
        trace!("RPC server is stopped");
    }
}

// unite with the http server subscription implementation?
struct SubscriptionManager {
    next_id: usize,
    subscribers: HashMap<usize, Subscriber>,
}

struct Subscriber {
    sender: SyncSender<Message>,
    // wants new blocks
    blocks: bool,
    // wants updates for these scripthashes
    scripthashes: HashSet<ScriptHash>,
}

impl SubscriptionManager {
    pub fn register(&mut self, sender: SyncSender<Message>) -> usize {
        let id = self.next_id;
        self.next_id = self.next_id + 1;
        self.subscribers.insert(
            id,
            Subscriber {
                sender,
                blocks: false,
                scripthashes: HashSet::new(),
            },
        );
        id
    }
    pub fn subscribe_blocks(&mut self, subscriber_id: usize) {
        self.subscribers
            .get_mut(&subscriber_id)
            .map(|s| s.blocks = true);
    }
    pub fn subscribe_scripthash(&mut self, subscriber_id: usize, scripthash: ScriptHash) {
        self.subscribers
            .get_mut(&subscriber_id)
            .map(|s| s.scripthashes.insert(scripthash));
    }
    pub fn remove(&mut self, subscriber_id: usize) {
        self.subscribers.remove(&subscriber_id);
    }
    pub fn dispatch(&mut self, changelog: Vec<IndexChange>) {
        if self.subscribers.is_empty() {
            return;
        }

        info!(
            "sending {} updates to {} rpc clients",
            changelog.len(),
            self.subscribers.len()
        );

        for change in changelog {
            self.subscribers.retain(|subscriber_id, subscriber| {
                let is_interested = match change {
                    IndexChange::ChainTip(..) => subscriber.blocks,
                    IndexChange::History(sh, ..) => subscriber.scripthashes.contains(&sh),
                    _ => unreachable!(), //we're not suppoed to be sent anything else
                };
                if is_interested {
                    // TODO determine status hash here, so its only computed once when there are multiple
                    // subscribers to the same scripthash. could also send the header hex directly, so
                    // bitcoind is only queried for it once.
                    let msg = Message::IndexChange(change.clone());
                    if let Err(TrySendError::Disconnected(_)) = subscriber.sender.try_send(msg) {
                        warn!("dropping disconnected subscriber #{}", subscriber_id);
                        return false;
                    }
                }
                true
            });
        }
    }
}

pub fn spawn_thread<F, T>(name: &str, f: F) -> thread::JoinHandle<T>
where
    F: FnOnce() -> T,
    F: Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(f)
        .unwrap()
}

pub struct SyncChannel<T> {
    tx: SyncSender<T>,
    rx: Receiver<T>,
}

impl<T> SyncChannel<T> {
    pub fn new(size: usize) -> SyncChannel<T> {
        let (tx, rx) = sync_channel(size);
        SyncChannel { tx, rx }
    }

    pub fn sender(&self) -> SyncSender<T> {
        self.tx.clone()
    }

    pub fn receiver(&self) -> &Receiver<T> {
        &self.rx
    }

    pub fn into_receiver(self) -> Receiver<T> {
        self.rx
    }
}

pub struct Channel<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
}

impl<T> Channel<T> {
    pub fn unbounded() -> Self {
        let (tx, rx) = channel();
        Channel { tx, rx }
    }

    pub fn sender(&self) -> Sender<T> {
        self.tx.clone()
    }

    pub fn receiver(&self) -> &Receiver<T> {
        &self.rx
    }

    pub fn into_receiver(self) -> Receiver<T> {
        self.rx
    }
}
