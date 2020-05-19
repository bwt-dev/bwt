#!/bin/bash
set -eo pipefail

NO_FUNDING=1
NO_WATCH=1
INTERVAL=1

# Start regtest, electrum wallet and bwt
source scripts/setup-env.sh

# Send some funds
addr=`ele1 createnewaddress`
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
  hist=`ele1 history`
  test `jq -r '.transactions | length' <<< "$hist"` == 2
  test `jq -r .transactions[0].confirmations <<< "$hist"` == 1
  test `jq -r .transactions[1].confirmations <<< "$hist"` == 0
  # end_balance and value used to have an "BTC" suffix in electrum prior to 3.3.8, use regex to support both cases
  [[ "`jq -r .summary.end_balance <<< "$hist"`" =~ ^6.912 ]]
  [[ "`jq -r .transactions[0].value <<< "$hist"`" =~ ^1.234 ]]

  echo - Testing listunspent
  utxos=`ele1 listunspent`
  test `jq -r length <<< "$utxos"` == 2
  test `jq -r .[0].address <<< "$utxos"` == $addr
  test `jq -r '.[] | select(.height != 0) | .value' <<< "$utxos"` == 1.234
fi

# Test HTTP API
if [[ $FEATURES == *"http"* ]]; then

  get() { curl -s "http://$BWT_HTTP_ADDR$1"; }

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
  [[ `jq -r .origin <<< "$(get /address/$addr)"` =~ /20$ ]]
  # we used `createnewaddress` with 20 prior addresses, which should've gave us the 21st key

  echo - Testing /address/:address/stats
  test `jq -r .confirmed_balance <<< "$(get /address/$addr/stats)"` == 123400000

  echo - Testing /address/:address/txs
  test `jq -r .[0].funding[0].address <<< "$(get /address/$addr/txs)"` == $addr

  echo - Testing /address/:address/utxos
  test `jq -r '.[] | select(.block_height == null) | .amount' <<< "$(get /address/$addr/utxos)"` == 567800000
fi

echo All tests pass!
