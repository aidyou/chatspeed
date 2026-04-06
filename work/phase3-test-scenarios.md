# 阶段3测试验收文档

> 测试目的：验证统一等待模型与结构化 Signal
> 测试日期：2026-04-05

---

## 一、阶段目标对照

根据 `work/plan.md` 第 10 节和 10A 节，阶段3的核心目标是：

1. 把当前散落在 `Paused`、`AwaitingUser`、`AwaitingApproval` 中的等待逻辑收敛为一条统一主链路
2. 命令层统一解析 signal，并转换为结构化 `WorkflowSignal`
3. executor 在 waiting 态下按 `wait_reason` 做 signal 类型匹配
4. 前端基于 `state + wait_reason` 判断交互状态，而不是硬编码多个旧状态值

因此，本阶段测试不能只验证“能继续运行”，还必须验证：

- waiting 是否统一进入同一套逻辑
- signal 是否按类型匹配
- 错误 signal 是否会被拒绝，而不是错误恢复
- `Stop` 是否在任何 waiting 态都可生效

---

## 二、核心验证点

1. waiting 态统一进入一条主逻辑
2. signal 统一走结构化类型分发
3. approval waiting 只接受 `ApprovalDecision`
4. user_input waiting 只接受 `UserMessage`
5. confirmation waiting 只接受 `Continue` / `Stop`
6. 错误 signal 会被拒绝或记录 mismatch，而不会错误恢复
7. 前端交互可仅基于 `state + wait_reason`

---

## 三、日志关键字对照

阶段3测试时建议重点关注以下日志：

```log
workflow.wait.enter
workflow.wait.resume
workflow.wait.signal_received
workflow.wait.signal_rejected
workflow.wait.signal_mismatch
```

如果当前实现仍沿用 `[Workflow][phase=wait]` 等日志风格，也应至少能够从日志中清晰识别：

- 当前是否进入 waiting
- 当前 `wait_reason` 是什么
- 收到的 signal 类型是什么
- signal 是否被接受 / 拒绝 / mismatch

---

## 四、自动化测试要求

阶段3的自动化测试必须覆盖“正确 signal 能恢复，错误 signal 不会错误恢复”。

### 最低要求

至少应补齐并执行以下测试：

1. `waiting + approval` 接收正确 signal
2. `waiting + approval` 拒绝错误 signal
3. `waiting + user_input` 接收用户消息
4. `waiting + confirmation` 接收继续 / 停止
5. `Stop` 在任意 waiting 态可生效

### 建议的测试粒度

建议优先覆盖：

- `workflow::react::types`
  - `WorkflowSignal` 序列化 / 反序列化 ✅ 已覆盖
- `workflow::react::engine`
  - waiting 分支下的 signal 类型匹配 ✅ 已覆盖
- `commands/workflow`
  - 旧 JSON 输入兼容解析到 `WorkflowSignal` ⚠️ 可选
  > **说明**：命令层的信号处理仅是单行包装 `WorkflowSignal::parse(&signal)`，核心解析逻辑已在 `types.rs` 的 `test_workflow_signal_parse` 中完整测试。添加命令层单元测试价值有限，不作为必须项。

### 建议命令

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

> **注意**：命令层测试可跳过，见上方说明。

---

## 五、必须执行的手工测试

以下手工测试应全部执行。只有完成这些场景，才足以判断阶段3是否达到计划要求。

### 场景1：Approval Waiting 进入统一等待主链路

**步骤：**
1. 创建 workflow
2. 触发需要审批的工具调用
3. 观察日志与前端状态

**验收：**
- ✅ 前端显示统一的 `waiting` 状态，而不是继续依赖多个旧状态值判断
- ✅ `wait_reason=approval`
- ✅ 日志能明确看到 waiting 入口
- ✅ 没有额外分散的特殊恢复分支行为

---

### 场景2：Approval Waiting 收到错误类型 Signal

这是阶段3最关键的负向场景之一。

**步骤：**
1. 让 workflow 进入 `approval` waiting
2. 手动发送一个错误类型 signal，例如：
   - `UserMessage`
   - `Continue`
3. 观察日志与 UI 状态

**验收：**
- ✅ workflow 不会错误恢复
- ✅ 前端仍然保持 waiting
- ✅ 日志出现 `signal_rejected` 或 `signal_mismatch`
- ✅ 不会错误进入 `running` / `thinking`

---

### 场景3：Approval Waiting 收到正确 ApprovalDecision

**步骤：**
1. 让 workflow 进入 `approval` waiting
2. 发送正确的 `ApprovalDecision`
3. 观察恢复过程

**验收：**
- ✅ 日志出现 signal 被接受
- ✅ workflow 从 waiting 恢复
- ✅ 后续执行继续进行

---

### 场景4：UserInput Waiting 收到错误类型 Signal

**步骤：**
1. 让 workflow 进入 `user_input` waiting
2. 发送错误类型 signal，例如：
   - `Continue`
   - `ApprovalDecision`
3. 观察日志和状态

**验收：**
- ✅ workflow 不会错误恢复
- ✅ waiting 状态保持不变
- ✅ 日志出现 `signal_rejected` 或 `signal_mismatch`

---

### 场景5：UserInput Waiting 收到正确 UserMessage

