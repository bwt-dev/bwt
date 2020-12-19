package dev.bwt.daemon

import android.util.Log
import com.google.gson.Gson
import com.google.gson.annotations.SerializedName
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.lang.annotation.Native
import java.text.SimpleDateFormat
import java.util.*

class BwtDaemon(
    var config: BwtConfig,
) {
    var shutdownPtr: Long? = null;
    var electrumAddr: String? = null;
    var httpAddr: String? = null;

    fun start(callback: ProgressNotifier? = null) {
        val jsonConfig = Gson().toJson(config)
        NativeBwtDaemon.start(jsonConfig, object : CallbackNotifier {
            override fun onBooting() {
                Log.d("bwt", "booting")
                callback?.onBooting()
            }

            override fun onSyncProgress(progress: Float, tipUnix: Int) {
                val tipDate = Date(tipUnix.toLong() * 1000)
                Log.d("bwt", "sync progress ${progress * 100}%")
                callback?.onSyncProgress(progress, tipDate)
            }

            override fun onScanProgress(progress: Float, eta: Int) {
                Log.d("bwt", "scan progress ${progress * 100}%")
                callback?.onScanProgress(progress, eta)
            }

            override fun onElectrumReady(addr: String) {
                Log.d("bwt", "electrum ready on $addr")
                electrumAddr = addr
            }

            override fun onHttpReady(addr: String) {
                Log.d("bwt", "http ready on $addr")
                httpAddr = addr
            }

            override fun onReady(shutdownPtr_: Long) {
                Log.d("bwt", "services ready, starting background sync")
                shutdownPtr = shutdownPtr_
                callback?.onReady(this@BwtDaemon)
            }
        })
    }

    fun shutdown() {
        Log.d("bwt-daemon","shutdown $shutdownPtr")
        if (shutdownPtr != null) {
            NativeBwtDaemon.shutdown(shutdownPtr!!)
            shutdownPtr = null
        }
    }
}

interface ProgressNotifier {
    fun onBooting() {};
    fun onSyncProgress(progress: Float, tipUnix: Date) {};
    fun onScanProgress(progress: Float, eta: Int) {};
    fun onReady(bwt: BwtDaemon) {};
}

data class BwtConfig(
    @SerializedName("network") var network: String? = null,
    @SerializedName("bitcoind_url") var bitcoindUrl: String? = null,
    @SerializedName("bitcoind_auth") var bitcoindAuth: String? = null,
    @SerializedName("bitcoind_dir") var bitcoindDir: String? = null,
    @SerializedName("bitcoind_cookie") var bitcoindCookie: String? = null,
    @SerializedName("bitcoind_wallet") var bitcoindWallet: String? = null,
    @SerializedName("descriptors") var descriptors: Array<String>? = null,
    @SerializedName("xpubs") var xpubs: Array<String>? = null,
    @SerializedName("rescan_since") var rescanSince: Int? = null,
    @SerializedName("gap_limit") var gapLimit: Int? = null,
    @SerializedName("initial_import_size") var initialImportSize: Int? = null,
    @SerializedName("poll_interval") var pollInterval: Array<Int>? = null,
    @SerializedName("verbose") var verbose: Int? = null,
    @SerializedName("tx_broadcast_cmt") var txBroadcastCmd: String? = null,
    @SerializedName("electrum_addr") var electrumAddr: String? = null,
    @SerializedName("electrum_skip_merkle") var electrumSkipMerkle: Boolean? = null,
    @SerializedName("http_addr") var httpAddr: String? = null,
    @SerializedName("http_cors") var httpCors: Boolean? = null,
    @SerializedName("webhooks_urls") var webhookUrls: Array<String>? = null,
    @SerializedName("unix_listener_path") var unixListenerPath: String? = null,
) {}
