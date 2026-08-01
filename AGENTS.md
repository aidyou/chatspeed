# ChatSpeed Root Rules

This file is for global rules only. Keep it short. When a subdirectory has its own `AGENTS.md` or `CONSTITUTION.md`, follow the more specific file there.

## Personal Preferences

- Prefer small, targeted changes. Do not do unrelated cleanup or refactors.
- Reuse existing patterns before adding abstractions.
- Do not stage, commit, branch, or rewrite git history unless explicitly asked.
- Git commit messages must be in English.
- Unless explicitly requested otherwise, reply in the same language as the user's message.
- Final reports should be concise and include what changed, what was verified, and any remaining risk.

## General Workflow

- Read relevant files before editing.
- Ask before broad refactors, dependency additions, schema migrations, destructive actions, or behavior changes outside the request.
- For small localized fixes, proceed directly.
- Prefer the narrowest useful verification. If you cannot run verification, say so.
- When editing or adding files or modules, test only the affected page, module, or focused behavior. Do not run whole-repository tests unless the user explicitly asks, because they are prone to timing out.

## CodeGraph Usage

- When CodeGraph MCP is available and its index is healthy, use it as the default tool for repository navigation, symbol lookup, source reading, and static impact analysis.
- Do not start with `grep`, or filesystem globbing to locate a known symbol, inspect a known source file, or assess static callers/callees.
- Use the narrowest CodeGraph query: `codegraph_search` then a file-pinned `codegraph_node` for a known symbol; `codegraph_node(file, symbolsOnly: true)` for a known file; and scoped `codegraph_files` for a known directory or file type.
- Before changing a resolved symbol, inspect it with `codegraph_node` and use bounded, file-pinned `codegraph_callers` and `codegraph_callees` when assessing its static blast radius.
- For string-based dispatch, Tauri command names, configuration or i18n keys, events, macros, generated wiring, dynamic property access, runtime data flow, external APIs, and cross-language boundaries, prefer a native permission-aware text-search tool when one is available. Otherwise use `grep`. Use it after CodeGraph for textual verification when needed.
- Treat CodeGraph edges as incomplete static evidence. Verify behavior-changing conclusions against current source and focused tests; empty caller/callee results do not prove a symbol is unused.
- If `.codegraph` is missing or CodeGraph reports an uninitialized index, run `codegraph init`. If CodeGraph is unavailable, continue with the native text-search tool when available, or `grep` and standard repository navigation.

## Project Constraints

- Rust/Tauri: use `Result` and `?`; avoid `unwrap()` and `expect()` in normal production code.
- Frontend: use Vue 3 Composition API, `<script setup>`, Element Plus, Pinia, and SCSS.
- Keep the final code clean. Temporary checks during development are fine, but the final result must not leave warnings such as unused variables, unused imports, dead code, or similar avoidable issues.
- All created or modified code files must use LF line endings.
- All code comments, Rust docs, and developer-facing code documentation must be in English unless a file explicitly requires another language.
- CSS: prefer existing variables from `src/style/element/css-vars.css`; use semantic `--cs-*` tokens instead of hard-coded values when possible.
- User-facing strings must use the i18n system. Do not hardcode user-visible text in Rust or Vue source code.
- Keep locale keys sorted and keep locale structures consistent across languages.
- Database changes must be explicit and cautious. Avoid destructive schema or data changes without approval.

## Critical Module Rules

- Workflow runtime and frontend: when changing `src-tauri/src/workflow/react`, `src/views/Workflow.vue`, `src/components/workflow`, `src/composables/workflow`, or workflow modules in `src/stores`, read and follow `src-tauri/src/workflow/react/CONSTITUTION.md`.
- CCProxy: when changing `src-tauri/src/ccproxy`, read and follow `src-tauri/src/ccproxy/CONSTITUTION.md`.
- CCProxy routing is order-sensitive.
- Proxy responses must use header filtering. Do not forward transport headers like `Content-Length`, `Transfer-Encoding`, `Connection`, or `Content-Encoding` directly.
