FROM alpine:3.22

ARG USE_CN_MIRRORS=0
ENV GIT_PAGER=cat \
    PAGER=cat \
    GIT_EDITOR=true

RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|https://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            -e 's|http://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            /etc/apk/repositories; \
    fi \
    && apk add --no-cache \
    ca-certificates \
    git \
    && adduser -D -u 1000 sandbox \
    && install -d -o sandbox -g sandbox /workspace

USER sandbox
WORKDIR /workspace
CMD ["/bin/sh"]
