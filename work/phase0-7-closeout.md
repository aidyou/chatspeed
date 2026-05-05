# 阶段 0-7 收尾说明

本文档作为阶段 0-7 的正式收尾记录，对应 [work/plan.md](/home/xc/dev/rust/chatspeed-workflow-refactor/work/plan.md) 第 18 节交付模板。

## 18.1 代码变更说明

- [src-tauri/src/commands/workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/commands/workflow.rs)
  - 收敛 snapshot 恢复入口
  - 将 orphan signal 恢复统一到 `ExecutionContext.state + wait_reason`
  - 将 `WorkflowManager` 状态同步接入运行态主链
- [src-tauri/src/workflow/react/child_tasks.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/child_tasks.rs)
  - 补齐子任务完成结果的结构化收敛逻辑
- [src-tauri/src/workflow/react/engine.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/engine.rs)
  - 接入子任务 `completed / failed / cancelled` 三分支父任务恢复
- [src-tauri/src/workflow/react/dispatcher.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/dispatcher.rs)
  - 将 sink 投递隔离为独立 worker 队列
  - 增加 per-sink 队列深度、丢弃数、平均延迟指标
- [src-tauri/src/workflow/react/sinks.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/sinks.rs)
  - 引入 `SinkDeliveryGuarantee`
  - 明确 `DBSink` 为 `Reliable`
- [src-tauri/src/workflow/react/child_tasks_tests.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/child_tasks_tests.rs)
  - 补齐 child-task 父子联动测试
- [src/composables/workflow/useWorkflowCore.ts](/home/xc/dev/rust/chatspeed-workflow-refactor/src/composables/workflow/useWorkflowCore.ts)
  - 审批恢复改为优先消费结构化 snapshot 数据
- [src/stores/workflow.js](/home/xc/dev/rust/chatspeed-workflow-refactor/src/stores/workflow.js)
  - 增加 `executionContext` 归一化与结构化审批请求抽取
- [work/phase0-7-audit-report.md](/home/xc/dev/rust/chatspeed-workflow-refactor/work/phase0-7-audit-report.md)
  - 固化审计发现与修复状态

## 18.2 数据结构变化

- 新增后端恢复辅助逻辑：
  - `legacy_wait_reason_from_status`
  - `restore_context_for_signal`
  - `is_resumable_from_context_for_user_message`
- 新增子任务收敛结构：
  - `ChildTaskResolution`
- 新增 sink 投递语义枚举：
  - `SinkDeliveryGuarantee`
- 扩展 dispatcher 配置：
  - `sink_channel_capacity`
- 扩展 dispatcher 指标：
  - per-sink `queue_depth`
  - per-sink `dropped`

## 18.3 兼容与迁移说明

- 保留 legacy fallback：
  - `workflow_signal` 在 `ExecutionContext` 不可恢复时，仍允许从旧 `workflow.status` 推导 `wait_reason`
  - 前端审批恢复在结构化 `pendingTools` 缺失时，仍可退回旧 `workflow_messages` 路径
- fallback 触发时机：
  - 旧 snapshot 或历史数据未包含完整 `executionContext`
  - 历史消息已存在但结构化审批字段暂未落齐

## 18.4 可观测性说明

- 新增日志关注点：
  - orphan signal 恢复时输出 `runtime_state`、`wait_reason`、`status`
  - child-task 完成恢复路径日志
  - dispatcher 的 event lag / sink lag / sink queue full 日志
- 新增指标：
  - dispatcher 总队列深度
  - dispatcher per-sink 队列深度
  - dispatcher per-sink dropped / processed / failed / avg_latency

## 18.5 测试说明

- 已覆盖的自动化测试：
  - snapshot restore hit
  - orphan user-message 恢复判定
  - legacy status 到 `wait_reason` 的兼容映射
  - child-task `completed / failed / cancelled` 三分支恢复
  - dispatcher 的 best-effort 丢弃与 reliable sink 隔离
- 建议手工测试：
  - 进入 `awaiting_approval` 后刷新页面，确认审批弹窗可恢复
  - 进入 `awaiting_user` 后刷新页面，再发送补充消息，确认可恢复
  - 触发 child-task，确认父任务进入 waiting，并在子任务完成后恢复
  - 观察 dispatcher 慢 sink 场景下 UI 不阻塞、DB sink 不丢终态

## 18.6 风险说明

- 当前阶段已知风险：
  - 仍存在一些历史 `dead_code` / `unused` warning，未纳入本轮 0-7 偏离项修复范围
  - 前端 phase 9 任务账本/Inspector 仍未完全从 transcript 扫描迁出
- 下一阶段前置条件：
  - 以当前提交点作为 phase 9 起点
  - phase 9 仅做结构化任务视图与 UI 收敛，不回写执行内核语义
