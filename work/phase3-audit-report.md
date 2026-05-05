# 阶段3审核报告

> 审核对象：`work/plan.md` 第 10 / 10A 节  
> 审核依据：
> - `work/phase3-test-scenarios.md`
> - `work/phase3-test-log.md`
> - 当前仓库实现代码
> 审核日期：2026-04-05

---

## 1. 审核结论

**阶段 3 已经完成了大部分核心目标，但当前还不建议正式进入下一阶段开发。**

原因不是 waiting 主链路失效，也不是 confirmation 逻辑缺失，而是还有一个明确的阶段边界问题未收口：

- `approval` 与 `request_confirm_broadcast` 这两类信号在命令层仍然没有稳定落入结构化 `WorkflowSignal` 主分发路径
- 从测试日志看，这两类信号进入命令层时仍被记录为 `type=unknown`
- 当前 approval 恢复成功，主要依赖 engine 内部的 legacy JSON 分支，而不是阶段 3 目标中的结构化 signal 主链路

这意味着：

- “waiting 统一主链路”已经基本成立
- “前端基于 `state + wait_reason` 驱动”已经基本成立
- 但“命令层统一把输入转换为结构化 signal，并作为内部主分发类型”还没有完全闭环

因此，本阶段的判断应为：

**接近完成，但仍有 1 个建议在进入阶段 4 前先补掉的收尾项。**

---

## 2. 总体评价

### 2.1 已完成的核心部分

从手工测试日志与代码审阅看，以下能力已经成立：

- waiting 已统一进入同一条主逻辑
- `wait_reason` 已成为 waiting 分类的主要依据
- `user_input` waiting 的正确 / 错误 signal 匹配已经成立
- `confirmation` waiting 的 `Continue` / 错误 signal 匹配在代码层已经成立
- `Stop` 在 waiting 态下已经可生效
- 前端已经开始基于 `state + wait_reason` 驱动 UI
- terminal 状态下重新发送消息的恢复链路已经补齐

### 2.2 尚未完全闭环的部分

仍未完全达标的点是：

- approval 类输入仍没有稳定转换成结构化 `WorkflowSignal`
- confirm broadcast 仍没有使用阶段 3 计划中的 `RebroadcastPending`

这两个问题都属于“阶段 3 内部主分发链路的最后收尾”，不属于新架构设计问题，但它们仍然影响阶段 3 的完成定义。

---

## 3. 逐项审核结果

## 3.1 目标 1：waiting 收敛为统一主链路

**结论：通过。**

### 证据

场景 1 日志已经明确出现：

- 进入 `awaiting_approval`
- snapshot 写入 `state=Waiting, wait_reason=Some(Approval)`
- `workflow.wait.enter`

对应证据来自 `work/phase3-test-log.md`：

- 场景 1：`executing -> awaiting_approval`
- 场景 1：`snapshot.write - state=Waiting, wait_reason=Some(Approval)`
- 场景 1：`[phase=wait][event=enter] Entering wait state, reason=Some(Approval)`

场景 5、场景 6、场景 7、场景 8、场景 9 也都能看到 waiting 主循环中的：

- `signal_received`
- `signal_rejected`
- `wait.enter`
- `wait.resume`

这说明阶段 3 的 waiting 主链路已经真正被跑通，不再只是文档目标。

---

## 3.2 目标 2：命令层统一解析 signal，并转换为结构化 `WorkflowSignal`

**结论：未完全通过。**

这是本次审核唯一的阻塞项。

### 证据 1：approval 在命令层仍被记为 `unknown`

`work/phase3-test-log.md` 场景 3 中：

```log
[Workflow][phase=signal] Signal received, type=unknown
...
Injecting signal, type=approval
...
Signal received, type=approval, wait_reason=Some(Approval)
```

这说明：

1. 前端发来的 approval JSON 到达了命令层
2. 命令层没有把它成功解析为结构化 `WorkflowSignal`
3. engine 最终是通过原始 JSON 分支处理掉了 approval

这与阶段 3 的目标不完全一致。

### 证据 2：confirm broadcast 在命令层仍被记为 `unknown`

`work/phase3-test-log.md` 场景 2 / 场景 3 中：

```log
[Workflow][phase=signal] Signal received, type=unknown
...
Injecting signal, type=request_confirm_broadcast
```

