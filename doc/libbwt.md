# `libbwt`

C FFI library for programmatically managing the Bitcoin Wallet Tracker Electrum RPC and HTTP API servers.

`libbwt` has two primary use-cases:

1. Using the bwt Electrum server as a compatibility layer for Electrum-backed wallets
   that wish to support using a self-hosted Bitcoin Core full node as their backend,
   by running the server *in* the wallet.
  
2. Shipping software that leverages the [bwt HTTP API](https://github.com/shesek/bwt#http-api)
   as an all-in-one package, without requiring the user to separately install bwt.

Pre-built signed & deterministic `libbwt` library files (`so`/`dylib`/`dll`) are available for download from the
[releases page](https://github.com/shesek/bwt/releases) for Linux, Mac, Windows and ARMv7/8, including an `electrum_only` variant.

> ⚠️ WARNING: This is an alpha preview, released to gather developers' feedback. It is not ready for general use.

## Binding libraries

A nodejs package wrapping the native library with a more convenient higher-level API is [available here](https://github.com/shesek/bwt/tree/master/contrib/nodejs-bwt-daemon).

(More binding libraries coming soon, [let me know](https://github.com/shesek/bwt/issues/69) which you'd like to see first!)


## C interface

The interface exposes two functions, for starting and stopping the bwt servers.
Everything else happens through the Electrum/HTTP APIs.

```c
typedef void (*bwt_callback)(const char* msg_type, float progress,
                             uint32_t detail_n, const char* detail_s);

int32_t bwt_start(const char* json_config,
                  bwt_callback callback,
                  void** shutdown_out);

int32_t bwt_shutdown(void* shutdown_ptr);
```

Both functions return `0` on success or `-1` on failure.

### `bwt_start(json_config, callback, shutdown_out)`

Start the configured server(s).

This will block the current thread until the initial indexing is completed and the API servers
are ready, which may take awhile on the first run (depending on the `rescan_since` configuration).
If you'd like this to happen in the background, call this function in a new thread.

`json_config` should be provided as a JSON-encoded string. The list of options is available [below](#config-options).
Example minimal configuration:

```
{
  "bitcoind_dir": "/home/satoshi/.bitcoin",
  "descriptors": [ "wpkh(xpub66../0/*)" ],
  "electrum_addr": "127.0.0.1:0",
  "http_addr": "127.0.0.1:0"
}
```

> You can configure `electrum_addr`/`http_addr` to `127.0.0.1:0` to bind on any available port.
> The assigned port will be reported back via the `ready:X` notifications (see below).

The `callback(msg_type, progress, detail_n, detail_s)` function will be called with progress updates and information
about the running services, with the `progress` argument indicating the current progress as a float from 0 to 1.
The meaning of the `detail_{n,s}` field varies for the different `msg_type`s, which are:

- `booting` - Sent after the configuration is validated, right before booting up. `detail_{n,s}` are both empty.
- `progress:sync` - Progress updates for bitcoind's initial block download. `detail_n` contains the unix timestamp
  that the chain is currently synced up to.
- `progress:scan` - Progress updates for historical transactions rescanning. `detail_n` contains the estimated
  remaining time in seconds.
- `ready:electrum` - The Electrum server is ready. `detail_s` contains the address the server is bound on,
  as an `<ip>:<port>` string (useful for ephemeral binding on port 0).
- `ready:http` - The HTTP server is ready. `detail_s` contains the address the server is bound on.
- `ready` - Everything is ready.
- `error` - An error occurred during the initial indexing. `detail_s` contains the error message.

> The `detail_s` argument will be deallocated after the callback is called. If you need to keep it around, make a copy of it.
>
> Note that `progress:X` notifications will be sent from a different thread.

After the initial indexing is completed, a new thread will be spawned to sync new blocks/transactions in the background.

A shutdown handler for stopping bwt will be written to `shutdown_out`.

### `bwt_shutdown(shutdown_ptr)`

Shutdown bwt's API server(s) and the background syncing thread.

Should be called with the shutdown handler written to `shutdown_out`.

## Config Options

All options are optional, except for `descriptors`/`xpubs`/`addresses` (of which there must be at least one).

If bitcoind is running locally on the default port, at the default datadir location and with cookie auth enabled (the default), connecting to it should Just Work™, no configuration needed.

#### Network and Bitcoin Core RPC
- `network` - one of `bitcoin`, `testnet` or `regtest` (defaults to `bitcoin`)
- `bitcoind_url` - bitcoind url (defaults to `http://localhost:<network-rpc-port>/`)
- `bitcoind_auth` - authentication in `<user>:<pass>` format (defaults to reading from the cookie file)
- `bitcoind_dir` - bitcoind data directory (defaults to `/.bitcoin` on Linux, `~/Library/Application Support/Bitcoin` on Mac, or `%APPDATA%\Bitcoin` on Windows)
- `bitcoind_cookie` - path to cookie file (defaults to `.cookie` in the datadir)
- `bitcoind_wallet` - bitcoind wallet to use (for use with multi-wallet)

#### Address tracking
- `descriptors` - an array of descriptors to track
- `xpubs` - an array of xpubs to track (SLIP32 ypubs/zpubs are supported too)
- `addresses` - an array of addresses to track
- `addresses_file` - path to file with addresses (one per line)
- `rescan_since` - the unix timestamp to begin rescanning from, or 'now' to track new transactions only (scans from genesis by default)
- `gap_limit` - the [gap limit](https://github.com/shesek/bwt#gap-limit) for address import (defaults to 20)
- `initial_import_size` - the chunk size to use during the initial import (defaults to 350)

#### General settings
- `poll_interval` - interval for polling new blocks/transactions from bitcoind in seconds (defaults to 5)
- `tx_broadcast_cmd` - [custom command](https://github.com/shesek/bwt#scriptable-transaction-broadcast) for broadcasting transactions
- `verbose` - verbosity level for stderr log messages (0-4, defaults to 0)
- `require_addresses` - when disabled, the daemon will start even without any configured wallet addresses (defaults to true)
- `setup_logger` - initialize logging via `pretty_env_logger` or `android_logger` (defaults to true)

#### Electrum
- `electrum_addr` - bind address for electrum server (off by default)
- `electrum_skip_merkle` - skip generating merkle proofs (off by default)

#### HTTP
- `http_addr` - bind address for http server (off by default)
- `http_cors` - allowed cross-origins for http server (none by default)

#### Web Hooks
- `webhooks_urls` - array of urls to notify with index updates

#### UNIX only
- `unix_listener_path` - path to bind the [sync notification](https://github.com/shesek/bwt#real-time-indexing) unix socket (off by default)
