# 场景1
数据表已手工创建。

# 场景2 ~ 4
这里已经包含了`ask_user`的事件链。
```json
{
"workflow_events": [
	{
		"id" : 51,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "workflow_started",
		"event_version" : "1.0.0",
		"event_data" : "{\"agent_id\":\"0ppah4g8m0400\"}",
		"created_at" : "2026-04-06 07:02:18"
	},
	{
		"id" : 52,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"pending\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:02:18"
	},
	{
		"id" : 53,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:02:21"
	},
	{
		"id" : 54,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"awaiting_user\"}",
		"created_at" : "2026-04-06 07:02:21"
	},
	{
		"id" : 55,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "wait_entered",
		"event_version" : "1.0.0",
		"event_data" : "{\"pending_tools\":[],\"wait_reason\":\"user_input\"}",
		"created_at" : "2026-04-06 07:02:21"
	},
	{
		"id" : 56,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"awaiting_user\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:02:58"
	},
	{
		"id" : 57,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "user_input_received",
		"event_version" : "1.0.0",
		"event_data" : "{\"content\":\"非常棒，接下来调用 write_file 随便写点啥\"}",
		"created_at" : "2026-04-06 07:02:58"
	},
	{
		"id" : 58,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:03:01"
	},
	{
		"id" : 59,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"awaiting_approval\"}",
		"created_at" : "2026-04-06 07:03:01"
	},
	{
		"id" : 60,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "wait_entered",
		"event_version" : "1.0.0",
		"event_data" : "{\"pending_tools\":[{\"tool_call_id\":\"tool_3a63a684\",\"tool_name\":\"write_file\"}],\"wait_reason\":\"approval\"}",
		"created_at" : "2026-04-06 07:03:01"
	},
	{
		"id" : 61,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "approval_requested",
		"event_version" : "1.0.0",
		"event_data" : "{\"tool_call_id\":\"tool_3a63a684\",\"tool_name\":\"write_file\"}",
		"created_at" : "2026-04-06 07:03:01"
	},
	{
		"id" : 62,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "approval_resolved",
		"event_version" : "1.0.0",
		"event_data" : "{\"approve_all\":false,\"approved\":true,\"tool_call_id\":\"tool_3a63a684\"}",
		"created_at" : "2026-04-06 07:05:56"
	},
	{
		"id" : 63,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"awaiting_approval\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:05:56"
	},
	{
		"id" : 64,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:05:59"
	},
	{
		"id" : 65,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:05:59"
	},
	{
		"id" : 66,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:02"
	},
	{
		"id" : 67,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:03"
	},
	{
		"id" : 68,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:05"
	},
	{
		"id" : 69,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:05"
	},
	{
		"id" : 70,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:12"
	},
	{
		"id" : 71,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:12"
	},
	{
		"id" : 72,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:16"
	},
	{
		"id" : 73,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:16"
	},
	{
		"id" : 74,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:22"
	},
	{
		"id" : 75,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:22"
	},
	{
		"id" : 76,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:27"
	},
	{
		"id" : 77,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:27"
	},
	{
		"id" : 78,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:32"
	},
	{
		"id" : 79,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:06:32"
	},
	{
		"id" : 80,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:06:37"
	},
	{
		"id" : 81,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"cancelled\"}",
		"created_at" : "2026-04-06 07:06:37"
	},
	{
		"id" : 82,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "workflow_cancelled",
		"event_version" : "1.0.0",
		"event_data" : "{}",
		"created_at" : "2026-04-06 07:06:37"
	},
	{
		"id" : 83,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"cancelled\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:12:45"
	},
	{
		"id" : 84,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:12:53"
	},
	{
		"id" : 85,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"awaiting_approval\"}",
		"created_at" : "2026-04-06 07:12:53"
	},
	{
		"id" : 86,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "wait_entered",
		"event_version" : "1.0.0",
		"event_data" : "{\"pending_tools\":[{\"tool_call_id\":\"tool_459f3319\",\"tool_name\":\"edit_file\"}],\"wait_reason\":\"approval\"}",
		"created_at" : "2026-04-06 07:12:53"
	},
	{
		"id" : 87,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "approval_requested",
		"event_version" : "1.0.0",
		"event_data" : "{\"tool_call_id\":\"tool_459f3319\",\"tool_name\":\"edit_file\"}",
		"created_at" : "2026-04-06 07:12:53"
	},
	{
		"id" : 88,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "approval_resolved",
		"event_version" : "1.0.0",
		"event_data" : "{\"approve_all\":false,\"approved\":false,\"tool_call_id\":\"tool_459f3319\"}",
		"created_at" : "2026-04-06 07:13:11"
	},
	{
		"id" : 89,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"awaiting_approval\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:13:11"
	},
	{
		"id" : 90,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:13:17"
	},
	{
		"id" : 91,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"cancelled\"}",
		"created_at" : "2026-04-06 07:13:17"
	},
	{
		"id" : 92,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "workflow_cancelled",
		"event_version" : "1.0.0",
		"event_data" : "{}",
		"created_at" : "2026-04-06 07:13:17"
	},
	{
		"id" : 93,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"cancelled\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:15:31"
	},
	{
		"id" : 94,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:15:44"
	},
	{
		"id" : 95,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:15:44"
	},
	{
		"id" : 96,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:15:49"
	},
	{
		"id" : 97,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:15:49"
	},
	{
		"id" : 98,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:15:55"
	},
	{
		"id" : 99,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:15:55"
	},
	{
		"id" : 100,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:16:02"
	},
	{
		"id" : 101,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:16:02"
	},
	{
		"id" : 102,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:16:08"
	},
	{
		"id" : 103,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:16:08"
	},
	{
		"id" : 104,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:16:16"
	},
	{
		"id" : 105,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"thinking\"}",
		"created_at" : "2026-04-06 07:16:16"
	},
	{
		"id" : 106,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"thinking\",\"to_state\":\"executing\"}",
		"created_at" : "2026-04-06 07:16:25"
	},
	{
		"id" : 107,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "state_changed",
		"event_version" : "1.0.0",
		"event_data" : "{\"from_state\":\"executing\",\"to_state\":\"completed\"}",
		"created_at" : "2026-04-06 07:16:25"
	},
	{
		"id" : 108,
		"session_id" : "0q06cfkxc0400",
		"event_type" : "workflow_completed",
		"event_version" : "1.0.0",
		"event_data" : "{\"summary\":null}",
		"created_at" : "2026-04-06 07:16:25"
	}
]}

```

