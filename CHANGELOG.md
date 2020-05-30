# Changelog

## Unreleased

- Electrum plugin: Restore the previous `oneserver` setting when the plugin is disabled,
  to prevent users from inadvertently connecting to public Electrum servers with this setting still on.

- Electrum plugin: allow specifying additional custom CLI arguments using the GUI

## 0.1.1 - 2020-05-27

- Make bwt available as an Electrum plugin! 💥

- HTTP: Implement the `synced-tip` option to catch up with missed events (#6)

- Unite the `History` event into `Txo{Funded,Spent}`

- Fix: Update the confirmation status of send-only (no change) transactions

## 0.1.0 - 2020-05-20

First release!
