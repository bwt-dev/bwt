#!/bin/bash
set -eo pipefail

source scripts/setup-env.sh

cat <<EOL

bwt is running:
- HTTP API server on http://$BWT_HTTP_ADDR
- Electrum RPC server on $BWT_ELECTRUM_ADDR
- Bitcoin Core RPC server on 127.0.0.1:$BTC_RPC_PORT
- Logs at $DIR/{bwt,check}.log

You can access bitcoind with:
$ bitcoin-cli -datadir=$BTC_DIR -rpcwallet=internal <cmd>
$ bitcoin-cli -datadir=$BTC_DIR -rpcwallet=bwt <cmd>

Electrum wallet xpubs:
- `ele1 getmpk` (segwit)
- `ele2 getmpk` (non-segwit)

EOL

if [ -z "$NO_GUI" ]; then
  echo Starting Electrum GUI...
  # disable "Would you like to be notified when there is a newer version of Electrum available?" popup
  # and enable some advanced features
  ele setconfig check_updates false > /dev/null
  ele setconfig dont_show_testnet_warning true > /dev/null
  for opt in fee addresses_tab utxo_tab console_tab; do ele setconfig show_$opt true > /dev/null; done

  ele stop > /dev/null 2>&1
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
