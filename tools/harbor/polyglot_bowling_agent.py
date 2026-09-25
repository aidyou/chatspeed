"""Bowling installed-agent adapter for Harbor (pinned to harbor==0.23.0).

Harbor installs and runs this adapter inside the Bowling task environment. It
stages a deterministic candidate (the digest-verified reference solution, or a
deliberately broken stub), executes the manifest's fixed unittest argv as a
non-root user against the read-only public-test root, and publishes only the
declared artifacts to `/logs/artifacts`.

Boundaries this adapter keeps:

* the candidate mode must come from the task instruction (strict JSON) or an
  explicitly allowed agent env override — anything else fails the trial;
* no transcript, task text, test text or credential is ever published: the
  declared artifacts are the runner evidence plus its bounded raw output and
  the digest-bound frozen manifest copy;
* the runner never installs dependencies and never opens the network — the
  task environment is declared `no-network` and the image owns everything.
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
import shlex
import tempfile
import uuid
from pathlib import Path, PurePosixPath
from typing import Any, ClassVar, override

from harbor.agents.installed.base import BaseInstalledAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

# The frozen identity mirrored from the contract module. The image-owned
# manifest is re-validated against these constants by the verifier, so the
# adapter only needs the values that bind the evidence it writes itself.
RESULT_KIND = "external_runner_result.v1"
RUNNER_ID = "harbor-python-unittest"
RUNNER_VERSION = "1"
VERIFIER_ID = "exercism-polyglot-bowling-verifier"
VERIFIER_VERSION = "1"

#: Roots owned by the task image.
MANIFEST_FILE = PurePosixPath("/opt/bowling/manifest.json")
REFERENCE_FILE = PurePosixPath("/opt/bowling/bowling_reference.py")
TEST_FILE = PurePosixPath("/tests/bowling_test.py")
WORKSPACE_ROOT = PurePosixPath("/workspace")
ARTIFACT_ROOT = PurePosixPath("/logs/artifacts")

#: The declared artifact names (the Bowling allowlist in the frozen manifest).
EVIDENCE_ARTIFACT = "runner_result.json"
OUTPUT_ARTIFACTS = ("runner_stdout.txt", "runner_stderr.txt", "task_manifest.json")
DECLARED_ARTIFACTS = (EVIDENCE_ARTIFACT, *OUTPUT_ARTIFACTS)

#: The manifest's resource profile, enforced around the fixed argv.
WALL_TIME_SEC = 300
OUTPUT_LIMIT_BYTES = 1024 * 1024

#: Bounded, non-secret instruction schema.
INSTRUCTION_SCHEMA = "bowling_candidate_spec.v1"
MODE_ENV = "BOWLING_CANDIDATE_MODE"
ALLOWED_MODES = ("reference", "broken")

#: A deliberately wrong candidate: it satisfies the import contract but
#: ignores strikes, spares and bonus throws. This is authored here — it is not
#: Exercism content and carries no task text.
_BROKEN_CANDIDATE = (
    "class BowlingGame:\n"
    "    def __init__(self):\n"
    "        self._rolls = []\n"
    "\n"
    "    def roll(self, pins):\n"
    "        self._rolls.append(pins)\n"
    "\n"
    "    def score(self):\n"
    "        return sum(self._rolls)\n"
)


def _truncate(payload: str) -> str:
    """Bounded output representation, per the manifest's output budget."""
    encoded = payload.encode("utf-8", errors="replace")
    if len(encoded) <= OUTPUT_LIMIT_BYTES:
        return payload
    return encoded[:OUTPUT_LIMIT_BYTES].decode("utf-8", errors="ignore")


