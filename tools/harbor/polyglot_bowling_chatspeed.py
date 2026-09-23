"""ChatSpeed candidate adapter for the Phase 3E Bowling task.

This adapter generates exactly one Bowling candidate through the *existing*
ChatSpeed control-plane (a headless instance plus one ordinary workflow
session) inside the Harbor task sandbox, and then grades whatever the agent
produced with the same fixed Bowling runner and the same independent verifier
as the deterministic task. It deliberately subclasses the deterministic
Bowling adapter and reuses the proven ChatSpeed smoke adapter's private
helpers unbound, so neither `chatspeed_agent.py` nor `artifact_contract.py`
needs any change (INV-1).

Boundaries:

* the model endpoint and token travel from the operator's job environment into
  the in-task 0600 config package file — never via argv, a published artifact
  or a log;
* the task's network is a Harbor-enforced egress allowlist (nftables + gost
  sidecar) that permits only the operator's local model proxy address; the
  adapter probes this boundary in-task before the candidate session and fails
  closed if any public egress is possible;
* the candidate session's tools are the control plane's file tools restricted
  to `/workspace` (`allowed_paths`); no shell tool is granted, and the agent
  runs under scheme-backed `sandbox_only` execution whose resolver denies any
  shell command without a runnable sandbox profile — there is no host fallback
  in either layer. The headless instance itself runs inside the task
  container, so every session-side operation stays under the Harbor sandbox
  boundary;
* the experiment's 2A artifact bundle (which contains the transcript) is
  captured *inside* the task at `/installed-agent/bowling-run-artifact` and is
  never published (INV-4);
* a failed candidate is a `candidate_failure` classification, not a trial
  error; only control-plane failures raise.
"""

from __future__ import annotations

import json
import os
import shlex
import tempfile
import urllib.parse
from pathlib import Path, PurePosixPath
from typing import Any, ClassVar, override

from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from chatspeed_agent import (
    CLI_BINARY,
    HEADLESS_BINARY,
    SMOKE_AGENT_ID,
    ChatSpeedAgent,
    _first_json_object,
    _model_row,
)

from polyglot_bowling_agent import (
    INSTRUCTION_SCHEMA,
    PolyglotBowlingAgent,
    RUNNER_ID,
    WORKSPACE_ROOT,
)

#: The operator-selected model for this candidate smoke (`cs@qwen3.8-flash` in
#: the operator's notation: the `cs` proxy group plus this model id). It is
#: pinned here instead of reusing the old smoke adapter's default so the
#: Bowling candidate trial runs exactly the model the operator named.
CANDIDATE_MODEL_ID = "qwen3.8-flash"

#: Where the candidate prompt is staged inside the task.
PROMPT_FILE = PurePosixPath("/installed-agent/bowling-prompt.txt")

#: The headless instance's data dir (same value `_start_headless` passes).
DOMAIN_ROOT = PurePosixPath("/installed-agent/chatspeed-domain")

#: How long the adapter waits for the candidate session to reach a terminal
#: state. The agent phase budget (task.toml `timeout_sec`) covers install,
#: headless start and the boundary probes before this window, and the driver
#: exec gets a small tail after the deadline so it can still print its status.
EXPERIMENT_WAIT_SEC = 1200
EXPERIMENT_EXEC_TAIL_SEC = 60

#: The in-task driver that drives one workflow session through the control
#: plane's ordinary HTTP surface. The bearer token is read from the discovery
#: document inside the task — it never appears on a command line or in a log.
_RUN_DRIVER_FILE = PurePosixPath("/installed-agent/bowling_run_driver.py")
_RUN_DRIVER_PY = """\
import json
import sys
import time
import urllib.error
import urllib.request

DISCOVERY = "/installed-agent/chatspeed-domain/runtime/control-plane-v1.json"
PROMPT = "/installed-agent/bowling-prompt.txt"
AGENT_ID = "__AGENT_ID__"
TERMINAL = ("completed", "error", "cancelled")


def call(method, path, body=None):
    request = urllib.request.Request(base + path, method=method)
    request.add_header("Authorization", "Bearer " + token)
    payload = None
    if body is not None:
        payload = json.dumps(body).encode("utf-8")
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, data=payload, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", errors="replace")[:400]


with open(DISCOVERY, encoding="utf-8") as handle:
    discovery = json.load(handle)
base = "http://{}:{}".format(discovery["host"], discovery["port"])
token = discovery["token"]

with open(PROMPT, encoding="utf-8") as handle:
    prompt = handle.read()

code, document = call(
    "POST",
    "/control/v1/workflows",
    {
        "user_query": None,
        "agent_id": AGENT_ID,
        "allowed_paths": ["/workspace"],
        "auto_approve_plan": True,
        "final_audit": False,
    },
)
if code != 201:
    print("workflow create failed: HTTP {}: {}".format(code, document), file=sys.stderr)
    raise SystemExit(1)
session_id = document["session_id"]

code, document = call(
    "POST",
    "/control/v1/workflows/{}/start".format(session_id),
    {"session_id": session_id, "agent_id": AGENT_ID, "initial_prompt": prompt},
)
if code not in (200, 201):
    print("workflow start failed: HTTP {}: {}".format(code, document), file=sys.stderr)
    raise SystemExit(1)

deadline = time.time() + __WAIT_SEC__
status = ""
while time.time() < deadline:
    code, document = call("GET", "/control/v1/workflows/{}".format(session_id))
    if code == 200:
        status = (document.get("workflow") or {}).get("status", "")
        if status in TERMINAL:
            break
    time.sleep(2)
print(
    json.dumps({"session_id": session_id, "status": status or "unknown"})
)
raise SystemExit(0 if status == "completed" else 1)
""".replace("__AGENT_ID__", SMOKE_AGENT_ID).replace("__WAIT_SEC__", str(EXPERIMENT_WAIT_SEC))


