FROM rust:1.97.1-slim-bookworm

RUN apt-get update \
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
