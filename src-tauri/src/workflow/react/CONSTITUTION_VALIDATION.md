# Workflow Constitution Validation Matrix

This matrix tracks the minimum validation required by section 15 of
`CONSTITUTION.md`. It is a merge checklist, not a claim that every row is
covered by the lightweight frontend suite.

Wire-format fields persisted by Rust or carried by gateway payloads use
snake_case. In particular, workflow message metadata uses `tool_name` and
`tool_call_id`. Camel-case fields such as `toolName` and `toolCallId` exist only
after an explicit frontend normalization boundary.

| Scenario | Authoritative structured state | Automated evidence | Manual validation when affected |
| --- | --- | --- | --- |
| Active execution | `ExecutionContext.state`, persisted tool execution status, and gateway tool events | Rust workflow engine/context tests; frontend tool-state projection tests | Start a workflow, observe a running tool, refresh once, and verify there is one stable tool row with no approval controls. |
| Approval wait | `wait_reason = approval`, `pending_tools`, and exact `tool_call_id` | `workflowApprovalRecovery.test.js`, `workflowUiContract.test.js`, Rust replay approval tests | Trigger Bash, edit, write, and submit-plan approvals; verify the badge count and inline controls refer to the same calls. |
| User-input wait | `wait_reason = user_input` and the structured `ask_user` tool identity | Rust replay/context tests; static structure-first checks | Trigger `ask_user`, refresh, answer once, and verify the workflow resumes without a duplicate prompt. |
| Sub-agent wait | `wait_reason = sub_agent` and `waiting_on_sub_agent_id` | `child_tasks_tests.rs` restart, serialization, and completion/resume tests | Run a blocking sub-agent, refresh while waiting, and verify the same child card resolves once. |
| Final-review wait | `review_display_state = final_review_pending`, child session identity, and explicit execution status | `messageProjectionRules.test.js` | Reach final review, refresh, and verify the task stays active until the reviewer resolves. |
| Compression wait | Compression boundary ID and compression status events | Rust compression and engine tests | Trigger blocking/background compression and verify progress clears without changing task or approval presentation. |
| Refresh during waiting | Persisted workflow snapshot plus execution context | Frontend approval recovery tests; Rust snapshot normalization tests | Refresh once for every affected wait reason and compare state, controls, badge counts, and call IDs before and after. |
| Restart/recovery during waiting | Event replay result and persisted `ExecutionContext` | Rust replay reducer, command recovery, and child-task restart tests | Restart the application during the affected wait and verify one recovered interaction resumes the original session. |
| Approval round-trip | Exact `session_id + tool_name + tool_call_id` and latest structured approval/execution state | `workflowUiContract.test.js`, `workflowApprovalRecovery.test.js`, Rust approval event/replay tests | Approve and reject each affected tool; verify pending count reaches zero and stale transcript rows do not retain controls. |
| Completed-session resume | Persisted terminal snapshot and the manager's single resume claim | Rust manager and command completed-resume tests | Resume a completed session once and verify a new active segment is created without duplicating the completed task. |
| Compression/context rebuild | Persisted compression summary, boundary ID, and rebuilt execution context | Rust compression, context, and execution-context hydration tests | Force compression, restart, and verify the active request, previous-task archive, tool state, and pending interaction remain consistent. |

For each workflow change, record which rows were affected and which manual
checks were actually run. Unrun manual rows remain explicit release risk even
when the targeted automated suites pass.
