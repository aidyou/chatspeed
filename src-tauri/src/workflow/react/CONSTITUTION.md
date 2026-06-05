# Workflow React Constitution

This document is the highest-priority maintenance contract for `src-tauri/src/workflow/react`.

It exists to keep the workflow runtime aligned with `work/plan.md`, prevent architectural drift, and stop regressions caused by convenience fallbacks becoming new main paths.

If this document conflicts with ad-hoc local behavior, this document wins.

## 1. Scope

This constitution governs:

- `src-tauri/src/workflow/react/*`
- `src-tauri/src/commands/workflow.rs`
- frontend workflow runtime consumers that depend on workflow state, signals, approvals, or tool observations

This constitution does not replace implementation guides. It constrains them.

## 2. Primary Goal

The workflow module is a reliable execution kernel, not a best-effort chat feature.

Its first responsibility is correctness of:

- session lifecycle
- waiting state modeling
- approval state modeling
- recovery
- context projection
- structured UI/runtime synchronization

New capabilities must not weaken those guarantees.

## 3. Non-Negotiable Invariants

### 3.1 Backend authority is absolute

The backend is the only authority for:

- session liveness
- runtime state
- wait reason
- pending approvals
- queued user messages
- resumability

The frontend may cache and optimistically render, but it must reconcile to backend state.

### 3.2 Structured state is always preferred over transcript text

Recoverable behavior must come from:

- `ExecutionContext`
- structured events
- structured tool metadata
- structured approval payloads

It must not come from:

- assistant text
- tool text
- Markdown blocks
- embedded JSON inside normal transcript content

Transcript content is presentation, not authority.

### 3.3 There must be one canonical path per concern

The module must not maintain parallel main paths for:

- waiting
- approval recovery
- signal parsing
- context rebuild
- session resume

Compatibility adapters may exist, but they must collapse immediately into the canonical path.

### 3.4 Compatibility logic is an adapter, not a second system

Legacy support is allowed only at explicit boundaries:

- signal wire aliases
- old persisted payload migration
- old frontend payload hydration

Legacy handling must normalize immediately into canonical structured form.

No new feature may be built directly on top of a legacy or fallback representation.

## 4. Session Lifecycle Law

### 4.1 `WorkflowManager` is the lifecycle registry

`WorkflowManager` owns:

- session registration
- executor lookup
- managed status
- hot-resume eligibility
- cleanup eligibility

No other structure may become a competing lifecycle registry.

### 4.2 Completed sessions must support hot resume within the configured grace window

If a completed session is still within its grace period, new user input must prefer executor reuse over executor reconstruction.

Cold recovery is allowed only when:

- the executor is gone
- the channels are stale
- recovery is explicitly required
- the grace period has elapsed

### 4.3 Cleanup must be state-safe

Delayed cleanup must only remove a session if both remain true:

- status is still terminal and eligible for cleanup
- the recorded completion/update marker still matches

Cleanup timers must never be able to delete an actively resumed session.

## 5. Waiting Law

### 5.1 Waiting is modeled by `state + wait_reason`

Any user-interactive pause must be represented by:

- a waiting-capable `WorkflowState`
- a canonical `WaitReason`

The UI and command layer must not infer waiting intent from text or tool names when structured wait state is available.

### 5.2 Waiting validation is centralized

Signals that resume a wait state must be validated against `wait_reason`.

Do not add new wait-state acceptance logic in unrelated files.

### 5.3 Stop must remain globally actionable

`stop` must continue to work:

- during active execution
- during waiting
- during retry/backoff windows
- during temporary signal drains

Any code that temporarily intercepts signals must preserve stop semantics.

## 6. Signal Law

### 6.1 Signals have one canonical shape

Every runtime signal must have:

- a canonical snake_case wire name
- a typed backend representation
- an explicit accepted-state contract

### 6.2 Gateway transport is not permission to stay untyped

`TauriGateway` may transport raw JSON strings as a wire detail, but command and engine layers must normalize them immediately into typed signal meaning.

Raw JSON strings are transport format only.

### 6.3 New signal types require full-path updates

Adding a signal requires all of:

1. canonical definition in backend types
2. compatibility mapping if needed
3. waiting/non-waiting handling rules
4. frontend emission mapping
5. logs
6. recovery expectations

Adding a signal in only one layer is prohibited.

## 7. Approval Law

### 7.1 Approval payloads must remain structured end-to-end

For every pending tool:

- `tool_call_id` is the canonical identifier
- `tool_name` is explicit
- `arguments` is a structured `Value`
- `details` is a structured `Value` or `null`
- `display_type` is explicit when rendering depends on it

Stringified JSON is not an acceptable primary representation for approvals.

### 7.2 Approval recovery must not parse transcript JSON

Approval restoration must come from:

- `ExecutionContext.pending_tools`
- structured events
- structured pending approval maps

It must not depend on reparsing:

- assistant messages
- tool message body strings
- approval dialog content text

### 7.3 Approval UI messages must carry canonical metadata

Pending approval tool messages must carry enough metadata for the frontend to render directly:

