FROM python:3.12-slim-bookworm

ARG USE_CN_MIRRORS=0

# Compatibility profile for packages that need glibc or native extensions.
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
        libffi-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && install -d -o sandbox -g sandbox /workspace

ENV PATH="/home/sandbox/.local/bin:${PATH}" \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

USER sandbox
WORKDIR /workspace
CMD ["python3"]
