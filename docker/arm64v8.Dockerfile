# rust:1.49-slim
FROM rust@sha256:2a44876432ba0cfbe7f7fcddd9b16f316ee13abecdee43b25f0645529966bc40 as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev
WORKDIR /usr/src/bwt
COPY . .
ARG FEATURES=electrum,http,webhooks,track-spends
ARG PREBUILT_BIN
RUN if [ -n "$PREBUILT_BIN" ]; then cp $PREBUILT_BIN /usr/local/bin; \
    else cargo install --locked --path . --root /usr/local/ --no-default-features --features "cli,$FEATURES"; fi

# debian:buster-slim
FROM debian@sha256:01b65c2928fed9427e59a679e287a75d98551ea2061cf03c61be0c7e1fc40fef
ARG FEATURES=electrum,http,webhooks,track-spends
RUN echo $FEATURES | grep -v webhooks > /dev/null || (apt-get update && apt-get install -y libssl-dev)
COPY --from=builder /usr/local/bin/bwt /usr/local/bin/
ENTRYPOINT [ "bwt", "--bitcoind-dir", "/bitcoin" ]

# The ARM32v7/ARM32v8 dockerfiles are automatically generated from the main Docker,
# see scripts/docker-generate.sh
