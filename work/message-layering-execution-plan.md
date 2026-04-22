# Workflow Message Layering Execution Plan

## 1. Goal

This document defines a bounded cleanup for workflow message layering during the current phase.

The goal is not to redesign the full workflow system. The goal is to remove the most harmful coupling between:

- persistent transcript messages
- runtime recovery state
- LLM provider protocol projection
- internal runtime observations
- frontend display state

This work must stay aligned with `work/plan.md`:

- Phase 7: stabilize `Call` sub-agents, parent waiting, structured child completion, and recovery.
- Phase 8: do not implement handoff or focused-agent routing in this stage.
- Phase 9: move UI toward structured state/events, without making frontend rendering part of core execution correctness.

## 2. Current Assessment

Complexity is medium.

The backend already has the necessary foundations:

- `workflow_events` exists.
- `ExecutionContext` exists.
- `GatewayPayload` carries typed runtime updates.
- `SubAgentCompletion` exists in `ExecutionContext`.
- `LlmProcessor::normalize_history_messages` already acts as a provider projection layer.
- `Dispatcher` already separates UI, audit, snapshot, and terminal sinks.

The main remaining problem is not missing infrastructure. The problem is that several code paths still use `workflow_messages.message` and string markers such as `<SYSTEM_REMINDER>` and `<tool_result>` as implicit control data.

Frontend cleanup is higher risk because `useWorkflowMessages.ts`, `useToolStateMapper.ts`, and `StatusPanel.vue` still derive important UI state from raw messages. This should not be fully removed in this stage.

Recommended scope for this stage:

- Do backend typed observation cleanup now.
- Keep frontend transcript compatibility.
- Add structured fields so frontend can migrate later.
- Do not delete the current message-based UI derivation yet.

## 3. Non-Goals

Do not implement these in this stage:

- Handoff.
- Focused agent routing.
- New task ledger database table.
- Full removal of `workflow_messages`.
- Full frontend rewrite around `workflow_events`.
- Provider-specific protocol redesign beyond current OpenAI-style message projection.
- Backward compatibility with old unpublished runtime data beyond safe fallback behavior.

## 4. Target Layering

### 4.1 Workflow Events

`workflow_events` are the append-only audit stream.

They should answer:

- What happened?
- When did it happen?
- Which session or sub-agent did it affect?
- What structured payload is needed for replay?

Examples:

- `workflow_started`
- `state_changed`
- `wait_entered`
- `approval_requested`
- `approval_resolved`
- `sub_agent_started`
- `sub_agent_completed`
- `workflow_completed`

Events must not depend on LLM prompt formatting.

### 4.2 Execution Context

`ExecutionContext` is the recovery source of truth.

It should answer:

- What runtime state is the workflow in?
- Is it waiting?
- What is it waiting for?
- Which approval tools are pending?
- Which call-mode sub-agent completions are pending or already consumed?

This layer must remain independent from `workflow_messages` text.

### 4.3 Workflow Messages

`workflow_messages` are the transcript and LLM-source material.

They may contain:

- user messages
- assistant messages
- tool observations
- runtime observations projected for LLM context

They should not be used as the primary recovery state.

For this stage, runtime observations can still be stored as messages, but they must be typed via metadata and rendered by a dedicated projection function.

### 4.4 LLM Projection

`LlmProcessor` converts stored messages and runtime observations into provider-compatible messages.

It should own:

- hiding internal observations from the wrong position
- preserving tool-call adjacency
- rendering typed observations into model-readable text
- adding error reminders when needed

It should not infer observation meaning from free-text markers unless as a fallback.

### 4.5 UI Projection

The frontend should eventually consume structured `GatewayPayload`, `ExecutionContext`, and workflow events.

For this stage:

- keep the existing message-based UI rendering
- enrich metadata so UI can prefer structured fields
- avoid adding new UI dependencies on raw `<SYSTEM_REMINDER>` text

## 5. Proposed Data Model Additions

### 5.1 Add Runtime Observation Type