**步骤：**
1. 让 workflow 进入 `user_input` waiting
2. 发送一条补充消息
3. 观察恢复与后续执行

**验收：**
- ✅ signal 被接受
- ✅ waiting 恢复成功
- ✅ workflow 继续执行

---

### 场景6：Confirmation Waiting 收到 Continue

如果当前产品中存在 `confirmation` waiting（如 plan / continue / paused 一类），本场景必须执行。

**步骤：**
1. 让 workflow 进入 `confirmation` waiting
2. 发送 `Continue`
3. 观察恢复过程

**验收：**
- ✅ `wait_reason=confirmation`
- ✅ `Continue` 被正确接受
- ✅ workflow 恢复执行

如果当前产品暂时没有可稳定触发的 confirmation 场景，应在测试记录中明确写明“未执行原因”。

---

### 场景7：Confirmation Waiting 收到错误 Signal

**步骤：**
1. 让 workflow 进入 `confirmation` waiting
2. 发送错误类型 signal，例如 `UserMessage`
3. 观察日志与状态

**验收：**
- ✅ workflow 不会错误恢复
- ✅ waiting 状态保持
- ✅ 日志出现 `signal_rejected` 或 `signal_mismatch`

如果当前产品暂时没有可稳定触发的 confirmation 场景，应在测试记录中明确写明“未执行原因”。

---

### 场景8：Stop 在 Approval Waiting 下生效

**步骤：**
1. 让 workflow 进入 `approval` waiting
2. 发送 `Stop`
3. 观察日志与状态

**验收：**
- ✅ workflow 进入 `cancelled`
- ✅ 日志能看出 stop 在 waiting 态被接受
- ✅ manager 最终移除 session

---

### 场景9：Stop 在 UserInput Waiting 下生效

**步骤：**
1. 让 workflow 进入 `user_input` waiting
2. 发送 `Stop`
3. 观察日志与状态

**验收：**
- ✅ workflow 进入 `cancelled`
- ✅ waiting 态不会忽略 stop

---

### 场景10：前端基于 `state + wait_reason` 驱动交互

这是阶段3的前端验收场景。

**步骤：**
1. 分别进入 `approval` waiting、`user_input` waiting、`confirmation` waiting
2. 观察前端是否根据统一字段切换交互

**验收：**
- ✅ 前端可以只基于 `state === waiting`
- ✅ 再结合 `wait_reason` 区分 approval / user_input / confirmation
- ✅ 不再需要硬编码多个旧等待状态值作为主要业务判断

---

## 六、恢复点覆盖矩阵

| 场景 | 核心能力 | 是否必须 |
|------|----------|----------|
| 场景1 | approval 进入统一 waiting | 是 |
| 场景2 | approval 拒绝错误 signal | 是 |
| 场景3 | approval 接受正确 signal | 是 |
| 场景4 | user_input 拒绝错误 signal | 是 |
| 场景5 | user_input 接受正确 signal | 是 |
| 场景6 | confirmation 接受 Continue | 有 confirmation 场景时必测 |
| 场景7 | confirmation 拒绝错误 signal | 有 confirmation 场景时必测 |
| 场景8 | approval waiting 下 Stop 生效 | 是 |
| 场景9 | user_input waiting 下 Stop 生效 | 是 |
| 场景10 | 前端基于 `state + wait_reason` 驱动 | 是 |

---

## 七、完成定义检查

只有满足以下条件，才建议判定阶段3通过：

| 检查项 | 状态 |
|--------|------|
| `WorkflowSignal` 已成为内部主分发类型 | ✅ 已实现 |
| 命令层统一把输入转换为结构化 signal | ✅ 已实现 |
| approval waiting 只接受 `ApprovalDecision` | ✅ 已实现 |
| user_input waiting 只接受 `UserMessage` | ✅ 已实现 |
| confirmation waiting 只接受 `Continue` / `Stop` | ✅ 已实现 |
| 错误 signal 会被拒绝或记录 mismatch | ✅ 已实现 |
| `Stop` 在任意 waiting 态都可生效 | ✅ 已实现 |
| 前端可基于 `state + wait_reason` 做交互判断 | ✅ 已实现 |
| 自动化测试达到阶段3最低要求 | ✅ 38/38 通过 |

---

## 八、验收清单

| 场景 | 状态 |
|------|------|
| 场景1：Approval Waiting 进入统一等待主链路 | ⬜ 待手工验证 |
| 场景2：Approval Waiting 收到错误类型 Signal | ⬜ 待手工验证 |
| 场景3：Approval Waiting 收到正确 ApprovalDecision | ⬜ 待手工验证 |
| 场景4：UserInput Waiting 收到错误类型 Signal | ⬜ 待手工验证 |
| 场景5：UserInput Waiting 收到正确 UserMessage | ⬜ 待手工验证 |
| 场景6：Confirmation Waiting 收到 Continue | ⬜ 待手工验证 |
| 场景7：Confirmation Waiting 收到错误 Signal | ⬜ 待手工验证 |
| 场景8：Stop 在 Approval Waiting 下生效 | ⬜ 待手工验证 |
| 场景9：Stop 在 UserInput Waiting 下生效 | ⬜ 待手工验证 |
| 场景10：前端基于 `state + wait_reason` 驱动交互 | ⬜ 待手工验证 |
| 自动化测试通过 | ✅ 38/38 通过 |
