FROM golang:1.26-bookworm AS gf-builder

ARG GF_VERSION=v2.10.2
ARG USE_CN_MIRRORS=0
RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        go env -w GOPROXY=https://goproxy.cn,direct; \
    fi \
    && GOBIN=/out go install "github.com/gogf/gf/cmd/gf/v2@${GF_VERSION}"

FROM golang:1.26-bookworm

ARG USE_CN_MIRRORS=0

COPY --from=gf-builder /out/gf /usr/local/bin/gf

# Build GoFrame and CGO projects on both amd64 and arm64.
RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|deb.debian.org/debian|mirrors.aliyun.com/debian|g' \
            -e 's|security.debian.org/debian-security|mirrors.aliyun.com/debian-security|g' \
            /etc/apt/sources.list.d/debian.sources; \
        go env -w GOPROXY=https://goproxy.cn,direct; \
    fi \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && install -d -o sandbox -g sandbox /workspace

ENV GOPATH=/home/sandbox/go \
    PATH="/home/sandbox/go/bin:${PATH}"

USER sandbox
WORKDIR /workspace
CMD ["go"]
