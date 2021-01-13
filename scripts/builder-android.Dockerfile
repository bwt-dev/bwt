FROM bwt-builder

WORKDIR /root

RUN apt-get update && apt-get install -y --no-install-recommends unzip wget \
  && rustup target add i686-linux-android x86_64-linux-android armv7-linux-androideabi aarch64-linux-android

# Java 11 (OpenJDK)
ENV JAVA_HOME=/usr/lib/jvm/java-11-openjdk-amd64
RUN mkdir -p /usr/share/man/man1 # https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=863199#23
RUN apt-get install -y --no-install-recommends openjdk-11-jdk-headless

# Android SKD tools
ARG ANDROID_SDK_VERSION=6858069
ARG ANDROID_SDK_HASH=87f6dcf41d4e642e37ba03cb2e387a542aa0bd73cb689a9e7152aad40a6e7a08
ARG ANDROID_NDK_VERSION=22.0.6917172
ENV ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_SDK_HOME=$ANDROID_SDK_ROOT
ENV NDK_HOME=$ANDROID_SDK_ROOT/ndk/$ANDROID_NDK_VERSION
ENV PATH=$PATH:$ANDROID_SDK_ROOT/cmdline-tools/tools/bin
ENV PATH=$PATH:$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/
RUN wget -q -O sdktools.zip https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_SDK_VERSION}_latest.zip \
  && echo "$ANDROID_SDK_HASH sdktools.zip" | sha256sum -c - \
  && mkdir -p $ANDROID_SDK_ROOT/cmdline-tools \
  && unzip -q sdktools.zip -d $ANDROID_SDK_ROOT/cmdline-tools && rm sdktools.zip \
  && mv $ANDROID_SDK_ROOT/cmdline-tools/cmdline-tools $ANDROID_SDK_ROOT/cmdline-tools/tools \
  && yes | (sdkmanager "platforms;android-29" "build-tools;29.0.3" && sdkmanager --licenses) > /dev/null 2>&1 \
  && sdkmanager --install "ndk;$ANDROID_NDK_VERSION" --channel=1 \
  && chmod 777 $ANDROID_SDK_HOME

# mount-in gradle cache directory for improved build speeds
ENV GRADLE_USER_HOME=/usr/local/gradle
RUN mkdir -p $GRADLE_USER_HOME && chmod 777 $GRADLE_USER_HOME

ENV TARGETS=arm32v7-android,arm64v8-android,i686-android,x86_64-android
