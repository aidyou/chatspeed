# Workflow React Development Guide

This guide is subordinate to [`../CONSTITUTION.md`](../CONSTITUTION.md).

If this guide and the constitution differ, the constitution wins.

This document defines the mandatory development rules for `src-tauri/src/workflow/react`.

Its purpose is to prevent architectural drift, avoid parallel logic branches, and keep workflow behavior aligned with `work/plan.md`.

## 1. Core Intent

The workflow runtime must remain a **single-state-machine system**:

- One lifecycle authority: `WorkflowManager`
- One wait-state main path: `run_loop_internal` waiting branch
- One structured signal contract: `WorkflowSignal` + typed compatibility mapping
- One state projection contract for UI: `state + wait_reason`

Do not add alternate execution paths that bypass these contracts.

## 2. Non-Negotiable Invariants

1. Waiting behavior is unified.
- `Paused`, `AwaitingUser`, `AwaitingApproval` are handled in one waiting loop.
- Resume happens only after signal validation against `wait_reason`.

2. Signals are structured first.
- Add new signal types through typed definitions and explicit handling.
- Legacy JSON compatibility is allowed, but must map immediately to structured meaning.

3. Runtime and persistence must stay coherent.
- Any user-visible or recoverable behavior change must update runtime handling and persistence path consistently.

4. No silent consumption of user signals.
- If a signal is consumed outside waiting branch, it must be intentionally queued/replayed with clear ownership.

5. Stop must always be effective.
- `Stop` must remain valid and actionable in any waiting state.

## 3. Where to Extend (And Where Not)

### Allowed extension points

- `types.rs`: add structured signal/state types
- `engine.rs`:
  - waiting branch for wait-state behavior
  - non-waiting signal drain for runtime-only updates
- `commands/workflow.rs`: command-layer compatibility mapping and routing
- `useWorkflowCore.ts`: frontend signal emission and wait-state UX

### Forbidden patterns

- Adding a second waiting branch in another file/module
- Introducing a new ad-hoc signal parser unrelated to structured signal handling
- Implementing wait-state recovery through transcript text inference
- UI logic relying on legacy status strings when `wait_reason` is available

## 4. Signal Design Rules

When introducing a signal:

1. Define canonical name in snake_case.
2. Define payload fields and accepted states.
3. Add backend handling in the unified waiting path (if wait-related).
4. Add non-waiting handling only if truly runtime-global.
5. Add frontend emission via a single mapping function.
6. Add logs with `session`, `state`, `wait_reason`, `signal_type`.

Compatibility aliases are acceptable, but canonical naming must be stable and documented.

## 5. Wait-State Rules

For wait-state changes:

1. Validate signal against current `wait_reason`.
2. Reject/ignore mismatched signals explicitly, with logs.
3. Keep state transitions explicit (`update_state`).
4. Avoid implicit resume side effects from unrelated signal drains.

## 6. UI Contract Rules

Frontend behavior must be based on:

- `state`
- `wait_reason`

Do not infer waiting intent from message text or tool output patterns.

If optimistic UI is needed (for responsiveness), it must later reconcile with backend authoritative events.

## 7. Bugfix Checklist (Required)

Before merging workflow/react changes:

1. Does this introduce a second logic path for waiting/resume?
2. Does this change signal shape or naming consistency?
3. Can a user signal be consumed but never applied?
4. Does stop still work during retries and waiting?
5. Does refresh/reconnect still preserve waiting behavior?
6. Are logs sufficient to trace the full signal path?

If any answer is unclear, the change is not ready.

## 8. Minimum Validation Scenarios

Run at least these manual checks for signal/waiting changes:

1. Enter `AwaitingUser`, send `user_message`, verify resume.
2. Enter `AwaitingApproval`, send wrong signal type, verify no incorrect resume.
3. In waiting state, send `stop`, verify immediate cancellation.
4. Refresh page in waiting state, verify behavior remains coherent.
5. During active execution, send user message, verify queue/visibility behavior is consistent and eventually applied.

## 9. Change Management Discipline

For any workflow/react feature or bugfix:

1. State the invariant touched.
2. State which single main path is updated.
3. State why no parallel branch is introduced.
4. Add/adjust tests or scenario evidence.

Do not ship workflow/react changes as isolated patches without this reasoning.
