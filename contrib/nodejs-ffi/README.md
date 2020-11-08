# bwt-daemon

Programmatically manage the Bitcoin Wallet Tracker daemons.

### Install

```
$ npm install bwt-daemon
```

### Use

Below is a minimally viable configuration. If bitcoind is running at the default location,
with the default ports and cookie auth enabled, this should Just Work™ \o/

```js
import BwtDaemon from 'bwt-daemon'

let bwt = await BwtDaemon({
  xpubs: [ [ 'xpub66...', 'now' ] ],
  electrum: true,
})

console.log('bwt electrum server ready on', bwt.electrum_rpc_addr)
```

With some more advanced options:

```js
let bwt = await BwtDaemon({
  // Network and Bitcoin Core RPC settings
  network: 'regtest',
  bitcoind_dir: '/home/satoshi/.bitcoin',
  bitcoind_url: 'http://127.0.0.1:9008/',
  bitcoind_wallet: 'bwt',

  // Enable HTTP and Electrum servers
  http: true,
  electrum: true,

  // Bind on port 0 to use any available port (the default)
  electrum_rpc_addr: '127.0.0.1:0',
  http_server_addr: '127.0.0.1:0',

  // Descriptors or xpubs to track as an array of (desc_or_xpub, rescan_since) tuples
  // Use 'now' to look for new transactions only, or the unix timestamp to begin rescanning from.
  descriptors: [ [ 'wpkh(tpub61.../0/*)', 'now' ] ],
  xpubs: [ [ 'tpub66...', 'now' ] ],

  gap_limit: 10000,

  // Progress notifications for history scanning (a full rescan from genesis can take 20-30 minutes)
  progress_fn: progress => console.log('bwt progress %f%%', progress*100),
})

// Get the assigned address/port for the Electrum/HTTP servers
console.log('bwt electrum server ready on', bwt.electrum_rpc_addr)
console.log('bwt http server ready on', bwt.http_server_addr)

// Shutdown
bwt.shutdown()
```

### Options

#### Network and Bitcoin Core RPC
- `network`
- `bitcoind_dir`
- `bitcoind_wallet`
- `bitcoind_url`
- `bitcoind_auth`
- `bitcoind_cookie`

#### Address tracking
- `descriptors`
- `xpubs`
- `bare_xpubs`

#### General settings
- `verbose`
- `gap_limit`
- `initial_import_size`
- `poll_interval`
- `tx_broadcast_cmd`

#### Electrum
- `electrum`
- `electrum_rpc_addr`
- `electrum_skip_merkle`

#### HTTP
- `http`
- `http_server_addr`
- `http_cors`

#### Web Hooks
- `webhooks_urls`

#### UNIX only
- `unix_listener_path`

### License
MIT
