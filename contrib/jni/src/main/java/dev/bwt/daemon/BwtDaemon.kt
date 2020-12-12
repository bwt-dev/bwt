package dev.bwt.daemon

import android.util.Log
import com.google.gson.Gson
import com.google.gson.annotations.SerializedName
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class BwtDaemon(
    private var shutdownPtr: Long?,
    val electrumAddr: String?,
    val httpAddr: String?
) {
    companion object {
        suspend fun init(
            config: BwtConfig,
            callback: ProgressNotifier? = null
        ): BwtDaemon = withContext(
            Dispatchers.IO
        ) {
            initBlocking(config, callback)
        }

        fun initBlocking(config: BwtConfig, callback: ProgressNotifier? = null): BwtDaemon {
            val jsonConfig = Gson().toJson(config)
            var electrumAddr: String? = null
            var httpAddr: String? = null

            val shutdownPtr =
                NativeBwtDaemon.start(jsonConfig, object : CallbackNotifier {
                    override fun onBooting() {
                        Log.d("bwt", "booting")
                    }
                    override fun onSyncProgress(progress: Float, tip: Int) {
                        Log.d("bwt", "sync progress ${progress * 100}%")
                        callback?.onSyncProgress(progress, tip)
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
                })

            Log.i("bwt", "all ready")

            return BwtDaemon(shutdownPtr, electrumAddr, httpAddr)
        }
    }

    fun shutdown() {
        NativeBwtDaemon.shutdown(this.shutdownPtr!!)
        this.shutdownPtr = null
    }
}

interface ProgressNotifier {
    fun onSyncProgress(progress: Float, tip: Int);
    fun onScanProgress(progress: Float, eta: Int);
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