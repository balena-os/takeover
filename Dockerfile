FROM debian:stretch

ENV INSIDE_DOCKER_CONTAINER 1

# Install git and compilers, let's toss gnupg and reprepro in there so we can
# use this container to make the apt repo as well
RUN apt-get update \
    && apt-get -y upgrade \
    && apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        git \
        pkg-config \
	vim \
	perl \
	make \
    && rm -rf /var/lib/apt/lists/*


RUN mkdir /toolchain
WORKDIR /toolchain

# Check out Raspbian cross-compiler (this will work on *ALL* Raspberry Pi versions)
RUN git clone --depth 1 git://github.com/raspberrypi/tools.git rpi-tools \
    && rm -rf rpi-tools/.git
ENV PATH "/toolchain/rpi-tools/arm-bcm2708/gcc-linaro-arm-linux-gnueabihf-raspbian-x64/bin/:${PATH}"

# Create wrapper around gcc to point to rpi sysroot
# Thanks @ https://github.com/herrernst/librespot/blob/build-rpi/.travis.yml
RUN echo '#!/bin/sh\narm-linux-gnueabihf-gcc --sysroot /toolchain/rpi-tools/arm-bcm2708/arm-bcm2708hardfp-linux-gnueabi/arm-bcm2708hardfp-linux-gnueabi/sysroot "$@"' \
        > rpi-tools/arm-bcm2708/gcc-linaro-arm-linux-gnueabihf-raspbian-x64/bin/gcc-wrapper \
    && chmod +x rpi-tools/arm-bcm2708/gcc-linaro-arm-linux-gnueabihf-raspbian-x64/bin/gcc-wrapper \
    && ln -s ld-linux.so.3 rpi-tools/arm-bcm2708/arm-bcm2708hardfp-linux-gnueabi/arm-bcm2708hardfp-linux-gnueabi/sysroot/lib/ld-linux-armhf.so.3

ENV PKG_CONFIG_ALLOW_CROSS 1
ENV PKG_CONFIG_PATH "/toolchain/rpi-tools/arm-bcm2708/arm-bcm2708hardfp-linux-gnueabi/arm-bcm2708hardfp-linux-gnueabi/sysroot/lib/pkgconfig"

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin/:${PATH}"
# ENV CARGO_TARGET_DIR "/build"
RUN mkdir /cargo-home
ENV CARGO_HOME "/cargo-home"

RUN mkdir -p /.cargo
RUN echo '[target.arm-unknown-linux-gnueabihf]\nlinker = "gcc-wrapper"' > /.cargo/config
RUN rustup target add arm-unknown-linux-gnueabihf

WORKDIR /build
