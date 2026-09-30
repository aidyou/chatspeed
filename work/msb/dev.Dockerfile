# Install the JavaScript toolchain in a separate stage so npm's temporary
# metadata and cache never become part of the final image.
FROM node:24-bookworm-slim AS node-tools

ARG PNPM_VERSION=10.5.2

RUN npm config set registry https://registry.npmmirror.com \
    && npm config set fetch-retries 5 \
    && npm config set fetch-retry-mintimeout 20000 \
    && npm config set fetch-retry-maxtimeout 120000 \
    && npm config set fetch-timeout 300000 \
    && npm install --global --prefix /opt/node-tools \
        "pnpm@${PNPM_VERSION}" \
        "create-vue@latest" \
        "eslint@latest" \
        "prettier@latest" \
        "typescript@latest" \
        "vue-tsc@latest" \
        "eslint-plugin-vue@latest" \
        "@typescript-eslint/parser@latest" \
        "@typescript-eslint/eslint-plugin@latest" \
    && npm cache clean --force

FROM node:24-bookworm-slim

COPY --from=node-tools /opt/node-tools /opt/node-tools

# Expose the preinstalled JavaScript tools through the standard executable path.
RUN ln -s /opt/node-tools/bin/pnpm /usr/local/bin/pnpm \
    && ln -s /opt/node-tools/bin/pnpx /usr/local/bin/pnpx \
    && ln -s /opt/node-tools/bin/eslint /usr/local/bin/eslint \
    && ln -s /opt/node-tools/bin/prettier /usr/local/bin/prettier \
    && ln -s /opt/node-tools/bin/tsc /usr/local/bin/tsc \
    && ln -s /opt/node-tools/bin/vue-tsc /usr/local/bin/vue-tsc \
    && ln -s /opt/node-tools/bin/create-vue /usr/local/bin/create-vue

# Keep the runtime image free of compiler toolchains. Project dependencies that
# need native compilation should install their own build image or toolchain.
RUN sed -i \
        -e 's|deb.debian.org/debian|mirrors.aliyun.com/debian|g' \
        -e 's|security.debian.org/debian-security|mirrors.aliyun.com/debian-security|g' \
        /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        coreutils \
        curl \
        diffutils \
        file \
        findutils \
        gawk \
        git \
        jq \
        less \
        patch \
        php-cli \
        python3 \
        python3-pip \
        sed \
        sqlite3 \
        tar \
        unzip \
        xz-utils \
        zip \
    && rm -rf /var/lib/apt/lists/* \
    && if id node >/dev/null 2>&1; then userdel --remove node; fi \
    && if getent group node >/dev/null 2>&1; then groupdel node; fi \
    && adduser --uid 1000 --shell /bin/bash sandbox \
    && ln -s /usr/bin/python3 /usr/local/bin/python \
    && install -d -o sandbox -g sandbox /workspace /home/sandbox/.cache

# Debian's package manager supplies Python and PHP; use domestic mirrors for
# package installation while keeping HTTPS certificate verification enabled.
ENV PATH="/opt/node-tools/bin:${PATH}" \
    NPM_CONFIG_REGISTRY=https://registry.npmmirror.com \
    PIP_INDEX_URL=https://mirrors.aliyun.com/pypi/simple/ \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    GIT_PAGER=cat \
    PAGER=cat \
    GIT_EDITOR=true \
    SHELL=/bin/bash

USER sandbox
WORKDIR /workspace
CMD ["/bin/bash"]
