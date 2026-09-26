---
name: help
description: Explains ChatSpeed and manages its local capabilities. Activate when a user asks how ChatSpeed works, needs official documentation, wants to install, check, list, or uninstall an Agent Skill, wants to install, inspect, enable, disable, refresh, restart, or remove an MCP server, or asks about the ChatSpeed CLI.
---

# ChatSpeed Help and Capability Skill

This is the only bundled capability Skill besides `commit`. It combines official product documentation with the operational guidance for Agent Skills, MCP servers, and the `cs` CLI.

## AI Assistant Instructions

When a user asks a question about ChatSpeed:

1. **Check the Index**: Scan the "Documentation Index" and "Capability Operations" sections below.
2. **Fetch Details**: For product documentation or troubleshooting, use the `web_fetch` tool to retrieve the actual content from the corresponding URL (Base URL: `https://docs.chatspeed.aidyou.ai/`).
3. **Use capability operations**: For local Skills or MCP management, use the existing capability operations and report their structured result. Do not guess an external tool's directory or write to an unverified path.
4. **Synthesize**: Provide a concise answer based on the official documentation or operation result. Include the source link when documentation was fetched.
5. **Fallback**: If the question is not covered or fetching fails, direct the user to `https://docs.chatspeed.aidyou.ai/`.

## Capability Operations

The capability control plane is the single authority for local Skill and MCP state. Existing CLI command formats remain stable:

- `cs skill targets` — list known Skill targets and whether their paths are verified.
- `cs skill list` — list installed Skills, ownership, and drift.
- `cs skill check --source-json '<document>'` — check a Skill without installing it.
- `cs skill install --source-json '<document>' [--target <id>]` — install a checked Skill.
- `cs skill uninstall <name> [--target <id>]` — uninstall only a managed, owned Skill.
- `cs mcp list` — list MCP servers and desired/runtime state.
- `cs mcp status <name>` — inspect one MCP server.
- `cs mcp install --descriptor-json '<document>' [--enable]` — register an MCP server; enabling is a separate effect.
- `cs mcp enable|disable|restart|refresh|uninstall <name>` — manage a registered MCP server.
- `cs mcp tools <name>` — list published tools without invoking one.
- `cs doctor capabilities` — report capability journal, ownership, runtime, and staging drift.
- `cs doctor reconcile` — converge only effects proven safe to recover.

Mutating operations must preserve the capability service's fail-closed behavior and idempotency handling. The CLI is an HTTP adapter; it does not open the database, capability directories, or MCP processes itself.

## Skill Installation and Vetting

Before installing a Skill from GitHub or another external source:

1. Identify the source, author, revision, and intended contents.
2. Read every file and check for credential access, unexpected network calls, obfuscation, process execution, package installation, system-file changes, and elevated permissions.
3. Record the files, filesystem scope, network destinations, and commands required.
4. Classify the risk as LOW, MEDIUM, HIGH, or EXTREME.
5. Ask for human approval before HIGH or EXTREME risk installation.
6. After a successful install, run the non-LLM Skill checker and report any blocked findings. If vetting fails, recommend removal and do not treat the install as trusted.

The bundled installer helpers were removed from the built-in Skill set; use the `cs skill` capability commands and this consolidated guidance instead.

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

This file is the manually maintained source of truth for the bundled Help Skill and the offline `cs help` command. When CLI capability commands, safety requirements, or documentation links change, update this file in the same change and rebuild ChatSpeed.

## Additional Resources
- **GitHub**: [https://github.com/chatspeed-ai/chatspeed](https://github.com/chatspeed-ai/chatspeed)
- **Official Website**: [https://chatspeed.aidyou.ai/](https://chatspeed.aidyou.ai/)
