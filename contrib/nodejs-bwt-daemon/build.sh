#!/bin/bash
set -xeo pipefail

version=${1:-`cat ../../Cargo.toml | egrep '^version =' | cut -d'"' -f2`}
dest_dir=${2:-../../dist}

echo Building the nodejs bwt-daemon v$version package for $dest_dir

npm version $version

(cd $dest_dir && sha256sum libbwt*.tar.gz) > SHA256SUMS

npm pack
mv bwt-daemon-$version.tgz $dest_dir/nodejs-bwt-daemon-$version.tgz
