# The Phase 3E Bowling task environment.
#
# One image serves both Bowling task directories: the deterministic runner task
# (`harbor/`, network-none) and the ChatSpeed candidate task
# (`harbor-chatspeed/`, controlled network to the operator's local proxy). The
# glibc base is required because the candidate smoke hosts the ChatSpeed
# control-plane binaries (`chatspeed-headless` links the full Tauri stack, so
# musl/Alpine bases cannot run them).
#
# Everything is copied in at build time and digest-verified by the staging
# script (`tools/harbor/build_bowling_task.py`); nothing is downloaded inside
# the task. Raw task/test content only ever exists in this image and in the
# ephemeral staging directory, never in a published artifact.
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

# The manifest's fixed runner argv starts with `python`; the distro only ships
# `python3`, so the alias is provided by the image instead of the adapter.
RUN ln -s /usr/bin/python3 /usr/local/bin/python

# A dedicated non-root user runs the fixed unittest: `/tests` is root-owned and
# read-only for it, so a candidate cannot modify the public tests it is judged
# against.
RUN useradd --user-group --create-home --shell /bin/sh candidate

# The ChatSpeed control-plane binaries under test for the candidate smoke.
# They are copied in by the build (never downloaded inside the task).
COPY environment/chatspeed-headless /usr/local/bin/chatspeed-headless
COPY environment/cs /usr/local/bin/cs
RUN chmod 0755 /usr/local/bin/chatspeed-headless /usr/local/bin/cs

# The declared roots. `/tests` is the read-only public-test root, `/workspace`
# the candidate workspace, `/opt/bowling` the digest-bound task metadata and
# reference candidate, `/opt/verifier` the independent verifier's own modules,
# and `/logs/artifacts` the Harbor artifact root.
RUN install -d -o candidate -g candidate -m 0775 /workspace \
    && install -d -m 0755 /opt/bowling /opt/verifier /installed-agent \
    && install -d -m 0555 /tests \
    && install -d -m 0750 /logs/artifacts

# The public test file and the verifier entrypoint live in `/tests`; Harbor's
# separate verifier expects the image to own `/tests/test.sh`.
COPY task-package/bowling_test.py /tests/bowling_test.py
COPY tests/test.sh /tests/test.sh

# Digest-bound task metadata and candidates (root-owned, read-only).
COPY task-package/manifest.json /opt/bowling/manifest.json
COPY task-package/bowling_reference.py /opt/bowling/bowling_reference.py
COPY task-package/bowling_contract.py /opt/verifier/bowling_contract.py
COPY tests/bowling_verify.py /opt/verifier/bowling_verify.py
RUN chmod 0444 /tests/bowling_test.py /tests/test.sh \
    /opt/bowling/manifest.json /opt/bowling/bowling_reference.py \
    /opt/verifier/bowling_contract.py /opt/verifier/bowling_verify.py \
    && chown -R root:root /tests /opt/bowling /opt/verifier

# The workspace is a Git repository so the run's output has a base revision to
# diff against. The safe.directory exception is needed because /workspace is
# owned by the candidate user while the image build runs as root.
RUN git config --global --add safe.directory /workspace \
    && git init -q /workspace \
    && git -C /workspace -c user.email=smoke@example.invalid -c user.name=smoke \
        commit -q --allow-empty -m base \
    && chown -R candidate:candidate /workspace

WORKDIR /workspace
