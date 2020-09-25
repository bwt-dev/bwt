FROM rust:1.46-slim as builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev gcc-mingw-w64-x86-64 zip && \
    rustup target add x86_64-pc-windows-gnu

WORKDIR /usr/src/bwt
COPY Cargo.toml Cargo.lock ./
RUN mkdir src .cargo && touch src/lib.rs src/main.rs && \
    cargo vendor > .cargo/config

VOLUME /usr/src/bwt
ENV TARGETS=linux,win
ENTRYPOINT [ "/usr/src/bwt/scripts/build.sh" ]
