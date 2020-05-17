#!/bin/bash
set -eo pipefail
shopt -s expand_aliases

PATH="./target/debug:./target/release:$PATH"

(command -v bwt && command -v electrum && command -v bitcoind && command -v bitcoin-cli) > /dev/null \
  || { echo >&2 "bwt, bitcoind, bitcoin-cli and electrum must be installed in PATH"; exit 1; }

: ${DIR:=`mktemp -d --suffix -bwt-dev-env`}
BTC_DIR=$DIR/bitcoin
BTC_RPC_PORT=30601
ELECTRUM_DIR=$DIR/electrum
WALLET1=$ELECTRUM_DIR/wallet1
WALLET2=$ELECTRUM_DIR/wallet2
BWT_HTTP_ADDR=127.0.0.1:3060
BWT_ELECTRUM_ADDR=127.0.0.1:30602
BWT_SOCKET=$DIR/bwt-socket

alias btc="bitcoin-cli -datadir=$BTC_DIR -rpcwallet=internal"
alias ele="electrum --regtest --dir $ELECTRUM_DIR"
alias ele1="ele --wallet $WALLET1"
alias ele2="ele --wallet $WALLET2"

trap 'trap - SIGTERM SIGINT; jobs -p | head -n -1 | xargs --no-run-if-empty kill; sleep 2; rm -rf $DIR; kill -- -$$' SIGINT SIGTERM EXIT

echo Setting up test envirnoment in $DIR

echo Setting up bitcoind...
mkdir -p $BTC_DIR

cat >$BTC_DIR/bitcoin.conf <<EOL
regtest=1
printtoconsole=0
nolisten=1

blocknotify=nc -U $BWT_SOCKET > /dev/null 2>&1
walletnotify=nc -U $BWT_SOCKET > /dev/null 2>&1

[regtest]
rpcport=$BTC_RPC_PORT
wallet=internal
EOL

bitcoind -datadir=$BTC_DIR &

echo - Waiting for bitcoind to warm up...
if command -v inotifywait > /dev/null; then
  sed --quiet '/^\.cookie$/ q' <(inotifywait -e create,moved_to --format '%f' -qmr $BTC_DIR)
else
  sleep 2
fi
btc -rpcwait getblockchaininfo > /dev/null
echo - Creating watch-only wallet...
btc createwallet bwt true > /dev/null

echo - Generating some blocks...
btc generatetoaddress 110 `btc getnewaddress` > /dev/null

echo Setting up electrum
mkdir -p $ELECTRUM_DIR

echo - Creating 2 wallets...
electrum create --regtest --segwit --wallet $WALLET1 > /dev/null
electrum create --regtest --wallet $WALLET2 > /dev/null

echo - Starting daemon and loading wallets...
ele daemon --server $BWT_ELECTRUM_ADDR --oneserver start > /dev/null 2>&1
ele daemon load_wallet --wallet $WALLET1 > /dev/null
ele daemon load_wallet --wallet $WALLET2 > /dev/null

echo - Sending some funds...
for i in `seq 1 3`; do
  for n in `seq 5 9`; do
    btc sendtoaddress `ele1 createnewaddress` $n.$i > /dev/null
    btc sendtoaddress `ele2 createnewaddress` $i$n > /dev/null
  done
  # leave the last round as unconfirmed
  [ $i != 3 ] && btc generatetoaddress $i `btc getnewaddress` > /dev/null
done

echo Setting up bwt >&2
bwt --network regtest --bitcoind-dir $BTC_DIR --bitcoind-url http://localhost:$BTC_RPC_PORT/ \
  --bitcoind-wallet bwt \
  --electrum-rpc-addr $BWT_ELECTRUM_ADDR \
  --unix-listener-path $BWT_SOCKET \
  --xpub `ele1 getmpk` --xpub `ele2 getmpk` \
  -v "$@" &> $DIR/bwt.log &

echo - Waiting for bwt to index...
sed --quiet '/Electrum RPC server running/ q' <(tail -F -n+0 $DIR/bwt.log 2> /dev/null)

cat <<EOL

bwt is running:
- HTTP API server on http://$BWT_HTTP_ADDR
- Electrum RPC server on $BWT_ELECTRUM_ADDR
- Logs at $DIR/bwt.log

You can access bitcoind with:
$ bitcoin-cli -datadir=$BTC_DIR -rpcwallet=internal <cmd>
$ bitcoin-cli -datadir=$BTC_DIR -rpcwallet=bwt <cmd>

EOL

if [ -z "$NO_GUI" ]; then
  echo Starting Electrum GUI...
  ele daemon stop > /dev/null 2>&1

  # disable "Would you like to be notified when there is a newer version of Electrum available?" popup
  # and enable some advanced features
  sed -i 's/^{/{"check_updates":false,"show_addresses_tab":true,"show_utxo_tab":true,"show_console_tab":true,"show_fee":true,/' \
    $ELECTRUM_DIR/regtest/config

  ele1 --oneserver --server $BWT_ELECTRUM_ADDR:t > /dev/null &
  sleep 2
  ele2 --oneserver --server $BWT_ELECTRUM_ADDR:t > /dev/null &
else
  cat <<EOL
You can access electrum with:
$ electrum --regtest --dir $ELECTRUM_DIR --wallet $WALLET1 <cmd>
$ electrum --regtest --dir $ELECTRUM_DIR --wallet $WALLET2 <cmd>
EOL
fi

echo
read -p 'Press enter to shutdown and clean up'
