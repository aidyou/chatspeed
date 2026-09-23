"""Strict contract and verifier primitives for the external Bowling slice.

This module deliberately does not acquire or execute the external task. The
current manifest is license-blocked and has no pinned Harbor Python image, so
runtime execution must remain ``not_evaluable``. The code here still freezes
the boundary used by a future adapter: strict manifest identity, durable
reference projection, declared-only artifacts, and independent result
classification.
"""

from __future__ import annotations

import hashlib
import json
import re
from copy import deepcopy
from dataclasses import dataclass
from typing import Any, Mapping

SCHEMA_VERSION = 1
RESULT_SCHEMA_VERSION = 1
RESULT_KIND = "external_runner_result.v1"
DATASET_ID = "exercism-polyglot"
DATASET_VERSION = 1
SPLIT = "python_smoke"
TASK_ID = "python/exercises/practice/bowling"
SNAPSHOT_DIGEST = "sha256:3dfe5f25603f104748b4205739c9d3d5676a470777c261255a501b45cec3a9cc"
TASK_DIGEST = "sha256:69c60c9ae3a9c8d880486f2f6044fc410ec937a35497a2412941088f61e41d0c"
RUNNER_ID = "harbor-python-unittest"
RUNNER_VERSION = "1"
VERIFIER_ID = "exercism-polyglot-bowling-verifier"
VERIFIER_VERSION = "1"
PUBLIC_TEST_CLASS = "public_tests"
_SECRET_MARKERS = ("sk-", "ghp_", "xoxb-", "-----begin", "akia")
_HEX_64 = re.compile(r"^[0-9a-f]{64}$")


