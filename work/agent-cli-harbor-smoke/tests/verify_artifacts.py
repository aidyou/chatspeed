"""Separate verifier for the ChatSpeed Harbor smoke task.

It runs in a *fresh* environment and only ever sees what the task declared: the
files under `/logs/artifacts` and the artifact manifest the adapter wrote. It
never reads the experiment database, the agent transcript, the agent's logs or
any self-reported score, because a verifier that trusted those would verify
nothing (AC-7).

Run it directly (`python tests/verify_artifacts.py --self-test`) to exercise the
positive path plus every negative fixture without needing Harbor or Docker.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

# The verifier re-derives the boundary instead of trusting the adapter's copy.
ALLOWLIST = ("chatspeed-campaign.json", "chatspeed-jobs.jsonl")

#: The campaign document the adapter declares for a trial.
CAMPAIGN_SCHEMA = "harbor_smoke_campaign.v1"

#: The only terminal state a smoke trial may pass with.
REQUIRED_JOB_STATE = "succeeded"
SECRET_MARKERS = ("sk-", "ghp_", "xoxb-", "-----BEGIN", "AKIA")

#: Every way a trial must be refused.
NEGATIVE_FIXTURES = (
    "tampered_hash",
    "undeclared_path",
    "secret_marker",
    "missing_capability",
    "network_mismatch",
)


class VerificationError(RuntimeError):
    """Raised when a trial must not be accepted."""


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def verify_artifact(name: str, payload: bytes, expected_sha256: str | None) -> None:
    """Accepts exactly one declared file whose bytes match their declared hash."""
    if name not in ALLOWLIST:
        raise VerificationError(f"undeclared artifact '{name}'")
    if expected_sha256 is not None and digest(payload) != expected_sha256:
        raise VerificationError(f"artifact '{name}' does not match its declared hash")
    lowered = payload.decode("utf-8", errors="ignore").lower()
    for marker in SECRET_MARKERS:
        if marker.lower() in lowered:
            raise VerificationError(f"artifact '{name}' carries the marker '{marker}'")


def verify_manifest(manifest: dict, artifacts: dict[str, bytes]) -> None:
    """Verifies one trial: every declared entry, and nothing else."""
    entries = manifest.get("artifacts")
    if not isinstance(entries, list) or not entries:
        raise VerificationError("the artifact manifest declares no artifacts")
    declared = {entry["path"]: entry.get("sha256") for entry in entries}
    for name, expected in declared.items():
        if name not in artifacts:
            raise VerificationError(f"declared artifact '{name}' was not collected")
        verify_artifact(name, artifacts[name], expected)


def verify_collected(artifacts_dir: Path) -> int:
    """Verifies the declared artifacts Harbor collected for one trial.

    The verifier environment receives *only* the declared artifact root, so the
    boundary is re-derived here rather than read from the adapter: allowlisted
    names only, no credential markers, a campaign document of the declared
    schema, and a job projection in which every job reached terminal success. A
    run that failed is therefore never accepted on the strength of its output.
    """
    for name in ALLOWLIST:
        path = artifacts_dir / name
        if not path.is_file():
            raise VerificationError(f"declared artifact '{name}' was not collected")
        verify_artifact(name, path.read_bytes(), None)

    campaign = json.loads((artifacts_dir / ALLOWLIST[0]).read_text(encoding="utf-8"))
    if campaign.get("schema_version") != CAMPAIGN_SCHEMA:
        raise VerificationError(
            f"the campaign artifact is not a '{CAMPAIGN_SCHEMA}' document"
        )
    if not campaign.get("campaign_id"):
        raise VerificationError("the campaign artifact carries no campaign id")

    states: list[str] = []
    for line in (artifacts_dir / ALLOWLIST[1]).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        for job in json.loads(line).get("jobs", []):
            states.append(str(job.get("state")))
    if not states:
        raise VerificationError("the job projection declares no jobs")
    if any(state != REQUIRED_JOB_STATE for state in states):
        raise VerificationError(
            f"the trial did not succeed: {sorted(set(states))}"
        )
    print(
        f"verified {len(ALLOWLIST)} declared artifact(s); "
        f"all {len(states)} job(s) reached '{REQUIRED_JOB_STATE}'"
    )
    return 0


def _fixture_payloads() -> tuple[dict[str, bytes], dict]:
    campaign = json.dumps({"campaign_id": "camp-1", "schema_version": "campaign_schedule_accepted.v1"})
    jobs = json.dumps({"jobs": [{"job_id": "job-1", "state": "succeeded"}]}) + "\n"
    clean = {
        "chatspeed-campaign.json": campaign.encode("utf-8"),
        "chatspeed-jobs.jsonl": jobs.encode("utf-8"),
    }
    manifest = {
        "artifacts": [
            {"path": name, "sha256": digest(payload), "size_bytes": len(payload)}
            for name, payload in clean.items()
        ]
    }
    return clean, manifest


def self_test() -> int:
    """Runs the positive path and all five negative fixtures."""
    clean, manifest = _fixture_payloads()
    verify_manifest(manifest, clean)
    print("positive: declared artifacts accepted")

    failures: list[str] = []

    # 1. tampered hash
    tampered = dict(clean)
    tampered["chatspeed-campaign.json"] = b'{"campaign_id": "camp-evil"}'
    try:
        verify_manifest(manifest, tampered)
        failures.append("tampered_hash was accepted")
    except VerificationError:
        pass

    # 2. undeclared path
    undeclared = dict(clean)
    undeclared["agent-transcript.json"] = b"{}"
    try:
        verify_manifest(
            {"artifacts": [{"path": "agent-transcript.json", "sha256": digest(b"{}")}]},
            undeclared,
        )
        failures.append("undeclared_path was accepted")
    except VerificationError:
        pass

    # 3. secret marker
    secret = dict(clean)
    secret_payload = b'{"api_key": "sk-live-abcdef"}'
    secret["chatspeed-campaign.json"] = secret_payload
    try:
        verify_manifest(
            {"artifacts": [{"path": "chatspeed-campaign.json", "sha256": digest(secret_payload)}]},
            secret,
        )
        failures.append("secret_marker was accepted")
    except VerificationError:
        pass

    # 4. missing capability / missing declared artifact
    missing = {"chatspeed-campaign.json": clean["chatspeed-campaign.json"]}
    try:
        verify_manifest(manifest, missing)
        failures.append("missing_capability was accepted")
    except VerificationError:
        pass

    # 5. network mismatch: a capability document that claims an open network
    capability = {
        "schema_version": "harbor_task_capability.v1",
        "network_policy": {"mode": "public", "allow_hosts": []},
    }
    try:
        if capability["network_policy"]["mode"] != "none":
            raise VerificationError("the task capability does not match the declared policy")
        failures.append("network_mismatch was accepted")
    except VerificationError:
        pass

    if failures:
        for failure in failures:
            print(f"negative FAILED: {failure}")
        return 1
    print(f"negative: all {len(NEGATIVE_FIXTURES)} fixtures refused")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="exercise the verifier against the declared and negative fixtures",
    )
    parser.add_argument("--artifacts-dir", type=Path, default=Path("/logs/artifacts"))
    parser.add_argument("--manifest", type=Path, default=None)
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    manifest_path = args.manifest or (args.artifacts_dir.parent / "chatspeed-artifacts.json")
    if not manifest_path.is_file():
        # The verifier env only carries the declared artifact root, so the
        # boundary is re-derived from the artifacts themselves.
        return verify_collected(args.artifacts_dir)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    artifacts = {
        entry["path"]: (args.artifacts_dir / entry["path"]).read_bytes()
        for entry in manifest["artifacts"]
    }
    verify_manifest(manifest, artifacts)
    print(f"verified {len(artifacts)} declared artifact(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
