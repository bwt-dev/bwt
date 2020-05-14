FROM rust:1.43-slim as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev
WORKDIR /usr/src/bwt
COPY . .
ARG FEATURES
RUN cargo install --path . \
  $([ -n "$FEATURES" ] && echo "--no-default-features --features $FEATURES")

FROM debian:buster-slim
RUN apt-get update && apt-get install -y libssl-dev
COPY --from=builder /usr/local/cargo/bin/bwt /usr/local/bin/
ENTRYPOINT [ "bwt", "--bitcoind-dir", "/bitcoin" ]
