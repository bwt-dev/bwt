#!/bin/bash
set -xeo pipefail

# Generate the arm32v7/arm64v8 dockerfiles using the main dockerfile as a template
generate_dockerfile() {
  local builder_image=$1
  local runtime_image=$2
  sed -r "s/^(FROM rust@sha256:)[^ ]+/\1$builder_image/; s/^(FROM debian@sha256:)[^ ]+/\1$runtime_image/;" docker/Dockerfile
}

# arm32v7/rust:1.49.0-slim and arm32v7/debian:buster-slim (10.7)
# https://hub.docker.com/r/arm32v7/rust/tags?name=slim
# https://hub.docker.com/r/arm32v7/debian/tags?name=buster-slim
generate_dockerfile 58cb29151843a8ba8e0e78e3f80096ed2f9514cf81d4f85ef43727140631e67b \
  d31590f680577ffde6bd08943e9590eaabdc04529ea60f4bb6f58cddbc33f628 \
  > docker/arm32v7.Dockerfile

# arm64v8/rust:1.49.0-slim and arm64v8/debian:buster-slim (10.7)
# https://hub.docker.com/r/arm64v8/rust/tags?name=slim
# https://hub.docker.com/r/arm64v8/debian/tags?name=buster-slim
generate_dockerfile 2a44876432ba0cfbe7f7fcddd9b16f316ee13abecdee43b25f0645529966bc40 \
  01b65c2928fed9427e59a679e287a75d98551ea2061cf03c61be0c7e1fc40fef \
  > docker/arm64v8.Dockerfile
