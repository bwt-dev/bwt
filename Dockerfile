FROM rust:1.44-slim as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev
WORKDIR /usr/src/bwt
COPY . .
ARG FEATURES="electrum http webhooks track-spends"
RUN cargo install --path . --no-default-features --features "$FEATURES"

FROM debian:buster-slim
RUN echo $FEATURES | grep webhooks > /dev/null && apt-get update && apt-get install -y libssl-dev || true
COPY --from=builder /usr/local/cargo/bin/bwt /usr/local/bin/
ENTRYPOINT [ "bwt", "--bitcoind-dir", "/bitcoin" ]
