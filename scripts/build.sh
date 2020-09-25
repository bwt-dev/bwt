#!/bin/bash
set -xeo pipefail

# `osx` is also available, but requires osxcross installed (see builder-os.Dockerfile)
TARGETS=${TARGETS:-linux,win}

build() {
  name=$1; target=$2; features=$3
  ext=$([[ $target != *"-windows-"* ]] || echo .exe)
  dest=dist/$name
  mkdir -p $dest

  echo Building $name for $target with features $features

  cargo build --release --target $target --no-default-features --features "$features"

  mv target/$target/release/bwt$ext $dest

  [[ $target == *"-apple-"* ]] || strip $dest/bwt$ext

  cp README.md LICENSE $dest/

  pack $name
}

# pack an tar.gz/zip archive file, with fixed/removed metadata attrs and deterministic file order for reproducibility
pack() {
  name=$1; dir=${2:-$1}
  pushd dist
  touch -t 1711081658 $name $name/*
  if [[ $name == *"-linux" ]]; then
    TZ=UTC tar --mtime='2017-11-08 16:58:00' --owner=0 --sort=name -I 'gzip --no-name' -chf $name.tar.gz $dir
  else
    find -H $dir | sort | xargs zip -X -q $name.zip
  fi
  popd
}

version=`cat Cargo.toml | egrep '^version =' | cut -d'"' -f2`

for cfg in linux,x86_64-unknown-linux-gnu \
           win,x86_64-pc-windows-gnu \
           osx,x86_64-apple-darwin; do
  IFS=',' read platform target <<< $cfg
  if [[ $TARGETS != *"$platform"* ]]; then continue; fi

  build bwt-$version-x86_64-$platform $target http,electrum,webhooks,track-spends
  build bwt-$version-electrum_only-x86_64-$platform $target electrum
done

echo Building electrum plugin
for platform in linux win osx; do
  if [[ $TARGETS != *"$platform"* ]]; then continue; fi

  name=bwt-$version-electrum_plugin-x86_64-$platform
  dest=dist/$name
  mkdir $dest
  cp contrib/electrum-plugin/*.py $dest
  cp dist/bwt-$version-electrum_only-x86_64-$platform/* $dest
  # needs to be inside a directory with a name that matches the plugin module name for electrum to load it,
  # create a temporary link to get tar/zip to pack it properly. (can also be done for tar.gz with --transform)
  ln -s $name dist/bwt
  pack $name bwt
  rm dist/bwt
done

# remove subdirectories, keep release tarballs
rm -r dist/*/

[ -n "$OWNER" ] && chown -R $OWNER dist target || true
