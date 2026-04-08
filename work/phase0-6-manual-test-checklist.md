# 阶段0-6手工测试清单

> 目的：验证阶段0-6（基线观测到事件分发）的正确性
> 状态：等待手工测试

## 测试环境要求

- 应用已编译并运行
- 日志级别建议设置为 `debug` 或 `trace`
- 数据库可访问（用于验证持久化）

---

## 阶段0：基线与观测测试

### 测试场景1：创建 workflow 并执行普通对话
**步骤：**
1. 启动应用
2. 创建新的 workflow
3. 发送一条普通消息（如"你好"）

**预期日志输出：**
```
[Workflow][session=xxx][phase=create] workflow create
[Workflow][session=xxx][phase=start] workflow start
[Workflow][session=xxx][phase=running] message processing...
[Workflow][session=xxx][phase=completed] workflow completed
```

**验收标准：**
- [ ] 能看到统一的 `[Workflow][session=xxx]` 日志前缀
- [ ] 关键字段（session_id, state, signal_type）已记录

---

### 测试场景2：进入 awaiting_approval 后刷新页面
**步骤：**
1. 创建一个需要工具调用的 workflow（如文件操作）
2. 等待 approval 弹窗出现
3. 刷新页面
4. 观察 approval 状态是否恢复

**预期日志输出：**
```
[Workflow][session=xxx][phase=waiting] entering awaiting_approval
[Workflow][session=xxx][state=waiting][wait_reason=approval] waiting for approval
# 刷新后
[Workflow][session=xxx][phase=resume] restoring session
[Workflow][session=xxx][state=waiting][wait_reason=approval] approval restored
```

**验收标准：**
- [ ] waiting 状态被结构化保存（不依赖消息文本反推）
- [ ] 刷新后能正确恢复 approval 状态

---

### 测试场景3：进入 awaiting_user 后发送补充消息
**步骤：**
1. 创建 workflow
2. 触发等待用户输入的状态
3. 发送补充消息
4. 观察 workflow 是否恢复执行

**验收标准：**
- [ ] 用户输入能正确触发 workflow 恢复
- [ ] 日志中能看到 signal 类型和状态转换

---

### 测试场景4：在 waiting 时发送 stop
**步骤：**
1. 创建 workflow
2. 进入 waiting 状态（approval 或 user_input）
3. 发送 stop 信号

**预期日志输出：**
```
[Workflow][session=xxx][signal_type=stop] workflow signal received
[Workflow][session=xxx][state=cancelled] workflow stopped
```

**验收标准：**
- [ ] Stop 信号在任何 waiting 态都能生效
- [ ] workflow 正确进入 cancelled 状态

---

### 测试场景5：活跃 workflow 执行时关闭前端窗口并重新打开
**步骤：**
1. 启动 workflow 执行
2. 关闭前端窗口
3. 重新打开前端
4. 检查 workflow 是否仍在运行

**验收标准：**
- [ ] session 不依赖 UI 页面存活
- [ ] 重新打开页面后能看到活跃 session

---

## 阶段1：WorkflowManager 测试

### 测试场景6：Session 生命周期管理
**步骤：**
1. 启动 workflow
2. 检查日志中是否有 `session_registered`
3. 结束 workflow
4. 检查日志中是否有 `session_removed`

**预期日志输出：**
```
[Workflow][session=xxx] workflow_manager.session_registered
[Workflow][session=xxx] workflow_manager.session_removed
```

**验收标准：**
- [ ] 所有活跃 session 都注册在 `WorkflowManager`
- [ ] session 生命周期不再由命令层零散管理

---

### 测试场景7：Signal 路由测试
**步骤：**
1. 启动 workflow
2. 进入 waiting 状态
3. 发送 signal（approval 或用户输入）
4. 检查日志

**预期日志输出：**
```
[Workflow][session=xxx] workflow_manager.session_lookup_hit
[Workflow][session=xxx] workflow_manager.signal_routed
```

**验收标准：**
- [ ] `workflow_signal` 优先通过 `WorkflowManager` 查找和路由
- [ ] 日志能看出每个 session 的注册、更新、移除

---

## 阶段2：Snapshot 与 ExecutionContext 测试

### 测试场景8：Approval 恢复不依赖消息反推
**步骤：**
1. 创建 workflow
2. 触发需要审批的工具调用
3. 在 approval 弹窗出现时关闭页面
4. 重新打开页面
5. 调用初始化接口

**验收标准：**
- [ ] approval 信息来自 `workflow_snapshots` 表，不来自 `workflow_messages`
- [ ] `workflow.snapshot.read` 日志出现

---

### 测试场景9：应用重启后 waiting 状态恢复
**步骤：**
1. 创建 workflow
2. 进入 waiting 状态
3. 完全关闭应用
4. 重新启动应用
5. 重新打开 workflow

**验收标准：**
- [ ] waiting 状态可在应用重启后恢复
- [ ] `ExecutionContext` 成为恢复的主要结构

---

