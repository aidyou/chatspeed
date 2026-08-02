ARG NODE_BASE=node:24-bookworm-slim
FROM ${NODE_BASE}

ARG PNPM_VERSION=10.14.0
ARG YARN_VERSION=4.9.2

ENV npm_config_manage_package_manager_versions=false

# glibc and native build tools cover substantially more npm packages than Alpine/musl.
RUN apt-get update \
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
    && npm install --global \
        "@yarnpkg/cli-dist@${YARN_VERSION}" \
        "pnpm@${PNPM_VERSION}" \
    && pnpm --version \
    && yarn --version \
    && install -d -o node -g node /workspace

USER node
WORKDIR /workspace
CMD ["node"]
