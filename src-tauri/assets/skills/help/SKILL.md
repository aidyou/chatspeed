---
name: help
description: Explains ChatSpeed and manages its local capabilities. Activate when a user asks how ChatSpeed works, needs official documentation, wants to install, check, list, or uninstall an Agent Skill, wants to install, inspect, enable, disable, refresh, restart, or remove an MCP server, wants to create, inspect, draft, apply, schedule, enable, disable, run, or delete a local workflow Automation, or asks about the ChatSpeed CLI.
---

# ChatSpeed Help and Capability Skill

This is the only bundled capability Skill besides `commit`. It combines official product documentation with the operational guidance for Agent Skills, MCP servers, and the `cscli` CLI.

## AI Assistant Instructions

When a user asks a question about ChatSpeed:

1. **Check the Index**: Scan the "Documentation Index" and "Capability Operations" sections below.
2. **Fetch Details**: For product documentation or troubleshooting, use the `web_fetch` tool to retrieve the actual content from the corresponding URL (Base URL: `https://docs.chatspeed.aidyou.ai/`).
3. **Use capability operations**: For local Skills or MCP management, use the existing capability operations and report their structured result. Do not guess an external tool's directory or write to an unverified path.
4. **Synthesize**: Provide a concise answer based on the official documentation or operation result. Include the source link when documentation was fetched.
5. **Fallback**: If the question is not covered or fetching fails, direct the user to `https://docs.chatspeed.aidyou.ai/`.

## Built-in CLI Invocation

`cscli` is short for ChatSpeed CLI. Inside ChatSpeed, invoke it directly through the `bash` tool as a standalone command, for example `cscli skill targets`. ChatSpeed automatically resolves this command to the bundled executable and runs it on the host; it does not rely on shell PATH lookup.

The bundled CLI is not automatically added to PATH. Do not use `which cscli`, `command -v cscli`, or similar shell lookup commands to locate or validate it: they do not test ChatSpeed's built-in resolution. Do not substitute a PATH executable or fall back to the old `cs` name. Outside ChatSpeed, use the bundled executable's explicit path unless you have separately configured PATH.

Prefer standalone commands without shell operators. A trailing `2>&1` is accepted for compatibility and ignored because the tool already captures stdout and stderr. Pipelines, other redirects, environment prefixes, and chained commands are not supported by built-in resolution. If a direct invocation fails, report the actual error instead of searching for another executable. Automatic resolution does not bypass approval for mutating operations.

## Capability Operations

The capability control plane is the single authority for local Skill and MCP state. Use these commands with the `cscli` executable:

- `cscli skill targets` — list known Skill targets and whether their paths are verified.
- `cscli skill list` — list installed Skills, ownership, and drift.
- `cscli skill check --source-json '<document>'` — check a Skill without installing it.
- `cscli skill install --source-json '<document>' [--target <id>]` — install a checked Skill.
- `cscli skill uninstall <name> [--target <id>]` — uninstall only a managed, owned Skill.
- `cscli mcp list` — list MCP servers and desired/runtime state.
- `cscli mcp status <name>` — inspect one MCP server.
- `cscli mcp install --descriptor-json '<document>' [--enable]` — register an MCP server; enabling is a separate effect.
- `cscli mcp enable|disable|restart|refresh|uninstall <name>` — manage a registered MCP server.
- `cscli mcp tools <name>` — list published tools without invoking one.
- `cscli doctor capabilities` — report capability journal, ownership, runtime, and staging drift.
- `cscli doctor reconcile` — converge only effects proven safe to recover.

Mutating operations must preserve the capability service's fail-closed behavior and idempotency handling. The CLI is an HTTP adapter; it does not open the database, capability directories, or MCP processes itself.

## Automation Operations

`cscli automation` manages the local workflow Automations the running ChatSpeed runtime owns. Every mutation goes through the runtime's single automation facade over the authenticated control plane; the CLI never opens the database, starts a workflow, or runs a shell command itself. The backend `AutomationSpec` and its schedules remain the only authority, so the CLI cannot grow a laxer notion of a valid automation.

Friendly field flags are the validated alternative to a raw `--spec-json`/`--spec-file` `AutomationSpec` document, and the two are mutually exclusive. A create needs `--title`, `--agent`, `--prompt`/`--prompt-file` and exactly one schedule selector:

- `--every <5m|1h>` — an interval schedule (minimum 5 minutes).
- `--daily <HH:MM>` — a daily schedule; repeat the flag for several times a day.
- `--once <RFC3339|YYYY-MM-DD HH:MM[:SS]>` — a one-shot schedule.

`--weekdays 1,2,3,4,5` (1 = Monday), `--start-date`/`--end-date` (`YYYY-MM-DD`) and `--anchor-time` (`HH:MM[:SS]`, interval only) shape the schedule; a one-shot schedule cannot express the weekday/date window. The tri-state pairs `--enabled`/`--disabled`, `--continuous-context`/`--no-continuous-context` and `--self-review`/`--no-self-review` default to disabled, no continuous context and no self review when neither half is given, and `--allowed-path` (repeatable) replaces the stored allowed paths, which are empty by default.

