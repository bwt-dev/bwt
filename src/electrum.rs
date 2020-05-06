use std::cmp;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use bitcoin::{BlockHash, Txid};
use bitcoin_hashes::{hex::FromHex, hex::ToHex, Hash};
use serde_json::{from_str, from_value, Value};

use crate::error::{fmt_error_chain, Result, ResultExt};
use crate::mempool::get_fee_histogram;
use crate::merkle::{get_header_merkle_proof, get_id_from_pos, get_merkle_proof};
use crate::query::Query;
use crate::types::{ScriptHash, StatusHash};

// Heavily based on the RPC server implementation written by Roman Zeyde for electrs,
// released under the MIT license. https://github.com/romanz/electrs

const PXT_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "1.4";
const MAX_HEADERS: u32 = 2016;

struct Connection {
    query: Arc<Query>,
    tip: Option<(u32, BlockHash)>,
    status_hashes: HashMap<ScriptHash, Option<StatusHash>>,
    stream: TcpStream,
    addr: SocketAddr,
    chan: SyncChannel<Message>,
}

impl Connection {
    pub fn new(query: Arc<Query>, stream: TcpStream, addr: SocketAddr) -> Connection {
        Connection {
            query,
            tip: None, // disable header subscription for now
            status_hashes: HashMap::new(),
            stream,
            addr,
            chan: SyncChannel::new(10),
        }
    }

    fn blockchain_headers_subscribe(&mut self) -> Result<Value> {
        let (tip_height, tip_hash) = self.query.get_tip()?;
        let tip_hex = self.query.get_header_by_hash(&tip_hash)?;
        self.tip = Some((tip_height, tip_hash));
        Ok(json!({ "height": tip_height, "hex": tip_hex }))
    }

    fn server_version(&self) -> Result<Value> {
        Ok(json!([format!("pxt {}", PXT_VERSION), PROTOCOL_VERSION]))
    }

    fn server_banner(&self) -> Result<Value> {
        Ok(json!("Welcome to pxt"))
    }

    fn server_donation_address(&self) -> Result<Value> {
        Ok(Value::Null)
    }

    fn server_peers_subscribe(&self) -> Result<Value> {
        Ok(json!([]))
    }

    fn mempool_get_fee_histogram(&self) -> Result<Value> {
        let histogram = get_fee_histogram(&self.query)?;
        Ok(json!(histogram))
    }

    fn blockchain_block_header(&self, params: Value) -> Result<Value> {
        let (height, cp_height): (u32, Option<u32>) = from_value(pad_params(params, 2))?;

        let header_hex = self.query.get_header(height)?;

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
        let heights: Vec<u32> = (start_height..(start_height + count)).collect();

        // TODO: "If the chain has not extended sufficiently far, only the available headers will
        // be returned. If more headers than max were requested at most max will be returned."
        let headers = self.query.get_headers(&heights)?;

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
        let (script_hash,): (String,) = from_value(params)?;
        let script_hash = decode_script_hash(&script_hash)?;

        let status_hash = get_status_hash(&self.query, &script_hash);
        self.status_hashes.insert(script_hash, status_hash.clone());

        Ok(json!(status_hash))
    }

    fn blockchain_scripthash_get_balance(&self, params: Value) -> Result<Value> {
        let (script_hash,): (String,) = from_value(params)?;
        let script_hash = decode_script_hash(&script_hash)?;

        let (confirmed_balance, mempool_balance) = self.query.get_balance(&script_hash)?;

        Ok(json!({
            "confirmed": confirmed_balance,
            "unconfirmed": mempool_balance,
        }))
    }

    fn blockchain_scripthash_get_history(&self, params: Value) -> Result<Value> {
        let (script_hash,): (String,) = from_value(params)?;
        let script_hash = decode_script_hash(&script_hash)?;

        let txs: Vec<Value> = self
            .query
            .get_history(&script_hash)?
            .into_iter()
            .map(|tx| {
                json!({
                    "height": tx.entry.status.electrum_height(),
                    "tx_hash": tx.txid,
                    "fee": tx.entry.fee,
                })
            })
            .collect();
        Ok(json!(txs))
    }

    fn blockchain_scripthash_listunspent(&self, params: Value) -> Result<Value> {
        let (script_hash,): (String,) = from_value(params)?;
        let script_hash = decode_script_hash(&script_hash)?;

        let utxos: Vec<Value> = self
            .query
            .list_unspent(&script_hash, 0)?
            .into_iter()
            .map(|utxo| {
                json!({
                    "height": utxo.status.electrum_height(),
                    "tx_hash": utxo.txid,
                    "tx_pos": utxo.vout,
                    "value": utxo.value
                })
            })
            .collect();
        Ok(json!(utxos))
    }

    fn blockchain_transaction_broadcast(&self, params: Value) -> Result<Value> {
        let (tx_hex,): (String,) = from_value(params)?;

        let txid = self.query.broadcast(&tx_hex)?;
        if let Err(e) = self.chan.sender().try_send(Message::PeriodicUpdate) {
            warn!("failed to issue PeriodicUpdate after broadcast: {}", e);
        }
        Ok(json!(txid.to_hex()))
    }

