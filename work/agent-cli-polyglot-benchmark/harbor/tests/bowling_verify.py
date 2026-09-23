"""The independent Phase 3E Bowling verifier.

Harbor runs this module in a fresh verifier environment that receives only the
declared artifacts and the image-owned frozen manifest. It never sees the
agent transcript, the durable experiment database, or anything the run
reported about itself beyond the declared artifact bytes.

The verifier re-derives everything:

1. The frozen manifest is re-validated strictly (identity, policy, license).
2. The published ``task_manifest.json`` must equal the image-owned manifest.
3. The raw runner output (``runner_stderr.txt``) is re-parsed, and the
   recomputed test facts must agree with the runner evidence — a mismatch is
   tamper, not a classification.
4. The strict contract re-check (``bowling_contract.verify_evidence``)
   recomputes artifact hashes, identity bindings, secret markers and the
   license gate.
5. The additive evaluation/verdict sidecars are written atomically into
   ``/logs/verifier`` (Harbor downloads that directory to the host trial).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from bowling_contract import (  # noqa: E402
    ContractError,
    RESULT_KIND,
    VERIFIER_ID,
    VERIFIER_VERSION,
    expected_chain_head,
    load_manifest,
    verify_evidence,
)

EVALUATION_SCHEMA = "bowling_evaluation.v1"
VERDICT_SCHEMA = "bowling_verdict.v1"

_DECLARED_EVIDENCE = "runner_result.json"
_OUTPUT_ARTIFACTS = ("runner_stdout.txt", "runner_stderr.txt", "task_manifest.json")
_RAN_RE = re.compile(r"^Ran (\d+) tests? in ", re.MULTILINE)
_OK_RE = re.compile(r"^OK( \(.+\))?$", re.MULTILINE)
_FAILED_RE = re.compile(r"^FAILED \((.+)\)$", re.MULTILINE)
_TIMEOUT_EXIT = 124
_NO_TESTS_EXIT = 5


def _sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _read_artifact(artifacts_dir: Path, name: str) -> bytes:
    path = artifacts_dir / name
    try:
        return path.read_bytes()
    except OSError as error:
        raise ContractError(
            "artifact_missing", f"declared artifact '{name}' cannot be read: {error}"
        ) from error


def _write_atomic(directory: Path, name: str, payload: bytes) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    tmp = directory / f".{name}.tmp"
    tmp.write_bytes(payload)
    os.replace(tmp, directory / name)


def _recompute_from_output(output_bytes: bytes, evidence: dict[str, Any]) -> str:
    """Recompute test facts from the runner output, fail closed on ambiguity."""
    output = output_bytes.decode("utf-8", errors="replace")
    exit_status = evidence["exit_status"]

    if evidence["timed_out"]:
        if exit_status != _TIMEOUT_EXIT:
            return "timed_out evidence does not carry the timeout exit status"
        return ""
    if evidence["infrastructure_error"]:
        if output.strip():
            return "infrastructure_error evidence carries runner output"
        return ""

    ran_matches = _RAN_RE.findall(output)
    if len(ran_matches) != 1:
        return "runner output must contain exactly one test summary"
    discovered = int(ran_matches[0])
    if discovered != evidence["tests"]["discovered"]:
        return "discovered test count disagrees with the raw runner output"
    if discovered == 0:
        if exit_status != _NO_TESTS_EXIT or not evidence["dependency_error"]:
            return "an empty discovery must be classified as a dependency error"
        return ""

    ok_matches = _OK_RE.findall(output)
    failed_matches = _FAILED_RE.findall(output)
    if len(ok_matches) + len(failed_matches) != 1:
        return "runner output must contain exactly one terminal test status"
    if ok_matches:
        if exit_status != 0:
            return "an OK runner output must carry exit status 0"
        if evidence["tests"]["failed"] != 0 or evidence["tests"]["errors"] != 0:
            return "an OK runner output cannot carry failures"
        return ""

    fields: dict[str, int] = {}
    for part in failed_matches[0].split(", "):
        key, _, value = part.partition("=")
        try:
            fields[key] = int(value)
        except ValueError:
            return "failure summary contains a non-integer count"
    if fields.get("failures", 0) != evidence["tests"]["failed"] or fields.get(
        "errors", 0
    ) != evidence["tests"]["errors"]:
        return "failure counts disagree with the raw runner output"
    if exit_status == 0:
        return "a failed runner output cannot carry exit status 0"
    return ""


def _build_evaluation(
    manifest: dict[str, Any],
    evidence: dict[str, Any],
    artifact_hashes: dict[str, str],
    verification: Any,
) -> dict[str, Any]:
    return {
        "schema_version": EVALUATION_SCHEMA,
        "dataset_id": manifest["dataset_id"],
        "dataset_version": manifest["dataset_version"],
        "split": manifest["split"],
        "task_id": manifest["task"]["task_id"],
        "snapshot_digest": manifest["snapshot"]["archive_digest"],
        "task_digest": manifest["task"]["tree_digest"],
        "runner_id": manifest["runner"]["id"],
        "runner_version": manifest["runner"]["version"],
        "verifier_id": VERIFIER_ID,
        "verifier_version": VERIFIER_VERSION,
        "license": {
            "status": manifest["license"]["status"],
            "publication": manifest["license"]["publication"],
        },
        "pollution": manifest["pollution"],
        "resource_profile": manifest["runner"]["resource_profile"],
        "evidence_sha256": artifact_hashes[_DECLARED_EVIDENCE],
        "artifact_hashes": artifact_hashes,
        "runner_facts": {
            "exit_status": evidence["exit_status"],
            "timed_out": evidence["timed_out"],
            "dependency_error": evidence["dependency_error"],
            "infrastructure_error": evidence["infrastructure_error"],
            "tests": evidence["tests"],
        },
        "classification": {
            "status": verification.status,
            "reason": verification.reason,
            "facts": verification.facts,
        },
        "provenance": evidence["provenance"],
    }


def _build_verdict(
    manifest: dict[str, Any],
    verification: Any,
    evaluation: dict[str, Any],
) -> dict[str, Any]:
    evaluation_bytes = json.dumps(evaluation, sort_keys=True, indent=2).encode("utf-8")
    return {
        "schema_version": VERDICT_SCHEMA,
        "dataset_id": manifest["dataset_id"],
        "dataset_version": manifest["dataset_version"],
        "split": manifest["split"],
        "task_id": manifest["task"]["task_id"],
        "task_digest": manifest["task"]["tree_digest"],
        "runner_id": manifest["runner"]["id"],
        "verifier_id": VERIFIER_ID,
        "verifier_version": VERIFIER_VERSION,
        "status": verification.status,
        "reason": verification.reason,
        "facts": verification.facts,
        "evaluation_sha256": _sha256(evaluation_bytes),
        # The public-test boundary is restated here so no consumer has to
        # re-derive it from the manifest: internal evaluation only, never a
        # private holdout, never promotion-eligible, never an official score.
        "internal_only": manifest["license"]["publication"]
        == "internal_only_with_attribution_no_public_task_package_no_official_scores",
        "public_tests": manifest["pollution"]["test_class"] == "public_tests",
        "private_holdout": False,
        "promotion_eligible": False,
        "official_aider_score": False,
        "license_publication": manifest["license"]["publication"],
        "provenance": evaluation["provenance"],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts-dir", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument(
        "--verifier-out", default="/logs/verifier",
        help="directory receiving evaluation.json/verdict.json (overridable by tests)",
    )
    args = parser.parse_args(argv)

    artifacts_dir = Path(args.artifacts_dir)
    try:
        manifest = load_manifest(args.manifest)

        # Declared-only: the artifact root may not carry anything outside the
        # frozen allowlist (INV-6); an undeclared file is tamper, not noise.
        declared = {_DECLARED_EVIDENCE, *_OUTPUT_ARTIFACTS}
        unexpected = sorted(
            entry.name for entry in artifacts_dir.iterdir() if entry.name not in declared
        )
        if unexpected:
            raise ContractError(
                "tamper_detected",
                f"undeclared artifact(s) present: {', '.join(unexpected)}",
            )

        payloads = {
            _DECLARED_EVIDENCE: _read_artifact(artifacts_dir, _DECLARED_EVIDENCE),
            **{
                name: _read_artifact(artifacts_dir, name)
                for name in _OUTPUT_ARTIFACTS
            },
        }
        artifact_hashes = {name: _sha256(body) for name, body in payloads.items()}

        # Identity binding: the published task manifest must equal the frozen
        # manifest this verifier environment owns.
        if payloads["task_manifest.json"].decode("utf-8", errors="strict") != Path(
            args.manifest
        ).read_text(encoding="utf-8"):
            raise ContractError(
                "tamper_detected",
                "published task manifest differs from the verifier's frozen manifest",
            )

        evidence = json.loads(payloads[_DECLARED_EVIDENCE])
        if evidence.get("result_kind") != RESULT_KIND:
            raise ContractError(
                "tamper_detected", "runner evidence is not a Bowling runner result"
            )

        mismatch = _recompute_from_output(
            payloads["runner_stdout.txt"] + payloads["runner_stderr.txt"], evidence
        )
        if mismatch:
            raise ContractError("tamper_detected", mismatch)

        verification = verify_evidence(manifest, evidence, payloads)
        if verification.status == "tamper_detected":
            # A tampered artifact set is refused outright: no evaluation or
            # verdict sidecar may be derived from it (INV-6 fail closed).
            raise ContractError("tamper_detected", verification.reason)

        evaluation = _build_evaluation(
            manifest, evidence, artifact_hashes, verification
        )
        verdict = _build_verdict(manifest, verification, evaluation)
        verdict_dir = Path(args.verifier_out)
        _write_atomic(
            verdict_dir,
            "evaluation.json",
            json.dumps(evaluation, sort_keys=True, indent=2).encode("utf-8"),
        )
        _write_atomic(
            verdict_dir,
            "verdict.json",
            json.dumps(verdict, sort_keys=True, indent=2).encode("utf-8"),
        )
        print(
            json.dumps(
                {
                    "verifier_id": VERIFIER_ID,
                    "status": verification.status,
                    "reason": verification.reason,
                },
                sort_keys=True,
            )
        )
        return 0
    except ContractError as error:
        print(
            json.dumps(
                {"verifier_id": VERIFIER_ID, "status": error.code, "reason": error.message},
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