这说明：

- 前端仍在发 `request_confirm_broadcast`
- 命令层没有把它落到 `WorkflowSignal::RebroadcastPending`

### 代码侧确认

从当前实现看，`WorkflowSignal` 虽然已经存在，但 approval 前端 payload 仍采用旧格式，且普通 approve/reject 时并不总带齐结构化字段；同时 confirm broadcast 也仍在走旧名字。

因此，阶段 3 的“结构化 signal 成为内部主分发类型”只能算**部分完成**，不能算完全完成。

---

## 3.3 目标 3：executor 在 waiting 态下按 `wait_reason` 做 signal 类型匹配

**结论：通过。**

### approval waiting

场景 2：

- `user_message` 在 `Approval` waiting 下被明确拒绝
- 日志出现 `signal_rejected`

场景 3：

- approval 被正确接受并恢复执行

虽然 approval 本身仍经由 legacy JSON 分支处理，但从 waiting 行为上看：

- 错误 signal 没有错误恢复
- 正确 signal 能恢复

### user_input waiting

场景 4：

- `continue` 在 `UserInput` waiting 下被拒绝

场景 5：

- `user_message` 在 `UserInput` waiting 下被接受
- workflow 恢复并最终完成

### confirmation waiting

场景 6：

- `Continue` 在 `Confirmation` waiting 下被接受
- workflow 恢复执行

场景 7：

- `user_message` 在 `Confirmation` waiting 下被拒绝

此外，代码审阅也确认当前 `WorkflowSignal::is_valid_for` 中：

- `Continue -> Confirmation`
- `Stop -> all waiting`
- `UserMessage -> UserInput`
- `ApprovalDecision -> Approval`

结构已经符合阶段 3 目标。

---

## 3.4 目标 4：前端基于 `state + wait_reason` 驱动交互

**结论：基本通过。**

### 证据

`work/phase3-test-log.md` 场景 10 中已经看到：

- `executing -> awaiting_user | wait_reason: user_input | isWaiting: true`
- `executing -> awaiting_approval | wait_reason: approval | isWaiting: true`

当前前端代码也已体现出：

- `isWaiting` 统一判断 waiting 族状态
- `isAwaitingApproval` 优先看 `waitReason === 'approval'`
- `paused + confirmation` 会触发 confirmation dialog

### 说明

场景 10 的 confirmation 部分没有做完整手工触发，但代码审阅确认：

- `paused + wait_reason=confirmation` 会弹出 confirmation dialog
- dialog 发送的是 `continue` / `stop`
- 这部分与阶段 3 目标一致

因此本项可判为基本通过，但带有一个限定：

**confirmation UI 已经从代码上具备，但缺少一次真实步数耗尽触发的手工闭环证据。**

---

## 4. 手工测试场景审核

## 4.1 已有正向证据的场景

以下场景已有明确日志证据：

- 场景 1：approval 进入统一 waiting
- 场景 2：approval 拒绝错误 signal
- 场景 3：approval 接受正确 signal
- 场景 4：user_input 拒绝错误 signal
- 场景 5：user_input 接受正确 signal
- 场景 6：confirmation 接受 Continue
- 场景 7：confirmation 拒绝错误 signal
- 场景 8 / 9：两种 waiting 下 Stop 生效
- 场景 10：前端已基于 `state + wait_reason` 驱动 approval / user_input

### 备注

测试日志中的场景 8 / 场景 9 与测试文档中的命名顺序是对调的：

- 日志里的场景 8 实际覆盖的是 `UserInput waiting + Stop`
- 日志里的场景 9 实际覆盖的是 `Approval waiting + Stop`

但这不影响覆盖面，两个场景都已经被手工验证到。

---

## 4.2 confirmation 场景的人工测试缺口

### 当前情况

你已经明确说明：

- 场景 10 中 confirmation 需要通过“步数耗尽”触发
- 这一条本次没有做稳定手工验证

### 审核判断

这不会单独构成阶段 3 的阻塞项，原因如下：

1. 场景 6、场景 7 已经从日志层验证了 `Confirmation` waiting 的核心 signal 匹配
2. 当前代码中 `showConfirmationDialog()` 已明确发送：
   - `continue`
   - `stop`
