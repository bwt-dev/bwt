FROM bwt-builder

ARG CROSSOSX_COMMIT=364703ca0962c4a12688daf8758802a5df9e3221
ARG OSX_SDK_VERSION=10.11
ARG OSX_SDK_SHASUM=694a66095a3514328e970b14978dc78c0f4d170e590fa7b2c3d3674b75f0b713

RUN apt-get update && apt-get install -y git wget clang cmake libxml2-dev zlib1g-dev && \
    rustup target add x86_64-apple-darwin

RUN git clone https://github.com/tpoechtrager/osxcross /usr/src/osxcross && \
    cd /usr/src/osxcross && \
    git checkout $CROSSOSX_COMMIT && \
    wget -q https://s3.dockerproject.org/darwin/v2/MacOSX$OSX_SDK_VERSION.sdk.tar.xz --directory-prefix=tarballs && \
    echo "$OSX_SDK_SHASUM tarballs/MacOSX$OSX_SDK_VERSION.sdk.tar.xz" | sha256sum -c - && \
    UNATTENDED=yes OSX_VERSION_MIN=10.7 ./build.sh

ENV TARGETS=x86_64-osx
ENV CC=x86_64-apple-darwin15-cc
ENV AR=x86_64-apple-darwin15-ar
ENV PATH=$PATH:/usr/src/osxcross/target/bin
