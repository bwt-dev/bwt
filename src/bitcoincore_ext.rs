use bitcoincore_rpc::{json, Client, Result, RpcApi};

// Extensions for rust-bitcoincore-rpc

pub trait RpcApiExt: RpcApi {
    // Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/114
    fn get_mempool_entry(&self, txid: &bitcoin::Txid) -> Result<GetMempoolEntryResult> {
        self.call("getmempoolentry", &[json!(txid)])
    }

    // Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/111
    fn list_since_block(
        &self,
        blockhash: Option<&bitcoin::BlockHash>,
        target_confirmations: usize,
        include_watchonly: bool,
        include_removed: bool,
    ) -> Result<ListSinceBlockResult> {
        let args = [
            json!(blockhash),
            json!(target_confirmations),
            json!(include_watchonly),
            json!(include_removed),
        ];
        self.call("listsinceblock", &args)
    }

    // Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/110
    fn get_network_info_(&self) -> Result<GetNetworkInfoResult> {
        self.call("getnetworkinfo", &[])
    }

    /// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/131
    fn get_net_totals(&self) -> Result<GetNetTotalsResult> {
        self.call("getnettotals", &[])
    }

    /// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/129
    fn uptime(&self) -> Result<u64> {
        self.call("uptime", &[])
    }

    /// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/130
    fn get_network_hash_ps(&self, nblocks: u64) -> Result<f64> {
        self.call("getnetworkhashps", &[json!(nblocks)])
    }

    /// Pending https://github.com/rust-bitcoin/rust-bitcoincore-rpc/pull/132
    fn get_tx_out_set_info(&self) -> Result<GetTxOutSetInfoResult> {
        self.call("gettxoutsetinfo", &[])
    }

    // Only supports the fields we're interested in, not upstremable
    fn get_block_stats(&self, blockhash: &bitcoin::BlockHash) -> Result<GetBlockStatsResult> {
        let fields = (
            "height",
            "time",
            "total_size",
            "total_weight",
            "txs",
            "totalfee",
            "avgfeerate",
            "feerate_percentiles",
        );
        self.call("getblockstats", &[json!(blockhash), json!(fields)])
    }
}