Add a backend enum, preferably in `src-tauri/src/workflow/react/types.rs` or a dedicated module:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeObservationType {
    SubAgentCompletion,
    SubAgentInterrupted,
    CompletionRejected,
    ActiveTodosBlocked,
    AuditRejected,
    NoToolCall,
    InvalidToolCall,
    LoopDetected,
    TurnBlockedPostponed,
    StepBudgetWarning,
    SkillActivated,
    FileContextAttached,
    GenericReminder,
}
```

Do not overfit the enum. If a reminder does not affect workflow control, `GenericReminder` is acceptable.

### 5.2 Add Runtime Observation Metadata Helper

Introduce a helper for metadata construction:

```rust
pub fn runtime_observation_metadata(
    observation_type: RuntimeObservationType,
    data: serde_json::Value,
) -> serde_json::Value
```

The metadata should consistently include:

- `message_kind = "runtime_observation"`
- `observation_type`
- `llm_visibility`
- `ui_visibility`
- `data`

Suggested visibility values:

- `llm_visibility = "preserve_position"`
- `llm_visibility = "defer"`
- `llm_visibility = "hide"`
- `ui_visibility = "show"`
- `ui_visibility = "hide"`
- `ui_visibility = "card"`

### 5.3 Add LLM Rendering Helper

Introduce a renderer used by `normalize_history_messages`:

```rust
fn render_runtime_observation_for_llm(message: &WorkflowMessage) -> Option<RenderedObservation>
```

`RenderedObservation` should contain:

- `content`
- `placement`

Suggested placement:

- `Preserve`
- `Defer`
- `Hide`

This moves the decision out of string checks like `message.contains("<SYSTEM_REMINDER>")`.

String checks may remain only as legacy fallback.

## 6. Execution Steps

### Step 1: Add Typed Runtime Observation Helpers

Files:

- `src-tauri/src/workflow/react/types.rs`
- optional new module: `src-tauri/src/workflow/react/runtime_observation.rs`
- `src-tauri/src/workflow/react/mod.rs`

Tasks:

- Add `RuntimeObservationType`.
- Add visibility enums or string constants.
- Add metadata builder.
- Add tests for metadata shape.

Acceptance:

- No behavior change.
- Existing tests pass.
- Metadata is stable snake_case JSON.

### Step 2: Replace High-Impact Runtime Observation Metadata

Files:

- `src-tauri/src/workflow/react/engine.rs`
- `src-tauri/src/workflow/react/interceptors.rs`
- `src-tauri/src/workflow/react/loop_detector.rs`
- `src-tauri/src/workflow/react/orchestrator.rs`
- `src-tauri/src/commands/workflow.rs`

Prioritize these observations:

- `sub_agent_completion`
- `sub_agent_interrupted`
- completion tool rejection
- active todo block
- audit rejection
- loop detected
- no tool call
- invalid tool call
- turn-block postponed

Do not try to convert every `<SYSTEM_REMINDER>` in the repository at once. Some are normal prompt content, file context hints, or ccproxy compatibility hints.

Acceptance:

- New observations have typed metadata.
- Existing visible content remains compatible.
- Sub-agent completion still produces an LLM-visible `<tool_result>` projection.

### Step 3: Refactor LLM Normalization Into Typed Projection

Files:

- `src-tauri/src/workflow/react/llm.rs`

Tasks:

- Extract runtime observation classification from `normalize_history_messages`.
- Prefer metadata:
  - `message_kind == runtime_observation`
  - `observation_type`
  - `llm_visibility`
- Preserve sub-agent completion observations in their original position.
- Defer generic reminders only when needed to protect provider tool-call adjacency.
- Keep legacy fallback for old messages containing `<SYSTEM_REMINDER>`.

Important rule:

Never insert a user/runtime observation between an assistant message with `tool_calls` and its corresponding `tool` messages.

Acceptance tests:

- Internal runtime reminders are deferred after tool results.
- Sub-agent completion observations stay in original order.
- Legacy `<SYSTEM_REMINDER>` messages still do not break tool-call adjacency.
- Error tool observations still receive error reminders.

### Step 4: Make Sub-Agent Completion Fully Typed

Files:

- `src-tauri/src/workflow/react/engine.rs`
- `src-tauri/src/workflow/react/orchestrator.rs`
- `src-tauri/src/workflow/react/replay.rs`

Tasks:

- Keep `SubAgentCompletion` in `ExecutionContext` as the recovery truth.
- Ensure `sub_agent_completed` event contains enough structured data for replay.
- Ensure parent resumption uses `ExecutionContext.pending_sub_agent_completions`, not transcript scanning.
- Ensure the message inserted into transcript is a projection of typed completion, not the canonical state.
- Mark completion consumed only after the parent has successfully applied the observation.

Acceptance:

- Call-mode parent waits.
- Child completes.
- Parent receives typed completion observation.
- Restart while waiting can replay durable completion.
- `sub_agent_output` can idempotently return already delivered call-mode result.
- Repeated identical call-mode task reuses existing result instead of spawning another child.

### Step 5: Reduce Frontend Dependence On Raw Message Text

Files:

- `src/composables/workflow/useWorkflowMessages.ts`
- `src/composables/workflow/useToolStateMapper.ts`
- `src/components/workflow/StatusPanel.vue`

Tasks:

- Prefer `metadata.observation_type` and `metadata.data` over parsing message content.
- Keep old content parsing as fallback.
- Ensure runtime observations with `ui_visibility = "hide"` are hidden without regexing `<SYSTEM_REMINDER>`.
- Ensure sub-agent cards can read `task`, `result`, `status`, and `sub_agent_id` from metadata when available.

Acceptance:

- Existing UI still renders old sessions.
- Delegated Task card still shows task/result correctly.
- System reminders do not leak into normal user-visible transcript.

### Step 6: Add Regression Tests

Backend tests:

- `normalize_history` preserves protocol adjacency.
- `normalize_history` preserves sub-agent completion position.
- `sub_agent_output` can read call-mode durable completion.
- repeated call-mode prompt reuses prior result.
- completion rejection does not cause repeated empty `complete_workflow_with_summary` loops when `summary` is valid.

Frontend tests are optional in this repo if no existing test harness covers the workflow UI, but run build at minimum.

Verification commands:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml normalize_history
cargo test --manifest-path src-tauri/Cargo.toml sub_agent_output
pnpm build
```

