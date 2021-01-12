# bwt-daemon

A nodejs library for programmatically managing the Bitcoin Wallet Tracker Electrum RPC and HTTP API servers
using the [`libbwt` C FFI interface](https://github.com/bwt-dev/libbwt).

> ⚠️ WARNING: This is an alpha preview, released to gather developers' feedback. It is not ready for general use.

### Install

```
$ npm install bwt-daemon
```

This will download the `libbwt` library for your platform.
The currently supported platforms are Linux, Mac, Windows and ARMv7/8.

The hash of the downloaded library is verified against the
[`SHA256SUMS`](https://github.com/shesek/bwt/blob/master/contrib/nodejs-bwt-daemon/SHA256SUMS)
file that ships with the npm package.

The library comes with the electrum and http servers by default.
If you're only interested in the Electrum server, you can install with `BWT_VARIANT=electrum_only npm install bwt-daemon`.
This reduces the download size by ~1.6MB.

> Note: `bwt-daemon` uses [`ffi-napi`](https://github.com/node-ffi-napi/node-ffi-napi), which requires
> a recent nodejs version. If you're running into errors during installation or segmentation faults,
> try updating to a newer version.

### Use

Below is a minimally viable setup. If bitcoind is running locally on the default port, at the default datadir location
and with cookie auth enabled (the default), this should Just Work™ \o/

```js
import BwtDaemon from 'bwt-daemon'

const bwtd = await BwtDaemon({
  xpubs: [ 'xpub66...' ],
  electrum: true,
}).start()

console.log('bwt electrum server ready on', bwtd.electrum_addr)
```

With some more advanced options:

```js
const bwtd = await BwtDaemon({
  // Network and Bitcoin Core RPC settings
  network: 'regtest',
  bitcoind_dir: '/home/satoshi/.bitcoin',
  bitcoind_url: 'http://127.0.0.1:9008/',
  bitcoind_wallet: 'bwt',

  // Descriptors and xpubs to track
  descriptors: [ 'wpkh(tpub61.../0/*)' ],
  xpubs: [ 'tpub66...' ],

  // Rescan since timestamp. Accepts unix timestamps, date strings, Date objects, or 'now' to look for new transactions only
  rescan_since: '2020-01-01',

  // Enable HTTP and Electrum servers
  http: true,
  electrum: true,

  // Bind on port 0 to use any available port (the default)
  electrum_addr: '127.0.0.1:0',
  http_addr: '127.0.0.1:0',

  // Set the gap limit of watched unused addresses
  gap_limit: 100,

  // Progress notifications for history scanning (a full rescan from genesis can take 20-30 minutes)
  progress: (type, progress, detail) => console.log('bwt %s progress %f%%', type, progress*100, detail),
}).start()

// Get the assigned address/port for the Electrum/HTTP servers
console.log('bwt electrum server ready on', bwtd.electrum_addr)
console.log('bwt http server ready on', bwtd.http_url)

// Shutdown
bwtd.shutdown()
```

See [`example.js`](https://github.com/shesek/bwt/blob/master/contrib/nodejs-bwt-daemon/example.js) for an even more complete
example, including connecting to the HTTP API.

The full list of options is available in the [libbwt documentation](https://github.com/bwt-dev/libbwt#config-options).
The nodejs wrapper also provides the following additional options:

- `progress` - callback for progress update notifications, invoked with `(type, progress, detail)` (optional)
- `electrum` - setting to `true` is an alias for `electrum_addr=127.0.0.1:0`
- `http` - setting to `true` is an alias for `http_addr=127.0.0.1:0`

### License
MIT
