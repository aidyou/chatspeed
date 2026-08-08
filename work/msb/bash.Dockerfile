FROM python:3.12-alpine3.22

# 避免产生 pyc 文件 & 保持 Python 标准输出无缓冲区
ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    SHELL=/bin/bash

ARG USE_CN_MIRRORS=0

# 安装 GNU 兼容工具链包、创建非 root 用户与工作目录
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

SHELL ["/bin/bash", "-c"]

USER sandbox
WORKDIR /workspace

CMD ["/bin/bash"]
