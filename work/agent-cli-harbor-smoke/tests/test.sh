#!/bin/sh
# The separate verifier entrypoint.
#
# Harbor runs this script in a *fresh* verifier environment that only receives
# the declared artifacts (`/logs/artifacts`) and this task's `tests/` directory.
# It recomputes the artifact boundary from the declared manifest and writes the
# reward, so nothing the agent reported about itself is trusted (AC-7).
set -u

mkdir -p /logs/verifier

if ! command -v python3 >/dev/null 2>&1; then
    echo "the verifier environment provides no python3" >&2
    echo 0 > /logs/verifier/reward.txt
    exit 1
fi

if python3 /tests/verify_artifacts.py \
        --artifacts-dir /logs/artifacts \
        --manifest /logs/chatspeed-artifacts.json; then
    echo 1 > /logs/verifier/reward.txt
    exit 0
fi

echo "artifact verification refused the trial" >&2
echo 0 > /logs/verifier/reward.txt
exit 1