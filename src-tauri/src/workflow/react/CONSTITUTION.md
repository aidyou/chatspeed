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

### 8.2 `context_messages` is a projection, not an independent state machine

`context_messages` exists to feed the LLM efficiently.

It must be rebuilt from runtime history according to explicit rules.

It must not accumulate hidden semantics through ad-hoc clone/append mutation.

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

## 9. Recovery Law

### 9.1 Snapshot first, replay fallback

Recovery must continue to prefer:

1. valid snapshot
2. structured replay fallback

### 9.2 Command-layer recovery cannot become a parallel state engine

`commands/workflow.rs` may route recovery, but it must not become a second, text-driven state machine.

Any temporary fallback based on legacy persisted status strings must be treated as migration debt and kept visibly isolated.

### 9.3 Safe failure is explicit

When recovery cannot be trusted, the workflow must fail safely and observably.

Silent best-effort recovery that may execute with unknown state is prohibited.

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
