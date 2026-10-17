# `cscli` — ChatSpeed CLI developer reference

`cscli` is the ChatSpeed command-line client. It is a thin HTTP/SSE adapter for
the local loopback control plane published by the running `chatspeed-runtime`
process, which the desktop app supervises and drives as one more control-plane
client: it never opens the database, never starts a workflow runtime or executor,
and never runs a shell command itself. Every read and mutation is performed by
the runtime, so the desktop, the scheduler and the CLI cannot diverge about
state.

This document is the developer-facing reference for the command surface, its
conventions, the `automation` family in particular, and the runtime lifecycle the
CLI and the desktop share. The canonical automation contract lives in the backend
at
`src-tauri/crates/backend/src/workflow/automation/types.rs` (`AutomationSpec`,
`AutomationDraftInput`, `AutomationPlanV1`, `AutomationApplyRequest`, the schedule
configs and the mutation/dispatch outcomes), and the lifecycle wire types live in
`src-tauri/crates/contracts/src/runtime_lifecycle.rs`; this file describes how the
CLI projects those contracts and must not widen them.

## Invocation and discovery

- Inside ChatSpeed, invoke `cscli` directly through the `bash` tool as a
  standalone command, for example `cscli skill targets`. ChatSpeed resolves it to
  the bundled executable and runs it on the host.
- The bundled CLI is **not** added to PATH. Do not use `which cscli`,
  `command -v cscli`, or similar lookups: they do not test ChatSpeed's built-in
  resolution. Outside ChatSpeed, use the bundled executable's explicit path or
  configure PATH yourself.
- Prefer standalone commands without shell operators. Pipelines, redirects,
  environment prefixes and chained commands are not supported by built-in
  resolution.
- The CLI discovers the control plane from
  `${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json`, or from the
  path given by `--discovery-file`. It completes a readiness handshake (instance
  id, pid, protocol and schema version) before running a command, then registers
  and heartbeats a short-lived lease.

Exit codes (stable):

| Code | Meaning |
| --- | --- |
| `0` | success |
| `1` | server-side or I/O error |
| `2` | CLI usage error |
| `3` | ChatSpeed not running / discovery missing or stale / connection failure |
| `4` | authentication failure |
| `5` | incompatible control-plane protocol version |

## Global options

| Option | Meaning |
| --- | --- |
| `--output <human\|json\|jsonl>` | Output format (default `human`). `human` is localized and may change; `json`/`jsonl` are versioned machine contracts. |
| `--lang <en\|zh-Hans\|zh-Hant>` | Locale for human output. |
| `--discovery-file <PATH>` | Explicit control-plane discovery document. |

Command families: `help`, `doctor`, `agent`, `workflow`, `skill`, `mcp`,
`automation`. `cscli help` prints the bundled Help Skill offline and needs no
running runtime; every other command needs the running runtime's control plane.

## Automation commands

`cscli automation` manages the local workflow Automations the runtime owns. Every
mutation goes through the runtime's single automation facade over the
authenticated control plane, carries an idempotency key (minted when
`--idempotency-key` is omitted), and is a compare-and-set where it changes an
existing automation.

| Command | Purpose |
| --- | --- |
| `list` | List automations with schedule, enabled state and revision. |
| `get <automation-id>` | Show one automation. |
| `runs <automation-id>` | List an automation's projected run lifecycle. |
| `draft [inputs]` | Produce a side-effect-free plan. Never mutates, never grants a new permission. |
| `apply --plan-json/--plan-file <plan>` | Apply a reviewed plan after verifying its hash, base revision and expiry. |
| `create [inputs]` | Create an automation; the id is backend-minted. |
| `update <automation-id> [inputs]` | Compare-and-set update on the caller's revision. |
| `enable <automation-id>` / `disable <automation-id>` | Flip the desired enabled state. |
| `run <automation-id>` | Trigger a manual run now. |
| `delete <automation-id> --confirm` | Permanently delete an automation and its workflow tree. |

### Inputs: friendly flags or a raw spec

`create`, `update` and `draft` accept either the **friendly field flags** or a
**raw `AutomationSpec` document**; the two are mutually exclusive, and `draft`
additionally accepts `--intent`. The friendly flags are translated into the
canonical `snake_case` `AutomationSpec`; the source document is validated by the
same backend that validates raw documents, so the CLI never accepts something the
service would reject.

A `create` needs `--title`, `--agent`, `--prompt`/`--prompt-file` and exactly one
schedule selector:

| Schedule | Flag | Meaning |
| --- | --- | --- |
| interval | `--every <5m\|1h>` | Run every interval (minimum 5 minutes). A bare number is refused. |
| daily | `--daily <HH:MM>` | Run at a local time; repeat the flag for several times a day. |
| once | `--once <RFC3339\|YYYY-MM-DD HH:MM[:SS]>` | Run once. |

