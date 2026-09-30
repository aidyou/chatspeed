"""ChatSpeed installed-agent adapter for Harbor (pinned to harbor==0.23.0).

Harbor installs and runs this adapter *inside a task environment*: it verifies
the ChatSpeed binaries that are already present in that environment, proves the
sandbox to the headless runtime through a capability manifest, drives one
scheduled run with `cs` over an explicit discovery file, and publishes only
allowlisted artifacts to `/logs/artifacts/`.

Two boundaries are deliberate:

* the adapter never opens the experiment database — it is an HTTP-only client,
  exactly like `cs` itself (INV-1);
* the adapter never publishes anything the run did not declare: artifact
  publication is driven by `artifact_contract.py`, and a run whose declared files
  are missing is reported as a failed trial rather than silently accepted.
"""

from __future__ import annotations

import asyncio
import json
import os
import shlex
import tempfile
import time
from pathlib import Path, PurePosixPath
from typing import Any, ClassVar, override

from harbor.agents.installed.base import BaseInstalledAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from artifact_contract import (
    ARTIFACT_ROOT,
    CAPABILITY_FILE,
    DISCOVERY_FILE,
    DOMAIN_ROOT,
    READ_ONLY_ROOTS,
    TASK_ROOT,
    WORKSPACE_ROOT,
    capability_manifest,
    declared_artifacts,
    publish_artifacts,
)

#: The binaries this adapter requires inside the task environment.
HEADLESS_BINARY = "chatspeed-headless"
CLI_BINARY = "cs"

#: Where the pinned smoke plan is written before it is scheduled.
PLAN_FILE = PurePosixPath("/installed-agent/chatspeed-plan.json")

#: The server-side execution profile this adapter registers for the task. A
#: durable schedule may only name a registered profile, and a `harbor_task`
#: profile is the one that proves the sandbox instead of creating a container.
HARBOR_PROFILE_REF = "harbor-task"
PROFILE_DIR = DOMAIN_ROOT / "execution-profiles"
PROFILE_FILE = PROFILE_DIR / f"{HARBOR_PROFILE_REF}.json"

#: The agent the pinned smoke plan names, and the config package that provides
#: it. A fresh experiment domain has no agents, and the adapter is HTTP-only, so
#: the agent arrives through the runtime's own credential/config input — a
#: permission-restricted file that carries no secret (INV-6).
SMOKE_AGENT_ID = "harbor-smoke-agent"
CONFIG_PACKAGE_FILE = PurePosixPath("/installed-agent/chatspeed-config-package.json")

#: The model endpoint and the token that reaches it. Both are supplied by the
#: job environment (`CHATSPEED_SMOKE_MODEL_BASE_URL` / `__TOKEN`), never
#: committed, and the package that carries them is written 0600 inside the
#: task root — it is an input, never a published artifact (INV-6).
MODEL_BASE_URL_ENV = "CHATSPEED_SMOKE_MODEL_BASE_URL"
MODEL_TOKEN_ENV = "CHATSPEED_SMOKE_MODEL_TOKEN"
MODEL_ID = "ds-v4-flash"
MODEL_ROW_ID = 1
#: The categories an `agents` import depends on (the runtime closes the set over
#: its dependencies). The package document names them the way the runtime
#: serializes them (camelCase); the CLI enum takes kebab-case.
CONFIG_PACKAGE_CATEGORIES = ("aiModels", "skills", "mcp", "sandbox", "agents")
CONFIG_CLI_CATEGORIES = ("ai-models", "skills", "mcp", "sandbox", "agents")
#: `--config-category` is repeatable and the import is never wholesale.
CONFIG_PACKAGE_FLAGS = " ".join(
    f"--config-category {category}" for category in CONFIG_CLI_CATEGORIES
)

#: The roots this adapter creates itself and can therefore guarantee exist.
#: Every other declared root belongs to the task image, and is required rather
#: than created: a capability that declares a missing root makes
#: `HarborTaskOwner::preflight` fail closed (INV-4).
OWNED_ROOTS = (TASK_ROOT, ARTIFACT_ROOT, DOMAIN_ROOT)


#: How long the adapter waits for the durable job to reach a terminal state.
TERMINAL_WAIT_SEC = 480.0
TERMINAL_POLL_SEC = 5.0

