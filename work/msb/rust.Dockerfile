FROM rust:1.97.1-slim-bookworm

ARG USE_CN_MIRRORS=0

RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|deb.debian.org/debian|mirrors.aliyun.com/debian|g' \
            -e 's|security.debian.org/debian-security|mirrors.aliyun.com/debian-security|g' \
            /etc/apt/sources.list.d/debian.sources; \
        export RUSTUP_DIST_SERVER=https://rsproxy.cn RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup; \
    fi \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        git \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt \
    && useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && install -d -o sandbox -g sandbox /workspace

ENV CARGO_HOME=/home/sandbox/.cargo \
    PATH="/home/sandbox/.cargo/bin:/usr/local/cargo/bin:${PATH}"

USER sandbox
WORKDIR /workspace
CMD ["cargo"]