Shared schedule parameters:

- `--weekdays 1,2,3,4,5` — allowed weekdays, `1` = Monday … `7` = Sunday.
- `--start-date YYYY-MM-DD` / `--end-date YYYY-MM-DD` — the effective window.
- `--anchor-time HH:MM[:SS]` — interval anchor; valid for `--every` only.

A one-shot schedule cannot express the weekday/date window or continuous context;
requesting either is rejected rather than silently dropped.

Tri-state field pairs default when neither half is given:

| Pair | Default on create |
| --- | --- |
| `--enabled` / `--disabled` | disabled (`enabled = false`) |
| `--continuous-context` / `--no-continuous-context` | no continuous context (`false`) |
| `--self-review` / `--no-self-review` | no self review (`false`) |

`--allowed-path` (repeatable) replaces the stored allowed paths, which are empty
by default; `--clear-allowed-paths` empties them on an existing automation and
cannot be combined with `--allowed-path`.

The raw path is `--spec-json <document>` or `--spec-file <path>` (`-` reads
stdin). On `update`, a raw document is posted **unchanged** as a full replacement
of the stored spec.

### Revision, compare-and-set and idempotency

- `update` is a compare-and-set on the revision the caller read. With friendly
  flags the CLI reads the current view first (a `GET`), uses its revision and
  overlays only the fields that were given, preserving every unspecified field.
  `--expected-revision` is optional there and must match if supplied.
- With a raw `--spec-json`/`--spec-file` document, `--expected-revision` is
  required, and the CLI does **not** read the view: the document is posted as-is
  with the pinned revision.
- A conflict is surfaced as-is: the revision is never re-read and the mutation is
  never retried behind the caller's back.
- Every mutation (`create`, `update`, `enable`, `disable`, `run`, `delete`,
  `apply`) carries an idempotency key. One is minted when `--idempotency-key` is
  omitted, so a retry can never double a schedule change, a dispatch, or a
  destructive delete. `draft` is a read and carries no key.

### `draft` and `apply`

`draft` is a side-effect-free `POST` that returns an `AutomationPlanV1`. It never
writes the database, never runs shell, and never starts a workflow. A natural
language `--intent` may only express ordinary content and references to
*existing* agents/paths/shell policy; it can never mint a new shell, path,
network, MCP or Skill permission. A plan whose intent requested such a permission
comes back `blocked` with warnings and cannot be applied.

`apply` re-submits a reviewed plan. It re-derives the plan hash, re-checks the
base revision and the plan expiry, and rejects a mismatch rather than overriding
it. `--expected-plan-hash` defaults to the plan's own hash. When the plan really
expands permissions (unlike the existing automation), `apply` requires
`--acknowledge-permission-changes`; the backend is the single authority on that
gate.

### `run` and `delete`

- `run` triggers a manual run now. An **accepted** start is not a completion: the
  CLI echoes the backend outcome (`accepted`, or `busy`/`skipped` when a run is
  active or the slot is stale) and never invents a terminal state. Observe
  `cscli automation runs <automation-id>` for the durable status.
- `delete` is destructive, requires `--confirm`, and refuses while a run is
  active. The confirmation is carried in the request body so the server, not the
  CLI, is the single authority on the destructive gate.

### Documented examples

```sh
cscli automation list
cscli automation get <automation-id>
cscli automation runs <automation-id>

# Friendly create, interval schedule.
cscli automation create --title "Nightly" --agent agent-1 \
  --prompt "Summarize new issues" --every 1h --allowed-path /home/me/project

# Friendly create, daily schedule with a weekday window, enabled.
cscli automation create --title "Morning" --agent agent-1 \
  --prompt-file ./prompt.txt --daily 09:00 --daily 18:00 \
  --weekdays 1,2,3,4,5 --enabled

# Raw spec create.
cscli automation create --spec-json '{"title":"Raw","prompt":"p","prompt_file_path":null,"agent_id":"agent-1","agent_config":null,"allowed_paths":[],"shell_config":null,"schedule_kind":"daily","schedule_config":{"times":["09:00"]},"continuous_context":false,"self_review":false,"enabled":true}'

# Draft a plan, then apply the reviewed plan.
cscli automation draft --title "Nightly" --agent agent-1 --prompt "hi" --every 1h
cscli automation draft --intent "run every day at 09:00"
cscli automation draft --automation-id <automation-id> --title "Renamed"
cscli automation apply --plan-file plan.json --acknowledge-permission-changes

# Friendly update (revision read first) and raw update (revision required).
cscli automation update <automation-id> --title "Renamed"
cscli automation update <automation-id> --spec-file spec.json --expected-revision 7

cscli automation enable <automation-id>
cscli automation disable <automation-id>
cscli automation run <automation-id>
cscli automation delete <automation-id> --confirm
```

