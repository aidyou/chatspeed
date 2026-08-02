FROM alpine:3.22

# Common shell profile with GNU-compatible tools for agent-generated commands.
RUN apk add --no-cache \
    bash \
    ca-certificates \
    coreutils \
    curl \
    diffutils \
    file \
    findutils \
    gawk \
    git \
    grep \
    jq \
    less \
    openssh-client \
    patch \
    ripgrep \
    sed \
    tar \
    unzip \
    wget \
    xz \
    zip \
    && adduser -D -u 1000 sandbox \
    && install -d -o sandbox -g sandbox /workspace

USER sandbox
WORKDIR /workspace
CMD ["/bin/bash"]
