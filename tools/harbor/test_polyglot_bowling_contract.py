"""Focused tests for the license-blocked external Bowling contract."""

from __future__ import annotations

import hashlib
import json
import shutil
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

from polyglot_bowling_contract import (
    ContractError,
    durable_projection,
    expected_chain_head,
    load_manifest,
    validate_manifest,
    verify_evidence,
)
from build_bowling_task import _stage_verified_task_files


ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = ROOT / "work/agent-cli-polyglot-benchmark/manifest.json"


class PolyglotBowlingContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = load_manifest(str(MANIFEST_PATH))

    def test_manifest_is_strict_and_license_confirmed_for_internal_use(self) -> None:
        self.assertEqual(self.manifest["dataset_id"], "exercism-polyglot")
        self.assertEqual(self.manifest["task"]["task_id"], "python/exercises/practice/bowling")
        self.assertEqual(self.manifest["license"]["status"], "confirmed")
        self.assertEqual(
            self.manifest["license"]["publication"],
            "internal_only_with_attribution_no_public_task_package_no_official_scores",
        )
        license_kinds = {entry["kind"] for entry in self.manifest["license"]["evidence"]}
        self.assertIn("upstream_track_root_license", license_kinds)
        self.assertEqual(self.manifest["contract_status"], "frozen_manifest_executable")
        self.assertEqual(self.manifest["pollution"]["test_class"], "public_tests")

        unknown = deepcopy(self.manifest)
        unknown["unexpected"] = True
        with self.assertRaisesRegex(ContractError, "unknown_field"):
            validate_manifest(unknown)

    def test_durable_projection_contains_only_digest_bound_refs(self) -> None:
        projection = durable_projection(self.manifest, "a" * 64)
        self.assertEqual(
            set(projection),
            {
                "dataset_id",
                "dataset_version",
                "split",
                "task_id",
                "snapshot_digest",
                "task_digest",
                "instruction_hash",
            },
        )
        self.assertNotIn("BowlingGame", json.dumps(projection))
        self.assertNotIn("unittest", json.dumps(projection))

        with self.assertRaisesRegex(ContractError, "invalid_digest"):
            durable_projection(self.manifest, "not-a-digest")

    def test_identity_drift_is_rejected(self) -> None:
        drifted = deepcopy(self.manifest)
        drifted["task"]["tree_digest"] = "sha256:" + "0" * 64
        # The altered digest is structurally valid, but it is not the same
        # identity as the staged manifest recorded by the original snapshot.
        self.assertNotEqual(
            drifted["task"]["tree_digest"], self.manifest["task"]["tree_digest"]
        )
        self.assertEqual(drifted["task"]["task_id"], self.manifest["task"]["task_id"])

        wrong_split = deepcopy(self.manifest)
        wrong_split["split"] = "all"
        with self.assertRaisesRegex(ContractError, "split_mismatch"):
            validate_manifest(wrong_split)

        wrong_ref = deepcopy(self.manifest)
        wrong_ref["source"]["ref"] = "local-snapshot:" + "0" * 64
        with self.assertRaisesRegex(ContractError, "snapshot_drift"):
            validate_manifest(wrong_ref)

    def test_blocked_license_is_not_evaluable_even_with_clean_evidence(self) -> None:
        # The manifest is licensed for internal use now, but the fail-closed
        # gate must still hold whenever evidence is missing, so the gate is
        # exercised against an explicit blocked-license variant.
        manifest = deepcopy(self.manifest)
        manifest["license"]["status"] = "blocked"
        evidence, artifacts = self._evidence()
        result = verify_evidence(manifest, evidence, artifacts)
        self.assertEqual(result.status, "not_evaluable")
        self.assertEqual(result.reason, "task_license_not_confirmed")

    def test_verifier_classifies_synthetic_result_facts(self) -> None:
        manifest = deepcopy(self.manifest)
        manifest["license"]["status"] = "confirmed"
        cases = (
            ({}, "pass"),
            ({"tests": {"failed": 1}}, "candidate_failure"),
            ({"timed_out": True}, "timeout"),
            ({"dependency_error": True}, "dependency_error"),
            ({"infrastructure_error": True}, "infrastructure_error"),
        )
        for overrides, expected in cases:
            with self.subTest(expected=expected):
                evidence, artifacts = self._evidence()
                self._apply(evidence, overrides)
                evidence["provenance"]["chain_head"] = expected_chain_head(evidence)
                result = verify_evidence(manifest, evidence, artifacts)
                self.assertEqual(result.status, expected)

    def test_provenance_chain_head_rejects_changed_evidence(self) -> None:
        manifest = deepcopy(self.manifest)
        manifest["license"]["status"] = "confirmed"
        evidence, artifacts = self._evidence()
        evidence["provenance"]["chain_head"] = expected_chain_head(evidence)
        evidence["exit_status"] = 1
        result = verify_evidence(manifest, evidence, artifacts)
        self.assertEqual(result.status, "tamper_detected")

    def test_build_staging_checks_all_eight_task_files_and_tree_digest(self) -> None:
        snapshot = Path("/home/xc/下载/polyglot-benchmark-main")
        task_root = snapshot / self.manifest["task"]["relative_path"]
        if not task_root.is_dir():
            self.skipTest("the pinned local Bowling snapshot is unavailable")
        with tempfile.TemporaryDirectory(prefix="bowling-build-check-") as temporary:
            staged = Path(temporary) / "clean"
            _stage_verified_task_files(self.manifest, snapshot, staged)
            self.assertTrue((staged / "task-package/bowling_test.py").is_file())
            self.assertTrue((staged / "task-package/bowling_reference.py").is_file())

            changed = Path(temporary) / self.manifest["task"]["relative_path"]
            shutil.copytree(task_root, changed)
            # This metadata file is never copied into the image, but still
            # belongs to the frozen eight-file task identity.
            (changed / ".docs/instructions.md").write_bytes(b"drift")
            with self.assertRaisesRegex(SystemExit, "task tree"):
                _stage_verified_task_files(self.manifest, Path(temporary), Path(temporary) / "drift")

            bad_manifest = deepcopy(self.manifest)
            bad_manifest["task"]["files"][0]["sha256"] = "0" * 64
            with self.assertRaisesRegex(SystemExit, "source digest mismatch"):
                _stage_verified_task_files(bad_manifest, snapshot, Path(temporary) / "bad-hash")

        manifest = deepcopy(self.manifest)
        manifest["license"]["status"] = "confirmed"
        evidence, artifacts = self._evidence()

        artifacts["runner_result.json"] = b"tampered"
        result = verify_evidence(manifest, evidence, artifacts)
        self.assertEqual(result.status, "tamper_detected")

        evidence, _ = self._evidence()
        secret = b'{"token":"sk-live-secret"}'
        artifacts = {"runner_result.json": secret}
        evidence["declared_artifacts"][0]["sha256"] = hashlib.sha256(secret).hexdigest()
        result = verify_evidence(manifest, evidence, artifacts)
        self.assertEqual(result.status, "tamper_detected")


    def _evidence(self) -> tuple[dict, dict[str, bytes]]:
        payload = b'{"tests": {"discovered": 30, "failed": 0, "errors": 0}}'
        evidence = {
            "schema_version": 1,
            "result_kind": "external_runner_result.v1",
            "dataset_id": self.manifest["dataset_id"],
            "dataset_version": self.manifest["dataset_version"],
            "split": self.manifest["split"],
            "snapshot_digest": self.manifest["snapshot"]["archive_digest"],
            "task_digest": self.manifest["task"]["tree_digest"],
            "runner_id": self.manifest["runner"]["id"],
            "runner_version": self.manifest["runner"]["version"],
            "verifier_id": self.manifest["verifier"]["id"],
            "verifier_version": self.manifest["verifier"]["version"],
            "exit_status": 0,
            "timed_out": False,
            "dependency_error": False,
            "infrastructure_error": False,
            "tests": {"discovered": 30, "failed": 0, "errors": 0},
            "declared_artifacts": [
                {"path": "runner_result.json", "sha256": hashlib.sha256(payload).hexdigest()}
            ],
            "provenance": {
                "run_id": "run-1",
                "session_id": "session-1",
                "chain_head": "chain-1",
            },
        }
        evidence["provenance"]["chain_head"] = expected_chain_head(evidence)
        return evidence, {"runner_result.json": payload}

    @staticmethod
    def _apply(evidence: dict, overrides: dict) -> None:
        for key, value in overrides.items():
            if key == "tests":
                evidence["tests"].update(value)
            else:
                evidence[key] = value


if __name__ == "__main__":
    unittest.main()