## 阶段4收尾补充（自动化）

### 场景1：Event Store migration 验证（自动化）

- 新增测试：`test_workflow_events_table_and_index_exist_after_migration`
- 位置：`src-tauri/src/db/workflow.rs`
- 断言：
  - `workflow_events` 表存在
  - `idx_workflow_events_session_id_id` 索引存在
- 执行：
  - `cargo test --manifest-path src-tauri/Cargo.toml test_workflow_events_table_and_index_exist_after_migration -- --nocapture`
- 结果：通过

### 场景5：last_event_id 对齐验证（自动化）

- 新增测试：`test_snapshot_last_event_id_aligns_with_event_tail_for_key_states`
- 位置：`src-tauri/src/db/workflow.rs`
- 覆盖：
  - waiting/completed/cancelled 三种 snapshot 状态
  - `snapshot.context_json.last_event_id == workflow_events MAX(id)`
- 执行：
  - `cargo test --manifest-path src-tauri/Cargo.toml test_snapshot_last_event_id_aligns_with_event_tail_for_key_states -- --nocapture`
- 结果：通过

### 场景6：append 失败可见性（自动化）

- 新增测试：`test_append_workflow_event_returns_error_when_table_missing`
- 位置：`src-tauri/src/db/workflow.rs`
- 方法：
  - 人为 `DROP TABLE workflow_events`
  - 调用 `append_workflow_event`
  - 断言返回 `Err`（非 silent fail）
- 执行：
  - `cargo test --manifest-path src-tauri/Cargo.toml test_append_workflow_event_returns_error_when_table_missing -- --nocapture`
- 结果：通过

### 日志关键字对齐

- 将引擎与拦截器中的 append 失败日志统一为：
  - `workflow.event.append_failed`
- snapshot 保存日志增加：
  - `last_event_id`

# 场景5
## waiting snapshots
```log
15:53:55.566 [I] src/workflow/react/engine.rs:3439 [Workflow][session=***][phase=snapshot] Saved: state=Waiting, wait_reason=Some(Approval), pending_tools=1, last_event_id=Some(148)
15:53:55.567 [I] src/db/workflow.rs:494 [Workflow][session=***] event.append - type=ApprovalRequested, event_id=149
15:53:55.618 [I] src/workflow/react/engine.rs:977 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Approval)

15:55:24.998 [I] src/workflow/react/engine.rs:2092 WorkflowExecutor 0q06rd4br0400: User cancelled operation: 操作已被用户取消
15:55:24.998 [I] src/workflow/react/engine.rs:2757 [Workflow][session=***][phase=state] State transition: cancelled -> cancelled
15:55:25.000 [I] src/db/workflow.rs:443 [Workflow][session=***] snapshot.write - state=Cancelled, wait_reason=None, pending_tools=0
15:55:25.000 [I] src/workflow/react/engine.rs:3439 [Workflow][session=***][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=0, last_event_id=Some(156)

15:56:48.430 [I] src/db/workflow.rs:443 [Workflow][session=***] snapshot.write - state=Completed, wait_reason=None, pending_tools=0
15:56:48.431 [I] src/workflow/react/engine.rs:3439 [Workflow][session=***][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0, last_event_id=Some(160)
```
