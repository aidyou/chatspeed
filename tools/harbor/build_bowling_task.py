"""Builds the Phase 3E Bowling task image with digest-verified staging.

The image context is assembled fresh on every build from exactly two sources:

* the frozen manifest (``work/agent-cli-polyglot-benchmark/manifest.json``),
  re-validated through the strict contract; and
* the locked local snapshot, whose Bowling files are re-hashed and compared
  against the manifest before anything is copied.

Raw task/test content only exists inside the ephemeral staging directory and
the built image; the staging directory is removed after the build. The script
is idempotent and never touches the old ChatSpeed smoke files.
"""

from __future__ import annotations

import argparse
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from polyglot_bowling_contract import load_manifest  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
TASK_DIR = REPO_ROOT / "work/agent-cli-polyglot-benchmark/harbor"
DEFAULT_SNAPSHOT = Path("/home/xc/下载/polyglot-benchmark-main")
DEFAULT_BINARIES = REPO_ROOT / "dev_data/2gh-harbor/context"
IMAGE_TAG = "chatspeed-bowling-task:3e"


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _tree_digest(root: Path) -> str:
    """Match the manifest's deterministic GNU tar tree digest."""
    command = [
        "tar",
        "--sort=name",
        "--mtime=@0",
        "--owner=0",
        "--group=0",
        "--numeric-owner",
        "--format=gnu",
        "-cf",
        "-",
        "-C",
        str(root),
        ".",
    ]
    try:
        result = subprocess.run(command, check=True, stdout=subprocess.PIPE)
    except (OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"failed to compute deterministic task digest: {error}") from error
    return hashlib.sha256(result.stdout).hexdigest()


def _stage_verified_task_files(
    manifest: dict, snapshot_root: Path, staging: Path
) -> None:
    """Copies the Bowling files whose digests match the frozen manifest."""
    task_package = staging / "task-package"
    task_package.mkdir(parents=True)
    relative = manifest["task"]["relative_path"]
    expected = {entry["path"]: entry["sha256"] for entry in manifest["task"]["files"]}
    task_root = snapshot_root / relative
    if not task_root.is_dir() or _tree_digest(task_root) != manifest["task"]["tree_digest"][7:]:
        raise SystemExit("the local Bowling task tree does not match the frozen manifest")
    for source_rel, expected_digest in expected.items():
        source = task_root / source_rel
        if not source.is_file() or _sha256(source) != expected_digest:
            raise SystemExit(
                f"source digest mismatch for '{source_rel}': the local snapshot "
                "does not match the frozen manifest (AC-1 fail closed)"
            )
    required = {
        "bowling_test.py": "bowling_test.py",
        ".meta/example.py": "bowling_reference.py",
    }
    for source_rel, staged_name in required.items():
        source = snapshot_root / relative / source_rel
        if not source.is_file() or _sha256(source) != expected[source_rel]:
            raise SystemExit(
                f"source digest mismatch for '{source_rel}': the local snapshot "
                "does not match the frozen manifest (AC-1 fail closed)"
            )
        shutil.copyfile(source, task_package / staged_name)
    shutil.copyfile(
        REPO_ROOT / "work/agent-cli-polyglot-benchmark/manifest.json",
        task_package / "manifest.json",
    )
    shutil.copyfile(
        Path(__file__).resolve().parent / "polyglot_bowling_contract.py",
        task_package / "bowling_contract.py",
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    stage = sub.add_parser(
        "stage", help="assemble the digest-verified build context and print it"
    )
    stage.add_argument(
        "--snapshot-root", type=Path, default=DEFAULT_SNAPSHOT,
        help="the locked local polyglot-benchmark snapshot",
    )
    stage.add_argument(
        "--binaries-dir", type=Path, default=DEFAULT_BINARIES,
        help="directory carrying the prebuilt chatspeed-headless and cs binaries",
    )

    cleanup = sub.add_parser(
        "cleanup", help="remove the ephemeral staging directory (raw task content)"
    )

    args = parser.parse_args(argv)
    if args.command == "stage":
        return _stage(args.snapshot_root, args.binaries_dir)
    _cleanup_staging()
    return 0


def _stage(snapshot_root: Path, binaries_dir: Path) -> int:
    manifest = load_manifest(str(REPO_ROOT / "work/agent-cli-polyglot-benchmark/manifest.json"))
    for binary in ("chatspeed-headless", "cs"):
        if not (binaries_dir / binary).is_file():
            raise SystemExit(f"missing prebuilt binary '{binary}' under {binaries_dir}")

    staging = TASK_DIR / "build"
    if staging.exists():
        shutil.rmtree(staging)
    _stage_verified_task_files(manifest, snapshot_root, staging)
    environment = staging / "environment"
    environment.mkdir()
    for binary in ("chatspeed-headless", "cs"):
        shutil.copyfile(binaries_dir / binary, environment / binary)
    shutil.copyfile(TASK_DIR / "environment/Dockerfile", environment / "Dockerfile")
    shutil.copytree(TASK_DIR / "tests", staging / "tests")
    print(staging)
    return 0


def _cleanup_staging() -> None:
    # The staging directory carries raw task/test content; it never stays in
    # the repository (INV-4).
    shutil.rmtree(TASK_DIR / "build", ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
