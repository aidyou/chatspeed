"""The strict artifact and capability boundary of the ChatSpeed Harbor adapter.

This module is the only place that decides three things, and it decides them
from declared data rather than from anything the run reports about itself:

1. **What may leave the sandbox.** `DECLARED_ARTIFACTS` is the allowlist; a file
   outside it is never copied, however useful it looks (AC-7).
2. **What a published artifact is.** Every declared file is re-hashed here, and
   a missing or changed file fails the trial instead of being published (§V-8).
3. **What the sandbox is.** `capability_manifest` produces the
   `harbor_task_capability.v1` document the headless runtime requires before it
   will treat this task environment as an execution owner.

Nothing here reads the agent's logs, its self-reported score or its transcript:
a verifier that trusted those would not be a verifier.
"""

from __future__ import annotations

import hashlib
import json
import shlex
import sys
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Awaitable, Callable

#: Where Harbor collects published artifacts.
ARTIFACT_ROOT = PurePosixPath("/logs/artifacts")

#: The isolated headless instance's experiment data directory. It is the same
#: value the adapter passes to `--data-dir`, and it is the directory the runtime
#: derives every one of its own paths from.
DOMAIN_ROOT = PurePosixPath("/installed-agent/chatspeed-domain")

#: Where the adapter proves this sandbox to the headless runtime.
#:
#: The runtime resolves this as `<data-dir>/runtime/harbor-task-capability.json`
#: (`chatspeed::headless::scheduler_runtime::harbor_capability_path`), so the
#: manifest must be written *inside the domain*, not next to it: a manifest the
#: runtime never reads leaves every Harbor-owner job refused with
#: `ownership_mismatch`.
CAPABILITY_FILE = DOMAIN_ROOT / "runtime" / "harbor-task-capability.json"

#: The discovery document the instance publishes, for the CLI's `--discovery-file`.
DISCOVERY_FILE = DOMAIN_ROOT / "runtime" / "control-plane-v1.json"

#: The schema version of the capability document (mirrors the Rust DTO).
CAPABILITY_SCHEMA_VERSION = "harbor_task_capability.v1"

#: The task environment's roots, as the adapter declares them.
TASK_ROOT = PurePosixPath("/installed-agent")
WORKSPACE_ROOT = PurePosixPath("/workspace")
READ_ONLY_ROOTS = (PurePosixPath("/tests"),)

#: Files that may be published, relative to the workspace. A run cannot widen
#: this set, and a file outside it is refused rather than copied.
DECLARED_ARTIFACTS: tuple[str, ...] = (
    "chatspeed-campaign.json",
    "chatspeed-jobs.jsonl",
)

#: Markers that must never appear in a published artifact (INV-6).
SECRET_MARKERS: tuple[str, ...] = ("sk-", "ghp_", "xoxb-", "-----BEGIN", "AKIA")


class ArtifactBoundaryError(RuntimeError):
    """Raised when an artifact would cross the declared boundary."""


def declared_artifacts() -> list[Path]:
    """The absolute paths this adapter is allowed to publish."""
    return [Path(ARTIFACT_ROOT, name) for name in DECLARED_ARTIFACTS]


def capability_manifest(
    environment: Any,
    *,
    nonce: str | None = None,
    task_id: str | None = None,
) -> dict[str, Any]:
    """Builds the capability document for this task environment.

    The owner token hash binds the document to this sandbox instance; the
    headless runtime re-derives it and refuses any path outside the declared
    roots, so this file is the sandbox's identity, not a hint.
    """
    token_source = f"{task_id or ''}:{nonce or ''}:{WORKSPACE_ROOT}"
    owner_token_hash = hashlib.sha256(token_source.encode("utf-8")).hexdigest()
    return {
        "schema_version": CAPABILITY_SCHEMA_VERSION,
        "task_id": task_id or "harbor-task",
        "nonce": nonce or "harbor-nonce",
        "owner_token_hash": owner_token_hash,
        "task_root": str(TASK_ROOT),
        "workspace_root": str(WORKSPACE_ROOT),
        "artifact_roots": [str(ARTIFACT_ROOT)],
        "network_policy": {"mode": "none", "allow_hosts": []},
        "read_only_roots": [str(root) for root in READ_ONLY_ROOTS],
        "issued_at": datetime.now(timezone.utc).isoformat(),
    }


