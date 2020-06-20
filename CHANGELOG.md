# Changelog

## Unreleased

- Implement mempool tracking, including support for an "effective feerate" metric that takes unconfirmed ancestors into account
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
