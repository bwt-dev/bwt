# Changelog

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
