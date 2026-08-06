FROM node:24-bookworm-slim AS node-runtime

FROM rust:1.97.1-slim-bookworm

ARG PNPM_VERSION=10.14.0
ARG YARN_VERSION=4.9.2
ARG TAURI_CLI_VERSION=2.11.0
ARG NPM_REGISTRY=https://registry.npmmirror.com

COPY --from=node-runtime /usr/local/ /usr/local/

# Tauri projects require the Node and Rust toolchains plus GTK/WebKit build dependencies.
RUN sed -i \
        -e 's|deb.debian.org/debian|mirrors.aliyun.com/debian|g' \
        -e 's|security.debian.org/debian-security|mirrors.aliyun.com/debian-security|g' \
        /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        file \
        git \
        libayatana-appindicator3-dev \
        libgtk-3-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        librsvg2-dev \
        patchelf \
        pkg-config \
        python3 \
    && rm -rf /var/lib/apt/lists/* \
    && corepack disable \
    && rm -f /usr/local/bin/yarn /usr/local/bin/yarnpkg /usr/local/bin/pnpm /usr/local/bin/pnpx \
    && npm config set registry "${NPM_REGISTRY}" \
    && npm config set fetch-retries 5 \
    && npm config set fetch-retry-mintimeout 20000 \
    && npm config set fetch-retry-maxtimeout 120000 \
    && npm config set fetch-timeout 300000 \
    && npm install --global \
        "@tauri-apps/cli@${TAURI_CLI_VERSION}" \
        "@yarnpkg/cli-dist@${YARN_VERSION}" \
        "pnpm@${PNPM_VERSION}" \
    && pnpm --version \
    && yarn --version \
    && tauri --version \
    && rm -f /usr/local/bin/pnpm \
    && printf '#!/bin/sh\nexport npm_config_manage_package_manager_versions=false\nexec node /usr/local/lib/node_modules/pnpm/bin/pnpm.cjs "$@"\n' \
        > /usr/local/bin/pnpm \
    && chmod 755 /usr/local/bin/pnpm \
    && useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && install -d -o sandbox -g sandbox /workspace /home/sandbox/.cargo \
    && RUSTUP_DIST_SERVER="https://rsproxy.cn" \
        RUSTUP_UPDATE_ROOT="https://rsproxy.cn/rustup" \
    && for tool in cargo rustc rustdoc rustfmt cargo-clippy clippy-driver; do \
        printf '#!/bin/sh\nexport RUSTUP_HOME=/usr/local/rustup\nexport CARGO_HOME=/home/sandbox/.cargo\nexec /usr/local/cargo/bin/%s "$@"\n' "$tool" \
          > "/usr/local/bin/$tool"; \
        chmod 755 "/usr/local/bin/$tool"; \
    done

# Microsandbox does not preserve Docker ENV PATH, so Rust wrappers live in /usr/local/bin.
ENV CARGO_HOME=/home/sandbox/.cargo \
    PATH="/home/sandbox/.cargo/bin:/usr/local/cargo/bin:${PATH}"

USER sandbox
WORKDIR /workspace
CMD ["/bin/bash"]