## Background execution, keep-alive and shutdown

Automations do not need the desktop UI to be open. The standalone
`chatspeed-runtime` process owns the scheduler and the control plane, and it
keeps itself alive while any of these holds:

- an enabled automation has an effective schedule — a legal future slot, or a due
  slot this instance was continuously online for;
- a run the workflow manager owns is live;
- a run has been admitted but not yet linked to a workflow session;
- the runtime cannot observe automation state at all — an unreadable summary is
  treated as "must stay alive", never as "nothing to do".

The idle supervisor re-reads the automation summary on a bounded cadence and once
more when its grace expires, so a newly enabled schedule or a last-moment run
keeps the runtime alive even without a client lease, and the scheduler keeps
ticking across those idle checks. A manual `cscli automation run` and the app's
own control-plane calls still need the running runtime process; only the
*scheduling* of already-configured automations survives with no client connected.

### No catch-up after a stop

A slot that became due while the runtime was stopped is never replayed. At
startup the runtime records a single start boundary and normalizes each enabled
automation exactly once: a stored slot at or before that boundary (missed while
offline) is advanced to its next future occurrence, and a non-recurring slot that
has already elapsed is cleared. A stored future slot is left untouched, a stored
`NULL` slot stays `NULL`, and every write is a revision compare-and-set, so a
concurrent edit is reported rather than overwritten. A due slot at or after the
start boundary is one this instance stayed online for; it is consumed at most
once by the scheduler while the runtime runs.

### One coordinated stop

Stopping is a single coordinated shutdown shared by the idle supervisor, the
process signal and an explicit request. The admission gate closes first and every
owner funnels into the same forward-only phase machine
`running -> quiescing -> cancelling -> draining -> stopped`. Nothing on this path
deletes the configuration or the run history.

## Runtime lifecycle control-plane routes

Besides the command routes `cscli` uses, the runtime serves a small typed
lifecycle surface on the same `/control/v1` control plane. `cscli` deliberately
exposes no runtime command; these routes are what the desktop and other clients
call directly.

- `GET /control/v1/runtime/status` — read-only authoritative status: the
  lifecycle phase, the admission gate's epoch and active count, the live lease
  count and the automation keep-alive summary.
- `POST /control/v1/runtime/shutdown` — request a coordinated shutdown.

The shutdown route is hard to call by accident. One request must carry all of:

| Requirement | Where |
| --- | --- |
| bearer token | `Authorization` header (shared middleware) |
| a live lease proof | the `X-Runtime-Client` and `X-Runtime-Lease` headers |
| the expected instance | the `expected_instance_id` body field |
| explicit confirmation | the `confirm` body field must be `true` |
| an explicit force flag | the `force` body field (recorded on the operation) |
| an idempotency key | a mandatory `Idempotency-Key` header |

The proof is validated before the idempotency cache, so a replayed key is still
backed by a currently valid instance, confirmation and lease; a released or
expired lease can never replay an old acceptance. Refusals use stable codes:
`instance_mismatch` (`409`), `not_confirmed` (`400`), `lease_invalid` (`403`),
`already_stopping` (`409`) and `missing_idempotency_key` (`400`).

An accepted request answers `202 Accepted` with the operation it started, and
**an accepted start is not a completion**: a client must re-read the status route
until `phase` reaches `stopped`. The runtime spends one graceful budget (15s
today) on the cancelling, drain and server stop; only an explicitly forced
request gets an extra forced budget (5s today) before the last-resort termination
fallback, which only the real runtime binary installs, so a host that embeds the
runtime library is never killed. The operation reports its terminal outcome —
`graceful`, `forced` or `failed` — so a forced or failed stop is never confused
with a graceful one.

## Desktop exit

The desktop's tray `quit` entry is the only route that opens a quit
confirmation. The dialog's "stop the shared runtime" checkbox starts unchecked:
leaving it unchecked releases the desktop's lease (bounded) and exits only the
desktop, leaving the shared runtime running. Checking it asks the shutdown route
to stop the instance the user confirmed: the target instance is recorded once and
never overwritten, so a runtime that restarted in between is never mistaken for a
successor; the exit is committed only once the runtime's own status reports
`stopped`, and a network failure is never treated as a successful stop. On
failure the user can retry or choose the desktop-only exit. A normal window close
is unchanged: it still only hides or closes that window and never starts a quit.

## Verification status

These capabilities were verified on Linux with focused automated tests. The
Windows Job Object code that owns a child-process tree is compiled only for
`target_os = "windows"` and target-checked, but no real Windows runner exercised
it, so it is not claimed as verified on Windows. The tools and MCP paths and the
owned process tree are Linux-verified. The GUI, the real system tray and the
runtime end-to-end exit flow are not headless-verified.