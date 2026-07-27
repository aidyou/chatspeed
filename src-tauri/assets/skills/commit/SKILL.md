---
name: commit
description: Create well-formatted Git commits with conventional commit messages. Use this skill when the user wants to create a commit.
---

# Commit
Create a local Git commit with real Git commands and a Conventional Commit message.

## Workflow
1. Inspect Git status, staged files, recent commits, and the relevant diff.
2. Keep unrelated changes out of the commit. Ask before including unclear changes or splitting commits.
3. Check candidate paths and diffs for sensitive data.
4. Run checks only when the user explicitly requests them.
5. Stage the intended files and create the commit.
6. Report the commit and any checks that actually ran.

## Optional Verification
Do not run lint, tests, builds, typechecks, or documentation generation just
because the user asked for a commit. A local commit is not a push or CI gate.

Run a check only when the user explicitly requests testing or verification. Use
the narrowest relevant existing project command.

## Staging Rules
If files are already staged, commit only those files unless instructed otherwise.
Otherwise, stage only files belonging to the requested change. Never discard,
reset, clean, or overwrite existing changes.

## Commit Message
Match the recent repository style and language. Use:

```text
<type>[optional scope]: <concise description>
```

Keep the subject under 72 characters and state the key change. Add a short body
only when it is needed to explain an important behavior change, compatibility
impact, or breaking change. Do not add exhaustive implementation detail.

## Sensitive Data Review
Before staging, inspect candidate files and diffs for secrets, credentials,
private keys, tokens, connection strings, local databases, or data exports.
If a possible secret is found, stop and ask the user whether to exclude or
explicitly include each affected file.

## Safety
- Use real Git commands; do not call a nonexistent custom commit command.
- Do not create branches, stashes, resets, checkouts, cleanups, or pushes without explicit approval.
- Create a commit only when the user explicitly asks for one.
