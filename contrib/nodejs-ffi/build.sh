#!/bin/bash
set -xeo pipefail

version=$1
dest_dir=$2

jq .version=\""$version"\" package.json > package.json.new
mv package.json.new package.json

(cd $dest_dir && sha256sum libbwt*.tar.gz) > SHA256SUMS

npm pack
mv bwt-daemon-$version.tgz $dest_dir/nodejs-bwt-daemon-$version.tgz