#: The sandbox scheme provisioned for the candidate agent. `sandbox_only`
#: execution means every shell command must run inside a sandbox profile and
#: is otherwise *denied* — there is no host fallback in the resolver. The
#: declared profile targets a nested no-network container, which is
#: deliberately not runnable inside the task (no Docker authorization), so
#: any shell attempt fails closed. The session surface is the file tools,
#: path-guarded to /workspace by the control plane's allowed_paths.
CANDIDATE_SANDBOX_SCHEME_ID = "bowling-candidate-sandbox"
_CANDIDATE_SANDBOX_CONFIG = {
    "runtimePreference": "docker",
    "profiles": [
        {
            "id": "bowling-workspace-shell",
            "name": "Bowling Workspace Shell",
            "enabled": True,
            "priority": 0,
            "commandPatterns": ["^git\\s"],
            "runtimePreference": "docker",
            "image": "chatspeed-bowling-task:3e",
        }
    ],
    "hostRules": [],
}


def _candidate_model_row() -> dict[str, Any]:
    """The single model row this trial provisions.

    Reuses the old smoke adapter's `_model_row` so the operator-supplied
    endpoint/token handling stays identical (env-only, never argv), then pins
    the model id the operator selected for this candidate smoke.
    """
    row = _model_row()
    row["models"] = json.dumps(
        [
            {
                "id": CANDIDATE_MODEL_ID,
                "name": CANDIDATE_MODEL_ID,
                "group": "Smoke",
                "functionCall": True,
                "contextSize": 32768,
                "maxTokens": 4096,
                "temperature": 1.0,
                "customParams": [],
            }
        ]
    )
    row["default_model"] = CANDIDATE_MODEL_ID
    return row


