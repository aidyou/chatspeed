# 阶段0手工测试场景

> 测试目的：验证统一日志能串起关键路径
> 测试日期：2025-04-03
> 测试前准备：启用 DEBUG 日志级别

---

## 测试前置条件

1. 启动应用（开发模式）
2. 确保日志输出到控制台或文件
3. 准备一个测试 agent

---

## 场景1：创建并执行普通对话

**步骤：**
1. 打开应用，创建新的 **workflow**
2. 输入一条简单消息（如 "你好"）
3. 等待响应完成

**预期日志序列：**
```
[Workflow][session=...][phase=create] Creating workflow for agent_id=...
[Workflow][session=...][phase=create] Workflow created successfully
[Workflow][session=...][phase=start] Starting workflow, agent_id=...
[Workflow][session=...][phase=gateway] Registering signal channel
[Workflow][session=...][phase=start] Executor registered to BACKGROUND_TASKS
[Workflow][session=...][phase=state] State transition: Pending -> Thinking
[Workflow][session=...][phase=state] State transition: Thinking -> Executing
[Workflow][session=...][phase=state] State transition: Executing -> Thinking
... (循环直到完成)
[Workflow][session=...][phase=state] State transition: Thinking -> Completed
```

**验收：** 能看到完整的创建→启动→状态转换日志链

---

## 场景2：进入审批后刷新页面

**步骤：**
1. 使用 Smart 或 Default 审批模式
2. 触发需要审批的工具调用（如写入文件）
3. 确认审批弹窗出现
4. **刷新页面**
5. 验证审批弹窗重新出现

**预期日志序列：**
```
... (正常执行直到审批)
[Workflow][session=...][phase=state] State transition: Thinking -> AwaitingApproval
[Workflow][session=...][phase=wait] Entering wait state, reason=approval
--- 页面刷新 ---
[Workflow][session=...][phase=signal] Signal received, type=request_confirm_broadcast
[Workflow][session=...][phase=signal] Signal injection failed: ...
[Workflow][session=...][phase=start] Starting workflow, agent_id=...
[Workflow][session=...][phase=init] Re-broadcasting pending approvals to UI
[Workflow][session=...][phase=gateway] Registering signal channel
```

**验收：**
- 能看到 `AwaitingApproval` 状态
- 刷新后能看到恢复日志
- 审批弹窗能重新显示

---

## 场景3：等待用户输入后发送补充消息

**步骤：**
1. 触发 agent 调用 `ask_user` 工具
2. 确认进入等待用户输入状态
3. 在输入框发送补充消息
4. 验证 workflow 恢复执行

**预期日志序列：**
```
[Workflow][session=...][phase=state] State transition: Thinking -> AwaitingUser
[Workflow][session=...][phase=wait] Entering wait state, reason=user_input
[Workflow][session=...][phase=wait] Signal received, type=user_input, wait_reason=user_input
[Workflow][session=...][phase=state] State transition: AwaitingUser -> Thinking
```

**验收：**
- 能看到 `AwaitingUser` 状态
- 能看到 `user_input` 信号接收
- 能看到状态恢复到 `Thinking`

---

## 场景4：在等待时发送 stop

**步骤：**
1. 触发需要审批的工具调用
2. 确认进入 `AwaitingApproval` 状态
3. 点击停止按钮
4. 验证 workflow 被取消

**预期日志序列：**
```
[Workflow][session=...][phase=state] State transition: Thinking -> AwaitingApproval
[Workflow][session=...][phase=wait] Entering wait state, reason=approval
[Workflow][session=...][phase=signal] Signal received, type=stop
[Workflow][session=...][phase=wait] Signal received, type=stop, wait_reason=approval
[Workflow][session=...][phase=state] State transition: AwaitingApproval -> Cancelled
[Workflow][session=...][phase=run_loop][event=cancelled] Workflow session was cancelled
```

**验收：**
- 能看到 `stop` 信号接收
- 能看到状态变为 `Cancelled`
- 能看到 `event=cancelled` 日志

---

## 场景5：活跃 workflow 时关闭窗口并重新打开

**步骤：**
1. 创建 workflow 并启动执行
2. 在执行过程中**关闭窗口**
3. 重新打开应用窗口
4. 发送一条消息或审批
5. 验证 workflow 仍在运行

**预期日志序列：**
```
--- 关闭窗口前 ---
[Workflow][session=...][phase=start] Starting workflow
[Workflow][session=...][phase=state] State transition: Pending -> Thinking
[Workflow][session=...][phase=state] State transition: Thinking -> Executing
--- 关闭窗口 ---
--- 重新打开并操作 ---
[Workflow][session=...][phase=signal] Signal received, type=...
[Workflow][session=...][phase=gateway] Injecting signal, type=...
[Workflow][session=...][phase=gateway] Signal injected successfully
[Workflow][session=...][phase=wait] Signal received, type=...
```

**验收：**
- 重新打开后，信号能成功注入
- workflow 继续执行
- 不出现 "No input channel" 错误

---
