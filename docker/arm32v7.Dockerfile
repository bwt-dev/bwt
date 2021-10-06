# rust:1.49-slim
FROM rust@sha256:58cb29151843a8ba8e0e78e3f80096ed2f9514cf81d4f85ef43727140631e67b as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev
WORKDIR /usr/src/bwt
COPY . .
ARG FEATURES=electrum,http,webhooks,track-spends
ARG PREBUILT_BIN
RUN if [ -n "$PREBUILT_BIN" ]; then cp $PREBUILT_BIN /usr/local/bin; \
    else cargo install --locked --path . --root /usr/local/ --no-default-features --features "cli,$FEATURES"; fi

# debian:buster-slim
FROM debian@sha256:d31590f680577ffde6bd08943e9590eaabdc04529ea60f4bb6f58cddbc33f628
ARG FEATURES=electrum,http,webhooks,track-spends
RUN echo $FEATURES | grep -v webhooks > /dev/null || (apt-get update && apt-get install -y libssl-dev)
COPY --from=builder /usr/local/bin/bwt /usr/local/bin/
ENTRYPOINT [ "bwt", "--bitcoind-dir", "/bitcoin" ]

# The ARM32v7/ARM32v8 dockerfiles are automatically generated from the main Docker,
# see scripts/docker-generate.sh