3. engine 中 step budget exhausted 已明确进入：
   - `Paused`
   - `wait_reason = confirmation`

也就是说：

- confirmation waiting 的机制已经被手工验证
- 步数耗尽只是该 waiting 的一个触发入口

因此，这一项可以作为**补充建议**，但不应作为当前阶段唯一阻塞项。

---

## 5. 代码审阅补充结论

## 5.1 confirmation 代码审阅结果

**结论：通过。**

从当前代码看：

- step budget exhausted 不再私有 `recv()`
- 只会进入 `Paused`
- waiting 主循环统一处理 `Continue` / `Stop`
- 前端 confirmation dialog 发送的也是 `continue` / `stop`

这部分与此前整改文档一致，没有再走回专用广播或 approval 伪装。

因此，虽然没有做完整手工“步数耗尽弹框”闭环，但代码层实现方向是正确的。

## 5.2 terminal resume 代码审阅结果

**结论：通过。**

当前命令层 recovery 已兼容：

- `user_message`
- terminal / pending 状态下重新 `workflow_start`

前端分流也已经将 terminal 状态视为重新开一轮，而不是继续发 waiting signal。

这说明“取消 / 完成 / 出错后可继续发送消息”的补充整改已经到位。

---

## 6. 当前阻塞项

当前只剩 1 个建议在进入阶段 4 前完成的收尾问题。

### 阻塞项：approval / rebroadcast 仍未完全纳入结构化 signal 主链路

#### 表现

- `approval` 在命令层日志中仍是 `type=unknown`
- `request_confirm_broadcast` 在命令层日志中仍是 `type=unknown`

#### 实际影响

当前功能虽然可用，但存在两个问题：

1. 阶段 3 的“结构化 signal 成为内部主分发类型”还没有完全达成
2. 如果直接进入阶段 4，把事件审计建立在这套半结构化、半 legacy 的输入链路上，会把不一致固化到后续设计里

#### 建议修法

在进入阶段 4 前，补一个很小的收尾修复：

1. 前端 approval 普通批准 / 拒绝时，也发完整的结构化 payload
   - 确保命令层可直接解析为 `ApprovalDecision`
2. 前端把 `request_confirm_broadcast` 改为阶段 3 计划中的结构化名字
   - 例如直接改为 `rebroadcast_pending`
3. 命令层日志中不再出现这两类信号的 `type=unknown`

这项修复很小，但它关系到阶段 3 是否真正闭环。

---

## 7. 是否建议进入下一阶段

**当前结论：暂不建议直接进入阶段 4。**

### 原因

不是因为 phase 3 主体失败，而是因为：

- waiting 模型主体已经成功
- 但结构化 signal 主分发链路还差最后一步收口

如果现在直接进入阶段 4：

- 事件审计链会建立在不完全统一的输入路径上
- 阶段 3 的“统一 signal”目标会以“功能可用但内部不纯”的状态结项

这不是理想的阶段切换点。

### 更合理的判断

建议把当前阶段状态定义为：

**阶段 3：主体完成，剩余一个收尾修复项。**

只要把这个收尾修掉，就可以进入阶段 4。

---

## 8. 建议的收尾动作

进入下一阶段前，建议只做下面这一项，不再扩展新需求：

### 收尾项

把以下信号彻底纳入结构化 `WorkflowSignal` 主路径：

- approval
- rebroadcast pending

### 完成标准

补完后应满足：

- approval 在命令层日志中显示为结构化 signal 类型，而不是 `unknown`
- confirm rebroadcast 在命令层日志中显示为结构化 signal 类型，而不是 `unknown`
- engine waiting 分支中的 legacy JSON 特判只保留必要兼容，不再承担主路径职责

---

## 9. 最终判断

本次审核的最终判断如下：

- 阶段 3 的 waiting 主链路：通过
- 阶段 3 的 wait_reason 驱动：通过
- 阶段 3 的 confirmation 设计：通过
- 阶段 3 的 terminal resume 补充：通过
- 阶段 3 的结构化 signal 主分发闭环：**未完全通过**

因此：

**当前不建议直接关闭阶段 3，也不建议立即进入阶段 4。**

**建议先补齐 approval / rebroadcast 的结构化 signal 收尾，再正式判定阶段 3 完成。**

