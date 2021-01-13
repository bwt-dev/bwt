#!/bin/bash
set -xeo pipefail

# `x86_64-osx` is also available, but requires osxcross installed (see builder-osx.Dockerfile)
TARGETS=${TARGETS:-x86_64-linux,x86_64-windows,arm32v7-linux,arm64v8-linux}

if [[ -n "$SCCACHE_DIR" && -d "$SCCACHE_DIR" ]]; then
  export RUSTC_WRAPPER=$(which sccache)
fi

build() {
  name=$1; target=$2; features=$3
  dest=dist/$name
  mkdir -p $dest

  echo Building $name for $target with features $features

  cargo build --release --target $target --no-default-features --features "cli,$features"

  filename=bwt$([[ $2 == *"-windows-"* ]] && echo .exe || echo '')
  mv target/$target/release/$filename $dest/
  strip_symbols $target $dest/$filename || true

  cp LICENSE README.md $dest/
  pack $name
}

strip_symbols() {
  case $1 in
    "x86_64-unknown-linux-gnu" | "x86_64-pc-windows-gnu") strip $2 ;;
    "x86_64-apple-darwin") x86_64-apple-darwin15-strip $2 ;;
    "armv7-unknown-linux-gnueabihf") arm-linux-gnueabihf-strip $2 ;;
    "aarch64-unknown-linux-gnu") aarch64-linux-gnu-strip $2 ;;
  esac
}

# pack a tar.gz or zip archive file, with fixed/removed metadata attrs and deterministic file order for reproducibility
pack() {
  name=$1; dir=${2:-$1}
  pushd dist
  touch -t 1711081658 $name $name/*
  if [[ $name == *"-linux" || $name == *"-arm"* ]]; then
    TZ=UTC tar --mtime='2017-11-08 16:58:00' --owner=0 --sort=name -I 'gzip --no-name' -chf $name.tar.gz $dir
  else
    find -H $dir | sort | xargs zip -X -q $name.zip
  fi
  popd
}

version=$(grep -E '^version =' Cargo.toml | cut -d'"' -f2)

for cfg in x86_64-linux,x86_64-unknown-linux-gnu \
           x86_64-osx,x86_64-apple-darwin \
           x86_64-windows,x86_64-pc-windows-gnu \
           arm32v7-linux,armv7-unknown-linux-gnueabihf \
           arm64v8-linux,aarch64-unknown-linux-gnu; do
  IFS=',' read platform target <<< $cfg
  if [[ $TARGETS != *"$platform"* ]]; then continue; fi


  if [ -z "$ELECTRUM_ONLY_ONLY" ]; then
    # The OpenSSL dependency enabled by the webhooks feature causes an error on ARM targets.
    # Disable it for now on ARM, follow up at https://github.com/shesek/bwt/issues/52
    complete_feat=http,electrum,track-spends$([[ $platform == "arm"* ]] || echo ',webhooks')
    build bwt-$version-$platform $target $complete_feat
  fi

  build bwt-$version-electrum_only-$platform $target electrum
done
