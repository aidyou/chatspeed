# The ChatSpeed Harbor smoke task environment.
#
# It provisions exactly the roots `tools/harbor/artifact_contract.py` declares,
# because `HarborTaskOwner::preflight` refuses a capability whose declared roots
# do not exist: the task root, the workspace the run mutates, the read-only
# fixture root the separate verifier works from, and the Harbor artifact root.
#
# The ChatSpeed binaries under test are copied in by the build (they are never
# downloaded inside the task), so the base image must be able to run them:
#
#   * glibc >= 2.38  — `cs` is linked against `GLIBC_2.38` on the build host;
#   * GTK3 + WebKitGTK 4.1 — `chatspeed-headless` links the full `chatspeed_lib`,
#                      which pulls in the Tauri backend (`libgdk-3`,
#                      `libwebkit2gtk-4.1`) even though the windowless runtime
#                      never opens a window.
#
# Alpine/musl bases and glibc < 2.38 bases cannot host these binaries, so a
# Harbor trial must start from a base like the one below.
#
# Measured on this host: `ubuntu:26.04` (glibc 2.43) runs `cs`, and the only
# missing runtime is the Tauri stack — installed here from the distribution
# mirror, which is reachable from the sandbox even when the Docker registry is
# not.
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        libgtk-3-0 \
        libwebkit2gtk-4.1-0 \
        python3 \
    && rm -rf /var/lib/apt/lists/*

# The declared roots. `/workspace` and `/tests` belong to the task; the adapter
# owns `/installed-agent` (its domain, plan and capability manifest) and Harbor
# owns `/logs/artifacts`. `/tests` is deliberately empty here: the separate
# verifier runs in its own fresh environment with the task's own `tests/`
# fixtures, so nothing the agent could write is ever visible to it.
RUN install -d -m 0755 /workspace \
    && install -d -m 0755 /tests \
    && install -d -m 0700 /installed-agent \
    && install -d -m 0750 /logs/artifacts

# The ChatSpeed binaries under test. They are copied in at build time; the
# adapter only verifies them and never downloads anything inside the task.
COPY environment/chatspeed-headless /usr/local/bin/chatspeed-headless
COPY environment/cs /usr/local/bin/cs
RUN chmod 0755 /usr/local/bin/chatspeed-headless /usr/local/bin/cs

# The workspace is a Git repository: the harbor-task owner collects the run's
# output as a patch inside it, which requires a base revision to diff against.
RUN git init -q /workspace \
    && git -C /workspace -c user.email=smoke@example.invalid -c user.name=smoke \
        commit -q --allow-empty -m base

# Harbor's separate verifier runs the task's own `/tests/test.sh` in a fresh
# environment, and it expects the image to own that file (the verifier never
# sees anything the agent produced).
COPY tests/test.sh /tests/test.sh
COPY tests/verify_artifacts.py /tests/verify_artifacts.py
RUN chmod 0755 /tests/test.sh

WORKDIR /workspace