- `tool_call`
- `tool_call_id`
- `tool_name`
- `details`
- `display_type`

The frontend may keep compatibility fallback for old data, but new live data must not require string re-parsing.

### 7.4 Mutation tools are not lossy-preview candidates

`edit_file`, `write_file`, and other file-mutation tools must not be passed through generic lossy truncation that destroys preview structure.

If a special preview policy is needed, it must preserve semantic renderability.

## 8. Context Law

### 8.1 `messages` is the durable history

`messages` is the source of truth for transcript history.

Database implication:

- `workflow_messages` is authoritative transcript storage
- audit, replay fallback, and semantic reporting must prefer it over derived caches

### 8.2 `context_messages` is a projection, not an independent state machine

`context_messages` exists to feed the LLM efficiently.

It must be rebuilt from runtime history according to explicit rules.

It must not accumulate hidden semantics through ad-hoc clone/append mutation.

Database implication:

- `workflow_context_messages` is a rebuildable AI-context cache
- it must not become authority for recovery, audit, reporting, or UI semantics
- active AI segment boundaries must recover from transcript/snapshot authority, not from cache rows
- if cache contents conflict with durable history, durable history wins and cache must be rebuilt

Consumer boundary implication:

- AI may read in-memory `context_messages` and persisted `workflow_context_messages` as cache
- recovery must not depend on `workflow_context_messages`
- UI must not depend on `workflow_context_messages` for semantic correctness
- reports and metrics must not depend on `workflow_context_messages`

### 8.3 Context rebuild must be rule-driven

At minimum, context rebuild rules must distinguish:

- no-compression full context
- active-task pressure compression
- task-boundary rollup compression
- completed-task-to-new-task segment carryover

Do not hide those semantics behind generic “copy current projection” behavior.

### 8.4 Completed-task carryover must stay explicit

When a new task starts after completed work, the projection must preserve exactly the carryover policy that the module defines.

It must not depend on whatever happened to remain in a previous projection.

Current required carryover contract:

- AI context must preserve the latest compression summary when one exists
- AI context must preserve the most recent completed task after that summary
- AI context must preserve the current unfinished task
- older completed tasks may be rolled into summary, but must not silently replace the retained latest completed task

Unless the compression algorithm is intentionally redesigned, changes that weaken this carryover contract are prohibited.

### 8.5 Compression thresholds are part of the design contract

Compression behavior is not an implementation detail. It is part of the workflow model.

Current required thresholds:

- pressure compression must preserve the latest completed task and only compress older completed work
- initial task-boundary rollup must not trigger until three completed tasks exist and a new active task has resumed
- after a summary already exists, rollup must continue to preserve the latest completed task and only compress older completed work
- the system must not collapse AI context to only the current task while removing both summary and latest completed-task carryover

Do not change these thresholds or retention rules unless the workflow compression design itself is explicitly being revised.

## 9. Recovery Law

### 9.1 Snapshot first, replay fallback

Recovery must continue to prefer:

1. valid snapshot
2. structured replay fallback

Database implication:

- `workflow_snapshots` is the structured recovery authority
- snapshot contents must remain structural runtime state, not a second transcript
- `current_segment_id` belongs to structured recovery authority and must be persisted in snapshot state
- transcript reconstruction from snapshot text is prohibited

### 9.2 Command-layer recovery cannot become a parallel state engine

`commands/workflow.rs` may route recovery, but it must not become a second, text-driven state machine.

Any temporary fallback based on legacy persisted status strings must be treated as migration debt and kept visibly isolated.

### 9.3 Safe failure is explicit

When recovery cannot be trusted, the workflow must fail safely and observably.

Silent best-effort recovery that may execute with unknown state is prohibited.

### 9.4 Cache corruption is rebuilt, not interpreted

If `workflow_context_messages` is missing, stale, or corrupted, the correct action is rebuild.

Recovery must not reinterpret cache rows as hidden authority.

If `workflow_messages` and `workflow_context_messages` disagree, `workflow_messages` wins.

## 10. UI Contract Law

### 10.1 Frontend workflow decisions must be structure-based

Frontend logic should prefer:

- `state`
- `wait_reason`
- `executionContext`
- structured `metadata`

It should avoid depending on:

- message body heuristics
- title text heuristics
- JSON embedded in transcript text

### 10.2 Fallback parsing must shrink over time

If the frontend contains fallback parsing for old payloads, it must be treated as compatibility debt.

New backend work must reduce reliance on fallback parsing, not introduce more of it.

### 10.3 UI filtering must not erase authoritative tool records

If a tool call is part of the authoritative execution record, frontend filtering must not hide it unless the filtered replacement preserves the same semantic information.

### 10.4 Frontend authority is concern-scoped

The frontend must not choose one global local source for all workflow UI behavior.

Each workflow concern has its own authority, and that authority must match the concern's lifecycle:

- session lifecycle, resumability, terminal state, and waiting:
  authority is backend workflow state (`state + wait_reason`)
- current active workflow inline approvals:
  authority is the current workflow's approval-wait state plus the latest structured state for each pending `tool_call_id`
