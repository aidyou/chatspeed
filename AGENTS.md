# ChatSpeed Root Rules

This file is for global rules only. Keep it short. When a subdirectory has its own `AGENTS.md` or `CONSTITUTION.md`, follow the more specific file there.

## Personal Preferences

- Prefer small, targeted changes. Do not do unrelated cleanup or refactors.
- Reuse existing patterns before adding abstractions.
- Do not stage, commit, branch, or rewrite git history unless explicitly asked.
- Git commit messages must be in English.
- Unless the user explicitly requests it, do not proactively commit the `chatspeed-docs` or `website` submodules.
- Unless explicitly requested otherwise, reply in the same language as the user's message.
- Final reports should be concise and include what changed, what was verified, and any remaining risk.

## Project Shape

- ChatSpeed is a Tauri 2 desktop application: the Rust backend and Tauri commands live under `src-tauri/`, while the Vue 3 + Vite frontend lives under `src/`.
- Frontend conventions are Vue 3 Composition API with `<script setup>`, Element Plus, Pinia, Vue Router, vue-i18n, and SCSS. Reuse these choices instead of introducing parallel UI or state-management stacks.
- Use pnpm for frontend dependencies and scripts; `package.json` and `pnpm-lock.yaml` are authoritative. Use Cargo from `src-tauri/` for Rust dependencies and commands. Do not use npm or yarn, and do not create another lockfile.
- Keep cross-boundary changes explicit: trace Vue/TypeScript or JavaScript calls through the Tauri command/event contract to Rust, and verify both sides when that contract changes.

## General Workflow

- Read relevant files before editing.
- Ask before broad refactors, dependency additions, schema migrations, destructive actions, or behavior changes outside the request.
- For small localized fixes, proceed directly.
- Prefer the narrowest useful verification. If you cannot run verification, say so.
- When editing or adding files or modules, test only the affected page, module, or focused behavior. Do not run whole-repository tests unless the user explicitly asks, because they are prone to timing out.

## Work Start Checklist

- Start from the user's strongest anchor: exact file, symbol, error, route, config key, test, or unique text. Inspect the target source and trace one concrete execution path before editing; do not begin with broad repository scans when a strong anchor exists.
- Before the first edit in a module, check that directory and its parents for `AGENTS.md`, `CONSTITUTION.md`, or equivalent guidance. Read each applicable local rule once per task segment, and re-check only when moving into another module or when the rule file changes.
- Before significant edits, inspect worktree status once, scoped to the intended files or module when practical. Significant edits include multiple files, refactors, configuration or schema changes, generated files, broad formatting, or any target that may already contain user changes. If relevant files are modified, inspect their diff before editing and preserve unrelated work.
- For multi-step, cross-layer, risky, or interruption-prone work, create a small todo list before implementation and keep one item in progress. Skip todo overhead for a single direct edit or check that can be completed and verified immediately.
- Define the focused verification path before editing. Prefer affected tests, then typecheck/lint/build or a focused runtime check; do not defer deciding how to verify until after implementation.
- Treat follow-up requests as continuations: reuse confirmed context and current task state, inspect only the changed assumption or newly affected boundary, and do not repeat startup checks unless their evidence may be stale.

## Project Constraints

- Rust/Tauri: use `Result` and `?`; avoid `unwrap()` and `expect()` in normal production code.
- Frontend: use Vue 3 Composition API, `<script setup>`, Element Plus, Pinia, and SCSS.
- Keep the final code clean. Temporary checks during development are fine, but the final result must not leave warnings such as unused variables, unused imports, dead code, or similar avoidable issues.
- Do not use `#[allow(dead_code)]` in final code. Mark code used exclusively by tests with `#[cfg(test)]`; delete genuinely unused code. Temporary intermediate code during active development is allowed, but it must be removed or resolved before completion.
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
