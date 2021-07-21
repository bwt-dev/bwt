# Bitcoin Wallet Tracker

[![Build Status](https://travis-ci.org/bwt-dev/bwt.svg?branch=master)](https://travis-ci.org/bwt-dev/bwt)
[![Latest release](https://img.shields.io/crates/v/bwt.svg?label=release)](https://github.com/bwt-dev/bwt/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/bwt-dev/bwt/total.svg?color=blueviolet)](https://github.com/bwt-dev/bwt/releases)
[![Docker release](https://img.shields.io/docker/pulls/shesek/bwt.svg)](https://hub.docker.com/r/shesek/bwt)
[![Crate package](https://img.shields.io/crates/d/bwt.svg?label=crate+installs)](https://crates.io/crates/bwt)
[![MIT license](https://img.shields.io/github/license/bwt-dev/bwt.svg?color=yellow)](https://github.com/bwt-dev/bwt/blob/master/LICENSE)
[![Pull Requests Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](#developing)
[![Chat on Telegram](https://img.shields.io/badge/chat-on%20telegram-blue)](https://t.me/bwt_dev)
[![Chat on IRC](https://img.shields.io/badge/chat-on%20IRC-green.svg)](https://webchat.freenode.net/##bwt)

`bwt` is a lightweight wallet descriptor/xpub tracker and query engine for Bitcoin, implemented in Rust.

🔸 Personal wallet indexer (EPS-like)<br>
🔸 Descriptor-based tracking<br>
🔸 Electrum RPC server (also available as a plugin!)<br>
🔸 Developer-friendly, modern HTTP REST API<br>
🔸 Real-time updates with Server-Sent-Events or Web Hooks

Support development: [⛓️ on-chain or ⚡ lightning via BTCPay](https://btcpay.shesek.info/)

- [Intro](#intro)
- [Setting up bwt](#setting-up-bwt)
  - [Installation](#installation)
  - [Electrum-only server](#electrum-only-server)
  - [Pruning](#pruning)
  - [Real-time indexing](#real-time-indexing)
  - [Advanced options](#advanced-options)
- [Electrum plugin](#electrum-plugin) 💥
- [Manual Electrum setup](#manual-electrum-setup-without-the-plugin)
- [HTTP API](#http-api)
  - [Wallets](#wallets)
  - [Transactions](#transactions)
  - [Addresses](#addresses-scripthashes--keys)
  - [Outputs](#outputs)
  - [Blocks](#blocks)
  - [Mempool & Fees](#mempool--fees)
  - [Server-Sent Events](#server-sent-events) 🌟
  - [Miscellaneous](#miscellaneous)
- [Web Hooks](#web-hooks)
- [Developing](#developing) 👩‍💻
- [Reproducible builds](#reproducible-builds)
- [Thanks](#thanks)

## Intro

`bwt` is a lightweight and performant descriptor-based wallet indexer backed by a bitcoin full node, using a model similar to that of Electrum Personal Server.
It can serve as a personal alternative to public Electrum servers or power bitcoin apps such as wallet backends, payment processors and more.

It uses bitcoind to keep track of your wallet addresses (derived from your xpub(s)) and builds an index of their
history that can be queried using the Electrum RPC protocol or using bwt's custom designed [HTTP API](#http-api).

Real-time updates are available through [Server-Sent events](#server-sent-events) (a streaming long-lived HTTP connection),
or using [Web Hooks](#web-hooks) push updates (an HTTP request sent to your URL with the event).

The index is currently managed in-memory and does not get persisted (this is expected to change), but building it is pretty fast: bwt can index thousands of transactions in under a second.

*TL;DR: EPS + Rust + Modern HTTP API*

## Setting up bwt

Get yourself a synced Bitcoin Core node (v0.19 is recommended, v0.17 is sufficient. `txindex` is not required, pruning [is supported](#pruning))
and install bwt using one of the methods below.

### Installation

💥 Electrum desktop users can also [install bwt as an Electrum plugin](https://github.com/bwt-dev/bwt-electrum-plugin),
which provides a GUI and doesn't require the standalone server installation described below.

#### Signed pre-built binaries

Available for download on [the releases page](https://github.com/bwt-dev/bwt/releases) (Linux, Mac, Windows and ARMv7/v8).

The releases are signed by Nadav Ivgi (@shesek).
The public key can be verified on
the [PGP WoT](http://keys.gnupg.net/pks/lookup?op=vindex&fingerprint=on&search=0x81F6104CD0F150FC),
[github](https://api.github.com/users/shesek/gpg_keys),
[twitter](https://twitter.com/shesek),
[keybase](https://keybase.io/nadav),
[hacker news](https://news.ycombinator.com/user?id=nadaviv)
and [this video presentation](https://youtu.be/SXJaN2T3M10?t=4) (bottom of slide).

```bash
# Download (change x86_64-linux to your platform)
$ wget https://github.com/bwt-dev/bwt/releases/download/v0.2.4/bwt-0.2.4-x86_64-linux.tar.gz

# Fetch public key
$ gpg --keyserver keyserver.ubuntu.com --recv-keys FCF19B67866562F08A43AAD681F6104CD0F150FC

# Verify signature
$ wget -qO - https://github.com/bwt-dev/bwt/releases/download/v0.2.4/SHA256SUMS.asc \
  | gpg --decrypt - | grep ' bwt-0.2.4-x86_64-linux.tar.gz$' | sha256sum -c -

$ tar zxvf bwt-0.2.4-x86_64-linux.tar.gz
$ ./bwt-0.1.5-x86_64-linux/bwt --xpub <xpub> ...
```

The signature verification should show `Good signature from "Nadav Ivgi <nadav@shesek.info>" ... Primary key fingerprint: FCF1 9B67 ...` and `bwt-0.2.4-x86_64-linux.tar.gz: OK`.

The builds are reproducible and can be verified against Travis CI. See [more details here](#reproducible-builds).

#### From source

[Install Rust](https://rustup.rs/) and:

```bash
$ sudo apt install build-essential
$ git clone https://github.com/bwt-dev/bwt && cd bwt
$ cargo build --release
$ ./target/release/bwt --xpub <xpub> ...
```

Or using the crates.io package:

```bash
$ cargo install bwt
$ bwt --xpub <xpub>
```

(Make sure `~/.cargo/bin` is in your `PATH`)

#### With Docker

Assuming your bitcoin datadir is at `~/.bitcoin`,

```bash
$ docker run -it --net host -v ~/.bitcoin:/bitcoin:ro shesek/bwt --xpub <xpub> ...
```

(Mounting the bitcoin datadir is not necessary if you're not using the cookie file.)

#### Running bwt

`bwt --xpub <xpub>` should be sufficient to get you rolling.

You can configure the `--network` (defaults to `bitcoin`),
your `--bitcoind-url` (defaults to `http://127.0.0.1:<default-rpc-port>`),
`--bitcoind-dir` (defaults to `~/.bitcoin`) and
`--bitcoind-auth <user:pass>` (defaults to using the cookie file from `bitcoind-dir`).

You can set multiple `--xpub`s to track. This also supports ypubs and zpubs.
To set an explicit script type, use `--xpub <xpub>:<type>`, where type is one of `wpkh`, `shwpkh` or `pkh`.

You can also track output script descriptors using `--descriptor`. For example, `--descriptor 'wpkh(<xpub>/0/*)'`.

Standalone addresses can be tracked with `--address <address>` or `--addresses-file <path>`.

To speed up rescanning for historical transactions, you can provide the wallet creation date with `--rescan-since <timestmap>`.
The timestamp can be a `YYYY-MM-DD` formatted string or 'now' to disable rescanning and watch for new
transactions only (for newly created wallets).
*Setting this is highly recommended.*

By default, the Electrum server will be bound on port `50001`/`60001`/`60401` (according to the network)
and the HTTP server will be bound on port `3060`. This can be controlled with `--electrum-addr`
and `--http-addr`.

> ⚠️ Both the HTTP API server and the Electrum server are *unauthenticated and unencrypted* by default.
> If you're exposing them over the internet, they should be put behind a secure transport like an SSH tunnel,
> a VPN, or a Tor hidden service.
>
> Authentication (but not encryption) can be enabled by following [the instructions here](doc/auth.md).


You may set `-v` to increase verbosity or `-vv` to increase it more.

See `--help` for the full list of options.

#### Configuration file

Configuration options can be set under `~/bwt.env` as environment variables in the dotenv format.

Options that accept multiple values (`DESCRIPTORS`, `XPUBS`, `ADDRESSES` and `WEBHOOKS_URLs`)
can be provided as a `;`-separated list, or using wildcard `XPUB_*`/`DESC_*`/`ADDRESS_*` variables.

For example:

```
NETWORK=regtest
GAP_LIMIT=20
RESCAN_SINCE=2020-01-01
XPUBS='<xpub1>;<xpub2>'
DESCRIPTORS=pkh(<xpub>/0/*)
XPUB_BOB=<xpub3>
XPUB_ALICE=<xpub4>
```

Setting the environment variables directly is also supported.

### Electrum-only server

If you're only interested in a standalone Electrum server, you may disable the HTTP API server
by downloading the `electrum_only` pre-built binary,
using the `shesek/bwt:electrum` docker image
or building bwt with `--no-default-features --features cli,electrum`.

This removes several large dependencies and disables the `track-spends` database index
(which is not needed for the electrum server).

(Also see the [Electrum plugin](https://github.com/bwt-dev/bwt-electrum-plugin).)

### Pruning

You can use bwt with pruning, but a pruned node is only able to scan the recent blocks it still has available for transactions related to your wallet. This means that the `--rescan-since` date has to be within the range of non-pruned blocks, or set to 'now'. This makes pruned nodes primarily suitable for tracking newly created wallets.

There is, however, an opportunity to scan for your wallet's full history during the initial sync of your node, as the blocks will get scanned before getting pruned. You'll need to set `--no-wait-sync` to import the addresses without waiting for bitcoind to finish syncing first and make sure bwt runs before the earliest block containing a wallet transaction gets processed.

You can use `--prune-until <target>` to automatically prune the chain up to the given target (height, unix timestamp or YYYY-MM-DD formatted date). This requires configuring bitcoind with `prune=1` to allow manual pruning via the RPC.

To connect Electrum, you will need to configure it with [`--skipmerklecheck`](https://github.com/spesmilo/electrum/pull/4957) to tolerate missing SPV proofs (they will be unavailable for transactions in pruned blocks).

> If you're running Electrum with `--skipmerklecheck`, you may also configure bwt with `--electrum-skip-merkle` to save some resources by not generating SPV proofs even when it's possible.
> Both of these settings are automatically enabled when using the Electrum plugin.

### Bitcoin Core multi-wallet

If you're using [multi-wallet](https://bitcoin.org/en/release/v0.15.0.1#multi-wallet-support),
you can specify which wallet to use with `--bitcoind-wallet <name>` (or `-w <name>`).

Using a separate wallet for bwt is recommended. You can set
`-w bwt --create-wallet-if-missing` to have bwt create one for you.

*Note that EPS and bwt should not be run on the same bitcoind wallet with the same xpub, they will conflict.*

### Advanced options

#### Real-time indexing

By default, bwt will query bitcoind for new blocks/transactions every 5 seconds.
This can be adjusted with `--poll-interval <seconds>`.

To get *real* real-time updates, you may configure your bitcoind node to send a `POST /sync` request to the bwt
http server whenever a new block or wallet transaction is found, using the `walletnotify` and `blocknotify` options.

Example bitcoind configuration:
```
walletnotify=curl -X POST http://localhost:3060/sync
blocknotify=curl -X POST http://localhost:3060/sync
```

After verifying this works, you may increase your `--interval-poll` to avoid unnecessary indexing and reduce cpu usage.

If you're using the electrum-only mode without the http server, you may instead configure bwt to bind
on a unix socket using `--unix-listener-path <path>` and open a connection to it initiate an indexer sync.

For example, start with `--unix-listener-path /home/satoshi/bwt-sync-socket` and configure your bitcoind with:
```
walletnotify=nc -U /home/satoshi/bwt-sync-socket
blocknotify=nc -U /home/satoshi/bwt-sync-socket
```

If `nc` is not available, you can also use `socat - UNIX-CONNECT:/home/satoshi/bwt-sync-socket`.

If you're using docker, you can bind the socket on a directory mounted from the host to make it available outside the container.
For example, `--unix-listener-path /bitcoin/bwt-socket`.

##### Gap limit

You may configure the gap limit with `--gap--limit <N>` (defaults to 20).
The gap limit sets the maximum number of consecutive unused addresses to be imported before assuming there are no more used addresses to be discovered.

You can import larger batches with a higher gap during the initial sync using `--initial-import-size <N>` (defaults to 350).
Higher value means less rescans. Should be increased for large wallets.

##### Scriptable transaction broadcast

You may set a custom command for broadcasting transactions via `--tx-broadcast-cmd <cmd>`. The string `{tx_hex}` will be replaced with the hex-encoded transaction.

The command will be used in place of broadcasting transactions using the full node,
which may provide better privacy in some circumstances.

For example, to broadcast transactions over Tor using the blockstream.info onion service, you can use:

```
--tx-broadcast-cmd '[ $(curl -s -x socks5h://localhost:9050 http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/api/tx -d {tx_hex} -o /dev/stderr -w "%{http_code}" -H "User-Agent: curl/7.$(shuf -n1 -e 47 58 64 68).0") -eq 200 ]'
```

(Replace port `9050` with `9150` if you're using the Tor browser bundle.)

## Electrum plugin

You can setup bwt as an Electrum plugin, which allows connecting Electrum to a Bitcoin Core full node backend
by running an embedded Electrum server within the Electrum wallet itself.

See https://github.com/bwt-dev/bwt-electrum-plugin for more details and installation instructions.

![Screenshot of bwt integrated into Electrum](https://raw.githubusercontent.com/bwt-dev/bwt/master/doc/img/electrum-plugin.png)

## Manual Electrum setup (without the plugin)

[Setup the bwt server](#setting-up-bwt), then start Electrum with:

```bash
$ electrum --skipmerklecheck --oneserver --server 127.0.0.1:50001:t
```

Alternatively, you can also set `127.0.0.1:50001:t` as your server using the server selection GUI
(note the `:t`, which disables TLS). Note, however, that it is not possible to configure `oneserver`
and `skipmerklecheck` using the GUI.

> Note that setting `--skipmerklecheck` is only necessary if your node is pruned,
but it can also be used to save some resource when combined with `--electrum-skip-merkle`.
See [more details here](#pruning).

## HTTP API

All the endpoints return JSON. All bitcoin amounts are in satoshis.

### Wallets

Each wallet represents an output script descriptor.

Note that xpubs specified via `--xpub` will be represented as two descriptor wallet entries, one for the
external chain (used for receive addresses) and one for the internal chain (used for change addresses).
You can associate the wallets to their parent xpub using the `bip32_origins` field.

#### Wallet format

- `desc` - the output script descriptor tracked by this wallet
- `network` - the network this wallet belongs to (`bitcoin`, `testnet`, `signet` or `regtest`)
- `is_wildcard` - a boolean indicating whether the descriptor includes wildcard keys (`xpub../*`)
- `bip32_origins` - bip32 origin information for the keys contained in the descriptor
- `gap_limit` - the gap limited configured for this wallet
- `initial_import_size` - the gap limit used during the initial import
- `rescan_policy` - how far back rescanning should take place
- `max_funded_index` - the maximum derivation index that is known to have history
- `max_imported_index` - the maximum derivation index imported into bitcoind
- `satisfaction_weight` - an upper bound on the weight of a satisfying witness to the transaction (also see [here](https://docs.rs/miniscript/3.0.0/miniscript/descriptor/enum.Descriptor.html#method.max_satisfaction_weight))

See [`GET /wallet/:checksum`](#get-walletchecksum) for an example.

#### `GET /wallets`

Get a map of all tracked descriptor wallets, as a json object indexed by the descriptor checksum.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/wallets

{
  "xjm8w0el": {
    "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/*)#xjm8w0el",
    "network": "bitcoin",
    "is_wildcard": true,
    "bip32_origins": [ "80e042a9/0" ],
    ...
  },
  "k38panl4": {
    "desc": "wsh(multi(2,xpub661MyMwAqRbcEuy9nKLTbGCi2NhqTWeQPT3gd2QdfmeaieDHLHiwTnSnw1GrP2xdaJwEDQJLasfw6LNK7hVADcCN9d1M1RtxitrR3CwvtjV/0/*,[16eabcf7/2]xpub684GUXwH4bY8Pf3fgSunTGz3hwJZhJzaNwgT55aWGWQM7KsiUFEXWLYPy1Q19gAEvc9LG5TN5PdmGPoyocmkkpKCCMV27ugL7XqHeHRwJzH/1/*))#k38panl4",
    "network": "bitcoin",
    "is_wildcard": true,
    "bip32_origins": [ "367e5b47/0", "16eabcf7/2/1" ],
    ...
  },
  "cletf5fc": {
    "desc": "pkh(xpub661MyMwAqRbcEoHAdGB6AaGRhLmHVemxe6acQikhJgfV3sr1SmapjQv8ZfBwWa1YKmFbyR6ta96TKiCNTctvZix58hAR7mDtjdWK2E18PjR/0/*)#cletf5fc",
    "network": "bitcoin",
    "is_wildcard": true,
    "bip32_origins": [ "7a32efaa/0" ],
    ...
  },
  "ftu25peq": {
    "desc": "pkh(xpub661MyMwAqRbcEoHAdGB6AaGRhLmHVemxe6acQikhJgfV3sr1SmapjQv8ZfBwWa1YKmFbyR6ta96TKiCNTctvZix58hAR7mDtjdWK2E18PjR/1/*)#ftu25peq",
    "network": "bitcoin",
    "is_wildcard": true,
    "bip32_origins": [ "7a32efaa/1" ],
    ...
  }
}

```

These wallets are the result of starting bwt with:

- `--descriptor 'wpkh(xpub...xToJdcy/0/*)'` (the `xjm8w0el` wallet)
- `--descriptor 'wsh(multi(2,xpub...3CwvtjV/0/*,[16eabcf7/2]xpub...eHRwJzH/1/*))'` (the `k38panl4` wallet)
- `--xpub 'xpub...2E18PjR'` (the `cletf5fc` and `ftu25peq` wallets)

</details>

#### `GET /wallet/:checksum`

Get information about the descriptor wallet identified by its `checksum`.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/wallet/xjm8w0el
{
  "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/*)#xjm8w0el",
  "network": "bitcoin",
  "is_wildcard": true,
  "bip32_origins": [ "80e042a9/0" ],
  "rescan_policy": "now",
  "max_funded_index": null,
  "max_imported_index": 19,
  "gap_limit": 20,
  "initial_import_size": 20
}

```
</details>

#### `GET /wallet/:checksum/:index`

Get basic information for the wallet child address at derivation index `index`.

<details><summary>Expand...</summary><p></p>

Returned fields:
- `address`
- `scripthash`
- `origin` - descriptor wallet origin information in `<checksum>/<index>` format
- `desc` - the descriptor for this address
- `bip32_origins` - an array of bip32 origins for the derived keys at this index

Examples:
```
$ curl localhost:3060/wallet/xjm8w0el/8
{
  "address": "bc1qlwdpe3a0ulss57tqxxkpepylazpy9f07f2jufa",
  "scripthash": "5e643e168a629406504ee2651f76efc3510a79d0aaa1fda9893b2a98c73440dc",
  "origin": "xjm8w0el/8",
  "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/8)#dtxjzdej",
  "bip32_origins": [ "80e042a9/0/8" ]
}

$ curl localhost:3060/wallet/k38panl4/5
{
  "address": "bc1q9r669mzwaal8cj0hplp7qsqd6kl4xu76w6nuprqd75zrll8s5lsqgeka5t",
  "scripthash": "2eb2e9b8b0ee5b6e2aea7a8ee5897d448ab089c7d360a287b27e767320d5902a",
  "origin": "k38panl4/5",
  "desc": "wsh(multi(2,xpub661MyMwAqRbcEuy9nKLTbGCi2NhqTWeQPT3gd2QdfmeaieDHLHiwTnSnw1GrP2xdaJwEDQJLasfw6LNK7hVADcCN9d1M1RtxitrR3CwvtjV/0/5,[16eabcf7/2]xpub684GUXwH4bY8Pf3fgSunTGz3hwJZhJzaNwgT55aWGWQM7KsiUFEXWLYPy1Q19gAEvc9LG5TN5PdmGPoyocmkkpKCCMV27ugL7XqHeHRwJzH/1/5))#5sd0njd0",
  "bip32_origins": [ "367e5b47/0/5", "16eabcf7/2/1/5" ]
}
```
</details>

#### `GET /wallet/:checksum/next`

Get the next unused address in the specified wallet.

<details><summary>Expand...</summary><p></p>

Issues a 307 redirection to the url of the next derivation index (`/wallet/:checksum/:index`) *and* responds with the derivation index in the responses body.

Note that the returned address is not marked as used until receiving funds; If you wish to skip it and generate a different
address without receiving funds to it, you can specify an explicit derivation index instead.

Non-wildcard descriptors always return `0` as their next index.

Examples:
```
$ curl localhost:3060/wallet/xjm8w0el/next
< HTTP/1.1 307 Temporary Redirect
< Location: /wallet/xjm8w0el/104
104

# Follow the redirect to get the full address json

$ curl --location localhost:3060/wallet/xjm8w0el/next
{
  "address": "bc1qu8k2dv6s8kjaywvdrrk3mvju6utyx537puaeal",
  "origin": "xjm8w0el/104",
  ...
}
```
</details>

#### `GET /wallet/:checksum/gap`

Get the current maximum number of consecutive unused addresses in the specified wallet.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/wallet/xjm8w0el/gap

7
```
</details>

### Transactions

#### Wallet transaction format

This format is only available for wallet transactions and includes contextual wallet information about funded outputs and spent inputs.
It does not include inputs/outputs that are unrelated the wallet.

Transaction fields:
- `txid`
- `block_height` - the confirming block height or `null` for unconfirmed transactions
- `funding` - contains an entry for every output created by this transaction that is owned by the wallet
  - `vout` - the output index
  - `amount` - the output amount in satoshis
  - scriptPubKey fields
    - `address` - the address funded by this output
    - `scripthash` - the scripthash funded by this output
    - `origin` - descriptor wallet origin information in `<checksum>/<index>` format
    - `desc` - the descriptor for this script
    - `bip32_origins` - an array of bip32 origins for this script keys
  - `spent_by` - the transaction input spending this output in `txid:vin` format, or `null` for unspent outputs (only available with `track-spends`)
- `spending` - contains an entry for every input spending a wallet output
  - `vin` - the input index
  - `amount` - the amount in satoshis of the previous output spent by this input
  - `prevout` - the `<txid>:<vout>` being spent
  - scriptPubKey fields
    - `address`, `scripthash`, `origin`, `desc`, `bip32_origins`<br>*(same format as above for `funding`)*
- `balance_change` - the net change to the wallet balance inflicted by this transaction

Additional fields for unconfirmed mempool transactions:

- `own_feerate` - the fee rate paid directly by the transaction, in `sat/vB`
- `effective_feerate` - the effective transaction fee rate, taking unconfirmed ancestor transactions into account
- `bip125_replacelable` - whether this transaction can be replaced due to BIP 125 replace-by-fee (because either it or one of its unconfirmed ancestors are signaling to opt-into rbf)
- `has_unconfirmed_parents` - whether this transaction has unconfirmed parents used as its inputs

The effective fee rate is calculated as `MIN(own_fee/own_vsize, (own_fee+ancestor_fee)/(own_vsize+ancestor_vsize))`.

> The mempool fields may be temporarily unavailable if an error is encountered while syncing the mempool entries.
> This should be a rare event, for example if Bitcoin Core crashes, but API clients should prepare for the possibility.

#### `GET /tx/:txid`

Get the transaction in the [wallet transaction format](#wallet-transaction-format).

<details><summary>Expand...</summary><p></p>

*Available for wallet transactions only.*

Example:
```
$ curl localhost:3060/tx/e700187477d262f370b4f1dfd17c496d108524ee2d440a0b7e476f66da872dda
{
  "txid": "e700187477d262f370b4f1dfd17c496d108524ee2d440a0b7e476f66da872dda",
  "block_height": 113,
  "fee": 141,
  "funding": [
    {
      "vout": 1,
      "amount": 949373,
      "address": "bcrt1qu04qqzwkjvya65g2agwx5gnqvgzwpjkr6q5jvf",
      "scripthash": "6bf2d435bc4e020d839900d708f02c721728ca6793024919c2c5bc029c00f033",
      "origin": "xjm8w0el/16",
      "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/16)#dtxjzdej",
      "bip32_origins": [ "80e042a9/0/16" ],
      "spent_by": "950cc16e572062fa16956c4244738b35ea7b05e16c8efbd6b9812d561d68be3a:0"
    }
  ],
  "spending": [
    {
      "vin": 0,
      "amount": 1049514,
      "prevout": "70650243572b90705f7fe95c9f30a85a0cc55e4ea3159a8ada5f4d62d9841d7b:1",
      "address": "bcrt1qxsvdm3jmwr79u67d82s08uykw6a82agzy42c6y",
      "scripthash": "a55c30f4f7d79600d568bdfa0b4f48cdce4e59b6ffbf286e99856c3e8699740d",
      "origin": "xjm8w0el/19",
      "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/19)#wjtzksqe",
      "bip32_origins": [ "80e042a9/0/19" ]
    }
  ],
  "balance_change": -100141
}
```
</details>

#### `GET /tx/:txid/verbose`

Get the transaction in JSON as formatted by [bitcoind's `getrawtransaction`](https://bitcoincore.org/en/doc/0.19.0/rpc/rawtransactions/getrawtransaction/) with `verbose=true`.

<details><summary>Expand...</summary><p></p>

Available for all transactions that bitcoind is aware of (i.e. not pruned).
Requires `txindex` to work for non-wallet transactions.

Example:
```
$ curl localhost:3060/tx/1f2e3c4cee8ea127a79c5dbc951f1e005671a1e8bf385e791ff95b780deda68f/verbose
{
  "blockhash": "7a9b99f78066f22a26c56b2035445285a5a992fc19719c9c27f2255f20f1f2f8",
  "blocktime": 1589376781,
  "confirmations": 1,
  "hash": "0ed38dcfe3de4e96852631d9c1f692db581513aa51845976a7097e618a1002a7",
  "hex": "0200000000010132d8a06f451ca6e8487a25343586f4186faecbbd185c324f5c3cfe674d1385460100000000feffffff0200e1f505000000001600143e730c6086a8417e2532356bd43e34b86c0f6055d6df0a1e010000001600146873ceae00e9140ea09b71963ee0e493b678a0ec02473044022025500722fd65172f8f7fe448ee484e5659aa9ff23d055546480588840b8eef40022023509d517614fa0c51d55511826250ead7d44ada9d9d6671837f44e8088d678b012102b3ce722e57fa6b66985154305e3d06831976499cf9f1db0c4e30450c1d5d7724af000000",
  "in_active_chain": true,
  "locktime": 175,
  "size": 222,
  "time": 1589376781,
  "txid": "1f2e3c4cee8ea127a79c5dbc951f1e005671a1e8bf385e791ff95b780deda68f",
  "version": 2,
  "vin": [ ... ],
  "vout": [ ... ],
  "vsize": 141,
  "weight": 561
}
```
</details>

#### `GET /tx/:txid/hex`

Get the raw transaction formatted as a hex string.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/tx/1f2e3c4cee8ea127a79c5dbc951f1e005671a1e8bf385e791ff95b780deda68f/hex
0200000000010132d8a06f451ca6e8487a25343586f4186faecbbd185c324f5c3cfe674d1385460100000000feffffff0200e1f505000000001600143e730c6086a8417e2532356bd43e34b86c0f6055d6df0a1e010000001600146873ceae00e9140ea09b71963ee0e493b678a0ec02473044022025500722fd65172f8f7fe448ee484e5659aa9ff23d055546480588840b8eef40022023509d517614fa0c51d55511826250ead7d44ada9d9d6671837f44e8088d678b012102b3ce722e57fa6b66985154305e3d06831976499cf9f1db0c4e30450c1d5d7724af000000
```

</details>

#### `GET /tx/:txid/proof`

Get the merkle inclusion proof for the transaction.

<details><summary>Expand...</summary><p></p>

Returned in [bitcoind's `merkleblock`](https://developer.bitcoin.org/reference/p2p_networking.html#merkleblock) format.

Example:
```
$ curl localhost:3060/tx/1f2e3c4cee8ea127a79c5dbc951f1e005671a1e8bf385e791ff95b780deda68f/proof

010000302d2659e4f39beb46eeef8579841250550a78d2a4fc2d53022a3ac0c069dbe865a987be21bcaae85f1423967b22c09b61ab00fd2c5aeccc247ef8ab421a960f950df7bb5effff7f20030000000400000003e7e777f5557142e8725218ae37547e1834a0466d5bb63952a7915d9cdc7adf392bb0a723f73dc23b52b7bd2641fb22d283b7c8a863b6dd012c64243b5da66b418fa6ed0d785bf91f795e38bfe8a17156001e1f95bc5d9ca727a18eee4c3c2e1f0115
```

</details>

#### `GET /txs`
#### `GET /txs/since/:block-height`

Get all wallet transactions confirmed at or after `block-height`, plus all unconfirmed transactions,
for all tracked addresses.

`GET /txs` is an alias for `GET /txs/since/0`.

<details><summary>Expand...</summary><p></p>

Returned in the [wallet transaction format](#wallet-transaction-format). Sorted with oldest first.

Example:
```
$ curl localhost:3060/txs
[
  {
    "txid": "e700187477d262f370b4f1dfd17c496d108524ee2d440a0b7e476f66da872dda",
    "funding": [ .. ],
    "spending": [ .. ],
    ...
  },
  ...
]
```

</details>

#### `GET /txs/since/:block-height/compact`

Get a compact minimal representation of all wallet transactions since `block-height`.

<details><summary>Expand...</summary><p></p>

Returns a simple JSON array of `[txid, block_height]` tuples, where `block_height` is null for unconfirmed transactions. Sorted with oldest first.

Example:
```
$ curl localhost:3060/txs/since/105/compcat
[
  ["859d5c41661426ab13a7816b9e845a3353b66f00a3c14bc412d20f87dcf19caa", 105],
  ["3c3c8722b493bcf43adab323581ea1da9f9a9e79628c0d4c89793f7fe21b68cf", 107],
  ["e51414f57bdee681d48a6ade696049c4d7569a062278803fb7968d9a022c6a96", null],
  ...
]
```

</details>

#### `POST /tx`

Broadcast a raw transaction to the Bitcoin network.

<details><summary>Expand...</summary><p></p>

Returns the `txid` on success.

Body parameters:
- `tx_hex` - the raw transaction encoded as a hex string

Example:

```
$ curl -X POST localhost:3060/tx -H 'Content-Type: application/json' \
       -d '{"tx_hex":"<hex-serialized-tx>"}'

33047288f0502eb3f2ad0729f6cfa24a8db87842f9c9a8eba7c0dbfaf7ea75b4
```

</details>

### Addresses, Scripthashes & Keys

#### `GET /address/:address`
#### `GET /scripthash/:scripthash`
#### `GET /wallet/:checksum/:index`


Get basic information for the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

Returned fields:
- `address`
- `scripthash`
- `origin` - descriptor wallet origin information in `<checksum>/<index>` format
- `desc` - the descriptor for this address
- `bip32_origins` - an array of bip32 origins for the derived keys at this index

Example:
```
$ curl localhost:3060/address/bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg
{
  "address": "bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg",
  "scripthash": "4c1af417b86da82af887678c36c93d3d8de15a5930f326600e533bf3ab9d0339",
  "origin": "xjm8w0el/10",
  "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/10)#v9use49n",
  "bip32_origins": [ "80e042a9/0/10" ]
}
```

</details>

#### `GET /address/:address/stats`
#### `GET /scripthash/:scripthash/stats`
#### `GET /wallet/:checksum/:index/stats`

Get basic information and stats for the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

Returned fields:
- scriptPubKey fields
  - `address`
  - `scripthash`
  - `origin`
  - `desc`
  - `bip32_origins`
- `tx_count`
- `confirmed_balanace`
- `unconfirmed_balanace`

Example:
```
$ curl localhost:3060/address/bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg/stats
{
  "address": "bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg",
  "origin": "xjm8w0el/10",
  ...,
  "tx_count": 2,
  "confirmed_balance": 120050000,
  "unconfirmed_balance": 0
}
```
</details>

#### `GET /address/:address/utxos`
#### `GET /scripthash/:scripthash/utxos`
#### `GET /wallet/:checksum/:index/utxos`

Get the list of unspent transaction outputs owned by the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

Query string parameters:
- `min_conf` - minimum number of confirmations, defaults to 0
- `include_unsafe` - whether to include outputs that are not safe to spend (unconfirmed from outside keys or with RBF), defaults to true

Examples:
```
$ curl localhost:3060/address/bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg/utxos
[
  {
    "txid": "664fba0bcc745b05fda0fbf1f6fb6fc003afd82e64caad2c9fea0e3d566f6a58",
    "vout": 1,
    "amount": 1500000,
    "address": "bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg",
    "scripthash": "4c1af417b86da82af887678c36c93d3d8de15a5930f326600e533bf3ab9d0339",
    "origin": "xjm8w0el/10",
    "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/10)#v9use49n",
    "bip32_origins": [ "80e042a9/0/10" ],
    "block_height": 114,
    "spent_by": null
  },
  {
    "txid": "3a1c4dea8d376a2762dd9be1d39f7f13376b4c9ccb961725574689183c20cb90",
    "vout": 1,
    "amount": 1440000,
    "address": "bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg",
    "scripthash": "4c1af417b86da82af887678c36c93d3d8de15a5930f326600e533bf3ab9d0339",
    "origin": "xjm8w0el/10",
    "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/10)#v9use49n",
    "bip32_origins": [ "80e042a9/0/10" ],
    "block_height": 115,
    "spent_by": null
  },
  ...
]
```
</details>

#### `GET /address/:address/txs`
#### `GET /scripthash/:scripthash/txs`
#### `GET /wallet/:checksum/:index/txs`

Get the list of all transactions in the history of the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

Returned in the [wallet transaction format](#wallet-transaction-format).

Example:
```
$ curl localhost:3060/address/bc1qaxlg48awxth5k72ltgrjp6qyegzdmfkfupyhhg/txs
[
  {
    "txid": "859d5c41661426ab13a7816b9e845a3353b66f00a3c14bc412d20f87dcf19caa",
    "block_height": 654835,
    "funding": [ ... ],
    "spending": [ ...],
    "balance_change": 11000000
  },
  ...
]
```

</details>

#### `GET /address/:address/txs/compact`
#### `GET /scripthash/:scripthash/txs/compact`
#### `GET /wallet/:checksum/:index/txs/compact`

Get a compact minimal representation of the history of the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

Returns a simple JSON array of `[txid, block_height]` tuples, where `block_height` is null for unconfirmed transactions.

Example:
```
$ curl localhost:3060/scripthash/c511375da743d7f6276db6cdaf9f03d7244c74d5569c9a862433e37c5bc84cb2/txs/minimal
[
  ["859d5c41661426ab13a7816b9e845a3353b66f00a3c14bc412d20f87dcf19caa", 105],
  ["3c3c8722b493bcf43adab323581ea1da9f9a9e79628c0d4c89793f7fe21b68cf", 107],
  ["e51414f57bdee681d48a6ade696049c4d7569a062278803fb7968d9a022c6a96", null],
  ...
]
```

</details>


### Outputs

#### Output format

- `txid` - the transaction funding this output
- `vout` - the output index
- `amount` - the output amount in satoshis
- scriptPubKey fields
  - `address` - the address funded by this output
  - `scripthash` - the scripthash funded by this output
  - `origin` - descriptor wallet origin information in `<checksum>/<index>` format
  - `desc` - the descriptor for the funded script
  - `bip32_origins` - an array of bip32 origins for this script keys
- `block_height` - the confirming block height or `null` for unconfirmed transactions
- `spent_by` - the transaction input spending this output in `txid:vin` format, or `null` for unspent outputs (only available with `track-spends`)

#### `GET /txo/:txid/:vout`

Get information about the specified transaction output.

<details><summary>Expand...</summary><p></p>

*Available for wallet outputs only.*

Example:
```
$ curl localhost:3060/txo/1b1170ac5996df9255299ae47b26ec3ad57c9801bc7bae68203b1222350d52fe/0
{
  "txid": "1b1170ac5996df9255299ae47b26ec3ad57c9801bc7bae68203b1222350d52fe",
  "vout": 0,
  "amount": 99791,
  "scripthash": "d7a6ac0b7af9fe218f24019dc2fe7919bd14fb56694056528464326a44917d20",
  "address": "bc1qrkud59a02lacfsa8hlp6yhg7qed30f2w7g2eh3",
  "origin": "xjm8w0el/32,
  "desc": "wpkh(xpub661MyMwAqRbcEhsxS9g2qyYKSGA3seqWVNhmVhU27ddQx952PaZ6G4V26msGKrqYBjoBRwFyzaucPUkhw7DNaeMVUYJV1bqosxzVxToJdcy/0/32)#v9use49n",
  "bip32_origins": [ "80e042a9/0/32" ],
  "block_height": 654712,
  "spent_by": null
}
```
</details>

#### `GET /utxos`

Get all unspent wallet outputs.

<details><summary>Expand...</summary><p></p>

Query string parameters:
- `min_conf` - minimum number of confirmations, defaults to 0
- `include_unsafe` - whether to include outputs that are not safe to spend (unconfirmed from outside keys or with RBF), defaults to true

Example:
```
$ curl localhost:3060/utxos?min_conf=1
[
  {
    "txid": "1973551cc7670237606561ba3f7579d46d38e7145a72cf6a55ff8975e7143fee",
    "vout": 0,
    "amount": 99791,
    ...
  },
  ...
]
```
</details>

> Also see: [`GET /address/:address/utxos`](#get-addressaddressutxos)


### Blocks

#### `GET /block/tip`

Get the current tip of the block chain.

<details><summary>Expand...</summary><p></p>

Returned fields:
- `height`
- `hash`

Example:
```
$ curl localhost:3060/block/tip
{
  "height": 176,
  "hash": "7a9b99f78066f22a26c56b2035445285a5a992fc19719c9c27f2255f20f1f2f8"
}
```

</details>

#### `GET /block/:hash`

Get the block header of the specified block hash as formatted by [bitcoind's `getblockheader`](https://bitcoincore.org/en/doc/0.19.0/rpc/blockchain/getblockheader/) with `verbose=true`.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/block/65e8db69c0c03a2a02532dfca4d2780a555012847985efee46eb9bf3e459262d
{
  "hash": "65e8db69c0c03a2a02532dfca4d2780a555012847985efee46eb9bf3e459262d",
  "confirmations": 2,
  "height": 175,
  "version": 805306369,
  "versionHex": "30000001",
  "merkleroot": "b7a646abfd377964da19837c454e9d2d30c61b9bc22246c8589f7e80fda1a3e5",
  "time": 1589360866,
  "mediantime": 1589269430,
  "nonce": 0,
  "bits": "207fffff",
  "difficulty": 4.6565423739069247e-10,
  "chainwork": "0000000000000000000000000000000000000000000000000000000000000160",
  "nTx": 1,
  "previousblockhash": "26d435bdea859667e9d396ad0f28b74e6fd98ca6c737f1bd895c3b6539b4ee76",
  "nextblockhash": "7a9b99f78066f22a26c56b2035445285a5a992fc19719c9c27f2255f20f1f2f8"
}
```
</details>

#### `GET /block/:hash/hex`

Get the block header of the specified block hash as a hex string.

<details><summary>Expand...</summary><p></p>

Example:
```
$ curl localhost:3060/block/65e8db69c0c03a2a02532dfca4d2780a555012847985efee46eb9bf3e459262d/hex

0100003076eeb439653b5c89bdf137c7a68cd96f4eb7280fad96d3e9679685eabd35d426e5a3a1fd807e9f58c84622c29b1bc6302d9d4e457c8319da647937fdab46a6b7e2b8bb5effff7f2000000000
```

</details>

#### `GET /block/:height`

Get the block hash at the specified block height.

<details><summary>Expand...</summary><p></p>

Issues a 307 redirection to the block url (`/block/:hash`) *and* responds with the block hash in the responses body.

Example:
```
$ curl localhost:3060/block/104
< HTTP/1.1 307 Temporary Redirect
< Location: /block/117324e95584f14ba767610f4ef9c939004b02c9f3881a94f46c0772d8e9b365
117324e95584f14ba767610f4ef9c939004b02c9f3881a94f46c0772d8e9b365

# Follow the redirect to get the block header json

$ curl --location localhost:3060/block/104
{
  "hash": "117324e95584f14ba767610f4ef9c939004b02c9f3881a94f46c0772d8e9b365",
  "confirmations": 73,
  "height": 104,
  ...
}
```
</details>

### Mempool & Fees

#### `GET /mempool/histogram`

Get the mempool feerate distribution histogram.

<details><summary>Expand...</summary><p></p>

Returns an array of `(feerate, vsize)` tuples, where each entry's `vsize` is the total vsize of transactions
paying more than `feerate` but less than the previous entry's `feerate` (except for the first entry, which has no upper bound).
This matches the format used by the Electrum RPC protocol for `mempool.get_fee_histogram`.

Cached for 2 minutes.

Example:

```
$ curl localhost:3060/mempool/histogram

[[53.01, 102131], [38.56, 110990], [34.12, 138976], [24.34, 112619], [3.16, 246346], [2.92, 239701], [1.1, 775272]]
```

> In this example, there are transactions weighting a total of 102,131 vbytes that are paying more than 53 sat/vB,
110,990 vbytes of transactions paying between 38 and 53 sat/vB, 138,976 vbytes paying between 34 and 38, etc.

</details>

#### `GET /fee-estimate/:target`

Get the feerate estimate for confirming within `target` blocks.
Uses bitcoind's `smartestimatefee`.

<details><summary>Expand...</summary><p></p>

Returned in `sat/vB`, or `null` if no estimate is available.

Cached for 2 minutes.

Example:
```
$ curl localhost:3060/fee-estimate/3

5.61
```

</details>


### Server-Sent Events

#### Event categories

- `ChainTip(block_height, block_hash)` - emitted whenever a new block extends the best chain.
- `Reorg(block_height, prev_block_hash, curr_block_hash)` - indicates that a re-org was detected on `block_height`, with the previous block hash at this height and the current one.
- `Transaction(txid, block_height)` - emitted for new transactions as well as transactions changing their confirmation status (typically from unconfirmed to confirmed, possibly the other way around in case of reorgs).
- `TransactionReplaced(txid)` - indicates that the transaction conflicts with another transaction and can no longer be confirmed (aka double-spent).
- `TxoFunded(funding_txid:vout, scripthash, amount, block_height)` - emitted when an unspent wallet output is created (for new transactions as well as confirmation status changes).
- `TxoSpent(spending_txid:vin, scripthash, prevout, block_height)` - emitted when a wallet output is spent (for new transactions as well as confirmation status changes).

For unconfirmed transactions, `block_height` will be `null`.

#### `GET /stream`

Subscribe to a real-time [Server-Sent Events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events) stream of indexer update notifications.

<details><summary>Expand...</summary><p></p>

Query string parameters for filtering the event stream:
- `category`
- `scripthash`
- `outpoint`

Examples:
```bash
$ curl localhost:3060/stream
< HTTP/1.1 200 OK
< content-type: text/event-stream

data:{"category":"ChainTip","params":[114,"0a1a199aed012b280b36370e393867e03b46eb39b7130bb017a6757b6d4014ec"]}

data:{"category":"Transaction","params":["ac42d918b45351835bf9448bbd0c2f8e9ddad56a8bd118fe93919cc74bd0c487",114]}

data:{"category":"TxoFunded","params":["ac42d918b45351835bf9448bbd0c2f8e9ddad56a8bd118fe93919cc74bd0c487:0","db576ad85b0f09680dfe3f3f7160be50c1a36db8b4949ffe21fe5b4564c1d42b",10000000,114]}

data:{"category":"TxoFunded","params":["ac42d918b45351835bf9448bbd0c2f8e9ddad56a8bd118fe93919cc74bd0c487:1","48138c88b8cb17544ac2450c4bd147106a9f773d6cf2b7f31a5a9dde75a8387a",399999856,114]}

data:{"category":"TxoSpent","params":["ac42d918b45351835bf9448bbd0c2f8e9ddad56a8bd118fe93919cc74bd0c487:0","5f26eb39e19b0bef205bb451082f941cef0707d38949d3ffe51f5614fab70f5d","aa5b889f6cf1c314bc02c5187f31d0d5ff56f568c85a384027cb155fdc377069:1",114]}
```

```
$ curl localhost:3060/stream?category=ChainTip

data:{"category":"ChainTip","params":[114,"0a1a199aed012b280b36370e393867e03b46eb39b7130bb017a6757b6d4014ec"]}

data:{"category":"ChainTip","params":[115,"1c293df0c95d94a345e7578868ee679c9f73b905ac74da51e692af18e0425387"]}
```

```
$ curl localhost:3060/stream?outpoint=aa5b889f6cf1c314bc02c5187f31d0d5ff56f568c85a384027cb155fdc377069:1

data:{"category":"TxoFunded","params":["43916225aeadc3d6f17ffd5cdcc72fe81508eab4de66532507bc032b50c89732:0","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd",99900000,null]}

data:{"category":"TxoSpent","params":["0ac67648be03f7fd547a828b78b920cb73f8c883320f30d770fb14d59655b125:0","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd","43916225aeadc3d6f17ffd5cdcc72fe81508eab4de66532507bc032b50c89732:0",null]}
```

</details>

#### `GET /address/:address/stream`
#### `GET /scripthash/:scripthash/stream`
#### `GET /wallet/:checksum/:index/stream`

Subscribe to a real-time notification stream of `TxoFunded`/`TxoSpent` events for the provided address, scripthash or descriptor index.

<details><summary>Expand...</summary><p></p>

This is equivalent to `GET /stream?scripthash=<scripthash>`.

Example:
```
$ curl localhost:3060/address/bcrt1qxs3mrrre37rphadyg4wu0zk4t33qklv0u0gmps/stream

data:{"category":"TxoFunded","params":["bb94b1547397cd89441edd74d0581913d8bb3005d070fa6f9744af44f654c25a:0","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd",77700000,115]}

data:{"category":"TxoFunded","params":["a0fe8a8fc855a9deaed533cf5f2053c77d640ff5f50a7c44d1cca314d4e00e5d:0","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd",100000,116]}

data:{"category":"TxoSpent","params":["a3bc61a974b113223c336c866bc656cd23481d1466e063e46930a5983e70c20d:0","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd","a0fe8a8fc855a9deaed533cf5f2053c77d640ff5f50a7c44d1cca314d4e00e5d:0",117]}

data:{"category":"TxoSpent","params":["a3bc61a974b113223c336c866bc656cd23481d1466e063e46930a5983e70c20d:1","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd","bb94b1547397cd89441edd74d0581913d8bb3005d070fa6f9744af44f654c25a:0",117]}
```
</details>

#### Catching up with missed events & re-org detection

To catch-up with historical events that your app missed while being down, you can specify the `synced-tip` query string parameter with the `<block-height>:<block-hash>` of the latest block known to be processed.

If the `synced-tip` is still part of the best chain, this will return all historical  `Transaction`, `TxoFunded` and `TxoSpent` events that occurred after `block-height` (exclusive, ordered with oldest first, unconfirmed included at the end), followed by a *single* `ChainTip` event with the currently synced tip, followed by a stream of real-time events.

If the `synced-tip` is no longer part of the best chain, a `410 Gone` error will be returned indicating that a reorg took place.
One way to recover from reorgs is to re-sync since `N` blocks before the orphaned `synced-tip` and consider any entries that
no longer show up as double-spent (where `N` is large enough such that reorgs deeper than it are unlikely).

You can specify `synced-tip` with just the height to skip reorg detection (for example, `0` to get all events since the genesis block).

<details><summary>Expand...</summary><p></p>
Example:

```
# Start by syncing everything from the beginning
$ curl localhost:3060/stream?synced-tip=0
data:{"category":"TxoFunded","params":["ac42d918b45351835bf9448bbd0c2f8e9ddad56a8bd118fe93919cc74bd0c487:1","48138c88b8cb17544ac2450c4bd147106a9f773d6cf2b7f31a5a9dde75a8387a",399999856,114]}
data:{"category":"ChainTip","params":[120,"5cc1fb1153f8eb12d445d0db06e96bbb39c45b8ed22d4f0de718aa6b0ef00cd1"]}

# Oops, we got disconnected! Let's try again with the last `ChainTip` we heard of
$ curl localhost:3060/stream?synced-tip=120:5cc1fb1153f8eb12d445d0db06e96bbb39c45b8ed22d4f0de718aa6b0ef00cd1
data:{"category":"TxoSpent","params":["a3bc61a974b113223c336c866bc656cd23481d1466e063e46930a5983e70c20d:1","97e9cc06a9a9d95a7ff26a9e5fdf9e1836792a3337c0ff718c88e012feb217bd","bb94b1547397cd89441edd74d0581913d8bb3005d070fa6f9744af44f654c25a:0",122]}
data:{"category":"ChainTip","params":[130,"57d17db78d5017c89e86e863a7397c02027f09327222feb72cdfe8372644c589"]}

# Disconnected again, this time while a reorg happened
$ curl localhost:3060/stream?synced-tip=130:57d17db78d5017c89e86e863a7397c02027f09327222feb72cdfe8372644c589
< HTTP/1.1 410 Gone
Reorg detected at height 130 (previous=57d17db78d5017c89e86e863a7397c02027f09327222feb72cdfe8372644c589 current=43b482862ba3fc883187f534be1971186b11c534494129397e8a2b4faf4bf2f4)

# Re-sync events from height 110 (N=20 blocks before the reported reorg)
$ curl localhost:3060/stream?synced-tip=110
```

The `synced-tip` functionality also supports the SSE `Last-Event-ID` header. This makes it work transparently with the [built-in automatic reconnection mechanism](https://kaazing.com/kaazing.io/doc/understanding-server-sent-events/#last-event-id). You'll still need to manually persist and specify the `synced-tip` in case your app restarts.

</details>

### Miscellaneous

#### `POST /sync`

Trigger an indexer sync. See [Real-time updates](#real-time-updates).

#### `GET /dump`

Dumps the contents of the index store as JSON.

#### `GET /debug`

Dumps the contents of the index store as a debug string.

#### `GET /banner.txt`

Get the welcome banner text.
(see [live example here](https://bwt.dev/banner.txt), or a [static one here](https://gist.githubusercontent.com/shesek/4986c4291df1a7c6de62c20bc72e58bf/raw/42539cd10f1836ae511f4c2ec7b4fc82ad52252a/bwt-welcome-banner.txt))

## Web Hooks

You can set `--webhook-url <url>` to have bwt send push notifications as a `POST` request to the provided `<url>`. Requests will be sent with a JSON-serialized *array* of one or more index updates as the body.

It is recommended to include a secret key within the URL to verify the authenticity of the request.

You can specify multiple `--webhook-url` to notify all of them.

Note that bwt currently attempts to send the webhook request once and does not retry in case of failures.
It is recommended to occasionally catch up using the [`GET /txs/since/:block-height`](#get-txssinceblock-height) or
[`GET /stream`](#get-stream) endpoints (see ["Catching up with missed events"](#catching-up-with-missed-events--re-org-detection)).

Tip: services like [webhook.site](https://webhook.site/) or [requestbin](http://requestbin.net/) can come in handy for debugging webhooks. (needless to say, for non-privacy-sensitive regtest/testnet use only)

> If you're building bwt from source, you'll need to set `--features webhooks` to enable web hooks support. This will also require to `apt install libssl-dev pkg-config`.
>
> The pre-built binaries ([except for ARM](https://github.com/bwt-dev/bwt/issues/52)) and the `shesek/bwt` docker image come with webhooks support enabled by default.

## Developing

### Developer Resources

To integrate bwt into non-Rust software, check out [libbwt](https://github.com/bwt-dev/libbwt) and [libbwt-jni](https://github.com/bwt-dev/libbwt-jni).

Documentation for the public Rust API is [available on docs.rs](https://docs.rs/bwt).

A yuml diagram showing how the big pieces interact together is [available here](https://yuml.me/edit/39229813).

An example of initializing bwt and issuing queries against its db from Rust is available at [`examples/use-from-rust.rs`](examples/use-from-rust.rs).
(Note that the Rust API provides weaker backwards compatibility guarantees compared to the HTTP API.)

An example JavaScript client utilizing the HTTP API for wallet tracking
is available at [`examples/wallet-tracker.js`](https://github.com/bwt-dev/bwt/blob/master/examples/wallet-tracker.js).


### Development environment

To quickly setup a development environment, you can use [`scripts/dev-env.sh`](https://github.com/bwt-dev/bwt/blob/master/scripts/dev-env.sh) to create a bitcoind regtest network and two Electrum wallets, fund the wallets, start bwt with tracking for both wallets' xpubs, and start the Electrum GUI.

To use it, simply run `$ ./scripts/dev-env.sh` from the root directory with `bitcoind`, `bitcoin-cli` and `electrum` installed in your `PATH`.

You can set `FEATURES` to specify which features to enable (see below) or set `NO_GUI=1` to leave the Electrum wallet running in daemon mode without starting the GUI.

If you have [`cargo watch`](https://github.com/passcod/cargo-watch) installed, it'll be used to watch for changes and automatically restart bwt.

### Features

bwt has 7 optional features: `cli`, `http`, `electrum`, `webhooks`, `track-spends`, `ffi` and `extra`.

All are enabled by default except for `webhooks` and `ffi`.

If you're working on code that is unrelated to the HTTP API, it is faster to build without the `http` feature.

You can use `scripts/check.sh` to run `cargo check` for some feature combos. This is important to ensure no errors were introduced for feature combos that you didn't use.

### Tests

End-to-end integration tests can be run with [`./test/tests.sh`](https://github.com/bwt-dev/bwt/blob/master/test/tests.sh).
The tests deploy a regtest network, a bwt instance and an Electrum wallet connected to it (in headless mode), then run some basic tests using the Electrum client and against the HTTP REST API.

Run with `bash -x test/tests.sh -v` to get more verbose output.

### Contributions

Are welcome!

The only guideline is to use `cargo fmt`.

You can check out [the list of enhancement issues](https://github.com/bwt-dev/bwt/issues?q=is%3Aopen+is%3Aissue+label%3Aenhancement).

## Reproducible builds

The builds can be reproduced in a Docker container environment as follows:

```
$ git clone https://github.com/bwt-dev/bwt && cd bwt
$ git checkout <tag>
$ git verify-commit HEAD

# Linux, Windows & ARMv7/v8
$ docker build -t bwt-builder - < scripts/builder.Dockerfile
$ docker run -it -u `id -u` -v `pwd`:/usr/src/bwt bwt-builder

# Mac OSX (cross-compiled via osxcross)
$ docker build -t bwt-builder-osx - < scripts/builder-osx.Dockerfile
$ docker run -it -u `id -u` -v `pwd`:/usr/src/bwt bwt-builder-osx

$ sha256sum dist/*
```

The builds are [reproduced on Travis CI](https://travis-ci.org/github/bwt-dev/bwt/branches) using the code from GitHub.
The SHA256 checksums are available under the "Reproducible builds" stage.

## Thanks

- [@romanz](https://github.com/romanz)'s [electrs](https://github.com/romanz/electrs) for the fantastic electrum server implementation that bwt is based on.

- [@chris-belcher](https://github.com/chris-belcher)'s [electrum-personal-server](https://github.com/chris-belcher/electrum-personal-server) for inspiring this project and the personal tracker model.

- [rust-bitcoin](https://github.com/rust-bitcoin), [rust-bitcoincore-rpc](https://github.com/rust-bitcoin/rust-bitcoincore-rpc) and the other incredible modules from the rust-bitcoin family.

## License

MIT
