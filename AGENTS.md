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

- Workflow runtime: when changing `src-tauri/src/workflow/react`, read and follow `src-tauri/src/workflow/react/CONSTITUTION.md`.
- CCProxy: when changing `src-tauri/src/ccproxy`, read and follow `src-tauri/src/ccproxy/CONSTITUTION.md`.
- CCProxy routing is order-sensitive. After routing, handler, adapter, or protocol changes, run `python3 test/ccproxy/api_all_test.py` when feasible.
- Proxy responses must use header filtering. Do not forward transport headers like `Content-Length`, `Transfer-Encoding`, `Connection`, or `Content-Encoding` directly.
