# Changelog

## Unreleased

- Allow setting `UNIX_LISTENER_MODE` to control permissions for the unix socket notification listener.

- Allow setting `NO_REQUIRE_ADDRESSES` as an env variable

Breaking CLI changes:

- The Electrum SOCKS5-based authentication needs to be explicitly enabled with `--electrum-socks-auth`,
  in addition to enabling the `--auth-*` options. By default, authentication will only be enabled for
  the HTTP API server.

## 0.2.2 - 2021-01-29

- [Authentication support](doc/auth.md) for the Electrum and HTTP API servers (#70)

- Compatibility with Bitcoin Core v0.21

- Support for Signet

- New `GET /bitcoin.pdf` HTTP API endpoint for extracting the Bitcoin whitepaper from the block chain
  (see https://twitter.com/shesek/status/1352368296553836544)

- New `--create-wallet-if-missing` option to ease the creation of a designated bitcoind wallet (#76)

- Docker: Multi-arch images for amd64, arm32v7 and arm64v8 (#79)

- Indexer: Fix detection of conflicted mempool transactions

- Support setting boolean options using environment variables
  (`FORCE_RESCAN`, `CREATE_WALLET_IF_MISSING`, `ELECTRUM_SKIP_MERKLE`, `NO_STARTUP_BANNER` and `VERBOSE`)

- Accept wildcard envirnoment variables for options that accept multiple values
  (`XPUB_*`, `DESC_*`/`DESCRIPTOR_*` and `ADDRESS_*`)

- Upgrade to rust-bitcoin v0.26.0, rust-miniscript v5.0.1 and bitcoincore-rpc v0.13.0

## 0.2.1 - 2021-01-14

- Migrated to the [@bwt-dev](https://github.com/bwt-dev) github org and split up into:

  [bwt](https://github.com/bwt-dev/bwt), [bwt-electrum-plugin](https://github.com/bwt-dev/bwt-electrum-plugin), [libbwt](https://github.com/bwt-dev/libbwt), [libbwt-jni](https://github.com/bwt-dev/libbwt-jni) and [libbwt-nodejs](https://github.com/bwt-dev/libbwt-nodejs).

- Java Native Bindings for `libbwt` ([libbwt-jni](https://github.com/bwt-dev/libbwt-jni), #73)

- Support for tracking standalone addresses (#14)

  Using `--address <address>` or `--address-file <path>`.

- New config options: `force_rescan` (9e7ccbe), `setup_logger` (35fc49f) and `require_addresses` (162790d)

- Gracefully wait for bitcoind to warm-up (dec6d46)

- Support for `android_logger` (74b2b2f)

- Scrub bitcoin authentication from logs (c31def7)

- Improved syncing/scanning progress updates (faba3f, 6e282fd, fdd46f3, 5ba2a0b)

- Indexer: Fix excessive importing/rescanning (a20ae79)

- Indexer: Fix cache invalidation for spends lookups (360eaee)

- Indexer: Fix handling of missing mempool entries (e9b7511)

- Electrum: Fix TCP listener not shutting down on shutdown signal (5bd639a)

- Docker/CI: Update to Rust v1.49 (5bd639a)

Breaking CLI changes:

- The `--bare-xpub` option was removed. Use a descriptor instead.

## 0.2.0 - 2020-11-24

- Descriptor based tracking! ✨🎉 (#1)

  You can now specify output script descriptors to track via `--descriptor <descriptor>`.
  Descriptors are also used internally to represent user-provided `--xpub`s.

  The HTTP API was updated to be fully descriptor-based. Wallets and wallet origins
  are now identified by the descriptor checksum, addresses have descriptors associated with them,
  and a new `bip32_origins` field is available based on the descriptor origin information.

- Support for Electrum multi-signature wallets (#60)

  For a manual server setup, this requires using the `sortedmulti()` descriptor.
  For example, for a 2-of-3 wallet: `sortedmulti(2,xpub1...,xpub2...,xpub3...)`.

  With the Electrum plugin, this should Just Work™.

- Alpha release of [`libbwt`](https://github.com/shesek/bwt/blob/master/doc/libbwt.md) (#64), a C FFI interface for managing the bwt servers,
  and of [`nodejs-bwt-daemon`](https://github.com/shesek/bwt/tree/master/contrib/nodejs-bwt-daemon) (#65), a nodejs package that wraps it.

- Support non-wallet transactions in `blockchain.transaction.get` / `GET /tx/:txid/hex`
  (requires txindex and no pruning)

- Emit wallet rescan and blockchain sync progress updates (via mpsc, [ffi](#64) and the console)

- Support binding on ephemeral port (e.g. `--http-addr 127.0.0.1:0`) (#63)

- Reduce the number of dependencies (#61)

- Shutdown cleanly, via `SIGINT`/`SIGTERM` for CLI or a custom signal for library users (#62, #66)

- HTTP: Alias `GET /txs/since/0` as `GET /txs`

- Fix `blockchain.scripthash.listunspent` / `Query::list_unspent` to return an empty set
  instead of erroring when there's no history.

- Electrum: Fix `mempool.get_fee_histogram` (5af7bfc62d7d98)

- Upgrade to rust-bitcoin v0.25, rust-miniscript v4.0.0 and rust-bitcoincore-rpc v0.12

Breaking CLI changes:

- The `-d` CLI option was changed to mean `--descriptor` instead of `--bitcoind-dir`
   (which is now available as `-r`).

- Renamed `--http-server-addr` to `--http-addr` and `--electrum-rpc-addr` to `--electrum-addr`

- The CLI now accepts a single `--rescan-since` timestamp instead of a separate one for each descriptor/xpub.

- The separator for environment variables with multiple values is now `;` instead of `,`.
  For example: `DESCRIPTORS="wpkh(xpub../0/*);wpkh(xpub../1/*)"`

## 0.1.5 - 2020-10-05

- Reproducible builds using Docker (#51)

- Pre-built binary releases for macOS (#24) and ARMv7/v8 (#19)

- Electrum plugin: Compatibility with Electrum v4 — *except for lightning* which is
  [tricky with personal servers](https://github.com/chris-belcher/electrum-personal-server/issues/174#issuecomment-577619460) (#53)

- Electrum: New welcome banner (#44)

- Scriptable transaction broadcast command via `--tx-broadcast-cmd <cmd>` (#7)

  The command will be used in place of broadcasting transactions using the full node,
  which may provide better privacy in some circumstances.
  The string `{tx_hex}` will be replaced with the hex-encoded transaction.

  For example, to broadcast transactions over Tor using the blockstream.info onion service, you can use:

  ```
  --tx-broadcast-cmd '[ $(curl -s -x socks5h://localhost:9050 http://explorerzydxu5ecjrkwceayqybizmpjjznk5izmitf2modhcusuqlid.onion/api/tx -d {tx_hex} -o /dev/stderr -w "%{http_code}" -H "User-Agent: curl/7.$(shuf -n1 -e 47 58 64 68).0") -eq 200 ]'
  ```

  (Replace port `9050` with `9150` if you're using the Tor browser bundle.)

  h/t @chris-belcher's EPS for inspiring this feature! 🎩

- Load bitcoind wallet automatically (#54)

- Electrum plugin: Fix hot wallet test (#47)

- Electrum: Fix docker image libssl dependency with the `http` feature (#48)

- Improve block download check on regtest (#45, #35)

- HTTP API: Fix `GET /block/tip` (#46)

- HTTP API: Add `GET /banner.txt` (#44)

- Tests: Upgrade to Electrum v4

## 0.1.4 - 2020-06-22

- Implement improved mempool tracking, including support for an "effective feerate" metric that takes unconfirmed ancestors into account
  (calculated as `MIN(own_fee/own_vsize, (own_fee+ancestor_fee)/(own_vsize+ancestor_vsize))`).

  HTTP API: the [wallet transaction format](https://github.com/shesek/bwt#wallet-transaction-format) now includes
  new `own_feerate`, `effective_feerate`, `bip125_replaceable` and `unconfirmed_parents` fields available for unconfirmed transactions.

  Electrum server: provide fee information for unconfirmed transactions using the effective feerate metric.
  This is unlike other Electrum server implementations, that report the direct own fee without regard to ancestors. (#10)

- Electrum server: Implement `--electrum-skip-merkle` to save some resources by not generating SPV proofs entirely, even when it's possible. (#34)

- Electrum plugin: Automatically enable `--skipmerklecheck` and `--electrum-skip-merkle`, for better out-of-the-box pruning support and to save some resources. (#34)

- Indexer: Use `listsinceblock` instead of `listtransactions`. This makes syncing more bandwidth-efficient and simplifies the implementation. (#33)

- Electrum server: Optimize dispatching notifications to subscribers.

- Electrum server: Use height of -1 to indicate that a transaction has unconfirmed parents as its inputs. (#40)

- Electrum plugin: Disable support for hot wallets.

## 0.1.3 - 2020-06-02

- Electrum: Use dummy SPV proofs to support pruning with the  `--skipmerklecheck` option.

## 0.1.2 - 2020-05-30

- Electrum plugin: restore the previous `oneserver` setting when the plugin is disabled,
  to prevent users from inadvertently connecting to public Electrum servers with this setting still on.

- Electrum plugin: allow specifying additional custom CLI arguments using the GUI

- Electrum plugin: check for permissions before attempting the bind the real-time sync unix socket.

- Make builds over 40% smaller by stripping symbols, which rust apparently doesn't do for release builds.
  Thanks @elichai for brining this to my attention.

## 0.1.1 - 2020-05-27

- Make bwt available as an Electrum plugin! 💥

- HTTP: Implement the `synced-tip` option to catch up with missed events (#6)

- Unite the `History` event into `Txo{Funded,Spent}`

- Fix: Update the confirmation status of send-only (no change) transactions

## 0.1.0 - 2020-05-20

First release!