def _digest_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _assert_no_secret(name: str, payload: bytes) -> None:
    text = payload.decode("utf-8", errors="ignore").lower()
    for marker in SECRET_MARKERS:
        if marker.lower() in text:
            raise ArtifactBoundaryError(
                f"artifact '{name}' carries the credential marker '{marker}'"
            )


async def publish_artifacts(
    exec_as_root: Callable[..., Awaitable[Any]],
    environment: Any,
    logs_dir: Path,
) -> list[dict[str, Any]]:
    """Publishes the declared artifacts and records their hashes.

    The publication is *declared-only*: each declared path is read from the
    workspace, refused if it carries a credential marker, and copied into the
    Harbor artifact root. The returned manifest is what a separate verifier can
    recompute against.
    """
    published: list[dict[str, Any]] = []
    for relative in DECLARED_ARTIFACTS:
        target = ARTIFACT_ROOT / relative
        read = await exec_as_root(
            environment,
            command=f"base64 -w0 {shlex.quote(str(WORKSPACE_ROOT / relative))}",
        )
        if read.return_code != 0:
            raise ArtifactBoundaryError(
                f"declared artifact '{relative}' is missing from the run workspace"
            )
        import base64

        payload = base64.b64decode((read.stdout or "").strip() or b"")
        _assert_no_secret(relative, payload)
        digest = _digest_bytes(payload)
        write = await exec_as_root(
            environment,
            command=(
                f"install -d -m 0750 {ARTIFACT_ROOT} && "
                f"printf '%s' {shlex.quote((read.stdout or '').strip())} "
                f"| base64 -d > {shlex.quote(str(target))}"
            ),
        )
        if write.return_code != 0:
            raise ArtifactBoundaryError(f"failed to publish '{relative}'")
        published.append({"path": relative, "sha256": digest, "size_bytes": len(payload)})

    manifest_path = Path(logs_dir, "chatspeed-artifacts.json")
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps({"artifacts": published}, indent=2), encoding="utf-8")
    return published


def verify_published_artifact(manifest_entry: dict[str, Any], payload: bytes) -> None:
    """Re-verifies one published artifact against its declared hash.

    Used by the separate verifier and by the negative fixtures: a tampered file
    must fail here rather than be accepted on the strength of the agent's own
    report.
    """
    expected = manifest_entry.get("sha256")
    actual = _digest_bytes(payload)
    if expected != actual:
        raise ArtifactBoundaryError(
            f"artifact '{manifest_entry.get('path')}' does not match its declared hash"
        )
    _assert_no_secret(str(manifest_entry.get("path")), payload)


__all__ = [
    "ARTIFACT_ROOT",
    "ArtifactBoundaryError",
    "CAPABILITY_FILE",
    "CAPABILITY_SCHEMA_VERSION",
    "DECLARED_ARTIFACTS",
    "DISCOVERY_FILE",
    "DOMAIN_ROOT",
    "READ_ONLY_ROOTS",
    "TASK_ROOT",
    "WORKSPACE_ROOT",
    "capability_manifest",
    "declared_artifacts",
    "publish_artifacts",
    "verify_published_artifact",
]


def _declared_paths() -> dict[str, Any]:
    """The paths this contract fixes, for the runtime hand-off tests."""
    return {
        "artifact_root": str(ARTIFACT_ROOT),
        "capability_file": str(CAPABILITY_FILE),
        "discovery_file": str(DISCOVERY_FILE),
        "domain_root": str(DOMAIN_ROOT),
        "read_only_roots": [str(root) for root in READ_ONLY_ROOTS],
        "task_root": str(TASK_ROOT),
        "workspace_root": str(WORKSPACE_ROOT),
    }


def _main(argv: list[str]) -> int:
    """`paths` prints the fixed layout; `emit` prints one capability manifest.

    Only the standard library is used, so the runtime hand-off can be checked
    without installing Harbor.
    """
    if len(argv) >= 2 and argv[1] == "paths":
        print(json.dumps(_declared_paths(), indent=2, sort_keys=True))
        return 0
    if len(argv) >= 2 and argv[1] == "emit":
        task_id = argv[2] if len(argv) > 2 else "harbor-task"
        nonce = argv[3] if len(argv) > 3 else "harbor-nonce"
        manifest = capability_manifest(None, nonce=nonce, task_id=task_id)
        print(json.dumps(manifest, indent=2, sort_keys=True))
        return 0
    print("usage: artifact_contract.py {paths|emit <task_id> <nonce>}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(_main(sys.argv))
