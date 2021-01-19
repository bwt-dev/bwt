#!/bin/bash
set -eo pipefail
shopt -s expand_aliases

(command -v electrum && command -v bitcoind && command -v bitcoin-cli) > /dev/null \
  || { echo >&2 "bitcoind, bitcoin-cli and electrum must be installed in PATH"; exit 1; }

export FEATURES=cli,${FEATURES:-http,electrum,webhooks,track-spends}

if [ -z "$DIR" ]; then
  DIR=`mktemp -d --suffix -bwt-env`
else
  KEEP_DIR=1
fi

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
ele1 () { ele $1 --wallet $WALLET1 "${@:2}"; }
ele2 () { ele $1 --wallet $WALLET2 "${@:2}"; }

export RUST_LOG_STYLE=${RUST_LOG_STYLE:-always}
export CARGO_TERM_COLOR=${CARGO_TERM_COLOR:-always}

runbwt () {
  echo - Running with "$@"
  if [ -n "$BWT_BIN" ]; then
    $BWT_BIN "$@" &> $DIR/bwt.log &
  elif [ -z "$NO_WATCH" ] && command -v cargo-watch > /dev/null; then
    echo - Using cargo-watch
    # https://github.com/passcod/cargo-watch#restarting-an-application-only-if-the-buildcheck-succeeds
    touch .trigger
    ARGS="$@" cargo-watch --no-gitignore -w .trigger -s 'cargo run --no-default-features --features "$FEATURES" -- $ARGS' &> $DIR/bwt.log &
    cargo-watch --postpone -w src -w Cargo.toml -s 'cargo check --no-default-features --features "$FEATURES" && touch .trigger' &> $DIR/check.log &
  else
    cargo run --no-default-features --features "$FEATURES" -- "$@" &> $DIR/bwt.log &
  fi
}

cleanup() {
  trap - SIGTERM SIGINT
  set +eo pipefail
  kill `jobs -rp` 2> /dev/null
  wait `jobs -rp` 2> /dev/null
  ele daemon stop &> /dev/null
  [ -n "$KEEP_DIR" ] || rm -rf $DIR
  kill -- -$$ 2> /dev/null
}
trap cleanup SIGINT SIGTERM EXIT

echo Setting up envirnoment in $DIR

echo Setting up bitcoind
mkdir -p $BTC_DIR

cat >$BTC_DIR/bitcoin.conf <<EOL
regtest=1
printtoconsole=0
nolisten=1
fallbackfee=0.00001

blocknotify=nc -U $BWT_SOCKET > /dev/null 2>&1
walletnotify=nc -U $BWT_SOCKET > /dev/null 2>&1

[regtest]
rpcport=$BTC_RPC_PORT
EOL

bitcoind -datadir=$BTC_DIR $BTC_OPTS &

echo - Waiting for bitcoind to warm up...
btc -rpcwait getblockchaininfo > /dev/null

echo - Creating wallets...
btc createwallet internal > /dev/null
btc createwallet bwt true true > /dev/null

echo - Generating some blocks...
btc generatetoaddress 110 `btc getnewaddress` > /dev/null

echo Setting up electrum
mkdir -p $ELECTRUM_DIR
ele --offline setconfig log_to_file true > /dev/null

echo - Creating 2 wallets...
electrum create --offline --regtest --seed_type segwit --wallet $WALLET1 > /dev/null
electrum create --offline --regtest --wallet $WALLET2 > /dev/null

echo - Starting daemon and loading wallets...
start_electrum(){
  ele daemon -d --server $BWT_ELECTRUM_ADDR:t --oneserver > /dev/null 2>&1
  ele load_wallet --wallet $WALLET1 > /dev/null
  ele load_wallet --wallet $WALLET2 > /dev/null
}
start_electrum

if [ -z "$NO_FUNDING" ]; then
  echo - Sending some funds...
  for i in `seq 1 3`; do
    for n in `seq 4 5`; do
      btc sendtoaddress `ele1 getunusedaddress` $n.$i > /dev/null
      btc sendtoaddress `ele2 getunusedaddress` $i$n > /dev/null
    done
    # leave the last round as unconfirmed
    [ $i != 3 ] && btc generatetoaddress $i `btc getnewaddress` > /dev/null
  done
fi

echo Setting up bwt
runbwt --no-startup-banner --network regtest \
  --bitcoind-dir $BTC_DIR --bitcoind-url http://localhost:$BTC_RPC_PORT/ --bitcoind-wallet bwt \
  --electrum-addr $BWT_ELECTRUM_ADDR \
  --unix-listener-path $BWT_SOCKET --poll-interval ${INTERVAL:=120} \
  --initial-import-size 30 \
  --xpub `ele1 getmpk` --xpub `ele2 getmpk` \
  $([[ $FEATURES == *"http"* ]] && echo "--http-addr $BWT_HTTP_ADDR --http-cors any") \
  -v "$@" $BWT_OPTS
pid=$!

echo - Waiting for bwt... "(building may take awhile)"
sed '/Electrum RPC server running/ q' <(tail -F -n+0 $DIR/bwt.log 2> /dev/null)

# these are showing up a lot because of the 1 second interval
annoying_msgs='syncing mempool transactions|fetching 25 transactions starting at'
[ -n "$PRINT_LOGS" ] && tail --pid $pid -F -n0 $DIR/bwt.log | egrep --line-buffered -v "$annoying_msgs" 2> /dev/null &

# restart daemon to make it re-try connecting to the server immediately
ele stop > /dev/null
start_electrum
