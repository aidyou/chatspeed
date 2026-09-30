"""Focused negative tests for the independent Bowling verifier.

The verifier module ships with the task image, but its decision logic must be
re-checkable on the host without Harbor: every tamper, secret, undeclared
path, identity drift and license gate case below fails closed with the stable
classification the Phase 3E contract promises.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
TOOLS_DIR = REPO_ROOT / "tools/harbor"
VERIFY_PATH = (
    REPO_ROOT
    / "work/agent-cli-polyglot-benchmark/harbor/tests/bowling_verify.py"
)
MANIFEST_PATH = REPO_ROOT / "work/agent-cli-polyglot-benchmark/manifest.json"

# The task image ships the contract module under the name `bowling_contract`;
# on the host the same module is `polyglot_bowling_contract.py`, so the test
# stages the image's view of it.
_contract_stage = Path(tempfile.mkdtemp(prefix="bowling-contract-"))
shutil.copyfile(
    TOOLS_DIR / "polyglot_bowling_contract.py", _contract_stage / "bowling_contract.py"
)
sys.path.insert(0, str(_contract_stage))
from bowling_contract import expected_chain_head  # noqa: E402

_spec = importlib.util.spec_from_file_location("bowling_verify", VERIFY_PATH)
bowling_verify = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(bowling_verify)


class BowlingVerifierNegativeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest_bytes = MANIFEST_PATH.read_bytes()
        cls.manifest = json.loads(cls.manifest_bytes)

    def _artifacts_dir(
        self,
        evidence: dict,
        stdout: bytes = b"",
        stderr: bytes = b'{"tests": {"discovered": 31, "failed": 0, "errors": 0}}\nRan 31 tests in 0.001s\n\nOK\n',
    ) -> tuple[Path, bytes]:
        """Builds a declared artifact set whose hashes bind an evidence doc."""
        stderr = stderr if stderr else b"Ran 31 tests in 0.001s\n\nOK\n"
        evidence["declared_artifacts"] = [
            {"path": "runner_stderr.txt", "sha256": hashlib.sha256(stderr).hexdigest()},
            {"path": "runner_stdout.txt", "sha256": hashlib.sha256(stdout).hexdigest()},
        ]
        evidence["provenance"]["chain_head"] = expected_chain_head(evidence)
        directory = Path(tempfile.mkdtemp(prefix="bowling-verify-"))
        (directory / "runner_result.json").write_bytes(
            json.dumps(evidence, sort_keys=True, indent=2).encode("utf-8")
        )
        (directory / "runner_stderr.txt").write_bytes(stderr)
        (directory / "runner_stdout.txt").write_bytes(stdout)
        (directory / "task_manifest.json").write_bytes(self.manifest_bytes)
        return directory, stderr

    def _evidence(self, **overrides) -> dict:
        evidence = {
            "schema_version": 1,
            "result_kind": "external_runner_result.v1",
            "dataset_id": "exercism-polyglot",
            "dataset_version": 1,
            "split": "python_smoke",
            "snapshot_digest": self.manifest["snapshot"]["archive_digest"],
            "task_digest": self.manifest["task"]["tree_digest"],
            "runner_id": "harbor-python-unittest",
            "runner_version": "1",
            "verifier_id": "exercism-polyglot-bowling-verifier",
            "verifier_version": "1",
            "exit_status": 0,
            "timed_out": False,
            "dependency_error": False,
            "infrastructure_error": False,
            "tests": {"discovered": 31, "failed": 0, "errors": 0},
            "declared_artifacts": [],
            "provenance": {
                "run_id": "run-1",
                "session_id": "session-1",
                "chain_head": "0" * 64,
            },
        }
        evidence.update(overrides)
        evidence["provenance"]["chain_head"] = expected_chain_head(evidence)
        return evidence

    def _run_verifier(self, directory: Path) -> tuple[int, dict | None]:
        out = Path(tempfile.mkdtemp(prefix="bowling-verdict-"))
        code = bowling_verify.main(
            [
                "--artifacts-dir", str(directory),
                "--manifest", str(MANIFEST_PATH),
                "--verifier-out", str(out),
            ]
        )
        verdict = None
        verdict_path = out / "verdict.json"
        if verdict_path.exists():
            verdict = json.loads(verdict_path.read_text())
        return code, verdict

    def test_clean_reference_evidence_is_accepted(self) -> None:
        directory, _ = self._artifacts_dir(self._evidence())
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 0)
        assert verdict is not None
        self.assertEqual(verdict["status"], "pass")
        self.assertTrue(verdict["internal_only"])
        self.assertFalse(verdict["promotion_eligible"])

    def test_blocked_license_stays_not_evaluable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            blocked_manifest = Path(tmp) / "manifest.json"
            blocked = deepcopy(self.manifest)
            blocked["license"]["status"] = "blocked"
            blocked_manifest.write_text(json.dumps(blocked))
            directory, _ = self._artifacts_dir(self._evidence())
            (directory / "task_manifest.json").write_bytes(blocked_manifest.read_bytes())
            out = Path(tmp) / "verdict"
            code = bowling_verify.main(
                [
                    "--artifacts-dir", str(directory),
                    "--manifest", str(blocked_manifest),
                    "--verifier-out", str(out),
                ]
            )
            self.assertEqual(code, 0)
            verdict = json.loads((out / "verdict.json").read_text())
            self.assertEqual(verdict["status"], "not_evaluable")
            self.assertEqual(verdict["reason"], "task_license_not_confirmed")

    def test_tampered_output_artifact_is_rejected(self) -> None:
        directory, _ = self._artifacts_dir(self._evidence())
        (directory / "runner_stdout.txt").write_bytes(b"tampered after the fact")
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

    def test_secret_marker_is_rejected(self) -> None:
        stdout = b"token=sk-live-secret\n"
        evidence = self._evidence()
        directory, _ = self._artifacts_dir(evidence, stdout=stdout)
        # Re-declare the hash of the secret payload so only the marker check
        # can reject it.
        evidence["declared_artifacts"] = [
            entry
            for entry in evidence["declared_artifacts"]
            if entry["path"] != "runner_stdout.txt"
        ]
        evidence["declared_artifacts"].append(
            {"path": "runner_stdout.txt", "sha256": hashlib.sha256(stdout).hexdigest()}
        )
        (directory / "runner_result.json").write_bytes(
            json.dumps(evidence, sort_keys=True, indent=2).encode("utf-8")
        )
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

    def test_undeclared_artifact_file_is_rejected(self) -> None:
        directory, _ = self._artifacts_dir(self._evidence())
        (directory / "smuggled.txt").write_bytes(b"not declared")
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

    def test_identity_drift_is_rejected(self) -> None:
        directory, _ = self._artifacts_dir(self._evidence(runner_id="other-runner"))
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

    def test_fact_mismatch_is_rejected_as_tamper(self) -> None:
        directory, _ = self._artifacts_dir(
            self._evidence(tests={"discovered": 31, "failed": 0, "errors": 0}),
            stderr=b"Ran 31 tests in 0.001s\n\nFAILED (failures=5)\n",
        )
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

    def test_candidate_output_cannot_hide_a_second_runner_result(self) -> None:
        evidence = self._evidence()
        directory, _ = self._artifacts_dir(
            evidence,
            stdout=b"Ran 31 tests in 0.001s\n\nOK\n",
            stderr=b"Ran 31 tests in 0.002s\n\nFAILED (failures=5)\n",
        )
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)

        evidence = self._evidence(exit_status=1)
        evidence["tests"] = {"discovered": 31, "failed": 21, "errors": 0}
        directory, _ = self._artifacts_dir(
            evidence,
            stderr=b"Ran 31 tests in 0.001s\n\nFAILED (failures=21)\n",
        )
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 0)
        assert verdict is not None
        self.assertEqual(verdict["status"], "candidate_failure")
        self.assertEqual(verdict["facts"]["tests"]["failed"], 21)

    def test_published_manifest_must_equal_frozen_manifest(self) -> None:
        directory, _ = self._artifacts_dir(self._evidence())
        drifted = deepcopy(self.manifest)
        drifted["license"]["status"] = "confirmed"
        (directory / "task_manifest.json").write_bytes(
            json.dumps(drifted, indent=2).encode("utf-8")
        )
        code, verdict = self._run_verifier(directory)
        self.assertEqual(code, 1)
        self.assertIsNone(verdict)


if __name__ == "__main__":
    unittest.main()
