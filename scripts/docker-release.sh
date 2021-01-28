#!/bin/bash
set -xeo pipefail
shopt -s expand_aliases

docker_name=shesek/bwt
version=$(grep -E '^version =' Cargo.toml | cut -d'"' -f2)
base_tag=$docker_name:$version

build_variant() {
  local docker_tag=$1
  local docker_alias=$2
  local features=$3
  local bin_variant=$4

  build $1-amd64   $features "$bin_variant" x86_64-linux  Dockerfile
  build $1-arm32v7 $features "$bin_variant" arm32v7-linux arm32v7.Dockerfile
  build $1-arm64v8 $features "$bin_variant" arm64v8-linux arm64v8.Dockerfile

  # can't tag manifests to create an alias, need to create them separately instead
  for target in $docker_tag $docker_alias; do
    docker manifest create --amend $target $docker_tag-amd64 $docker_tag-arm32v7 $docker_tag-arm64v8
    docker manifest annotate $target $docker_tag-amd64 --os linux --arch amd64
    docker manifest annotate $target $docker_tag-arm32v7 --os linux --arch arm --variant v7
    docker manifest annotate $target $docker_tag-arm64v8 --os linux --arch arm64 --variant v8
    docker manifest push $target -p
  done
}

build() {
  local docker_tag=$1
  local features=$2
  local bin_variant=$3
  local bin_platform=$4
  local dockerfile=$5

  docker build -t $docker_tag --build-arg FEATURES=$features \
    --build-arg PREBUILT_BIN=dist/bwt-$version$bin_variant-$bin_platform/bwt \
    -f docker/$dockerfile .

  docker push $docker_tag
}

build_variant $base_tag          $docker_name:latest   http,electrum,webhooks,track-spends ''
build_variant $base_tag-electrum $docker_name:electrum electrum                            '-electrum_only'