#: The durable states that end a trial.
TERMINAL_JOB_STATES = frozenset(
    {"succeeded", "failed", "failed_precondition", "cancelled", "unknown_manual"}
)


def _model_row() -> dict[str, Any]:
    """The single model the sandbox may reach.

    The durable run is executed *inside* the sandbox, so its model has to be
    reachable from there: this row points at the operator-supplied endpoint (the
    host's own chat-completion proxy in the smoke setup) and carries the token
    that endpoint issued. The token is read from the job environment, so it is
    never written into the repository, the task or a published artifact.
    """
    base_url = os.environ.get(MODEL_BASE_URL_ENV, "")
    token = os.environ.get(MODEL_TOKEN_ENV, "")
    if not base_url or not token:
        raise RuntimeError(
            f"the trial needs {MODEL_BASE_URL_ENV} and {MODEL_TOKEN_ENV} so the "
            "scheduled run can reach a model from inside the sandbox"
        )
    return {
        "id": MODEL_ROW_ID,
        "name": "Smoke model endpoint",
        "models": json.dumps(
            [
                {
                    "id": MODEL_ID,
                    "name": MODEL_ID,
                    "group": "Smoke",
                    "functionCall": True,
                    "contextSize": 32768,
                    "maxTokens": 4096,
                    "temperature": 1.0,
                    "customParams": [],
                }
            ]
        ),
        "default_model": MODEL_ID,
        "api_protocol": "openai",
        "base_url": base_url,
        "api_key": token,
        "max_tokens": 4096,
        "temperature": 1.0,
        "top_p": 1.0,
        "top_k": 40,
        "sort_index": 0,
        "is_default": True,
        "disabled": False,
        "is_official": False,
        "official_id": "",
        "metadata": None,
    }


def _first_json_object(payload: str) -> dict[str, Any] | None:
    """The first JSON object in a CLI response, or `None`.

    `cs` is a thin HTTP client: a successful response is exactly one JSON
    document, but an operator-facing banner may precede it, so the parse takes
    the first decodable object instead of trusting the whole stream.
    """
    decoder = json.JSONDecoder()
    index = payload.find("{")
    while index != -1:
        try:
            value, _ = decoder.raw_decode(payload[index:])
        except json.JSONDecodeError:
            index = payload.find("{", index + 1)
            continue
        if isinstance(value, dict):
            return value
        index = payload.find("{", index + 1)
    return None


def _job_states(jobs_jsonl: str) -> set[str]:
    """The durable states of a `campaign jobs` JSONL projection."""
    states: set[str] = set()
    for line in jobs_jsonl.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            document = json.loads(line)
        except json.JSONDecodeError:
            continue
        for job in document.get("jobs", []):
            state = job.get("state")
            if isinstance(state, str):
                states.add(state)
    return states


