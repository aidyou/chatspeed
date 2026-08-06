FROM php:8.3-cli-alpine3.22

ARG USE_CN_MIRRORS=0

COPY --from=composer:2.8 /usr/bin/composer /usr/local/bin/composer

# PDO drivers cover MySQL/MariaDB and PostgreSQL without shipping database servers.
RUN if [ "$USE_CN_MIRRORS" = "1" ]; then \
        sed -i \
            -e 's|https://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            -e 's|http://dl-cdn.alpinelinux.org/alpine|https://mirrors.aliyun.com/alpine|g' \
            /etc/apk/repositories; \
    fi \
    && apk add --no-cache \
        bash \
        ca-certificates \
        curl \
        git \
        libpq \
        libzip \
        unzip \
    && apk add --no-cache --virtual .build-deps \
        $PHPIZE_DEPS \
        libzip-dev \
        postgresql-dev \
    && docker-php-ext-install -j"$(nproc)" pdo_mysql pdo_pgsql zip \
    && apk del .build-deps \
    && adduser -D -u 1000 sandbox \
    && mkdir -p /workspace \
    && chown sandbox:sandbox /workspace

USER sandbox
WORKDIR /workspace
CMD ["php", "-a"]