def _evidence_chain_head(evidence: dict[str, Any]) -> str:
    """Bind the evidence payload to its opaque adapter-issued provenance IDs."""
    payload = {key: value for key, value in evidence.items() if key != "provenance"}
    return hashlib.sha256(
        json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()


class PolyglotBowlingAgent(BaseInstalledAgent):
    """Runs the fixed Bowling unittest inside a Harbor task sandbox."""

    _NAME: ClassVar[str] = "polyglot-bowling"

    @staticmethod
    def name() -> str:
        return PolyglotBowlingAgent._NAME

    def version(self) -> str | None:
        return "0.1.0"

    def get_version_command(self) -> str | None:
        return "python --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        """Verifies the digest-bound task package inside the environment.

        A missing or drifted file is a trial failure, never something this
        adapter could paper over: the image is the only trusted source of the
        public tests and the frozen manifest.
        """
        checks = (
            f"test -f {TEST_FILE}",
            f"test -f {REFERENCE_FILE}",
            f"test -f {MANIFEST_FILE}",
        )
        result = await self.exec_as_root(environment, command=" && ".join(checks))
        if result.return_code != 0:
            raise RuntimeError(
                "the task image does not provide the digest-bound Bowling "
                f"package: {(result.stderr or '').strip()}"
            )
        # The image-owned manifest must carry the frozen identity; the verifier
        # re-validates the whole document strictly.
        result = await self.exec_as_root(
            environment,
            command=(
                f"python - {MANIFEST_FILE} <<'PYEOF'\n"
                "import json, sys\n"
                "document = json.load(open(sys.argv[1]))\n"
                "runner = document['runner']\n"
                "assert runner['id'] == 'harbor-python-unittest', runner\n"
                "assert document['dataset_id'] == 'exercism-polyglot'\n"
                "assert document['task']['task_id'] == 'python/exercises/practice/bowling'\n"
                "print('manifest-identity-ok')\n"
                "PYEOF"
            ),
        )
        if result.return_code != 0:
            raise RuntimeError(
                f"the image-owned manifest is not the frozen Bowling contract: "
                f"{(result.stdout or '').strip()} {(result.stderr or '').strip()}"
            )

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        mode = await self._resolve_mode(instruction, environment)
        await self._stage_candidate(environment, mode)
        result = await self._run_fixed_runner(environment)
        evidence, artifacts = self._build_evidence(result, mode, context)
        await self._publish(environment, evidence, artifacts)
        context.metadata = {
            **(context.metadata or {}),
            "bowling_candidate_mode": mode,
            "bowling_tests": evidence["tests"],
            "bowling_published_artifacts": list(artifacts),
        }

    # ------------------------------------------------------------------ steps

    async def _resolve_mode(self, instruction: str, environment: BaseEnvironment) -> str:
        """Resolves the candidate mode from the instruction, fail closed.

        The per-trial override travels as an agent env var, which lives in the
        task container's environment — the adapter process itself runs outside
        the sandbox, so the override is read back with an exec, never from
        `os.environ`.
        """
        try:
            document = json.loads(instruction)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"the instruction is not valid JSON: {error}") from error
        if document.get("schema_version") != INSTRUCTION_SCHEMA:
            raise RuntimeError(
                f"unsupported instruction schema: {document.get('schema_version')!r}"
            )
        if document.get("runner_id") != RUNNER_ID:
            raise RuntimeError("the instruction names a different runner")
        allowed = document.get("allowed_candidate_modes")
        if allowed is not None and list(allowed) != list(ALLOWED_MODES):
            raise RuntimeError(f"unexpected candidate mode allowlist: {allowed!r}")
        probe = await self.exec_as_root(
            environment, command=f"printf '%s' \"${{{MODE_ENV}}}\""
        )
        override = (probe.stdout or "").strip()
        mode = override or document.get("candidate_mode")
        if mode not in ALLOWED_MODES:
            raise RuntimeError(
                f"candidate mode {mode!r} is not one of {list(ALLOWED_MODES)}"
            )
        return mode

    async def _stage_candidate(self, environment: BaseEnvironment, mode: str) -> None:
        """Places the candidate `bowling.py` into the workspace.

        The reference candidate is copied from the image's digest-verified
        copy; the broken stub is staged locally and uploaded, so neither the
        task text nor the test text ever appears in a command or a log.
        """
        clean = await self.exec_as_root(
            environment,
            command=(
                f"rm -f {WORKSPACE_ROOT}/bowling.py && "
                f"chown -R candidate:candidate {WORKSPACE_ROOT}"
            ),
        )
        if clean.return_code != 0:
            raise RuntimeError(
                f"failed to reset the candidate workspace: {(clean.stderr or '').strip()}"
            )
        if mode == "reference":
            staged = await self.exec_as_root(
                environment,
                command=(
                    f"install -o candidate -g candidate -m 0444 "
                    f"{REFERENCE_FILE} {WORKSPACE_ROOT}/bowling.py"
                ),
            )
            if staged.return_code != 0:
                raise RuntimeError(
                    "failed to stage the reference candidate: "
                    f"{(staged.stderr or '').strip()}"
                )
            return
        with _temporary_candidate_file(_BROKEN_CANDIDATE) as staged:
            await environment.upload_file(staged, f"{WORKSPACE_ROOT}/bowling.py")
        restricted = await self.exec_as_root(
            environment,
            command=(
                f"chown candidate:candidate {WORKSPACE_ROOT}/bowling.py && "
                f"chmod 0444 {WORKSPACE_ROOT}/bowling.py"
            ),
        )
        if restricted.return_code != 0:
            raise RuntimeError("failed to place the broken candidate")

    async def _run_fixed_runner(self, environment: BaseEnvironment) -> dict[str, Any]:
        """Executes the manifest's fixed unittest argv as the candidate user.

        Resource enforcement: wall time via `timeout`, CPU/memory/processes via
        `ulimit`, memory additionally via the task cgroup. The unittest process
        runs as the non-root `candidate` user against the read-only `/tests`.
        """
        inner = (
            "ulimit -t 300; ulimit -v 524288; ulimit -u 8; "
            "exec python -m unittest discover -s /tests -p bowling_test.py"
        )
        command = (
            f"cd {WORKSPACE_ROOT} && "
            # /bin/bash (not dash) because dash's ulimit has no -u option.
            "runner_output=$(mktemp) && "
            "trap 'rm -f \"$runner_output\"' EXIT && "
            f"timeout {WALL_TIME_SEC + 10} su -s /bin/bash candidate "
            f"-c {shlex.quote(inner)} >\"$runner_output\" 2>&1; "
            "runner_exit=$?; cat \"$runner_output\"; "
            "printf '\\n__BOWLING_RUNNER_EXIT__=%s\\n' \"$runner_exit\"; "
            "exit 0"
        )
        result = await self.exec_as_root(environment, command=command, timeout_sec=WALL_TIME_SEC + 60)
        # The wrapper deliberately exits successfully so Harbor does not turn a
        # candidate test failure into an agent exception. Its final marker is
        # emitted after the captured child output, making the runner status an
        # adapter fact even when candidate code writes arbitrary stdout.
        output = (result.stdout or "") + (result.stderr or "")
        match = re.search(r"(?m)^__BOWLING_RUNNER_EXIT__=([0-9]+)$", output)
        if match is None:
            raise RuntimeError("the fixed runner did not publish an exit status")
        exit_status = int(match.group(1))
        if not 0 <= exit_status <= 255:
            raise RuntimeError(f"the fixed runner returned an invalid exit status: {exit_status!r}")
        output = output[: match.start()]
        stdout = _truncate(output)
        stderr = ""
        timed_out = exit_status == 124
        dependency_error = exit_status == 5
        return {
            "exit_status": exit_status,
            "timed_out": timed_out,
            "dependency_error": dependency_error,
            "infrastructure_error": False,
            "stdout": stdout,
            "stderr": stderr,
        }

    def _build_evidence(
        self,
        result: dict[str, Any],
        mode: str,
        context: AgentContext,
    ) -> tuple[dict[str, Any], dict[str, bytes]]:
        """Builds the strict `external_runner_result.v1` evidence document."""
        tests = _parse_tests(result["stdout"] + result["stderr"])
        run_id = f"bowling-{uuid.uuid4().hex}"
        session_id = f"session-{uuid.uuid4().hex}"
        provenance = {
            "run_id": run_id,
            "session_id": session_id,
            "chain_head": "0" * 64,
        }
        # Raw output artifacts are declared before the evidence document is
        # serialized so the evidence binds their exact bytes.
        artifacts: dict[str, bytes] = {
            "runner_stdout.txt": result["stdout"].encode("utf-8"),
            "runner_stderr.txt": result["stderr"].encode("utf-8"),
        }
        evidence = {
            "schema_version": 1,
            "result_kind": RESULT_KIND,
            "dataset_id": "exercism-polyglot",
            "dataset_version": 1,
            "split": "python_smoke",
            "snapshot_digest": "sha256:3dfe5f25603f104748b4205739c9d3d5676a470777c261255a501b45cec3a9cc",
            "task_digest": "sha256:69c60c9ae3a9c8d880486f2f6044fc410ec937a35497a2412941088f61e41d0c",
            "runner_id": RUNNER_ID,
            "runner_version": RUNNER_VERSION,
            "verifier_id": VERIFIER_ID,
            "verifier_version": VERIFIER_VERSION,
            "exit_status": result["exit_status"],
            "timed_out": result["timed_out"],
            "dependency_error": result["dependency_error"],
            "infrastructure_error": result["infrastructure_error"],
            "tests": tests,
            "declared_artifacts": [
                {"path": name, "sha256": hashlib.sha256(body).hexdigest()}
                for name, body in sorted(artifacts.items())
            ],
            "provenance": provenance,
        }
        evidence["provenance"]["chain_head"] = _evidence_chain_head(evidence)
        context.metadata = {
            **(context.metadata or {}),
            "bowling_provenance": provenance,
        }
        return evidence, artifacts

    async def _publish(
        self,
        environment: BaseEnvironment,
        evidence: dict[str, Any],
        artifacts: dict[str, bytes],
    ) -> None:
        """Publishes only the declared artifacts, with hash-bound evidence."""
        payloads = {
            **artifacts,
            "task_manifest.json": await self._read_remote_manifest(environment),
        }
        payloads[EVIDENCE_ARTIFACT] = json.dumps(
            evidence, indent=2, sort_keys=True
        ).encode("utf-8")
        for name, body in payloads.items():
            encoded = base64.b64encode(body).decode("ascii")
            write = await self.exec_as_root(
                environment,
                command=(
                    f"install -d -m 0750 {ARTIFACT_ROOT} && "
                    f"printf '%s' {shlex.quote(encoded)} | base64 -d > "
                    f"{ARTIFACT_ROOT / name}"
                ),
            )
            if write.return_code != 0:
                raise RuntimeError(f"failed to publish artifact '{name}'")

    async def _read_remote_manifest(self, environment: BaseEnvironment) -> bytes:
        read = await self.exec_as_root(
            environment, command=f"cat {MANIFEST_FILE}"
        )
        if read.return_code != 0:
            raise RuntimeError("the frozen manifest disappeared from the task image")
        return (read.stdout or "").encode("utf-8")


class _temporary_candidate_file:
    """Context manager for a locally staged candidate payload."""

    def __init__(self, content: str) -> None:
        self._handle = tempfile.NamedTemporaryFile(
            "w", suffix=".py", delete=False, encoding="utf-8"
        )
        self._handle.write(content)
        self._handle.close()

    def __enter__(self) -> Path:
        return Path(self._handle.name)

    def __exit__(self, *exc: object) -> None:
        Path(self._handle.name).unlink(missing_ok=True)


def _parse_tests(output: str) -> dict[str, int]:
    """Parse exactly one unittest summary; reject candidate-spoofed ambiguity."""
    ran_matches = re.findall(r"^Ran (\d+) tests? in ", output, re.MULTILINE)
    if len(ran_matches) != 1:
        raise RuntimeError("runner output must contain exactly one test summary")
    discovered = int(ran_matches[0])
    if discovered == 0:
        raise RuntimeError("runner output discovered no tests")
    ok_matches = re.findall(r"^OK( \(.+\))?$", output, re.MULTILINE)
    failed_matches = re.findall(r"^FAILED \((.+)\)$", output, re.MULTILINE)
    if len(ok_matches) + len(failed_matches) != 1:
        raise RuntimeError("runner output must contain exactly one terminal test status")
    if ok_matches:
        return {"discovered": discovered, "failed": 0, "errors": 0}
    fields: dict[str, int] = {}
    for part in failed_matches[0].split(", "):
        key, _, value = part.partition("=")
        try:
            fields[key] = int(value)
        except ValueError as error:
            raise RuntimeError("runner failure summary contains a non-integer count") from error
    return {
        "discovered": discovered,
        "failed": fields.get("failures", 0),
        "errors": fields.get("errors", 0),
    }


__all__ = ["ARTIFACT_ROOT", "DECLARED_ARTIFACTS", "PolyglotBowlingAgent"]
