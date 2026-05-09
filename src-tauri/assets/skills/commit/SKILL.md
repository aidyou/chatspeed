---
name: commit
description: Create well-formatted Git commits with conventional commit messages. Use this skill when the user wants to create a commit.
---

# Commit
Create well-formatted Git commits using real Git commands and Conventional Commits.

## Core Rule
Use actual Git commands such as `git status`, `git diff`, `git add`, and `git commit`.
Do not call `/commit` or any nonexistent custom commit command.

## Options
The user may request behavior such as:
- skip verification
- simple or detailed commit message
- a specific commit type, such as `feat`, `fix`, or `docs`
Treat these as instructions for how to perform the Git workflow, not as command-line options to a custom command.

## Workflow
1. Inspect Git status and staged files.
2. Inspect recent commit history to match the repository's commit message style and language.
3. Analyze staged and unstaged changes with `git diff` and `git diff --staged`.
4. Detect whether changes should be split into atomic commits.
5. Run relevant project checks unless the user asks to skip verification.
6. Stage only the files that belong in the commit.
7. Generate a Conventional Commit message.
8. Use `git commit` to create the commit.
9. Summarize what was committed and what was verified.

## Pre-commit Checks
Unless the user asks to skip verification, run the project's relevant checks before committing.
Prefer existing package scripts or project conventions, such as lint, test, build, typecheck, or docs generation.
Only run checks that exist in the project. Do not invent scripts.

## Staging Rules
- If files are already staged, commit only the staged changes unless the user asks otherwise.
- If no files are staged, stage the relevant modified and new files for the requested change.
- Do not stage unrelated files.
- Do not overwrite, discard, reset, clean, or remove user changes.
- Ask before staging changes whose purpose is unclear or unrelated.

## Commit Message Language
Match the repository's existing commit message language.
Rules:
- Inspect recent commit messages before generating a new commit message.
- Use the dominant language from recent user/project commits.
  - Mostly Chinese history → write the description/body in Chinese.
  - Mostly English history → write the description/body in English.
- Keep Conventional Commit types in English, such as `feat`, `fix`, `docs`, `refactor`.
- If the repository has no commit history:
  - prefer English for open-source or public projects
  - for private projects, use the language that best matches the README, docs, issue text, or user request
- If history is mixed with no clear dominant language, use the language of the user's current request.
## Conventional Commit Format
Simple style:
```text
<emoji> <type>[optional scope]: <description>
```
Example:
```text
✨ feat(auth): add JWT token validation
```
Full style:
```text
<emoji> <type>[optional scope]: <description>
<body>
<footer>
```
Use full style for breaking changes, complex features, multi-system changes, or bug fixes that need explanation.

## Commit Types
| Type       | Emoji | Description      | When to Use                          |
| ---------- | ----- | ---------------- | ------------------------------------ |
| `feat`     | ✨     | New feature      | Adding new functionality             |
| `fix`      | 🐛     | Bug fix          | Fixing an issue                      |
| `docs`     | 📝     | Documentation    | Documentation only changes           |
| `style`    | 🎨     | Code style       | Formatting, missing semi-colons, etc |
| `refactor` | ♻️     | Code refactoring | Neither fixes bug nor adds feature   |
| `perf`     | ⚡️     | Performance      | Performance improvements             |
| `test`     | ✅     | Testing          | Adding missing tests                 |
| `chore`    | 🔧     | Maintenance      | Changes to build process or tools    |
| `ci`       | 👷     | CI/CD            | Changes to CI configuration          |
| `build`    | 📦     | Build system     | Changes affecting build system       |
| `revert`   | ⏪     | Revert           | Reverting previous commit            |

## Message Guidelines
- Use imperative mood: `add`, not `added`.
- Keep the subject concise, preferably under 50 characters and no more than 72.
- Do not end the subject with a period.
- Use a clear scope when helpful, such as `auth`, `api`, `ui`, `db`, `config`, or `deps`.
- In full style, explain what changed and why, not low-level implementation details.
- Wrap body lines around 72 characters.
- Use footers for breaking changes and issue references.
Footer examples:
```text
BREAKING CHANGE: rename config.auth to config.authentication
Closes: #123
Fixes: #456
Refs: #789
```

## Commit Splitting
Suggest splitting commits when changes include:
- mixed types, such as feature work plus unrelated fixes
- unrelated concerns
- dependency updates mixed with implementation changes
- broad changes across unrelated modules
- generated files or formatting mixed with logic changes
Do not split automatically unless the user approves or the workflow clearly supports multiple commits.

## Safety
- Do not call `/commit` or any nonexistent custom commit command.
- Use real Git commands only.
- Do not commit broken code unless the user explicitly wants a WIP commit.
- Do not include secrets, credentials, tokens, private keys, or sensitive data.
- Do not create branches, stashes, resets, checkouts, cleanups, or pushes without explicit approval.
- Only create a commit when the user explicitly asked for one.