- `cscli automation list` — list automations with schedule, enabled state and revision.
- `cscli automation get <id>` — show one automation.
- `cscli automation runs <id>` — list an automation's projected run lifecycle.
- `cscli automation draft ...` — produce a side-effect-free plan from a raw spec, a constrained `--intent` or friendly flags.
- `cscli automation apply ...` — apply a reviewed plan after verifying its hash, base revision and expiry.
- `cscli automation create ...` — create an automation from a raw spec or friendly flags; the id is backend-minted.
- `cscli automation update <id> ...` — a compare-and-set update on the caller's revision.
- `cscli automation enable <id>` / `cscli automation disable <id>` — flip the desired enabled state.
- `cscli automation run <id>` — trigger a manual run now.
- `cscli automation delete <id> --confirm` — permanently delete an automation and its associated workflow tree.

State and safety rules:

- `update` reads the current revision first and compare-and-sets on it. With `--spec-json`/`--spec-file` the revision stays required via `--expected-revision`, and the raw document is posted unchanged as a full replacement. A conflict is surfaced as-is: the revision is never re-read and the mutation is never retried behind the caller.
- Every mutation carries an idempotency key, minted when `--idempotency-key` is omitted, so a retry cannot double an effect.
- `draft` never mutates and never grants a new shell/path/network/MCP/Skill permission; a plan whose intent requested such a permission is returned `blocked` and cannot be applied. `apply` re-derives the hash, re-checks the base revision and the plan expiry, and requires `--acknowledge-permission-changes` only when the plan really expands permissions.
- `run` reports an accepted start, which is not a completion; observe `cscli automation runs` for the durable terminal state.
- `delete` is destructive and requires `--confirm`; it also refuses while a run is active.

Background execution is implemented and available for configured automations. The runtime keeps itself alive for an enabled automation that has an effective or continuously-online schedule, for a live run, and for a run admitted but not yet linked to a workflow session, even with no client connected, and it keeps scheduling across its own ticks. A manual `cscli automation run` and the app's own control-plane calls still need the running runtime process; only the scheduling of already-configured automations survives with no client connected. A slot missed while the runtime was stopped is never replayed: at startup a stored slot at or before the instance start boundary is advanced to its next future occurrence (or cleared when it was non-recurring and already elapsed), and a due slot this instance stayed online for is consumed at most once while the runtime runs. Stopping is a single coordinated shutdown shared by the idle supervisor, the process signal and an explicit request, and it never deletes the configuration or the run history. See `docs/cscli.md` in the repository for the full developer reference.

## Skill Installation and Vetting

Before installing a Skill from GitHub or another external source:

1. Identify the source, author, revision, and intended contents.
2. Read every file and check for credential access, unexpected network calls, obfuscation, process execution, package installation, system-file changes, and elevated permissions.
3. Record the files, filesystem scope, network destinations, and commands required.
4. Classify the risk as LOW, MEDIUM, HIGH, or EXTREME.
5. Ask for human approval before HIGH or EXTREME risk installation.
6. After a successful install, run the non-LLM Skill checker and report any blocked findings. If vetting fails, recommend removal and do not treat the install as trusted.

The bundled installer helpers were removed from the built-in Skill set; use the `cscli skill` capability commands and this consolidated guidance instead.

## Creating or Updating a Skill

1. Define the trigger, user-visible behavior, inputs, outputs, and edge cases.
2. Write `SKILL.md` with YAML frontmatter containing `name` and `description`.
3. Keep the Skill directory self-contained; use `scripts/`, `references/`, and `assets/` only when needed.
4. Test with representative prompts and refine the description when triggering is too broad or too narrow.
5. Update this Help Skill when supported capability operations, safety rules, or documentation links change.

## Documentation Index

Below are the key topics. All links are relative to `https://docs.chatspeed.aidyou.ai/`.

### 🚀 Getting Started
- **Quick Start Guide**: `/guide/quickStart.html`
- **Installation**: `/guide/installation.html`
- **Features Overview**: `/guide/features/overview.html`

### 🛠️ Configuration & Core Engine (CCProxy)
- **Introduction to CCProxy**: `/ccproxy/`
- **Configuration Guide**: `/ccproxy/configuration.html`
- **Client Integrations**:
  - **Claude Code**: `/ccproxy/claude-code.html`
  - **Cline / Roo-Code**: `/ccproxy/cline.html` & `/ccproxy/roo-code.html`
  - **Zed Editor**: `/ccproxy/zed.html`
  - **Gemini**: `/ccproxy/gemini.html`
  - **Crush**: `/ccproxy/crush.html`

### 🔌 Model Context Protocol (MCP)
- **MCP Hub**: `/mcp/`

### 🧠 Prompt Engineering & Enhancement
- **Prompt Enhancement Overview**: `/prompt/`
- **Claude Code Enhancement**: `/prompt/claude-code-prompt-enhance.html`
- **Native Tool Call Support**: `/prompt/claude-code-prompt-enhance-native-tool-call.html`
- **Common Prompts**: `/prompt/common.html`

### 💻 Development & API
- **Developer Guide**: `/guide/development.html`
- **API Reference**: `/api/`

## Maintenance

This file is the manually maintained source of truth for the bundled Help Skill and the offline `cscli help` command. When CLI capability commands, safety requirements, or documentation links change, update this file in the same change and rebuild ChatSpeed.

## Additional Resources
- **GitHub**: [https://github.com/chatspeed-ai/chatspeed](https://github.com/chatspeed-ai/chatspeed)
- **Official Website**: [https://chatspeed.aidyou.ai/](https://chatspeed.aidyou.ai/)
