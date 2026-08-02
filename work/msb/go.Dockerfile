FROM golang:1.26-bookworm AS gf-builder

ARG GF_VERSION=v2.10.2
RUN GOBIN=/out go install "github.com/gogf/gf/cmd/gf/v2@${GF_VERSION}"

FROM golang:1.26-bookworm

COPY --from=gf-builder /out/gf /usr/local/bin/gf

# Build GoFrame and CGO projects on both amd64 and arm64.
RUN apt-get update \
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
