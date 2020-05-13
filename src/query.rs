use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use bitcoin::util::bip32::Fingerprint;
use bitcoin::{BlockHash, OutPoint, Txid};
use bitcoincore_rpc::{json as rpcjson, Client as RpcClient, RpcApi};

use crate::error::{OptionExt, Result};
use crate::hd::{HDWallet, KeyOrigin};
use crate::indexer::Indexer;
use crate::store::{FundingInfo, HistoryEntry, ScriptInfo, SpendingInfo, TxEntry};
use crate::types::{BlockId, ScriptHash, TxStatus};
use crate::util::make_fee_histogram;

#[cfg(feature = "track-spends")]
use crate::types::TxInput;

lazy_static! {
    static ref FEE_HISTOGRAM_TTL: Duration = Duration::from_secs(60);
    static ref FEE_ESTIMATES_TTL: Duration = Duration::from_secs(60);
}

pub struct Query {
    rpc: Arc<RpcClient>,
    indexer: Arc<RwLock<Indexer>>,

    cached_histogram: RwLock<Option<(FeeHistogram, Instant)>>,
    cached_estimates: RwLock<HashMap<u16, (Option<f64>, Instant)>>,
}

type FeeHistogram = Vec<(f32, u32)>;

impl Query {
    pub fn new(rpc: Arc<RpcClient>, indexer: Arc<RwLock<Indexer>>) -> Self {
        Query {
            rpc,
            indexer,
            cached_histogram: RwLock::new(None),
            cached_estimates: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_tip(&self) -> Result<BlockId> {
        let tip_height = self.get_tip_height()?;
        let tip_hash = self.get_block_hash(tip_height)?;
        Ok(BlockId(tip_height, tip_hash))
    }

    pub fn get_tip_height(&self) -> Result<u32> {
        Ok(self.rpc.get_block_count()? as u32)
    }

    pub fn get_header(&self, height: u32) -> Result<String> {
        self.get_header_by_hash(&self.get_block_hash(height)?)
    }

    pub fn get_headers(&self, heights: &[u32]) -> Result<Vec<String>> {
        Ok(heights
            .iter()
            .map(|h| self.get_header(*h))
            .collect::<Result<Vec<String>>>()?)
    }

    pub fn get_header_by_hash(&self, blockhash: &BlockHash) -> Result<String> {
        Ok(self
            .rpc
            .call("getblockheader", &[json!(blockhash), false.into()])?)
    }

    pub fn get_block_hash(&self, height: u32) -> Result<BlockHash> {
        Ok(self.rpc.get_block_hash(height as u64)?)
    }

    pub fn get_block_txids(&self, blockhash: &BlockHash) -> Result<Vec<Txid>> {
        let info = self.rpc.get_block_info(blockhash)?;
        Ok(info.tx)
    }

    pub fn broadcast(&self, tx_hex: &str) -> Result<Txid> {
        Ok(self.rpc.send_raw_transaction(tx_hex)?)
    }

    pub fn get_raw_mempool(&self) -> Result<HashMap<Txid, Value>> {
        Ok(self.rpc.call("getrawmempool", &[json!(true)])?)
    }

    pub fn estimate_fee(&self, target: u16) -> Result<Option<f64>> {
        ensure!(target < 1024, "target out of range");

        ttl_cache!(
            self.cached_estimates,
            *FEE_ESTIMATES_TTL,
            || -> Result<Option<f64>> {
                let feerate = self
                    .rpc
                    .estimate_smart_fee(target, None)?
                    .fee_rate
                    // from sat/kB to sat/b
                    .map(|rate| (rate.as_sat() as f64 / 1000f64) as f64);
                Ok(feerate)
            },
            target
        );
    }

    pub fn relay_fee(&self) -> Result<f64> {
        let feerate = self.rpc.call::<Value>("getmempoolinfo", &[])?["minrelaytxfee"]
            .as_f64()
            .or_err("invalid getmempoolinfo reply")?;

        // from BTC/kB to sat/b
        Ok((feerate * 100_000f64) as f64)
    }

    pub fn fee_histogram(&self) -> Result<FeeHistogram> {
        ttl_cache!(
            self.cached_histogram,
            *FEE_HISTOGRAM_TTL,
            || -> Result<FeeHistogram> {
                let mempool_entries = self.get_raw_mempool()?;
                Ok(make_fee_histogram(mempool_entries))
            }
        );
    }

    pub fn debug_index(&self) -> String {
        format!("{:#?}", self.indexer.read().unwrap().store())
    }

    pub fn dump_index(&self) -> Value {
        json!(self.indexer.read().unwrap().store())
    }

    pub fn get_history(&self, scripthash: &ScriptHash) -> Vec<HistoryEntry> {
        self.indexer
            .read()
            .unwrap()
            .store()
            .get_history(scripthash)
            .map_or_else(|| Vec::new(), |entries| entries.iter().cloned().collect())
    }

    pub fn list_unspent(
        &self,
        scripthash: Option<&ScriptHash>,
        min_conf: usize,
        include_unsafe: Option<bool>,
    ) -> Result<Vec<Txo>> {
        let (BlockId(tip_height, _), unspents) =
            self.list_unspent_raw(scripthash, min_conf, include_unsafe)?;

        let req_script_info =
            scripthash.map_or(Ok(None), |scripthash| -> Result<Option<ScriptInfo>> {
                let indexer = self.indexer.read().unwrap();
                let info = indexer.store().get_script_info(scripthash);
                Ok(Some(info.or_err("unknown scripthash")?))
            })?;

        Ok(unspents
            .into_iter()
            .filter_map(|unspent| {
                // XXX we assume that any unspent output with a "bwt/..." label is ours, this may not necessarily be true.
                let script_info = req_script_info.clone().or_else(|| {
                    let address = unspent.address.as_ref()?;
                    let label = unspent.label.as_ref()?;
                    let origin = KeyOrigin::from_label(label)?;
                    Some(ScriptInfo::from_address(address, origin))
                })?;
                Some(Txo::from_unspent(unspent, script_info, tip_height))
            })
            .collect())
    }

    fn list_unspent_raw(
        &self,
        scripthash: Option<&ScriptHash>,
        min_conf: usize,
        include_unsafe: Option<bool>,
    ) -> Result<(BlockId, Vec<rpcjson::ListUnspentResultEntry>)> {
        let address =
            scripthash.map_or(Ok(None), |scripthash| -> Result<Option<bitcoin::Address>> {
                let indexer = self.indexer.read().unwrap();
                let address = indexer.store().get_script_address(scripthash);
                Ok(Some(address.or_err("unknown scripthash")?))
            })?;

        // an empty array indicates not to filter by the address
        let addresses = address.as_ref().map_or(vec![], |address| vec![address]);

        loop {
            let tip_height = self.rpc.get_block_count()? as u32;
            let tip_hash = self.rpc.get_block_hash(tip_height as u64)?;

            let unspents = self.rpc.list_unspent(
                Some(min_conf),
                None,
                Some(&addresses[..]),
                include_unsafe,
                None,
            )?;

            if tip_hash != self.rpc.get_best_block_hash()? {
                warn!("tip changed while fetching unspents, retrying...");
                continue;
            }

            return Ok((BlockId(tip_height, tip_hash), unspents));
        }
    }

    pub fn lookup_txo(&self, outpoint: &OutPoint) -> Option<Txo> {
        let indexer = self.indexer.read().unwrap();
        let store = indexer.store();

        let FundingInfo(scripthash, amount) = store.lookup_txo_fund(outpoint)?;
        let script_info = store.get_script_info(&scripthash).unwrap();
        let status = store.get_tx_status(&outpoint.txid)?;

        Some(Txo {
            txid: outpoint.txid,
            vout: outpoint.vout,
            amount,
            script_info,
            status,
            safe: None,
            #[cfg(feature = "track-spends")]
            spent_by: store.lookup_txo_spend(outpoint),
        })
    }

    // avoid unnecessary copies by directly operating on the history entries as a reference
    pub fn map_history<T>(
        &self,
        scripthash: &ScriptHash,
        f: impl Fn(&HistoryEntry) -> T,
    ) -> Vec<T> {
        self.indexer
            .read()
            .unwrap()
            .store()
            .get_history(scripthash)
            .map(|history| history.into_iter().map(f).collect())
            .unwrap_or_else(|| vec![])
    }

    // -> get_tx_fee
    pub fn with_tx_entry<T>(&self, txid: &Txid, f: impl Fn(&TxEntry) -> T) -> Option<T> {
        self.indexer
            .read()
            .unwrap()
            .store()
            .get_tx_entry(txid)
            .map(f)
    }

    pub fn get_script_info(&self, scripthash: &ScriptHash) -> Option<ScriptInfo> {
        self.indexer
            .read()
            .unwrap()
            .store()
            .get_script_info(scripthash)
    }

    /// Get the scripthash balance as a tuple of (confirmed_balance, unconfirmed_balance)
    pub fn get_balance(&self, scripthash: &ScriptHash) -> Result<(u64, u64)> {
        let (_, unspents) = self.list_unspent_raw(Some(scripthash), 0, None)?;

        let (confirmed, unconfirmed): (Vec<_>, Vec<_>) = unspents
            .into_iter()
            .partition(|utxo| utxo.confirmations > 0);

        Ok((
            confirmed.iter().map(|u| u.amount.as_sat()).sum(),
            unconfirmed.iter().map(|u| u.amount.as_sat()).sum(),
        ))
    }

    pub fn get_tx_raw(&self, txid: &Txid) -> Result<Vec<u8>> {
        Ok(self.rpc.get_transaction(txid, Some(true))?.hex)
    }

    pub fn get_tx_json(&self, txid: &Txid) -> Result<Value> {
        let blockhash = self.find_tx_blockhash(txid)?;

        Ok(self.rpc.call(
            "getrawtransaction",
            &[json!(txid), true.into(), json!(blockhash)],
        )?)
    }

    pub fn find_tx_blockhash(&self, txid: &Txid) -> Result<Option<BlockHash>> {
        let indexer = self.indexer.read().unwrap();
        let tx_entry = indexer.store().get_tx_entry(txid).or_err("tx not found")?;
        Ok(match tx_entry.status {
            TxStatus::Confirmed(height) => Some(self.rpc.get_block_hash(height as u64)?),
            _ => None,
        })
    }

    pub fn map_history_since<T>(
        &self,
        min_block_height: u32,
        f: impl Fn(&HistoryEntry) -> T,
    ) -> Vec<T> {
        self.indexer
            .read()
            .unwrap()
            .store()
            .get_history_since(min_block_height)
            .into_iter()
            .map(f)
            .collect()
    }

    pub fn get_history_since(&self, min_block_height: u32) -> Vec<HistoryEntry> {
        self.map_history_since(min_block_height, |history_entry| history_entry.clone())
    }

    pub fn get_script_stats(&self, scripthash: &ScriptHash) -> Result<ScriptStats> {
        let indexer = self.indexer.read().unwrap();
        let store = indexer.store();
        let script_info = store
            .get_script_info(scripthash)
            .or_err("scripthash not found")?;
        let tx_count = store.get_tx_count(scripthash);
        let (confirmed_balance, unconfirmed_balance) = self.get_balance(scripthash)?;

        Ok(ScriptStats {
            script_info,
            tx_count,
            confirmed_balance,
            unconfirmed_balance,
        })
    }

    pub fn get_hd_wallets(&self) -> HashMap<Fingerprint, HDWallet> {
        self.indexer.read().unwrap().watcher().wallets().clone()
    }

    pub fn get_hd_wallet(&self, fingerprint: &Fingerprint) -> Option<HDWallet> {
        self.indexer
            .read()
            .unwrap()
            .watcher()
            .get(fingerprint)
            .cloned()
    }

    // get the ScriptInfo entry of a derived hd key, without it necessarily being indexed
    pub fn get_hd_script_info(&self, fingerprint: &Fingerprint, index: u32) -> Option<ScriptInfo> {
        let indexer = self.indexer.read().unwrap();
        let wallet = indexer.watcher().get(fingerprint)?;
        let key = wallet.derive(index);
        let address = wallet.to_address(&key);
        let scripthash = ScriptHash::from(&address);
        let origin = KeyOrigin::Derived(key.parent_fingerprint, index);
        Some(ScriptInfo::new(scripthash, address, origin))
    }

    pub fn get_tx_detail(&self, txid: &Txid) -> Option<TxDetail> {
        let index = self.indexer.read().unwrap();
        let store = index.store();
        let tx_entry = store.get_tx_entry(txid)?;

        let funding = tx_entry
            .funding
            .iter()
            .map(|(vout, FundingInfo(scripthash, amount))| {
                TxDetailFunding {
                    vout: *vout,
                    script_info: store.get_script_info(scripthash).unwrap(), // must exists
                    #[cfg(feature = "track-spends")]
                    spent_by: store.lookup_txo_spend(&OutPoint::new(*txid, *vout)),
                    amount: *amount,
                }
            })
            .collect::<Vec<TxDetailFunding>>();

        let spending = tx_entry
            .spending
            .iter()
            .map(|(vin, SpendingInfo(scripthash, prevout, amount))| {
                TxDetailSpending {
                    vin: *vin,
                    script_info: store.get_script_info(scripthash).unwrap(), // must exists
                    amount: *amount,
                    prevout: *prevout,
                }
            })
            .collect::<Vec<TxDetailSpending>>();

        let balance_change = {
            let funding_sum = funding.iter().map(|f| f.amount).sum::<u64>();
            let spending_sum = spending.iter().map(|s| s.amount).sum::<u64>();
            funding_sum as i64 - spending_sum as i64
        };

        Some(TxDetail {
            txid: *txid,
            status: tx_entry.status,
            fee: tx_entry.fee,
            funding: funding,
            spending: spending,
            balance_change: balance_change,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Txo {
    pub txid: Txid,
    pub vout: u32,
    pub amount: u64,
    #[serde(flatten)]
    pub script_info: ScriptInfo,
    #[serde(flatten)]
    pub status: TxStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe: Option<bool>, // not be available for txos we get from bwt's internal index
    #[cfg(feature = "track-spends")]
    pub spent_by: Option<TxInput>,
}

impl Txo {
    pub fn from_unspent(
        unspent: rpcjson::ListUnspentResultEntry,
        script_info: ScriptInfo,
        tip_height: u32,
    ) -> Self {
        Self {
            txid: unspent.txid,
            vout: unspent.vout,
            amount: unspent.amount.as_sat(),
            script_info: script_info,
            status: TxStatus::new(unspent.confirmations as i32, tip_height),
            safe: Some(unspent.safe),
            #[cfg(feature = "track-spends")]
            spent_by: None,
        }
    }
}

#[derive(Serialize, Debug)]
pub struct TxDetail {
    txid: Txid,
    #[serde(flatten)]
    status: TxStatus,
    fee: Option<u64>,
    funding: Vec<TxDetailFunding>,
    spending: Vec<TxDetailSpending>,
    balance_change: i64,
}

#[derive(Serialize, Debug)]
struct TxDetailFunding {
    vout: u32,
    #[serde(flatten)]
    script_info: ScriptInfo,
    amount: u64,
    #[cfg(feature = "track-spends")]
    spent_by: Option<TxInput>,
}

#[derive(Serialize, Debug)]
struct TxDetailSpending {
    vin: u32,
    #[serde(flatten)]
    script_info: ScriptInfo,
    amount: u64,
    prevout: OutPoint,
}

#[derive(Serialize, Debug)]
pub struct ScriptStats {
    #[serde(flatten)]
    script_info: ScriptInfo,
    tx_count: usize,
    confirmed_balance: u64,
    unconfirmed_balance: u64,
}
