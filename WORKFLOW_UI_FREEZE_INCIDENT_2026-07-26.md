# Workflow UI Freeze During Long-Running Shell Commands

## Status

Diagnosed on 2026-07-26. No production-code change is included in this document.

## Symptom

While a Workflow shell tool runs a long command with frequent output, for example:

```text
cargo fmt --manifest-path "src-tauri/Cargo.toml" && cargo test --manifest-path "src-tauri/Cargo.toml" --lib db::agent && cargo check --manifest-path "src-tauri/Cargo.toml"
```

the Workflow window can become unresponsive:

- The window cannot be dragged smoothly.
- Tooltips and click interactions stop responding.
- Scrolling is delayed or stops.
- Memory consumption remains normal.

The command continues running in the backend. The issue is not an out-of-memory condition and is not evidence that the Rust command execution is synchronous.

## Root Cause

`ToolStream` bypasses the existing 100 ms batching used for LLM text and reasoning chunks. The shell executor produces one `ToolStream` payload for each stdout or stderr line. The gateway immediately emits each of those payloads into the WebView.

The frontend handles every event synchronously on the WebView main thread. Each event updates several reactive data structures, replaces the complete workflow `messages` array, recomputes the task/message projection, patches Vue DOM, schedules scrolling, and may perform user-message overflow measurement that forces layout.

High-frequency shell output therefore saturates the WebView main thread. Native asynchronous backend execution does not prevent this: Tauri event listeners and Vue rendering still run on the UI thread.

## Event Path

```text
Shell stdout/stderr line
  -> GatewayPayload::ToolStream
  -> TauriGateway event queue
  -> Immediate `workflow://event/<session>` emit
  -> useWorkflowCore listener
  -> workflowStore.appendToolStream
  -> Task Ledger + messages projection update
  -> Vue computed/watch work, DOM patch, scroll and layout measurement
  -> WebView event loop starvation
