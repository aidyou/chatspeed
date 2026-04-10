# 阶段 0-7 审计报告

本文档记录截至当前代码状态，对 [work/plan.md](/home/xc/dev/rust/chatspeed-workflow-refactor/work/plan.md) 阶段 0-7 的对照审计结果，并作为后续修正的执行基线。

## 审计结论

当前实现已经覆盖了以下主干能力：

- `WorkflowManager` 已引入并注册为全局状态
- `ExecutionContext` / `workflow_snapshots` / `workflow_events` 已落库
- snapshot-first / replay-fallback 恢复链已初步建立
- dispatcher / sink 主链已接入 executor
- `child_task` 等待字段已经进入 `ExecutionContext`

本轮修正前，存在若干与计划文档不完全一致、且建议在进入下一阶段前优先处理的问题。以下条目保留为审计记录，并标注当前处理结果。

## 高优先级问题

### 1. 审批恢复仍主依赖消息反推

状态：已修复

现状：

- 前端在选中 workflow 时仍通过历史消息查找 `pendingApprovalMessage`
- orphan `awaiting_approval` 场景会直接使用消息内容重建审批弹窗

偏离计划：

- 阶段 2 明确要求 approval 恢复优先使用 snapshot 中的结构化 `pending_tools`
- `workflow_messages` 只允许作为临时 fallback，而不应继续作为主路径

影响：

- 结构化 snapshot 已存在，但前端仍然绑在旧消息语义上
- 后续推进 waiting/replay/child task 时，前端仍会继续依赖 transcript 细节

### 2. `workflow_signal` 恢复入口仍主要依赖旧状态字符串

状态：已修复

现状：

- orphan 恢复仍按 `awaiting_user` / `awaiting_approval` / `paused` 分支拼接
- 判定入口主要基于 `workflow.status` 字符串，而不是 `ExecutionContext.state + wait_reason`

偏离计划：

- 阶段 3 要求命令层与 executor 都基于统一等待模型
- 当前命令层和 engine 的 waiting 判定仍是双轨

影响：

- 新增等待类型时命令层仍需继续堆旧状态判断
- `child_task_complete` 之类的结构化等待恢复没有真正统一入口

## 中优先级问题

### 3. `WorkflowManager` 尚未成为完整的运行态真相源

状态：已修复

现状：

- 生产路径中主要只使用 `register_session(Active)` 和 `remove_session`
- `update_session_status`、`Waiting/Completed/Failed/Cancelled` 基本只在测试中被消费

偏离计划：

- 阶段 1 要求 manager 提供 session 只读状态查询和统一生命周期视图

影响：

- manager 目前更像 session registry，而不是完整生命周期投影
- 后续 handoff / 调度 / 观测会继续绕回 executor 实时状态

### 4. 阶段 7 验收测试不够闭环

状态：已修复

现状：

- child task 测试覆盖了 registry、字段序列化和快照持久化
- 但没有覆盖 completed / failed / cancelled 三分支的父子联动集成测试

偏离计划：

- 阶段 7 明确要求父任务等待、子任务三种结束路径收敛、重启后仍可恢复定位

影响：

- 当前实现虽然已有主链代码，但失败/取消路径缺乏验收级测试保护

## 低优先级问题

### 5. 阶段 6 观测指标还不算完全闭环

状态：已修复

现状：

- `Dispatcher` 已有总队列深度、失败数、平均延迟
- 但缺少每 sink 队列深度与 per-sink dropped 计数

偏离计划：

- 阶段 6 要求对 lag / dropped / queue depth 提供更完整证据

影响：

- 当前足以排查大多数问题，但距离阶段文档的“最小验收证据模板”还差一层

## 修正顺序

建议按以下顺序修正：

1. 审批恢复改为优先消费结构化 snapshot 数据
2. `workflow_signal` orphan 恢复改为优先基于 `ExecutionContext.state + wait_reason`
3. 将 `WorkflowManager` 状态更新接入主链
4. 补 child task completed / failed / cancelled 集成测试
5. 补齐 dispatcher 的 per-sink 队列/丢弃指标，并验证 reliable / best-effort 隔离

## 当前执行状态

- [x] 审计结论固化入文档
- [x] 修正审批恢复主路径
- [x] 修正 `workflow_signal` 恢复判定主路径
- [x] 收敛 `WorkflowManager` 状态更新
- [x] 补 child task 集成测试
- [x] 补齐 dispatcher 观测指标与投递隔离验证
