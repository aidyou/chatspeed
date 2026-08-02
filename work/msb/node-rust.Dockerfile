FROM node:24-bookworm-slim AS node-runtime

FROM rust:1.97.1-slim-bookworm

ARG PNPM_VERSION=10.14.0
ARG YARN_VERSION=4.9.2

ENV npm_config_manage_package_manager_versions=false

COPY --from=node-runtime /usr/local/ /usr/local/

# Composite profile for projects that require both JavaScript and Rust toolchains.
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
        python3 \
    && rm -rf /var/lib/apt/lists/* \
    && corepack disable \
    && npm install --global \
        "@yarnpkg/cli-dist@${YARN_VERSION}" \
        "pnpm@${PNPM_VERSION}" \
    && pnpm --version \
    && yarn --version \
    && rustup component add clippy rustfmt \
    && useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && install -d -o sandbox -g sandbox /workspace

ENV CARGO_HOME=/home/sandbox/.cargo \
    PATH="/home/sandbox/.cargo/bin:/usr/local/cargo/bin:${PATH}"

USER sandbox
WORKDIR /workspace
CMD ["/bin/bash"]
