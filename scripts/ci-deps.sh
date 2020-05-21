#!/bin/bash
set -eo pipefail

rustup component add rustfmt
rustup component add clippy

BITCOIN_VERSION=0.19.1
BITCOIN_FILENAME=bitcoin-$BITCOIN_VERSION-x86_64-linux-gnu.tar.gz
BITCOIN_URL=https://bitcoincore.org/bin/bitcoin-core-$BITCOIN_VERSION/$BITCOIN_FILENAME
BITCOIN_SHA256=5fcac9416e486d4960e1a946145566350ca670f9aaba99de6542080851122e4c

ELECTRUM_URL=https://download.electrum.org/3.3.8/electrum-3.3.8-x86_64.AppImage
ELECTRUM_SHA256=fd2526f7a37530bb655375d3c95ce6a51e3c2beb83ff88a54e28aa365033794a

mkdir -p /opt/bin /opt/bitcoin

cd /opt/bitcoin
wget -qO "$BITCOIN_FILENAME" "$BITCOIN_URL"
echo "$BITCOIN_SHA256 $BITCOIN_FILENAME" | sha256sum -c -
BD=bitcoin-$BITCOIN_VERSION/bin
tar -xzvf "$BITCOIN_FILENAME" $BD/bitcoind $BD/bitcoin-cli --strip-components=1
mv bin/* /opt/bin/

wget -O /opt/bin/electrum $ELECTRUM_URL
echo "$ELECTRUM_SHA256 /opt/bin/electrum" | sha256sum -c -
chmod +x /opt/bin/electrum