## 阶段3：统一等待模型与结构化 Signal 测试

### 测试场景10：结构化 Signal 类型匹配
**步骤：**
1. 让 workflow 进入 `approval` waiting
2. 发送错误类型的 signal（如 `Continue` 而不是 `ApprovalDecision`）
3. 验证 workflow 不会错误恢复
4. 发送正确的 `ApprovalDecision`
5. 验证恢复成功

**预期日志输出：**
```
[Workflow][session=xxx] workflow.wait.signal_mismatch
[Workflow][session=xxx] workflow.wait.signal_rejected
# 正确 signal 后
[Workflow][session=xxx] workflow.wait.resume
```

**验收标准：**
- [ ] 错误类型的 signal 被拒绝
- [ ] 正确类型的 signal 触发恢复

---

## 阶段4：Event Store 测试

### 测试场景11：关键事件序列记录
**步骤：**
1. 执行一个完整的 workflow
2. 查询数据库中的 `workflow_events` 表

**SQL 查询：**
```sql
SELECT event_type, event_version, created_at 
FROM workflow_events 
WHERE session_id = 'your-session-id'
ORDER BY id;
```

**预期结果：**
- workflow_started
- state_changed
- wait_entered (如果有 waiting)
- approval_requested (如果有 approval)
- approval_resolved (如果有 approval)
- workflow_completed / workflow_failed / workflow_cancelled

**验收标准：**
- [ ] 任意 session 都能查到结构化关键事件链
- [ ] `workflow.event.append` 日志出现

---

## 阶段5：Snapshot + Event Replay 测试

### 测试场景12：Snapshot 优先恢复
**步骤：**
1. 创建 workflow 并进入 waiting 状态
2. 确认 snapshot 已写入
3. 刷新页面

**预期日志输出：**
```
[Workflow][session=xxx] workflow.restore.snapshot_hit
```

**验收标准：**
- [ ] snapshot 存在时优先从 snapshot 恢复
- [ ] 不进入 event replay 流程

---

### 测试场景13：Snapshot 缺失时的 Replay Fallback
**步骤：**
1. 手动删除 `workflow_snapshots` 表中某 session 的记录
2. 刷新该 workflow 页面
3. 观察恢复过程

**预期日志输出：**
```
[Workflow][session=xxx] workflow.restore.snapshot_miss
[Workflow][session=xxx] workflow.restore.replay_fallback
[Workflow][session=xxx] workflow.replay.start
[Workflow][session=xxx] workflow.replay.done
```

**验收标准：**
- [ ] snapshot 缺失时自动切换到 replay 恢复
- [ ] 能正确重建 waiting 状态

---

## 阶段6：Dispatcher 与多 Sink 测试

### 测试场景14：UI Sink 失败不影响 DB Sink
**步骤：**
1. 启动 workflow
2. 模拟 UI 连接中断（如关闭浏览器标签页）
3. 检查数据库中的事件记录

**验收标准：**
- [ ] UI sink 故障时，DB sink 仍持续写入事件/快照
- [ ] `workflow.dispatch` 相关日志显示事件被分发到所有 sinks

---

### 测试场景15：DB Sink 延迟不阻塞 Executor
**步骤：**
1. 启动一个长时间运行的 workflow
2. 观察日志中的 lag 警告
3. 确认 workflow 仍能正常完成

**预期日志输出：**
```
[Dispatcher] event lag detected - elapsed=Xms
```

**验收标准：**
- [ ] DB sink 变慢不阻断 executor 完成
- [ ] 最终状态事件（completed/failed/cancelled）不丢失

---

## 综合验收检查清单

| 验收项 | 状态 |
|--------|------|
| 所有关键路径都能在日志中串起来 | ⬜ |
| `signal channel closed` 的出现路径可被完整定位 | ⬜ |
| 能明确确认哪些状态是真正需要保留的 | ⬜ |
| `WorkflowManager` 接管 session 生命周期 | ⬜ |
| session 不依赖 UI 页面存活 | ⬜ |
| waiting 状态被结构化保存 | ⬜ |
| approval 恢复不再依赖消息反推 | ⬜ |
| 用户输入和 approval 通过统一 signal 入口流转 | ⬜ |
| 前端等待态逻辑可以基于 `state + wait_reason` 工作 | ⬜ |
| 任意 session 都能查到结构化关键事件链 | ⬜ |
| snapshot 缺失时可由 replay 重建 waiting 状态 | ⬜ |
| UI 中断不影响 DB 恢复数据 | ⬜ |
| DB 延迟不会直接拖死 executor | ⬜ |

---

## 手工测试完成确认

**测试日期：** ___________

**测试人员：** ___________

**测试结果：** ⬜ 通过 / ⬜ 未通过

**备注：**
_____________________________________
_____________________________________

**签名：** ___________

---

> ⚠️ **重要提示**：
> 1. 手工测试通过后方可进入阶段7开发
> 2. 请详细记录任何异常日志或行为
> 3. 如有问题，请提供日志片段以便定位
