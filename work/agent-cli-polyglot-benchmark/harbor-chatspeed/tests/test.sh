#!/bin/sh
# The separate Bowling verifier entrypoint.
#
# Harbor runs this script in a *fresh* verifier environment that only receives
# the declared artifacts (`/logs/artifacts`) and this image-owned `/tests`
# (the verifier scripts themselves live in `/opt/verifier`, mounted from the
# same image). It recomputes the result classification from artifact bytes,
# cross-checks the runner evidence against the raw runner output, and writes
# the evaluation/verdict sidecars into /logs/verifier — which Harbor downloads
# back to the host trial directory. Nothing the agent or the runner reported
# about itself is trusted on its own (AC-4).
set -u

mkdir -p /logs/verifier

if ! command -v python3 >/dev/null 2>&1; then
    echo "the verifier environment provides no python3" >&2
    echo 0 > /logs/verifier/reward.txt
    exit 1
fi

if python3 /opt/verifier/bowling_verify.py \
        --artifacts-dir /logs/artifacts \
        --manifest /opt/bowling/manifest.json; then
    echo 1 > /logs/verifier/reward.txt
    exit 0
fi

echo "bowling verification refused the trial" >&2
echo 0 > /logs/verifier/reward.txt
exit 1
