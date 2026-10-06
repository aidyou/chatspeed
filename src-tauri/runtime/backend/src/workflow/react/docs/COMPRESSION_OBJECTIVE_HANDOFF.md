# Compression Objective Handoff

This document defines the compression handoff contract for ReAct workflows. It
is subordinate to [`../CONSTITUTION.md`](../CONSTITUTION.md): durable structured
authority and a stable ordered projection remain controlling.

## Problem

An initial read-only audit request can be retained while later explicit
instructions to implement and refine fixes disappear after repeated
compression. A later turn can then plan read-only or revert work without a new
user request.

This is not a frontend rendering or completion-report defect. A generated
handoff became the only practical record of a user scope transition. A
successful `complete_workflow` is a runtime segment boundary, not proof that
the user has cleared active context.

## Invariant

Until manual clear-context, the runtime must not silently discard an active
user goal during compression.

The contract has four owners:

1. `workflow_messages` is the durable transcript authority.
2. `EffectiveTaskObjective` records ordered directive sources in snapshots and
   events for recovery. It resets only at manual clear-context, not at
   `complete_workflow`.
3. The compressor may phrase one compact current goal; it never owns source
   IDs, completion evidence, todo state, or the decision to erase active work.
4. The runtime validates the generated goal state and persists it at the
   compression boundary before it can enter the LLM projection.

This keeps model-generated context concise without making a model summary the
authority for task continuity.

## Projection Order

For a compressed task, LLM-visible history has this semantic order:

```text
stable system prompt
fixed current-task goal observation
logical compression summary
raw tail after the compression boundary
fixed non-empty todo snapshot from the compression event
```

The stable system prompt never receives frequently changing task/todo data.
`workflow_context_messages` is a rebuildable projection cache, not an authority.

### Fixed Current Goal

At every blocking compression boundary, the runtime persists one hidden
user-role `current_task_goal` observation before the logical summary. Its LLM
content is compact:

```json
{
  "version": 1,
  "status": "active",
  "current_goal": "short concise goal",
  "source_count": 3,
  "latest_source_message_id": 42,
  "latest_directive": {
    "source_message_id": 42,
    "content": "bounded latest effective user directive"
  },
  "completion_evidence_message_id": null
}
```

Hidden metadata contains all authoritative `source_message_ids` and any
completion evidence ID. These values are assigned by the runtime, never
accepted from the model. `latest_directive` is also runtime-owned and bounded
to 1,200 characters. It preserves the latest explicit user intent alongside
the model's compact phrasing so a prior read-only request cannot be revived as
the current execution mode. The observation also contains a `SYSTEM_REMINDER`
that it is a fixed boundary record and later raw-tail user messages are newer.
It is not appended to the stable system prompt, preserving a cache-stable
system prefix.

Only the goal observation for the latest compression boundary is projected.
Older records remain durable history but are not duplicated into live context.

### Goal Source Ledger

Before a blocking compression call, the runtime derives a source ledger from
durable user messages after the latest manual clear-context marker and at or
before the compression boundary.

- Ordinary non-observation user messages are ordered directive sources.
- Canonical `ask_user` responses remain factual tool answers, not task scope.
- Runtime observations, summaries, todos, plans, and reviews are excluded.
- Complete runtime `<SYSTEM_REMINDER>...</SYSTEM_REMINDER>` blocks are removed
  from otherwise user-authored text. The surrounding user directive remains.
- `complete_workflow` does not restart the ledger. It provides completion
  evidence only when it occurs after the latest user directive.

The compressor receives a bounded ledger: source count/endpoints, prior compact
goal state, the highest-precedence latest directive, completion evidence, and
first-plus-recent bounded previews. A later directive determines execution mode
when it conflicts with an old one. When the prior compact goal is still
`active`, later directives are refinements by default: the model must retain
unresolved components of the prior goal and add the refinement, unless the user
explicitly abandons or replaces earlier work. Full source IDs stay durable
metadata, and large user messages remain in `workflow_messages` behind existing
database-reference previews. Raw user text is therefore not injected repeatedly
as the session grows.

### Model Output And Runtime Validation

A blocking compressor returns exactly five semantic fields:

```json
{
  "task_state": {
    "status": "active",
    "current_goal": "short concise goal"
  },
  "confirmed_facts": [],
  "boundary_open_items": [],
  "completed_work": [],
  "constraints_and_guards": []
}
```

`task_state.status` is `active`, `complete`, or `none`. `current_goal` is a
non-empty concise string for `active`/`complete` and `null` for `none`. The
runtime rejects output when an active durable source ledger becomes `complete`
or `none`; when completion lacks a successful `complete_workflow` after the
latest directive; when the shape is malformed/oversized; or when the model
emits source IDs, completion evidence, todos, next action, plans, or another
task-scope duplicate.

After validation, the runtime removes `task_state` from the logical summary and
persists it as the hidden current-goal observation. It injects the source IDs
and completion evidence from the ledger. A model cannot erase a source by
omitting or inventing an identifier.

Completed-task rollups do not generate live task state; their output remains
`confirmed_facts`, `completed_work`, `unresolved_carryovers`, and
`constraints_and_guards`.

### Raw Tail And Todo Snapshot

Messages after the boundary remain in original durable order and may refine the
goal or resolve an open item. When the authoritative todo list is non-empty,
compression writes one hidden `compression_todo_snapshot` after the summary;
projection places it after the raw tail. It is a fixed historical checkpoint,
not mutable request-time state and never a source of task scope. Empty todo
lists write no snapshot.

## Recovery And Compatibility

Recovery remains snapshot-first with event-replay fallback. Goal observations
and `EffectiveTaskObjective` are durable boundary records; the AI projection
cache can be rebuilt from them and the transcript. LLM request construction
must not query mutable task/todo state, synthesize a goal record, or reorder
semantic messages.

Older sessions may contain `current_task_user_context` observations. New
projection code excludes them when a compact current-goal observation exists.
Rows are not rewritten; the next new-format compression creates the compact
record.

## Deterministic File Changes

The runtime computes `file_changes` from successful file mutation observations.
After normalization, `/tmp` and descendants are excluded, including paths
carried from prior handoffs. Existing application normalization already maps
relevant macOS temporary paths to `/tmp`; no `/private/tmp` special case exists.

## Required Regression Coverage

1. Read-only then implementation directives produce an active implementation
   goal with both source IDs and the bounded latest implementation directive.
2. Appended runtime reminders are removed without removing user text.
3. A continuation after `complete_workflow` preserves active source continuity.
4. Repeated compression keeps only the latest compact goal record while source
   IDs survive restart/rebuild.
5. An active goal cannot become `none` on second/third compression without
   completion evidence; valid evidence permits `active -> complete -> none`.
6. Long user history keeps all source IDs while preview and live projection size
   stay bounded.
7. Raw-tail corrections occur only after goal/summary records; todo snapshots
   remain fixed and non-empty-only; `/tmp` changes remain excluded.
8. A later narrow correction preserves unresolved components of an active
   preceding goal instead of replacing it.
