# Workflow Context Incident: Orphaned Tool Result After Compression

Date: 2026-07-26

## Impact

Workflow session `0r3cp0fxc0400` could not continue after the upstream model
rejected a request with:

```json
{
  "error": {
    "message": "No tool call found for function call output with call_id fc_tool_0d06b14f.",
    "type": "invalid_request_error"
  }
}
```

The failure happened after the workflow had moved from the original release-note
request into the SQLite `DbRuntime` refactor task. No durable transcript data
was lost.

## Evidence

The relevant assistant message and tool batch are in `workflow_messages`:

| Message | Role | Detail |
| --- | --- | --- |
| `73344` | assistant | One response with five tool calls. |
| `73345` to `73348` | tool | Results for tool-call indexes 0 through 3. |
| `73349` | tool | `grep` result for `tool_0d06b14f`, tool-call index 4. |

At `2026-07-26 07:37:38`, compression summary `73525` was written with
`compressed_until_message_id = 73348`. This split the fifth result from its
assistant tool-call message:

```text
73344 assistant: tool_calls = [0, 1, 2, 3, 4]
73345 tool result: 0
73346 tool result: 1
73347 tool result: 2
73348 tool result: 3  <- compressed
73349 tool result: 4  <- retained in the tail
```

The rebuilt segment-30 cache therefore contained row `6539037`, sourced from
`73349`, with `tool_call_id = tool_0d06b14f` and `llm_content`, but no assistant
message in that segment declaring the matching tool call. The external Responses
protocol reported the corresponding call with the `fc_tool_0d06b14f` prefix.

## Root Cause

Two context-projection behaviors combined to emit the invalid request.

1. `is_safe_pressure_compression_boundary` evaluates an individual completed
   tool result as a safe compression boundary. It does not preserve the atomic
   relationship between an assistant tool-call batch and every result in that
   batch.
2. `protocol_safe_context_projection` normally removes a tool result that has
   no currently open matching assistant tool call. Its compatibility path keeps
   any orphaned `tool` message that has `metadata.llm_content`, allowing a
   function-call output to bypass the protocol pairing rule.

Relevant implementation locations:

- `src-tauri/src/workflow/react/context.rs`: `is_safe_pressure_compression_boundary`
- `src-tauri/src/workflow/react/context.rs`: `build_active_task_pressure_compression_candidate`
- `src-tauri/src/workflow/react/context.rs`: `should_preserve_unpaired_tool_projection`
- `src-tauri/src/workflow/react/context.rs`: `protocol_safe_context_projection`

## Production Recovery Performed

The repair intentionally changes only the two records known to be invalid:

1. Summary `73525` advances its compression boundary from `73348` to `73349`.
   Durable transcript rows are retained; the orphaned `grep` result is omitted
   from future LLM replay with the rest of its compressed batch.
2. Cached context row `6539037` is removed. It is a derived cache row, not
   transcript authority. A cold recovery rebuilds the active segment cache from
   `workflow_messages`.

No workflow messages, events, snapshots, or unrelated context-cache rows are
deleted.

The desktop app must be restarted before resuming this workflow so a live
executor cannot retain the old in-memory projection. After restart, continue
the existing task normally.

## Required Code Fix

The production data repair prevents this session from replaying an invalid
tool result, but the code must prevent future occurrences.

1. Treat an assistant tool-call batch and all of its tool results as an atomic
   compression unit. A compression boundary may not fall between the assistant
   call and its final terminal result.
2. Do not retain a `role = tool` message with `tool_call_id` when it has no
   matching assistant tool call in the projected sequence. `llm_content` may
   select reduced content only; it must not bypass function-call protocol
   validation.
3. Add a regression test for a multi-tool assistant batch where compression
   would retain only the final `llm_content` tool result. The final LLM message
   list must contain neither that orphaned tool result nor an unresolved
   assistant tool call.