## 7. Risk Analysis

### Low Risk

- Adding typed metadata helpers.
- Adding LLM projection tests.
- Preserving current rendered content while adding metadata.

### Medium Risk

- Changing `normalize_history_messages`.
- Changing sub-agent completion consumption timing.
- Changing frontend hiding logic for runtime observations.

### High Risk

- Removing message-based UI derivation.
- Removing legacy `<SYSTEM_REMINDER>` fallback.
- Changing `workflow_messages` schema.
- Treating workflow events as the only UI source immediately.

High-risk items should not be done in this stage.

## 8. Recommended Scope For This Stage

Do now:

- Add typed runtime observation metadata.
- Refactor LLM normalization to prefer metadata.
- Keep sub-agent completion as a typed observation and stable LLM projection.
- Keep `sub_agent_output` call-mode fallback.
- Update frontend to prefer metadata but keep old parsing.
- Add regression tests.

Do later:

- Dedicated task ledger table.
- Full frontend task view model from events.
- Removal of transcript parsing.
- Handoff/focus-agent routing.

## 9. Definition Of Done

This cleanup is done when:

- `Call` sub-agent result delivery does not depend on parsing previous message text.
- Runtime recovery uses `ExecutionContext`, not transcript inference.
- LLM context construction uses typed observation metadata first.
- `<SYSTEM_REMINDER>` string matching remains only as backward-compatible fallback.
- Frontend hides/renders runtime observations using metadata where available.
- Existing old sessions remain displayable.
- `cargo check`, targeted Rust tests, and `pnpm build` pass.

## 10. Final Recommendation

This is worth doing in the current phase, but only with the bounded scope above.

The key is to avoid turning this into a full event-sourcing or frontend rewrite. The correct near-term cleanup is to make runtime observations typed and make LLM/UI projections consume those types. That directly addresses the current bugs around call-mode sub-agents, completion loops, and misplaced `SYSTEM_REMINDER` messages without crossing into Phase 8.