    fn blockchain_transaction_get(&self, params: Value) -> Result<Value> {
        let (txid, verbose): (Txid, Option<bool>) = from_value(pad_params(params, 2))?;
        let verbose = verbose.unwrap_or(false);

        Ok(if verbose {
            json!(self.query.get_transaction_decoded(&txid)?)
        } else {
            json!(self.query.get_transaction_hex(&txid)?)
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
        info!("handle command {} {:?}", method, params);
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
        info!("reply for {}: {:?}", method, result);

        Ok(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => {
                warn!("rpc #{} {} failed: {:?}", id, method, e,);
                json!({"jsonrpc": "2.0", "id": id, "error": fmt_error_chain(&e)})
            }
        })
    }

    fn update_subscriptions(&mut self) -> Result<Vec<Value>> {
        let mut result = vec![];
        if let Some(ref mut last_tip) = self.tip {
            let tip = self.query.get_tip()?;
            if *last_tip != tip {
                *last_tip = tip;
                let hex_header = self.query.get_header_by_hash(&tip.1)?;
                let header = json!({"hex": hex_header, "height": tip.0 });
                result.push(json!({
                    "jsonrpc": "2.0",
                    "method": "blockchain.headers.subscribe",
                    "params": [header]}));
            }
        }
        for (script_hash, status_hash) in self.status_hashes.iter_mut() {
            let new_status_hash = get_status_hash(&self.query, &script_hash);
            if new_status_hash == *status_hash {
                continue;
            }
            info!(
                "status hash for {:?} updated to {:?} (was {:?})",
                script_hash, new_status_hash, status_hash
            );
            result.push(json!({
                "jsonrpc": "2.0",
                "method": "blockchain.scripthash.subscribe",
                "params": [encode_script_hash(script_hash), new_status_hash]
            }));
            *status_hash = new_status_hash;
        }
        if result.len() > 0 {
            info!("sending notifications: {:#?}", result);
        }
        Ok(result)
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
            trace!("RPC {:?}", msg);
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
                Message::PeriodicUpdate => {
                    let values = self
                        .update_subscriptions()
                        .context("failed to update subscriptions")?;
                    self.send_values(&values)?
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
        debug!("[{}] shutting down connection", self.addr);
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

pub fn get_status_hash(query: &Query, scripthash: &ScriptHash) -> Option<StatusHash> {
    query.with_history_ref(scripthash, |entries| {
        let p = entries
            .iter()
            .map(|hist| format!("{}:{}:", hist.txid, hist.status.electrum_height()))
            .collect::<Vec<String>>();
        StatusHash::hash(&p.join("").into_bytes())
    })
}

fn encode_script_hash(hash: &ScriptHash) -> String {
    reverse_hash(*hash).to_hex()
}

fn decode_script_hash(s: &str) -> Result<ScriptHash> {
    Ok(reverse_hash(
        ScriptHash::from_hex(s).context("invalid script hash")?,
    ))
}

fn reverse_hash(hash: ScriptHash) -> ScriptHash {
    let mut inner = hash.into_inner();
    inner.reverse();
    ScriptHash::from_slice(&inner).unwrap()
}

#[derive(Debug)]
pub enum Message {
    Request(String),
    PeriodicUpdate,
    Done,
}

pub enum Notification {
    Periodic,
    Exit,
}

pub struct ElectrumServer {
    notification: Sender<Notification>,
    server: Option<thread::JoinHandle<()>>, // so we can join the server while dropping this ojbect
}

impl ElectrumServer {
    fn start_notifier(
        notification: Channel<Notification>,
        senders: Arc<Mutex<Vec<SyncSender<Message>>>>,
        acceptor: Sender<Option<(TcpStream, SocketAddr)>>,
    ) {
        spawn_thread("notification", move || {
            for msg in notification.receiver().iter() {
                let mut senders = senders.lock().unwrap();
                match msg {
                    Notification::Periodic => {
                        for sender in senders.split_off(0) {
                            if let Err(TrySendError::Disconnected(_)) =
                                sender.try_send(Message::PeriodicUpdate)
                            {
                                continue;
                            }
                            senders.push(sender);
                        }
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
                let senders = Arc::new(Mutex::new(Vec::<SyncSender<Message>>::new()));
                let acceptor = Self::start_acceptor(addr);
                Self::start_notifier(notification, senders.clone(), acceptor.sender());
                let mut children = vec![];
                while let Some((stream, addr)) = acceptor.receiver().recv().unwrap() {
                    let query = query.clone();
                    let senders = senders.clone();
                    children.push(spawn_thread("peer", move || {
                        info!("[{}] connected peer", addr);
                        let conn = Connection::new(query, stream, addr);
                        senders.lock().unwrap().push(conn.chan.sender());
                        conn.run();
                        info!("[{}] disconnected peer", addr);
                    }));
                }
                trace!("closing {} RPC connections", senders.lock().unwrap().len());
                for sender in senders.lock().unwrap().iter() {
                    let _ = sender.send(Message::Done);
                }
                trace!("waiting for {} RPC handling threads", children.len());
                for child in children {
                    let _ = child.join();
                }
                trace!("RPC connections are closed");
            })),
        }
    }

    pub fn notify(&self) {
        self.notification.send(Notification::Periodic).unwrap();
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