class ContractError(ValueError):
    """A fail-closed contract error with a stable machine code."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"


@dataclass(frozen=True)
class Verification:
    """The independently derived result status and supporting facts."""

    status: str
    reason: str
    facts: dict[str, Any]


def _require_keys(value: Mapping[str, Any], required: set[str], where: str) -> None:
    missing = sorted(required - set(value))
    unknown = sorted(set(value) - required)
    if missing:
        raise ContractError("missing_field", f"{where} is missing {', '.join(missing)}")
    if unknown:
        raise ContractError("unknown_field", f"{where} contains {', '.join(unknown)}")


def _string(value: Any, where: str) -> str:
    if not isinstance(value, str) or not value:
        raise ContractError("invalid_value", f"{where} must be a non-empty string")
    return value


def _digest(value: Any, where: str) -> str:
    value = _string(value, where)
    if not value.startswith("sha256:") or not _HEX_64.fullmatch(value[7:]):
        raise ContractError("invalid_digest", f"{where} must be sha256:<64 lowercase hex>")
    return value


def _sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def expected_chain_head(evidence: Mapping[str, Any]) -> str:
    """Return the integrity binding for the non-provenance evidence fields."""
    payload = {key: value for key, value in evidence.items() if key != "provenance"}
    return _sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8"))


def _assert_no_secret(name: str, payload: bytes) -> None:
    lowered = payload.decode("utf-8", errors="ignore").lower()
    for marker in _SECRET_MARKERS:
        if marker in lowered:
            raise ContractError("secret_marker", f"artifact '{name}' contains a secret marker")


def load_manifest(path: str) -> dict[str, Any]:
    """Load and validate one strict manifest from a local audit checkout."""
    try:
        with open(path, "rb") as handle:
            document = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError("manifest_invalid", f"cannot read manifest: {error}") from error
    return validate_manifest(document)


def validate_manifest(document: Any) -> dict[str, Any]:
    """Validate the additive Bowling manifest without reading task source."""
    if not isinstance(document, dict):
        raise ContractError("manifest_invalid", "manifest must be a JSON object")
    _require_keys(
        document,
        {
            "schema_version",
            "dataset_id",
            "dataset_version",
            "split",
            "source",
            "snapshot",
            "task",
            "license",
            "pollution",
            "runner",
            "verifier",
            "artifact_policy",
            "contract_status",
            "durable_projection",
        },
        "manifest",
    )
    if document["schema_version"] != SCHEMA_VERSION:
        raise ContractError("unsupported_version", "unsupported manifest schema version")
    if document["dataset_id"] != DATASET_ID or document["dataset_version"] != DATASET_VERSION:
        raise ContractError("dataset_mismatch", "manifest dataset identity is not the pinned contract")
    if document["split"] != SPLIT:
        raise ContractError("split_mismatch", "manifest split is not the pinned Python smoke split")

    source = document["source"]
    if not isinstance(source, dict):
        raise ContractError("manifest_invalid", "source must be an object")
    _require_keys(source, {"kind", "ref", "source_path_is_runtime_input", "upstream"}, "source")
    if source["kind"] != "local_snapshot" or source["source_path_is_runtime_input"] is not False:
        raise ContractError("acquisition_policy", "runtime input must be a fixed local snapshot")
    source_ref = _string(source["ref"], "source.ref")
    if not source_ref.startswith("local-snapshot:") or not _HEX_64.fullmatch(source_ref[15:]):
        raise ContractError("invalid_source_ref", "source.ref must contain a fixed snapshot digest")
    upstream = source["upstream"]
    if not isinstance(upstream, dict):
        raise ContractError("manifest_invalid", "source.upstream must be an object")
    _require_keys(upstream, {"name", "attribution_url", "exercise_source_url"}, "source.upstream")
    for key in ("name", "attribution_url", "exercise_source_url"):
        _string(upstream[key], f"source.upstream.{key}")

    snapshot = document["snapshot"]
    if not isinstance(snapshot, dict):
        raise ContractError("manifest_invalid", "snapshot must be an object")
    _require_keys(snapshot, {"archive_digest", "archive_algorithm", "file_count", "attribution_file"}, "snapshot")
    _digest(snapshot["archive_digest"], "snapshot.archive_digest")
    if snapshot["archive_digest"] != SNAPSHOT_DIGEST:
        raise ContractError("snapshot_drift", "snapshot digest does not match the frozen external source")
    if source_ref[15:] != snapshot["archive_digest"][7:]:
        raise ContractError("snapshot_drift", "source ref does not match snapshot digest")
    _string(snapshot["archive_algorithm"], "snapshot.archive_algorithm")
    if not isinstance(snapshot["file_count"], int) or snapshot["file_count"] <= 0:
        raise ContractError("invalid_value", "snapshot.file_count must be positive")
    attribution = snapshot["attribution_file"]
    if not isinstance(attribution, dict):
        raise ContractError("manifest_invalid", "snapshot.attribution_file must be an object")
    _require_keys(attribution, {"path", "sha256"}, "snapshot.attribution_file")
    _string(attribution["path"], "snapshot.attribution_file.path")
    if not _HEX_64.fullmatch(_string(attribution["sha256"], "snapshot.attribution_file.sha256")):
        raise ContractError("invalid_digest", "snapshot attribution digest must be lowercase hex")

    task = document["task"]
    if not isinstance(task, dict):
        raise ContractError("manifest_invalid", "task must be an object")
    _require_keys(task, {"task_id", "relative_path", "tree_digest", "tree_algorithm", "file_count", "files"}, "task")
    if task["task_id"] != TASK_ID or task["relative_path"] != TASK_ID:
        raise ContractError("task_mismatch", "manifest task identity is not Bowling")
    _digest(task["tree_digest"], "task.tree_digest")
    if task["tree_digest"] != TASK_DIGEST:
        raise ContractError("task_drift", "task tree digest does not match the frozen Bowling source")
    _string(task["tree_algorithm"], "task.tree_algorithm")
    if task["file_count"] != 8 or not isinstance(task["files"], list):
        raise ContractError("task_files_invalid", "Bowling task must declare its fixed eight-file package")
    roles: set[str] = set()
    paths: set[str] = set()
    for index, entry in enumerate(task["files"]):
        if not isinstance(entry, dict):
            raise ContractError("task_files_invalid", f"task.files[{index}] must be an object")
        _require_keys(entry, {"path", "role", "sha256"}, f"task.files[{index}]")
        path = _string(entry["path"], f"task.files[{index}].path")
        role = _string(entry["role"], f"task.files[{index}].role")
        digest = _string(entry["sha256"], f"task.files[{index}].sha256")
        if path in paths or path.startswith("/") or ".." in path.split("/"):
            raise ContractError("task_files_invalid", f"unsafe or duplicate task path '{path}'")
        if not _HEX_64.fullmatch(digest):
            raise ContractError("invalid_digest", f"task.files[{index}].sha256 is invalid")
        paths.add(path)
        roles.add(role)
    if not {"starter", PUBLIC_TEST_CLASS, "metadata"}.issubset(roles):
        raise ContractError("task_files_invalid", "starter, metadata and public_tests roles are required")

    license_document = document["license"]
    if not isinstance(license_document, dict):
        raise ContractError("manifest_invalid", "license must be an object")
    _require_keys(license_document, {"status", "evidence", "missing", "publication"}, "license")
    if license_document["status"] not in {"blocked", "confirmed"}:
        raise ContractError("license_status", "license status must be blocked or confirmed")
    if not isinstance(license_document["evidence"], list) or not isinstance(license_document["missing"], list):
        raise ContractError("license_status", "license evidence and missing fields must be arrays")
    _string(license_document["publication"], "license.publication")

    pollution = document["pollution"]
    if not isinstance(pollution, dict):
        raise ContractError("manifest_invalid", "pollution must be an object")
    _require_keys(pollution, {"test_class", "contamination_risk", "private_holdout", "promotion_eligible", "official_aider_score"}, "pollution")
    if pollution["test_class"] != PUBLIC_TEST_CLASS or pollution["private_holdout"] is not False:
        raise ContractError("pollution_policy", "Bowling tests must remain public_tests and non-holdout")
    if pollution["promotion_eligible"] is not False or pollution["official_aider_score"] is not False:
        raise ContractError("pollution_policy", "public tests cannot produce promotion or official score evidence")

    runner = document["runner"]
    if not isinstance(runner, dict):
        raise ContractError("manifest_invalid", "runner must be an object")
    _require_keys(runner, {"id", "version", "command", "language", "runtime", "network", "cwd", "candidate_root", "readonly_test_root", "artifact_root", "dependency_policy", "resource_profile"}, "runner")
    if runner["id"] != RUNNER_ID or runner["version"] != RUNNER_VERSION:
        raise ContractError("runner_mismatch", "runner identity is not the pinned Harbor Python contract")
    if runner["network"] != "none" or runner["dependency_policy"] != "standard_library_only_no_runtime_install":
        raise ContractError("runner_policy", "runner must be network-none and install-free")
    if runner["command"] != ["python", "-m", "unittest", "discover", "-s", "/tests", "-p", "bowling_test.py"]:
        raise ContractError("runner_policy", "runner command is not the fixed unittest argv")
    for key in ("cwd", "candidate_root", "readonly_test_root", "artifact_root"):
        _string(runner[key], f"runner.{key}")
    limits = runner["resource_profile"]
    if not isinstance(limits, dict):
        raise ContractError("runner_policy", "runner.resource_profile must be an object")
    for key in ("wall_time_ms", "cpu_time_ms", "memory_bytes", "processes", "output_bytes", "disk_bytes"):
        if not isinstance(limits.get(key), int) or limits[key] <= 0:
            raise ContractError("runner_policy", f"runner.resource_profile.{key} must be positive")

    verifier = document["verifier"]
    if not isinstance(verifier, dict):
        raise ContractError("manifest_invalid", "verifier must be an object")
    _require_keys(verifier, {"id", "version", "independent", "accepts_agent_transcript", "accepts_self_reported_score", "required_evidence"}, "verifier")
    if verifier["id"] != VERIFIER_ID or verifier["version"] != VERIFIER_VERSION:
        raise ContractError("verifier_mismatch", "verifier identity is not the pinned contract")
    if verifier["independent"] is not True or verifier["accepts_agent_transcript"] is not False or verifier["accepts_self_reported_score"] is not False:
        raise ContractError("verifier_policy", "verifier must reject transcript and self-reported score")
    if not isinstance(verifier["required_evidence"], list) or "chain_head" not in verifier["required_evidence"]:
        raise ContractError("verifier_policy", "verifier provenance requirements are incomplete")

    artifact_policy = document["artifact_policy"]
    if not isinstance(artifact_policy, dict):
        raise ContractError("manifest_invalid", "artifact_policy must be an object")
    _require_keys(artifact_policy, {"declared_only", "allowed_paths", "secrets_and_source", "raw_task_and_tests"}, "artifact_policy")
    if artifact_policy["declared_only"] is not True or artifact_policy["secrets_and_source"] != "reject":
        raise ContractError("artifact_policy", "artifact publication must be declared-only and secret rejecting")
    if not isinstance(artifact_policy["allowed_paths"], list) or not artifact_policy["allowed_paths"]:
        raise ContractError("artifact_policy", "artifact allowlist must not be empty")
    if any(not isinstance(path, str) or path.startswith("/") or ".." in path.split("/") for path in artifact_policy["allowed_paths"]):
        raise ContractError("artifact_policy", "artifact paths must be safe relative paths")
    if document["contract_status"] not in {"frozen_manifest_not_yet_executable", "frozen_manifest_executable"}:
        raise ContractError("contract_status", "the Bowling contract status is not a frozen state")
    if document["durable_projection"] != ["dataset_id", "dataset_version", "split", "task_id", "snapshot_digest", "task_digest", "instruction_hash"]:
        raise ContractError("durable_projection", "durable projection changed from the approved privacy boundary")
    return deepcopy(document)


def durable_projection(manifest: Mapping[str, Any], instruction_hash: str) -> dict[str, Any]:
    """Return the only fields allowed in a durable schedule/job reference."""
    validate_manifest(manifest)
    if not _HEX_64.fullmatch(instruction_hash):
        raise ContractError("invalid_digest", "instruction_hash must be lowercase sha256 hex")
    projection = {
        "dataset_id": manifest["dataset_id"],
        "dataset_version": manifest["dataset_version"],
        "split": manifest["split"],
        "task_id": manifest["task"]["task_id"],
        "snapshot_digest": manifest["snapshot"]["archive_digest"],
        "task_digest": manifest["task"]["tree_digest"],
        "instruction_hash": instruction_hash,
    }
    serialized = json.dumps(projection, sort_keys=True)
    if any(marker in serialized.lower() for marker in ("bowlinggame", "assert", "unittest")):
        raise ContractError("durable_privacy", "durable projection contains task content")
    return projection


def _validate_result_shape(evidence: Any) -> None:
    if not isinstance(evidence, dict):
        raise ContractError("result_invalid", "runner evidence must be an object")
    _require_keys(
        evidence,
        {
            "schema_version", "result_kind", "dataset_id", "dataset_version", "split", "snapshot_digest", "task_digest",
            "runner_id", "runner_version", "verifier_id", "verifier_version", "exit_status", "timed_out",
            "dependency_error", "infrastructure_error", "tests", "declared_artifacts", "provenance",
        },
        "runner evidence",
    )
    if evidence["schema_version"] != RESULT_SCHEMA_VERSION or evidence["result_kind"] != RESULT_KIND:
        raise ContractError("result_invalid", "unsupported runner evidence schema")
    for key in ("dataset_id", "split", "runner_id", "runner_version", "verifier_id", "verifier_version"):
        _string(evidence[key], f"runner evidence.{key}")
    if not isinstance(evidence["dataset_version"], int) or not isinstance(evidence["exit_status"], int):
        raise ContractError("result_invalid", "dataset_version and exit_status must be integers")
    for key in ("timed_out", "dependency_error", "infrastructure_error"):
        if not isinstance(evidence[key], bool):
            raise ContractError("result_invalid", f"runner evidence.{key} must be boolean")
    tests = evidence["tests"]
    if not isinstance(tests, dict):
        raise ContractError("result_invalid", "tests must be an object")
    _require_keys(tests, {"discovered", "failed", "errors"}, "runner evidence.tests")
    if any(not isinstance(tests[key], int) or tests[key] < 0 for key in ("discovered", "failed", "errors")):
        raise ContractError("result_invalid", "test counts must be non-negative integers")
    artifacts = evidence["declared_artifacts"]
    if not isinstance(artifacts, list):
        raise ContractError("result_invalid", "declared_artifacts must be an array")
    provenance = evidence["provenance"]
    if not isinstance(provenance, dict):
        raise ContractError("result_invalid", "provenance must be an object")
    _require_keys(provenance, {"run_id", "session_id", "chain_head"}, "runner evidence.provenance")
    for key in ("run_id", "session_id", "chain_head"):
        _string(provenance[key], f"runner evidence.provenance.{key}")
    if not _HEX_64.fullmatch(provenance["chain_head"]):
        raise ContractError("result_invalid", "runner evidence.provenance.chain_head must be a sha256 digest")


def verify_evidence(manifest: Mapping[str, Any], evidence: Mapping[str, Any], artifacts: Mapping[str, bytes]) -> Verification:
    """Recompute a result classification from trusted structured evidence.

    The function intentionally has no score input. Hashes, exit status, test
    counts, timeout/dependency/infrastructure signals, and artifact provenance
    are the only accepted facts.
    """
    try:
        normalized = validate_manifest(manifest)
        _validate_result_shape(evidence)
        for key, expected in (
            ("dataset_id", normalized["dataset_id"]),
            ("dataset_version", normalized["dataset_version"]),
            ("split", normalized["split"]),
            ("snapshot_digest", normalized["snapshot"]["archive_digest"]),
            ("task_digest", normalized["task"]["tree_digest"]),
            ("runner_id", RUNNER_ID),
            ("runner_version", RUNNER_VERSION),
            ("verifier_id", VERIFIER_ID),
            ("verifier_version", VERIFIER_VERSION),
        ):
            if evidence[key] != expected:
                raise ContractError("tamper_detected", f"runner evidence field '{key}' does not match manifest")
        if evidence["provenance"]["chain_head"] != expected_chain_head(evidence):
            raise ContractError("tamper_detected", "runner evidence provenance chain_head does not match evidence")
        allowed = set(normalized["artifact_policy"]["allowed_paths"])
        declared = evidence["declared_artifacts"]
        declared_names: set[str] = set()
        for entry in declared:
            if not isinstance(entry, dict):
                raise ContractError("tamper_detected", "artifact declaration must be an object")
            _require_keys(entry, {"path", "sha256"}, "runner evidence.declared_artifacts[]")
            name = _string(entry["path"], "artifact.path")
            digest = _string(entry["sha256"], "artifact.sha256")
            if name not in allowed or name in declared_names or not _HEX_64.fullmatch(digest):
                raise ContractError("tamper_detected", f"artifact declaration '{name}' is not allowed")
            if name not in artifacts or _sha256(artifacts[name]) != digest:
                raise ContractError("tamper_detected", f"artifact '{name}' is missing or hash-mismatched")
            try:
                _assert_no_secret(name, artifacts[name])
            except ContractError as error:
                raise ContractError("tamper_detected", error.message) from error
            declared_names.add(name)
        if not declared_names:
            raise ContractError("tamper_detected", "no declared artifact evidence was supplied")
        if normalized["license"]["status"] != "confirmed":
            return Verification("not_evaluable", "task_license_not_confirmed", {"license_status": "blocked"})
        if evidence["infrastructure_error"]:
            return Verification("infrastructure_error", "runner_infrastructure_error", {})
        if evidence["timed_out"]:
            return Verification("timeout", "runner_timeout", {})
        if evidence["dependency_error"]:
            return Verification("dependency_error", "runner_dependency_error", {})
        tests = evidence["tests"]
        if evidence["exit_status"] == 0 and tests["failed"] == 0 and tests["errors"] == 0:
            return Verification("pass", "all_public_tests_passed", {"tests": tests})
        return Verification("candidate_failure", "public_test_failure", {"tests": tests, "exit_status": evidence["exit_status"]})
    except ContractError as error:
        if error.code == "tamper_detected":
            return Verification("tamper_detected", error.message, {})
        raise


__all__ = [
    "ContractError",
    "DATASET_ID",
    "RESULT_KIND",
    "Verification",
    "durable_projection",
    "load_manifest",
    "validate_manifest",
    "verify_evidence",
]