class PolyglotBowlingChatSpeedAgent(PolyglotBowlingAgent):
    """Generates one Bowling candidate through the ChatSpeed control-plane."""

    _NAME: ClassVar[str] = "polyglot-bowling-chatspeed"

    @staticmethod
    def name() -> str:
        return PolyglotBowlingChatSpeedAgent._NAME

    def version(self) -> str | None:
        return "0.1.0"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        """Requires both the digest-bound Bowling package and the binaries."""
        await super().install(environment)
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
        spec = self._parse_instruction(instruction)
        # Prove this sandbox to the runtime, provision the agent/model package
        # and start the isolated headless instance.
        await ChatSpeedAgent._write_capability_manifest(self, environment)
        await self._write_candidate_config_package(environment)
        await ChatSpeedAgent._start_headless(self, environment)
        # Fail closed before any model traffic unless the task's egress is
        # exactly the Harbor allowlist (proxy reachable, public egress denied)
        # and snapshot the credential package so the candidate session can be
        # audited for touching it.
        boundary = await self._verify_network_boundary(environment)
        package_digest = await self._config_package_digest(environment)
        await self._run_experiment(spec, environment, context)
        if await self._config_package_digest(environment) != package_digest:
            raise RuntimeError(
                "the candidate config package changed during the session; "
                "refusing to grade"
            )
        # Diagnose the candidate workspace before grading: this stays in the
        # trial metadata (never a published artifact) and distinguishes "the
        # agent produced no candidate" from "the candidate is wrong".
        probe = await self.exec_as_root(
            environment,
            command=(
                f"ls -la {WORKSPACE_ROOT} ; "
                f"git -C {WORKSPACE_ROOT} status --porcelain ; "
                f"tail -20 {DOMAIN_ROOT}/headless.log"
            ),
        )
        context.metadata = {
            **(context.metadata or {}),
            "bowling_workspace_state": (probe.stdout or "").strip()[-1500:],
        }
        # Grade whatever the agent produced with the shared fixed runner.
        result = await self._run_fixed_runner(environment)
        evidence, artifacts = self._build_evidence(result, "chatspeed", context)
        await self._publish(environment, evidence, artifacts)
        context.metadata = {
            **(context.metadata or {}),
            "bowling_candidate_mode": "chatspeed",
            "bowling_network_boundary": boundary,
            "bowling_tests": evidence["tests"],
            "bowling_published_artifacts": list(artifacts),
        }

    # ------------------------------------------------------------------ steps

    async def _write_candidate_config_package(self, environment: BaseEnvironment) -> None:
        """Provisions the smoke agent pinned to the operator-selected model.

        Structurally identical to the old smoke adapter's package (same file
        path, same 0600 in-task flow, same env-only credential handling), with
        the model row and every phase binding pointed at the operator-selected
        candidate model instead of the old smoke default.
        """
        from chatspeed_agent import (
            CONFIG_PACKAGE_CATEGORIES,
            CONFIG_PACKAGE_FILE,
            MODEL_ROW_ID,
        )

        package = {
            "formatVersion": 1,
            "exportedAt": "1970-01-01T00:00:00Z",
            "categories": list(CONFIG_PACKAGE_CATEGORIES),
            "aiModels": {"rows": [_candidate_model_row()], "config": {}},
            "skills": {"rows": [], "config": {}},
            "mcp": {"rows": [], "config": {}},
            "sandbox": {
                "rows": [
                    {
                        "id": CANDIDATE_SANDBOX_SCHEME_ID,
                        "name": "Bowling Candidate Sandbox",
                        "description": (
                            "Fail-closed sandbox scheme for the in-task Bowling "
                            "candidate: shell execution is denied unless a sandbox "
                            "profile can run; there is no host fallback."
                        ),
                        "config": json.dumps(_CANDIDATE_SANDBOX_CONFIG),
                        "disabled": 0,
                    }
                ],
                "config": {},
            },
            "agents": {
                "rows": [
                    {
                        "id": SMOKE_AGENT_ID,
                        "name": "Bowling Candidate Agent",
                        "description": "Minimal agent for the Bowling candidate smoke trial.",
                        "system_prompt": (
                            "You are a focused coding agent. Complete the given task "
                            "exactly as instructed and nothing else."
                        ),
                        "agent_type": "autonomous",
                        "models": json.dumps(
                            {
                                phase: {"id": MODEL_ROW_ID, "model": CANDIDATE_MODEL_ID}
                                for phase in ("plan", "act", "utility", "lite")
                            }
                        ),
                        # Only the control plane's file tools, additionally
                        # path-guarded to /workspace by allowed_paths in the
                        # workflow create call below. No shell/web surface, so
                        # the candidate cannot execute commands or reach
                        # beyond the declared task contract.
                        "available_tools": json.dumps(
                            ["read_file", "write_file", "edit_file", "list_dir"]
                        ),
                        "auto_approve": "[]",
                        "final_audit": False,
                        # No human is present in the sandbox, so approval is
                        # delegated to the trial boundary itself.
                        "approval_level": "full",
                        "role": "primary",
                        "skill_enabled": False,
                        "is_system": False,
                        "disabled": False,
                        "phase": "standard",
                        "sort_index": 0,
                        "version": 1,
                        # Scheme-backed sandbox_only execution: the resolver
                        # structurally denies any shell command that cannot run
                        # inside a sandbox profile (no host fallback), and the
                        # profile's nested no-network container is deliberately
                        # not runnable in-task, so shell attempts fail closed.
                        # The control plane fails closed as well unless a real
                        # sandbox scheme backs this mode (workflow.rs refuses
                        # scheme-less auto/sandbox_only). The actual session
                        # surface is the file tools above, additionally
                        # path-guarded to /workspace by allowed_paths.
                        "sandbox_execution_mode": "sandbox_only",
                        "sandbox_scheme_id": CANDIDATE_SANDBOX_SCHEME_ID,
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
        with tempfile.TemporaryDirectory(prefix="cs-bowling-") as staging:
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
            raise RuntimeError("failed to restrict the candidate config package")

    async def _verify_network_boundary(self, environment: BaseEnvironment) -> str:
        """Fails closed unless the task's egress is the Harbor allowlist.

        The in-task probe must reach the operator's local model proxy (any
        HTTP answer, including an auth error, proves the allowlisted route is
        open) and must be unable to open a single public egress connection. A
        probe that can reach the internet means the egress-control sidecar is
        not enforcing the declared policy, so the trial must not continue.
        """
        from chatspeed_agent import MODEL_BASE_URL_ENV

        base_url = os.environ.get(MODEL_BASE_URL_ENV, "")
        if not base_url:
            raise RuntimeError(f"the candidate trial needs {MODEL_BASE_URL_ENV}")
        target = urllib.parse.urlsplit(base_url)
        # The allowlist entry in harbor-chatspeed/task.toml is an IP: the
        # probe target must be the same address or the route check fails
        # closed below.
        route = f"http://{target.hostname}:{target.port or 80}/cs/v1/models"
        probe = (
            "import sys\n"
            "import urllib.error\n"
            "import urllib.request\n"
            f"try:\n    urllib.request.urlopen({route!r}, timeout=8)\n"
            "except urllib.error.HTTPError:\n"
            "    pass\n"
            "except Exception as error:\n"
            "    print('proxy route unreachable:', error)\n"
            "    raise SystemExit(1)\n"
            "try:\n"
            "    urllib.request.urlopen('http://1.1.1.1/', timeout=8)\n"
            "except Exception:\n"
            "    print('proxy route open; public egress denied')\n"
            "    raise SystemExit(0)\n"
            "print('public egress was NOT denied')\n"
            "raise SystemExit(1)\n"
        )
        result = await self.exec_as_root(
            environment,
            command=f"python3 -c {shlex.quote(probe)}",
            timeout_sec=40,
        )
        summary = (result.stdout or "").strip()[-300:]
        if result.return_code != 0:
            raise RuntimeError(
                "the task network boundary does not match the declared "
                f"allowlist: {summary or (result.stderr or '').strip()[-300:]}"
            )
        return summary

    async def _config_package_digest(self, environment: BaseEnvironment) -> str:
        """The in-task sha256 of the 0600 credential package."""
        from chatspeed_agent import CONFIG_PACKAGE_FILE

        result = await self.exec_as_root(
            environment, command=f"sha256sum {CONFIG_PACKAGE_FILE}"
        )
        parts = (result.stdout or "").split()
        if result.return_code != 0 or not parts:
            raise RuntimeError(
                "failed to hash the candidate config package: "
                f"{(result.stderr or '').strip()[-300:]}"
            )
        return parts[0]

    def _parse_instruction(self, instruction: str) -> dict[str, Any]:
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
        if document.get("candidate_mode") != "chatspeed":
            raise RuntimeError("the chatspeed task expects candidate_mode=chatspeed")
        agent_id = document.get("agent_id")
        if agent_id != SMOKE_AGENT_ID:
            raise RuntimeError(f"unexpected agent id: {agent_id!r}")
        prompt = document.get("candidate_prompt")
        if not isinstance(prompt, str) or not prompt.strip():
            raise RuntimeError("the instruction must carry a candidate prompt")
        return document

    async def _run_experiment(
        self,
        spec: dict[str, Any],
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        """Runs one candidate-generating workflow session in the sandbox.

        This uses the control plane's ordinary workflow surface (create + start
        + poll), not the budgeted experiment facade: a budgeted experiment
        scope pauses on any recorded overrun/unknown effect, which would abort
        a tool-driven candidate run for reasons outside the task contract. The
        isolation claim stays the Harbor sandbox itself.
        """
        prompt = spec["candidate_prompt"]
        write = await self.exec_as_root(
            environment,
            command=(
                f"install -d -m 0700 {PurePosixPath(PROMPT_FILE).parent} && "
                f"printf '%s\\n' {shlex.quote(prompt)} > {PROMPT_FILE}"
            ),
        )
        if write.return_code != 0:
            raise RuntimeError(
                f"failed to stage the experiment inputs: {(write.stderr or '').strip()}"
            )
        with tempfile.TemporaryDirectory(prefix="cs-bowling-driver-") as staging:
            driver = Path(staging) / "bowling_run_driver.py"
            driver.write_text(_RUN_DRIVER_PY, encoding="utf-8")
            await environment.upload_file(driver, str(_RUN_DRIVER_FILE))
        result = await self.exec_as_root(
            environment,
            command=f"python3 {_RUN_DRIVER_FILE}",
            timeout_sec=EXPERIMENT_WAIT_SEC + EXPERIMENT_EXEC_TAIL_SEC,
        )
        if result.return_code != 0:
            raise RuntimeError(
                "the candidate workflow session did not complete: "
                f"{(result.stdout or '').strip()[-600:]} "
                f"{(result.stderr or '').strip()[-600:]}"
            )
        outcome = _first_json_object(result.stdout or "")
        session_id = (outcome or {}).get("session_id", "")
        if not session_id:
            raise RuntimeError(
                f"the candidate driver returned no session id: "
                f"{(result.stdout or '').strip()[:400]!r}"
            )
        context.metadata = {
            **(context.metadata or {}),
            "bowling_candidate_session_id": session_id,
            "bowling_candidate_status": (outcome or {}).get("status", ""),
        }


__all__ = ["PolyglotBowlingChatSpeedAgent"]
