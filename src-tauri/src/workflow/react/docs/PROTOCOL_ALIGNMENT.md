# Workflow Protocol Alignment

This document is the single reference for wire-level names used by:
- frontend `src/composables/workflow/signalTypes.ts`
- backend `src-tauri/src/workflow/react/{signals.rs,types.rs}`

Rule:
- Add/rename protocol names in both sides in one PR.
- Prefer canonical names; keep legacy aliases only for compatibility.

## 1) Signal Types

| Purpose | Canonical wire name | Backend enum | Legacy alias | Notes |
|---|---|---|---|---|
| User text input | `user_message` | `SignalType::UserMessage` / `WorkflowSignal::UserMessage` | `user_input` | `user_input` is accepted for backward compatibility only. |
| Approval decision | `approval` | `SignalType::Approval` / `WorkflowSignal::ApprovalDecision` | - | Frontend sends `id`, backend maps to `tool_call_id`. |
| Continue execution | `continue` | `SignalType::Continue` / `WorkflowSignal::Continue` | - | Valid for `wait_reason=confirmation`. |
| Stop execution | `stop` | `SignalType::Stop` / `WorkflowSignal::Stop` | - | Valid for all waiting states and active loop. |
| Re-broadcast pending approvals | `rebroadcast_pending` | `SignalType::RebroadcastPending` / `WorkflowSignal::RebroadcastPending` | `request_confirm_broadcast` | Legacy alias kept for older clients. |
| Update final audit config | `update_final_audit` | `SignalType::UpdateFinalAudit` | - | Runtime config signal. |
| Update auto-compress config | `update_auto_compress` | `SignalType::UpdateAutoCompress` | - | Runtime config signal. |
| Update approval level config | `update_approval_level` | `SignalType::UpdateApprovalLevel` | - | Runtime config signal. |
| Update phase config | `update_phase` | `SignalType::UpdatePhase` / `WorkflowSignal::UpdatePhase` | - | Runtime config signal. |
| Update allowed paths | `update_allowed_paths` | `SignalType::UpdateAllowedPaths` | - | Runtime config signal. |
| Update model config | `update_model_config` | `SignalType::UpdateModelConfig` | - | Runtime config signal. |
| Update skills config | `update_skills_config` | `SignalType::UpdateSkillsConfig` | - | Runtime config signal. |
| Remove shell policy item | `remove_shell_policy_item` | `SignalType::RemoveShellPolicyItem` / `WorkflowSignal::RemoveShellPolicyItem` | - | Supports UI policy updates. |
| Remove auto-approved tool | `remove_auto_approved_tool` | `SignalType::RemoveAutoApprovedTool` / `WorkflowSignal::RemoveAutoApprovedTool` | - | Supports UI auto-approve updates. |
| Sub-agent completion | `sub_agent_complete` | `SignalType::SubAgentComplete` / `WorkflowSignal::SubAgentComplete` | - | Valid for `wait_reason=sub_agent`. |

## 2) Workflow State Names

| Wire name | Backend enum |
|---|---|
| `pending` | `WorkflowState::Pending` |
| `thinking` | `WorkflowState::Thinking` |
| `executing` | `WorkflowState::Executing` |
| `auditing` | `WorkflowState::Auditing` |
| `paused` | `WorkflowState::Paused` |
| `awaiting_user` | `WorkflowState::AwaitingUser` |
| `awaiting_approval` | `WorkflowState::AwaitingApproval` |
| `awaiting_auto_approval` | `WorkflowState::AwaitingAutoApproval` |
| `awaiting_sub_agent` | `WorkflowState::AwaitingSubAgent` |
| `completed` | `WorkflowState::Completed` |
| `error` | `WorkflowState::Error` |
| `cancelled` | `WorkflowState::Cancelled` |

## 3) Wait Reasons

| Wire name | Backend enum | Meaning |
|---|---|---|
| `user_input` | `WaitReason::UserInput` | Executor is waiting for user text input. |
| `approval` | `WaitReason::Approval` | Executor is waiting for approval/rejection. |
| `confirmation` | `WaitReason::Confirmation` | Executor is waiting for continue/stop confirmation. |
| `sub_agent` | `WaitReason::SubAgent` | Executor is waiting for a delegated sub-agent to complete. |

## 4) Backend-to-Frontend Events

| Purpose | Canonical wire name | Backend type | Notes |
|---|---|---|---|
| Accepted top-level task boundary | `task_completed` | `WorkflowEventType::TaskCompleted` / `GatewayPayload::TaskCompleted` | Emitted after `complete_workflow` and any configured final review succeed. It is emitted even when queued input keeps the session running. |
| Session terminal completion | `state` with `completed` | `GatewayPayload::State` / `WorkflowEventType::WorkflowCompleted` | Session lifecycle authority; not a per-task display boundary. |

## Notes

- `user_message` is a **signal type**; `user_input` is a **wait reason**. They are intentionally different.
- If a protocol rename is required, update:
  - frontend constants in `src/composables/workflow/signalTypes.ts`
  - backend parser in `src-tauri/src/workflow/react/signals.rs`
  - any waiting-state validation in `src-tauri/src/workflow/react/types.rs`
