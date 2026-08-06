FROM alpine:3.22

# Alpine is the smallest supported base with both curl and wget available.
RUN apk add --no-cache \
    ca-certificates \
    curl \
    wget \
    && adduser -D -u 1000 sandbox \
    && install -d -o sandbox -g sandbox /workspace

USER sandbox
WORKDIR /workspace
CMD ["/bin/sh"]