```

## Evidence

### 1. Shell output is emitted once per line

`src-tauri/src/tools/shell.rs` reads stdout and stderr with `next_line()` and emits a `GatewayPayload::ToolStream` for each non-empty line.

- `src-tauri/src/tools/shell.rs:1647`
- `src-tauri/src/tools/shell.rs:1657`
- `src-tauri/src/tools/shell.rs:1685`
- `src-tauri/src/tools/shell.rs:1722`

### 2. Gateway batches text chunks but not tool-stream events

`TauriGateway` batches `Chunk` and `ReasoningChunk` for 100 ms. `ToolStream` falls through the default branch, which flushes buffers and immediately invokes `app_handle.emit`.

- `src-tauri/src/workflow/react/gateway.rs:61`
- `src-tauri/src/workflow/react/gateway.rs:68`
- `src-tauri/src/workflow/react/gateway.rs:95`
- `src-tauri/src/workflow/react/gateway.rs:112`

The 5 ms sleep at the end of the receiver loop limits throughput only roughly. It does not coalesce `ToolStream` payloads, so a noisy command can still deliver approximately hundreds of UI events per second.

### 3. The frontend processes every event synchronously

The workflow listener forwards every `tool_stream` payload directly to `appendToolStream`.

- `src/composables/workflow/useWorkflowCore.ts:1170`
- `src/composables/workflow/useWorkflowCore.ts:1174`

### 4. A stream event invalidates broad reactive state

For each line, `appendToolStream`:

1. Appends to the tool stream list.
2. Copies the tool's stream output array.
3. Rebuilds Task Ledger maps through `upsertToolViewState`.
4. Replaces `messages.value` using a full-array `map` through `patchToolMessage`.

- `src/stores/workflow.js:1050`
- `src/stores/workflow.js:1066`
- `src/stores/workflow.js:1072`
- `src/stores/workflow.js:1085`
- `src/stores/workflow.js:1149`
- `src/stores/workflow.js:1152`
- `src/stores/workflow.js:812`
- `src/stores/workflow.js:819`

The maximum of 100 stream lines limits retained memory but does not limit update frequency.

### 5. The messages projection and layout path run after each invalidation

The task-window watcher is explicitly synchronous. The active task group is then enhanced through scans, signatures, tool-display calculation, and message reuse logic.

- `src/composables/workflow/useWorkflowMessages.ts:340`
- `src/composables/workflow/useWorkflowMessages.ts:353`
- `src/composables/workflow/useWorkflowMessages.ts:423`
- `src/composables/workflow/useWorkflowMessages.ts:757`
- `src/composables/workflow/useWorkflowMessages.ts:1031`

`WorkflowMessageList` also rebuilds visible message groups and watches them to schedule scrolling and user-message overflow measurement.

- `src/components/workflow/WorkflowMessageList.vue:1814`
- `src/components/workflow/WorkflowMessageList.vue:1883`
- `src/components/workflow/WorkflowMessageList.vue:2664`

Overflow measurement creates a temporary `pre`, reads computed styles and `scrollHeight`, then removes the element. This can force layout, and it currently runs after any visible-message update rather than only after a user-message or width change.

- `src/components/workflow/WorkflowMessageList.vue:2187`
- `src/components/workflow/WorkflowMessageList.vue:2214`
- `src/components/workflow/WorkflowMessageList.vue:2251`

## Secondary Risk

The normal LLM chunk path is substantially safer because the backend batches it, but it is not constant-cost. `useWorkflowChat` re-runs inline-reasoning extraction against the entire accumulated `rawContent` for each received chunk. Very long streamed reasoning can therefore still produce increasing frontend work.

- `src/composables/workflow/useWorkflowChat.ts:67`
- `src/composables/workflow/useWorkflowChat.ts:92`
- `src/composables/workflow/useWorkflowChat.ts:152`

This is not the primary cause of the shell-command freeze described above.

## Recommended Fix

Implement the changes in this order. The first two layers are required; later layers reduce residual work and prevent regressions.

### 1. Batch `ToolStream` in the Rust gateway

Extend `TauriGateway` so `ToolStream` is accumulated per `tool_id`, then emitted at a bounded interval such as 100 ms, or when a per-tool byte threshold is reached. Flush pending tool output before emitting a non-stream control event and when the tool reaches a terminal state.

Requirements:

- Preserve output ordering for each tool.
- Keep stdout/stderr labels already prepared by the shell tool.
- Bound buffered bytes per tool/session so a producer cannot create unbounded memory use.
- Emit a final pending batch before completion/failure is rendered.
- Do not apply a global delay that mixes output across different tools.

The preferred wire change is a dedicated batched payload, for example `ToolStreamBatch { tool_id, outputs, timestamp }`. A temporary compatibility option is to concatenate lines into the existing `output` field, but a typed batch is clearer and avoids later parsing ambiguity.

### 2. Add frontend frame/interval coalescing as a defensive boundary

In `useWorkflowCore`, queue incoming `tool_stream` payloads by `tool_id`. Flush queued data once per animation frame or at a maximum interval of approximately 100 ms. This protects the UI if a future backend producer bypasses gateway batching.

The frontend queue must:

- Flush before handling a matching terminal event (`tool_completed` or `tool_failed`).
- Clear its timer/frame handles on unmount and workflow switching.
- Cap queued bytes/lines and preserve a visible truncation indication if the cap is reached.
- Avoid one Vue mutation per incoming raw line.

### 3. Make one stream flush produce one minimal state update

Refactor `appendToolStream` to operate on a batch and avoid changing state that is not needed for live output:

- Store stream output independently from the persisted-message projection.
- Update collapsed tool summaries at the stream flush cadence, not for every raw line.
- Avoid `messages.value = messages.value.map(...)` for every line.
- Avoid cloning `TaskLedger` maps and stream-output arrays more than once per batch.
- Keep the 100-line display cap, but cap by bytes as well when individual lines can be very large.

The stream output currently renders only for an expanded tool. The collapsed UI does not need a full messages-projection invalidation for every raw line.

### 4. Narrow layout work in `WorkflowMessageList`

`measureUserMessageOverflow` should be scheduled only when one of these changes:

- The current workflow changes.
- User-message identities or user-message text changes.
- The message-list width changes.

It should not be triggered by tool-stream summary updates, execution status changes, or assistant/tool display-only changes. Keep auto-scroll separately throttled to one animation frame; do not couple layout measurement to the generic `visibleMessages` watcher.

### 5. Profile and consider list virtualization only after event pressure is fixed

The current message projection is intentionally rich and does broad scanning/signature work for active groups. Once stream events are batched, profile workflows with large histories. Add list virtualization only if the resulting render cost remains material. Virtualization alone does not solve the current event-rate issue.

## Validation Plan

### Automated coverage

1. Add Rust tests for gateway batching:
   - Many `ToolStream` sends for one tool yield bounded UI emissions.
   - Different tools retain per-tool ordering.
   - A terminal/control event flushes pending stream output first.
   - Oversized buffered output remains bounded.
2. Add frontend unit tests for queueing:
   - Many received stream events produce one store flush per scheduled interval/frame.
   - Tool completion flushes buffered output before final status is applied.
   - Switching workflows/unmounting cancels queued work.
3. Add a store test that one batch causes at most one message and Task Ledger projection update.

### Manual performance verification

Run a high-output command in Workflow, for example a verbose test/build command, with Chrome/WebView performance recording enabled.

Acceptance criteria:

- The window remains draggable and tooltips continue to respond throughout execution.
- The main thread has no sustained long-task sequence caused by `workflowEvent:tool_stream`.
- The number of rendered tool-stream state flushes is bounded to roughly 10 per second per active tool, excluding terminal flushes.
- The final displayed stream output and exit result are complete and ordered.
- Memory and buffered stream size stay bounded for a deliberately noisy command.

## Non-Goals

- Do not change workflow backend authority or event semantics.
- Do not remove shell output streaming.
- Do not use the existing 100-line UI cap as the only form of backpressure.
- Do not solve this by blocking or dropping all Tauri workflow events.
