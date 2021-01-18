# rust:1.49-slim
FROM rust@sha256:3c1012af9fa01b63f14c077fbdf6bf6ea16f85389dd8ccc80f9c13d65ed4bce1 as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev
WORKDIR /usr/src/bwt
COPY . .
ARG FEATURES=electrum,http,webhooks,track-spends
RUN cargo install --locked --path . --no-default-features --features "cli,$FEATURES"

# debian:buster-slim
FROM debian@sha256:59678da095929b237694b8cbdbe4818bb89a2918204da7fa0145dc4ba5ef22f9
ARG FEATURES=electrum,http,webhooks,track-spends
RUN echo $FEATURES | grep -v webhooks > /dev/null || (apt-get update && apt-get install -y libssl-dev)
COPY --from=builder /usr/local/cargo/bin/bwt /usr/local/bin/
ENTRYPOINT [ "bwt", "--bitcoind-dir", "/bitcoin" ]