- approval recovery after reload or cold resume:
  authority is `ExecutionContext.pendingTools`, normalized into the current inline approval view model
- cross-session top-bar user-action reminders:
  authority is a background notification cache built from structured approval and user-input wait events, reconciled by workflow state transitions
- tool execution lifecycle:
  authority is the structured tool ledger or latest structured tool metadata for the same `tool_call_id`
- message rendering:
  authority is the message projection derived from durable messages and structured metadata

No concern may borrow another concern's authority just because the data is convenient.

Examples:

- the top-bar user-action reminder cache must not decide which approval buttons appear inside the active message list
- rendered message scans must not decide global reminder counts
- old transcript messages must not keep an approval alive after backend state has left approval wait
- a pending approval message must be ignored when a newer structured state for the same `tool_call_id` is approved, running, rejected, failed, interrupted, or completed

### 10.5 Approval view models must be lifecycle-gated

The current active workflow's inline approval view model must be valid only while the workflow is waiting for approval:

- `wait_reason == approval`, or
- a canonical approval-waiting workflow state is present

If the workflow is running, completed, failed, cancelled, awaiting user input, awaiting a sub-agent, or waiting for confirmation, the current inline approval view model must be empty even if old messages still contain `approval_status = pending`.

Within an approval-waiting workflow, inline approval membership must be derived by reducing structured records by `tool_call_id` with latest-state semantics:

- pending states add or keep the item
- approved, submitted, running, rejected, completed, failed, or interrupted states remove the item
- duplicate historical messages for the same `tool_call_id` must collapse to one current item
- bulk approval targets must come from this current inline approval view model, not from rendered DOM state or global reminders

This rule exists specifically to make old persisted messages safe: old pending records may remain in transcript history, but they must not become current business state after the workflow state or latest tool state has moved on.

### 10.6 Background user-action reminders are a notification cache

The top-bar indicator is a cross-session user-action reminder cache, not the approval protocol itself.

It may contain:

- background approval requests from structured `confirm` events
- background ask-user reminders from structured user-input waits
- handoff entries produced when the active workflow is switched away while it is still waiting

It must be reconciled by:

- structured per-tool resolution events (`approval_resolved`, `tool_started`)
- workflow state transitions that leave approval or user-input waiting
- terminal workflow state transitions
- active-session selection, which must remove that session from the background reminder cache and rebuild active inline state from the active workflow authority

The user-action reminder cache may notify about both approvals and ask-user waits, but it must never resurrect old active-session approvals or override the current active workflow's inline approval view model. Ask-user waits and tool approvals must remain distinct entries with distinct `kind` values.

### 10.7 Message lists are projections, not business-state engines

Frontend message lists may merge, collapse, or restyle data for readability.

They must not become the authority for:

- global user-action reminder membership
- resumability
- wait-state meaning
- terminal vs running tool state when a structured source already exists
- approval counts or bulk approval target sets

If a message projection conflicts with the canonical authority for its concern, the authority wins and the projection must reconcile to it.

## 11. Command-Layer Discipline

`commands/workflow.rs` is allowed to do orchestration.

It is not allowed to become:

- a second planner
- a second context engine
- a second approval protocol
- a transcript-interpreting recovery engine

If command-layer logic must inspect transcript history, that logic must be explicitly justified as presentation or migration compatibility, not core correctness.

## 12. Observability Law

Any change to lifecycle, waiting, approval, recovery, or context rules must keep logs good enough to answer:

- what session changed
- what phase handled it
- what signal or event triggered it
- what authoritative state changed
- whether recovery or compatibility logic was used

If a change reduces traceability, it is not acceptable.

## 13. Forbidden Patterns

The following patterns are forbidden unless they are strictly isolated compatibility shims:

- reparsing JSON from transcript text to recover state
- introducing a second pending approval representation with different semantics
- inferring wait state from status strings when structured wait state is available
- copying `context_messages` as a hidden state shortcut
- hiding tool mutations behind generic truncation
- adding new signal names on only one side of the wire
- fixing a structured payload problem by adding more frontend string parsing

## 14. Required Review Questions

Every change touching this module must answer:

1. What is the single authoritative state source for this behavior?
2. Does this add a parallel path or just normalize into the existing one?
3. Is recovery still structure-first?
4. Is the frontend consuming structure or guessing from text?
5. Does this increase or decrease compatibility debt?
6. What invariant from this constitution is being protected?

If these answers are not explicit, the change is not ready.

## 15. Minimum Validation Before Merge

Changes touching lifecycle, waiting, approval, recovery, or context must validate the relevant scenarios:

1. active execution
2. each wait reason
3. refresh during waiting
4. restart/recovery during waiting
5. approval round-trip
6. completed-session resume
7. compression or context rebuild if affected

Manual validation is acceptable, but it must be stated.

## 16. Amendment Rule

This constitution may be changed only when:

- the new rule is more explicit than the old one
- the change is justified by architecture, not convenience
- the change reduces ambiguity instead of introducing it

If a future patch needs to bypass this constitution to “quickly fix” something, the correct assumption is that the patch is probably wrong.
