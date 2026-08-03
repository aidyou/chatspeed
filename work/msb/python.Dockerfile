FROM python:3.12-alpine3.22

# Small profile for scripts and packages that publish musllinux wheels.
RUN apk add --no-cache \
        bash \
        ca-certificates \
        curl \
        git \
    && adduser -D -u 1000 sandbox \
    && mkdir -p /workspace \
    && chown sandbox:sandbox /workspace

ENV PATH="/home/sandbox/.local/bin:${PATH}" \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

USER sandbox
WORKDIR /workspace
CMD ["python3"]
