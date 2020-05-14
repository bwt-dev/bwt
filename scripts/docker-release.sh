#!/bin/bash
set -eo pipefail

name=shesek/bwt
version=`cat Cargo.toml | egrep '^version =' | cut -d'"' -f2`
tag=$name:$version

docker build -t $tag .
docker build -t $tag-electrum --build-arg FEATURES=electrum .

docker tag $tag $name:latest
docker tag $tag-electrum $name:electrum

docker push $name
