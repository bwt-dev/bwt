# rust:1.49-slim
FROM rust@sha256:3c1012af9fa01b63f14c077fbdf6bf6ea16f85389dd8ccc80f9c13d65ed4bce1
RUN apt-get update && apt-get install -y pkg-config make zip wget \
        libssl-dev gcc-mingw-w64-x86-64 gcc-arm-linux-gnueabihf gcc-aarch64-linux-gnu && \
    rustup target add x86_64-pc-windows-gnu armv7-unknown-linux-gnueabihf aarch64-unknown-linux-gnu
    # macOS is built using a separate image, see builder-osx.Dockerfile

# Install sccache
# Mount-in /usr/local/sccache to enable
ARG SCCACHE_VERSION=0.2.14
ARG SCCACHE_HASH=071d7cce6e588a0b7239ed1a9e0baa1f3f0e293b1f0d37e17d9594526e7622f9
ENV SCCACHE_DIR=/usr/local/sccache
RUN wget -q -O sccache.tar.gz https://github.com/mozilla/sccache/releases/download/$SCCACHE_VERSION/sccache-$SCCACHE_VERSION-x86_64-unknown-linux-musl.tar.gz \
    && echo "$SCCACHE_HASH sccache.tar.gz" | sha256sum -c - \
    && tar xf sccache.tar.gz \
    && mv sccache-$SCCACHE_VERSION-x86_64-unknown-linux-musl/sccache /usr/local/bin/ \
    && rm -r sccache*

WORKDIR /usr/src/bwt
VOLUME /usr/src/bwt
ENV TARGETS=x86_64-linux,x86_64-windows,arm32v7-linux,arm64v8-linux
ENTRYPOINT [ "/usr/src/bwt/scripts/build.sh" ]
