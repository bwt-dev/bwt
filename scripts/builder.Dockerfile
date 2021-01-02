FROM rust:1.49-slim as builder
RUN apt-get update && apt-get install -y pkg-config make zip \
        libssl-dev gcc-mingw-w64-x86-64 gcc-arm-linux-gnueabihf gcc-aarch64-linux-gnu && \
    rustup target add x86_64-pc-windows-gnu armv7-unknown-linux-gnueabihf aarch64-unknown-linux-gnu
    # macOS is built using a separate image, see builder-osx.Dockerfile

WORKDIR /usr/src/bwt
VOLUME /usr/src/bwt
ENV TARGETS=x86_64-linux,x86_64-win,arm32v7-linux,arm64v8-linux
ENTRYPOINT [ "/usr/src/bwt/scripts/build.sh" ]