impl RpcApiExt for Client {}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct ListSinceBlockResult {
    pub transactions: Vec<json::ListTransactionResult>,
    #[serde(default)]
    pub removed: Vec<json::ListTransactionResult>,
    pub lastblock: bitcoin::BlockHash,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetNetworkInfoResult {
    pub version: usize,
    pub subversion: String,
    #[serde(rename = "protocolversion")]
    pub protocol_version: usize,
    #[serde(rename = "localservices")]
    pub local_services: String,
    #[serde(rename = "localrelay")]
    pub local_relay: bool,
    #[serde(rename = "timeoffset")]
    pub time_offset: isize,
    pub connections: usize,
    #[serde(rename = "networkactive")]
    pub network_active: bool,
    pub networks: Vec<json::GetNetworkInfoResultNetwork>,
    #[serde(rename = "relayfee", with = "bitcoin::util::amount::serde::as_btc")]
    pub relay_fee: bitcoin::Amount,
    #[serde(
        rename = "incrementalfee",
        with = "bitcoin::util::amount::serde::as_btc"
    )]
    pub incremental_fee: bitcoin::Amount,
    #[serde(rename = "localaddresses")]
    pub local_addresses: Vec<json::GetNetworkInfoResultAddress>,
    pub warnings: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetMempoolEntryResult {
    /// Virtual transaction size as defined in BIP 141. This is different from actual serialized
    /// size for witness transactions as witness data is discounted.
    pub vsize: u64,
    /// Transaction weight as defined in BIP 141. Added in Core v0.19.0.
    pub weight: Option<u64>,
    /// Local time transaction entered pool in seconds since 1 Jan 1970 GMT
    pub time: u64,
    /// Block height when transaction entered pool
    pub height: u64,
    /// Number of in-mempool descendant transactions (including this one)
    #[serde(rename = "descendantcount")]
    pub descendant_count: u64,
    /// Virtual transaction size of in-mempool descendants (including this one)
    #[serde(rename = "descendantsize")]
    pub descendant_size: u64,
    /// Number of in-mempool ancestor transactions (including this one)
    #[serde(rename = "ancestorcount")]
    pub ancestor_count: u64,
    /// Virtual transaction size of in-mempool ancestors (including this one)
    #[serde(rename = "ancestorsize")]
    pub ancestor_size: u64,
    /// Hash of serialized transaction, including witness data
    pub wtxid: bitcoin::Txid,
    /// Fee information
    pub fees: GetMempoolEntryResultFees,
    /// Unconfirmed transactions used as inputs for this transaction
    pub depends: Vec<bitcoin::Txid>,
    /// Unconfirmed transactions spending outputs from this transaction
    #[serde(rename = "spentby")]
    pub spent_by: Vec<bitcoin::Txid>,
    /// Whether this transaction could be replaced due to BIP125 (replace-by-fee)
    #[serde(rename = "bip125-replaceable")]
    pub bip125_replaceable: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetMempoolEntryResultFees {
    /// Transaction fee in BTC
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub base: bitcoin::Amount,
    /// Transaction fee with fee deltas used for mining priority in BTC
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub modified: bitcoin::Amount,
    /// Modified fees (see above) of in-mempool ancestors (including this one) in BTC
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub ancestor: bitcoin::Amount,
    /// Modified fees (see above) of in-mempool descendants (including this one) in BTC
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub descendant: bitcoin::Amount,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetNetTotalsResult {
    /// Total bytes received
    #[serde(rename = "totalbytesrecv")]
    pub total_bytes_recv: u64,
    /// Total bytes sent
    #[serde(rename = "totalbytessent")]
    pub total_bytes_sent: u64,
    /// Current UNIX time in milliseconds
    #[serde(rename = "timemillis")]
    pub time_millis: u64,
    /// Upload target statistics
    #[serde(rename = "uploadtarget")]
    pub upload_target: GetNetTotalsResultUploadTarget,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetNetTotalsResultUploadTarget {
    /// Length of the measuring timeframe in seconds
    #[serde(rename = "timeframe")]
    pub time_frame: u64,
    /// Target in bytes
    pub target: u64,
    /// True if target is reached
    pub target_reached: bool,
    /// True if serving historical blocks
    pub serve_historical_blocks: bool,
    /// Bytes left in current time cycle
    pub bytes_left_in_cycle: u64,
    /// Seconds left in current time cycle
    pub time_left_in_cycle: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetTxOutSetInfoResult {
    /// The current block height (index)
    pub height: u64,
    /// The hash of the block at the tip of the chain
    //#[serde(with = "::serde_hex", rename = "bestblock")]
    //pub best_block: Vec<u8>,
    /// The number of transactions with unspent outputs
    pub transactions: u64,
    /// The number of unspent transaction outputs
    #[serde(rename = "txouts")]
    pub tx_outs: u64,
    /// A meaningless metric for UTXO set size
    pub bogosize: u64,
    /// The serialized hash
    pub hash_serialized_2: bitcoin::hashes::sha256::Hash,
    /// The estimated size of the chainstate on disk
    pub disk_size: u64,
    /// The total amount
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    pub total_amount: bitcoin::Amount,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GetBlockStatsResult {
    pub height: u64,
    pub time: u64,
    pub txs: u64,
    pub total_weight: u64,
    pub total_size: u64,
    #[serde(rename = "totalfee", with = "bitcoin::util::amount::serde::as_sat")]
    pub total_fee: bitcoin::Amount,
    #[serde(rename = "avgfeerate")]
    pub avg_fee_rate: u64,
    pub feerate_percentiles: (u64, u64, u64, u64, u64),
}