class ChatSpeedAgent(BaseInstalledAgent):
    """Runs one ChatSpeed scheduled campaign inside a Harbor task sandbox."""

    #: Harbor's registry name for this adapter.
    _NAME: ClassVar[str] = "chatspeed"

    @staticmethod
    def name() -> str:
        return ChatSpeedAgent._NAME

    def version(self) -> str | None:
        # The adapter's own version; the binaries' versions are verified in
        # `install` so a mismatch is reported per trial, not per adapter.
        return "0.1.0"

    def get_version_command(self) -> str | None:
        return f"{CLI_BINARY} --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        """Verifies the ChatSpeed binaries in the task environment.

        It never downloads them: the task environment is provisioned with the
        exact binaries under test, so an absent binary is a trial failure rather
        than something this adapter could paper over.
        """
        for binary in (HEADLESS_BINARY, CLI_BINARY):
            result = await self.exec_as_root(
                environment,
                command=f"command -v {binary} && {binary} --version",
            )
            if result.return_code != 0:
                raise RuntimeError(
                    f"the task environment does not provide '{binary}': "
                    f"{(result.stderr or '').strip()}"
                )

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        """Runs one scheduled campaign and publishes its declared artifacts."""
        await self._write_plan(instruction, environment)
        await self._write_config_package(environment)
        await self._write_capability_manifest(environment)
        await self._start_headless(environment)
        await self._schedule_and_wait(environment, context)
        published = await publish_artifacts(
            self.exec_as_root, environment, self.logs_dir
        )
        context.metadata = {
            **(context.metadata or {}),
            "chatspeed_published_artifacts": published,
        }

    # ------------------------------------------------------------------ steps

    async def _write_config_package(self, environment: BaseEnvironment) -> None:
        """Provides the agent the pinned plan names.

        The credential travels as a *file*: Harbor logs every command it executes
        verbatim, so a token embedded in a `printf ... > file` command would be
        persisted into the job/trial logs. The package is therefore staged
        locally and copied in through the environment's upload API, and only a
        secret-free `chmod`/`chown` is executed against it (INV-6).

        The domain is fresh and the adapter never touches its database (INV-1),
        so the agent arrives the way an operator would supply it: a
        permission-restricted config package imported for one explicitly selected
        category. It declares the agent only — no model, no key, no secret.
        """
        package = {
            "formatVersion": 1,
            "exportedAt": "1970-01-01T00:00:00Z",
            "categories": list(CONFIG_PACKAGE_CATEGORIES),
            # The categories the agent import depends on are declared empty: the
            # sandbox needs none of them, and an undeclared payload would be a
            # category the operator did not select.
            "aiModels": {"rows": [_model_row()], "config": {}},
            "skills": {"rows": [], "config": {}},
            "mcp": {"rows": [], "config": {}},
            "sandbox": {"rows": [], "config": {}},
            "agents": {
                "rows": [
                    {
                        "id": SMOKE_AGENT_ID,
                        "name": "Harbor Smoke Agent",
                        "description": "Minimal agent for the ChatSpeed Harbor smoke trial.",
                        "system_prompt": (
                            "You are a minimal smoke-test agent. Follow the instruction "
                            "exactly and reply with nothing else."
                        ),
                        "agent_type": "autonomous",
                        "models": json.dumps(
                            {
                                phase: {"id": MODEL_ROW_ID, "model": MODEL_ID}
                                for phase in ("plan", "act", "utility", "lite")
                            }
                        ),
                        "available_tools": "[]",
                        "auto_approve": "[]",
                        "final_audit": False,
                        "approval_level": "default",
                        "role": "primary",
                        "skill_enabled": False,
                        "is_system": False,
                        "disabled": False,
                        "phase": "standard",
                        "sort_index": 0,
                        "version": 1,
                        "sandbox_execution_mode": "host_only",
                    }
                ],
                "config": {},
            },
        }
        await self.exec_as_root(
            environment,
            command=(
                f"install -d -m 0700 -o root -g root "
                f"{PurePosixPath(CONFIG_PACKAGE_FILE).parent}"
            ),
        )
        with tempfile.TemporaryDirectory(prefix="cs-harbor-") as staging:
            staged = Path(staging) / PurePosixPath(CONFIG_PACKAGE_FILE).name
            staged.write_text(json.dumps(package, indent=2), encoding="utf-8")
            staged.chmod(0o600)
            await environment.upload_file(staged, str(CONFIG_PACKAGE_FILE))
        restricted = await self.exec_as_root(
            environment,
            command=(
                f"chmod 0600 {CONFIG_PACKAGE_FILE} && "
                f"chown root:root {CONFIG_PACKAGE_FILE}"
            ),
        )
        if restricted.return_code != 0:
            raise RuntimeError("failed to restrict the agent config package")

    async def _write_plan(self, instruction: str, environment: BaseEnvironment) -> None:
        """Writes the frozen plan the instruction resolved to.

        The instruction is the checked-in task text; the plan is written verbatim
        so the durable schedule stores refs, never the instruction body itself
        (INV-6).
        """
        plan = json.loads(instruction)
        await environment.exec(
            command=(
                f"mkdir -p {PLAN_FILE.parent} && "
                f"cat > {PLAN_FILE} <<'CHATSPEED_PLAN'\n"
                f"{json.dumps(plan, indent=2)}\n"
                "CHATSPEED_PLAN\n"
            ),
            user="root",
        )

    async def _write_capability_manifest(self, environment: BaseEnvironment) -> None:
        """Proves this sandbox to the headless runtime.

        The manifest is written *inside the experiment domain*, at the exact
        path the runtime resolves (`harbor_capability_path`), because a manifest
        the runtime never reads leaves every Harbor-owner job refused with
        `ownership_mismatch`.

        It also reconciles the declared roots with what this environment really
        provides: the adapter creates the roots it owns, and requires the ones
        the task image must ship. A capability that declared a missing root
        would make `HarborTaskOwner::preflight` fail closed, so the trial is
        failed here with the concrete root named instead.

        The manifest carries the owner token hash the headless instance uses to
        fence its owner, so it must not be readable by anyone else
        (AC-4/INV-8).
        """
        created = " && ".join(
            f"install -d -m 0700 -o root -g root {root}" for root in OWNED_ROOTS
        )
        required = " ".join(
            str(root) for root in (WORKSPACE_ROOT, *READ_ONLY_ROOTS)
        )
        checked = await self.exec_as_root(
            environment,
            command=(
                f"{created} && "
                f"for root in {required}; do "
                '[ -e "$root" ] || { echo "missing declared root: $root" >&2; exit 1; }; '
                "done"
            ),
        )
        if checked.return_code != 0:
            raise RuntimeError(
                "the task environment does not provide every root the capability "
                f"declares ({required}): {(checked.stderr or '').strip()}"
            )
        manifest = capability_manifest(environment)
        payload = json.dumps(manifest, indent=2)
        command = (
            f"install -d -m 0700 -o root -g root {PurePosixPath(CAPABILITY_FILE).parent} && "
            f"printf '%s\\n' {shlex.quote(payload)} > {CAPABILITY_FILE} && "
            f"chmod 0600 {CAPABILITY_FILE}"
        )
        await self.exec_as_root(environment, command=command)

    async def _start_headless(self, environment: BaseEnvironment) -> None:
        """Starts the isolated headless instance on an explicit data dir."""
        start = (
            f"mkdir -p {DOMAIN_ROOT} && "
            f"nohup {HEADLESS_BINARY} --data-dir {DOMAIN_ROOT} "
            f"--base-repo {WORKSPACE_ROOT} "
            f"--config-package {CONFIG_PACKAGE_FILE} "
            f"{CONFIG_PACKAGE_FLAGS} "
            f"> {DOMAIN_ROOT}/headless.log 2>&1 & echo $!"
        )
        await self.exec_as_root(environment, command=start)
        doctor = (
            f"{CLI_BINARY} --discovery-file {DISCOVERY_FILE} doctor"
        )
        result = await self.exec_as_root(environment, command=doctor)
        if result.return_code != 0:
            raise RuntimeError(
                "the headless instance did not become reachable: "
                f"{(result.stderr or '').strip()}"
            )

    async def _schedule_and_wait(
        self, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        """Registers the task profile, schedules one campaign and waits for it.

        The execution profile is provisioned here, before the schedule, because
        a durable schedule may only name a profile the *server* registered
        (`unknown_execution_profile` otherwise). It is a `harbor_task` profile:
        the sandbox Harbor provisioned is the isolation boundary, so the owner
        proves it with the capability manifest instead of creating a container.
        """
        profile = {
            "schema_version": "execution_profile.v1",
            "profile_ref": HARBOR_PROFILE_REF,
            "owner_kind": "harbor_task",
            "base_repo_ref": str(WORKSPACE_ROOT),
            "base_revision": "HEAD",
            "network_policy": {"mode": "none", "allow_hosts": []},
            "mounts": [
                {
                    "source_kind": "workspace",
                    "container_path": str(WORKSPACE_ROOT),
                    "read_only": False,
                }
            ],
            "resources": {
                "cpu_millis": 2000,
                "memory_bytes": 4294967296,
                "pids": 512,
                "no_new_privileges": True,
            },
            "allowed_bundle_refs": [],
        }
        payload = json.dumps(profile, indent=2)
        await self.exec_as_root(
            environment,
            command=(
                f"install -d -m 0700 -o root -g root {PROFILE_DIR} && "
                f"printf '%s\\n' {shlex.quote(payload)} > {PROFILE_FILE}"
            ),
        )

        discovery = str(DISCOVERY_FILE)
        schedule = (
            f"{CLI_BINARY} --discovery-file {discovery} --output json "
            f"experiment campaign schedule --plan {PLAN_FILE} "
            f"--profile {HARBOR_PROFILE_REF}"
        )
        result = await self.exec_as_root(environment, command=schedule)
        if result.return_code != 0:
            raise RuntimeError(
                f"the durable schedule was rejected: {(result.stderr or '').strip()}"
            )
        accepted = _first_json_object(result.stdout or "")
        if accepted is None:
            raise RuntimeError(
                "the durable schedule returned no readable JSON document: "
                f"{(result.stdout or '').strip()[:400]!r}"
            )
        campaign_id = accepted.get("campaign_id", "")
        context.metadata = {
            **(context.metadata or {}),
            "chatspeed_campaign_id": campaign_id,
            "chatspeed_job_ids": accepted.get("job_ids", []),
        }
        jobs = await self.exec_as_root(
            environment,
            command=(
                f"{CLI_BINARY} --discovery-file {discovery} --output jsonl "
                f"experiment campaign jobs --campaign-id {shlex.quote(campaign_id)}"
            ),
        )
        # The durable rows are the authority, so the trial waits for the job to
        # reach a terminal state instead of trusting the schedule response.
        deadline = time.monotonic() + TERMINAL_WAIT_SEC
        while time.monotonic() < deadline:
            jobs = await self.exec_as_root(
                environment,
                command=(
                    f"{CLI_BINARY} --discovery-file {discovery} --output jsonl "
                    f"experiment campaign jobs --campaign-id {shlex.quote(campaign_id)}"
                ),
            )
            states = _job_states(jobs.stdout or "")
            if states and states <= TERMINAL_JOB_STATES:
                break
            await asyncio.sleep(TERMINAL_POLL_SEC)
        context.metadata = {
            **(context.metadata or {}),
            "chatspeed_job_states": jobs.stdout,
        }
        await self._write_declared_artifacts(environment, accepted, jobs.stdout)
        states = _job_states(jobs.stdout or "")
        if states != {"succeeded"}:
            # The runtime log is the authority on why a job did not reach its
            # terminal state, so it is surfaced here (Harbor prints command output).
            diagnostic = await self.exec_as_root(
                environment,
                command=f"tail -40 {DOMAIN_ROOT}/headless.log || true",
            )
            context.metadata = {
                **(context.metadata or {}),
                "chatspeed_runtime_log_tail": diagnostic.stdout,
            }
            raise RuntimeError(
                "the scheduled campaign did not succeed inside the task environment: "
                f"{sorted(states) or ['<unknown>']}; runtime log:\n"
                f"{(diagnostic.stdout or '').strip()[-2000:]}"
            )

    async def _write_declared_artifacts(
        self,
        environment: BaseEnvironment,
        accepted: dict[str, Any],
        jobs_jsonl: str,
    ) -> None:
        """Writes the declared artifacts the verifier recomputes against.

        Only the durable projection leaves the sandbox: the campaign identity
        and the ordered job rows (state, dispatch marker, run id, error code).
        No prompt, response, transcript or credential is ever written here
        (INV-6), and the artifact contract re-checks that on publication.
        """
        campaign_doc = json.dumps(
            {
                "campaign_id": accepted.get("campaign_id", ""),
                "execution_profile_ref": accepted.get("execution_profile_ref", ""),
                "job_ids": accepted.get("job_ids", []),
                "plan_hash": accepted.get("plan_hash", ""),
                "schema_version": "harbor_smoke_campaign.v1",
            },
            indent=2,
            sort_keys=True,
        )
        for name, body in (
            ("chatspeed-campaign.json", campaign_doc),
            ("chatspeed-jobs.jsonl", jobs_jsonl),
        ):
            write = await self.exec_as_root(
                environment,
                command=(
                    f"printf '%s\\n' {shlex.quote(body)} > "
                    f"{shlex.quote(str(WORKSPACE_ROOT / name))}"
                ),
            )
            if write.return_code != 0:
                raise RuntimeError(f"failed to write the declared artifact '{name}'")


def declared_artifact_paths() -> list[Path]:
    """The artifact paths this adapter is allowed to publish."""
    return declared_artifacts()


__all__ = [
    "ARTIFACT_ROOT",
    "ChatSpeedAgent",
    "declared_artifact_paths",
]
