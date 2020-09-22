#!/bin/bash
set -xeo pipefail

build() {
  name=$1; platform=$2; features=$3
  ext=$([[ $platform != *"-windows-"* ]] || echo .exe)
  dest=dist/$name
  mkdir -p $dest

  echo Building $name for $platform with features $features

  # drop PE timestamps in windows builds for reproducibility (https://wiki.debian.org/ReproducibleBuilds/TimestampsInPEBinaries#building_with_mingw-w64)
  RUSTFLAGS="$RUSTFLAGS $([[ $platform != *"-windows-"* ]] || echo '-Clink-arg=-Wl,--no-insert-timestamp')" \
  cargo build --release --target $platform --no-default-features --features "$features"

  mv target/$platform/release/bwt$ext $dest
  strip $dest/bwt$ext

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

rm -rf dist/*

for target in linux,x86_64-unknown-linux-gnu \
              win,x86_64-pc-windows-gnu; do
  IFS=',' read pnick platform <<< $target

  build bwt-$version-x86_64-$pnick $platform http,electrum,webhooks,track-spends
  build bwt-$version-electrum_only-x86_64-$pnick $platform electrum
done

echo Building electrum plugin
for pnick in linux win; do
  name=bwt-$version-electrum_plugin-x86_64-$pnick
  dest=dist/$name
  mkdir $dest
  cp contrib/electrum-plugin/*.py $dest
  cp dist/bwt-$version-electrum_only-x86_64-$pnick/* $dest
  # needs to be inside a directory with a name that matches the plugin module name for electrum to load it,
  # create a temporary link to get tar/zip to pack it properly. (can also be done for tar.gz with --transform)
  ln -s $name dist/bwt
  pack $name bwt
  rm dist/bwt
done

# remove subdirectories, keep release tarballs
rm -r dist/*/

[ -n "$OWNER" ] && chown -R $OWNER dist || true
