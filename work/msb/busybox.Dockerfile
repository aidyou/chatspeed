FROM busybox:1.36.1-musl

# Minimal profile for POSIX shell checks and simple filesystem operations.
RUN mkdir -p /workspace && chown 1000:1000 /workspace

USER 1000:1000
WORKDIR /workspace
CMD ["sh"]
