#!/bin/bash
set -eo pipefail

BITCOIN_VERSION=0.20.0
BITCOIN_FILENAME=bitcoin-$BITCOIN_VERSION-x86_64-linux-gnu.tar.gz
BITCOIN_URL=https://bitcoincore.org/bin/bitcoin-core-$BITCOIN_VERSION/$BITCOIN_FILENAME
BITCOIN_SHA256=35ec10f87b6bc1e44fd9cd1157e5dfa483eaf14d7d9a9c274774539e7824c427

ELECTRUM_URL=https://download.electrum.org/4.0.3/electrum-4.0.3-x86_64.AppImage
ELECTRUM_SHA256=512b58c437847048a9629cb6bf2eb786b8969a1e17b7b51510b11672c9b29fc7

mkdir -p /opt/bin /opt/bitcoin

pushd /opt/bitcoin
wget -qO "$BITCOIN_FILENAME" "$BITCOIN_URL"
echo "$BITCOIN_SHA256 $BITCOIN_FILENAME" | sha256sum -c -
BD=bitcoin-$BITCOIN_VERSION/bin
tar -xzvf "$BITCOIN_FILENAME" $BD/bitcoind $BD/bitcoin-cli --strip-components=1
mv bin/* /opt/bin/
popd

wget -qO /opt/bin/electrum $ELECTRUM_URL
echo "$ELECTRUM_SHA256 /opt/bin/electrum" | sha256sum -c -
chmod +x /opt/bin/electrum

export PATH=/opt/bin:$PATH
