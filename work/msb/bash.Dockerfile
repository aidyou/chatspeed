FROM alpine:3.22

ARG USE_CN_MIRRORS=0

# Common shell profile with GNU-compatible tools for agent-generated commands.
RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|https://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            -e 's|http://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            /etc/apk/repositories; \
    fi \
    && apk add --no-cache \
    bash \
    ca-certificates \
    coreutils \
    diffutils \
    file \
    findutils \
    gawk \
    git \
    jq \
    less \
    patch \
    sed \
    sqlite \
    tar \
    unzip \
    xz \
    zip \
    && adduser -D -u 1000 sandbox \
    && install -d -o sandbox -g sandbox /workspace

USER sandbox
WORKDIR /workspace
CMD ["/bin/bash"]
