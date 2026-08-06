ARG NODE_BASE=node:24-bookworm-slim
FROM ${NODE_BASE}

ARG PNPM_VERSION=10.14.0
ARG YARN_VERSION=4.9.2
ARG USE_CN_MIRRORS=0

ENV npm_config_manage_package_manager_versions=false

# glibc and native build tools cover substantially more npm packages than Alpine/musl.
RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|deb.debian.org/debian|mirrors.aliyun.com/debian|g' \
            -e 's|security.debian.org/debian-security|mirrors.aliyun.com/debian-security|g' \
            /etc/apt/sources.list.d/debian.sources; \
    fi \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        build-essential \
        ca-certificates \
        curl \
        git \
        openssh-client \
        python3 \
    && rm -rf /var/lib/apt/lists/* \
    && corepack disable \
    && if [ "$USE_CN_MIRRORS" = "1" ]; then \
        npm config set registry https://registry.npmmirror.com; \
    fi \
    && npm install --global \
        "@yarnpkg/cli-dist@${YARN_VERSION}" \
        "pnpm@${PNPM_VERSION}" \
    && pnpm --version \
    && yarn --version \
    && install -d -o node -g node /workspace

USER node
WORKDIR /workspace
CMD ["node"]
