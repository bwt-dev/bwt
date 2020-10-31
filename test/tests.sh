#!/bin/bash
set -eo pipefail

# Run unit tests
cargo test

# Run integration tests

NO_FUNDING=1
NO_WATCH=1
INTERVAL=1
PRINT_LOGS=1

# Start regtest, electrum wallet and bwt
source scripts/setup-env.sh

# Send some funds
addr=`ele1 getunusedaddress`
btc sendtoaddress $addr 1.234 > /dev/null
btc generatetoaddress 1 `btc getnewaddress` > /dev/null
btc sendtoaddress $addr 5.678 > /dev/null

sleep 1

# Test Electrum RPC
if [[ $FEATURES == *"electrum"* ]]; then
  echo = Running Electrum tests =

  echo - Testing getbalance
  balance=`ele1 getbalance`
  test `jq -r .confirmed <<< "$balance"` == 1.234
  test `jq -r .unconfirmed <<< "$balance"` == 5.678

  echo - Testing history
  hist=`ele1 onchain_history`
  test `jq -r '.transactions | length' <<< "$hist"` == 2
  test `jq -r .transactions[0].confirmations <<< "$hist"` == 1
  test `jq -r .transactions[1].confirmations <<< "$hist"` == 0
  test `jq -r .summary.end_balance <<< "$hist"` == 6.912
  test `jq -r .transactions[0].bc_value <<< "$hist" | cut -d' ' -f1` == 1.234

  echo - Testing listunspent
  utxos=`ele1 listunspent`
  test `jq -r length <<< "$utxos"` == 2
  test `jq -r .[0].address <<< "$utxos"` == $addr
  test `jq -r '.[] | select(.height != 0) | .value' <<< "$utxos"` == 1.234
fi

# Test HTTP API
if [[ $FEATURES == *"http"* ]]; then

  get() { curl -s "${@:2}" "http://$BWT_HTTP_ADDR$1"; }
  get_jq() { jq -r "$1" <(get "${@:2}"); }

  echo = Running HTTP tests =
  echo - Testing /txs/since/:height
  txs=`get /txs/since/0`
  test `jq -r length <<< "$txs"` == 2
  test `jq -r .[0].funding[0].address <<< "$txs"` == $addr
  test `jq -r .[0].funding[0].amount <<< "$txs"` == 123400000
  test `jq -r .[1].funding[0].amount <<< "$txs"` == 567800000

  echo - Testing /tx/:txid
  txid=`jq -r .[0].txid <<< "$txs"`
  tx=`get /tx/$txid`
  test `jq -r .balance_change <<< "$tx"` == 123400000
  test `jq -r .txid <<< "$tx"` == $txid

  echo - Testing /address/:address
  test `get_jq .origin /address/$addr | cut -d/ -f2` == 0

  echo - Testing /address/:address/stats
  test `get_jq .confirmed_balance /address/$addr/stats` == 123400000

  echo - Testing /address/:address/txs
  test `get_jq .[0].funding[0].address /address/$addr/txs` == $addr

  echo - Testing /address/:address/utxos
  test `get_jq '.[] | select(.block_height == null) | .amount' /address/$addr/utxos` == 567800000

  echo - Testing /stream
  btc sendtoaddress $addr 9.777 &
  # collect events for 1 second
  while read evt; do
    category=`jq -r .category <<< "$evt"`
    declare "evt_$category=`jq -c .params <<< "$evt"`"
  done < <(get /stream --max-time 1 | grep '^data:' | cut -d: -f2-)
  # and check we got the expected ones
  test -n "$evt_Transaction" -a -n "$evt_TxoFunded"
  txid=`jq -r .[0] <<< "$evt_Transaction"`
  test `get_jq .funding[0].amount /tx/$txid` == 977700000
  test `jq -r .[0] <<< "$evt_TxoFunded" | cut -d: -f1` == $txid
fi

echo -e "\e[32mAll tests pass.\e[0m"
