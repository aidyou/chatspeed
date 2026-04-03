# 阶段 0 验收评审意见

> 评审日期：2026-04-03
> 评审依据：
> - `work/plan.md` 第 7 节「阶段 0：重构基线与观测补齐」
> - `work/phase0-test-log.md`
> - `work/phase0-test-scenarios.md`
> - `work/workflow-state-transitions.md`

---

## 1. 评审结论

阶段 0 的核心目标是“建立运行基线与补齐观测”，不是修复工作流架构本身。

基于当前测试日志，本阶段可以判定为：

- **基本达成阶段目标**
- **可以进入阶段 1**
- **但不能视为行为稳定，也不能视为问题已解决**

更准确地说，阶段 0 已经把关键链路、等待态和异常路径暴露出来，具备了继续推进重构的观测基础；但同时，测试日志也确认当前实现仍存在刷新、取消、信号通道和 UI 绑定方面的真实缺陷，这些问题应作为阶段 1 之后的重点处理对象。

---

## 2. 与计划目标的对照结论

### 2.1 阶段目标是否达成

`plan.md` 对阶段 0 的定义是：

> 建立当前系统的运行基线，把现有问题变成可复现、可记录、可比较的对象。

从这个目标出发，当前结果是 **达成** 的。原因如下：

- 关键运行链路已经可以通过日志串起来。
- waiting 相关状态和 signal 路径已经能被直接观察。
- `signal channel closed` 已不再是“模糊感知”的问题，而是有明确出现条件和上下文的可定位问题。
- 当前真正需要被保留的状态类型，已经可以从日志和状态流转文档中抽离出来。

### 2.2 阶段 0 验收标准是否通过

依据 `work/plan.md` 第 7.6 节：

| 验收标准 | 结论 | 说明 |
|---|---|---|
| 每个关键路径都能在日志中串起来 | 通过 | start、wait、signal、approval、stop、cancel 等链路都已出现 |
| `signal channel closed` 的出现路径可被完整定位 | 通过 | 测试日志已覆盖刷新后 channel closed 的典型路径 |
| 能明确确认哪些状态是真正需要保留的 | 通过 | `awaiting_user`、`awaiting_approval`、`cancelled` 等已得到验证 |

结论：**阶段 0 按验收标准可判定通过。**

---

## 3. 已达成项

### 3.1 手工场景覆盖基本完整

对照 `work/plan.md` 第 7.5 节，5 个必测场景均已在 `work/phase0-test-log.md` 中得到覆盖或部分覆盖：

| 必测场景 | 覆盖情况 | 证据 |
|---|---|---|
| 创建 workflow 并执行普通对话 | 已覆盖 | 场景 1 |
| 进入 `awaiting_approval` 后刷新页面 | 已覆盖 | 场景 2 ~ 3、场景 4 |
| 进入 `awaiting_user` 后发送补充消息 | 已覆盖 | 场景 2 ~ 3 |
| 在 waiting 时发送 stop | 已覆盖 | 场景 4 |
| 活跃 workflow 执行时关闭前端窗口并重新打开 | 已覆盖，但结果暴露问题 | 场景 5 |

### 3.2 日志链路已经具备排障价值

当前日志已经能够串起如下关键路径：

- `workflow start`
- `gateway register`
- `executor registered`
- `state transition`
- `entering wait state`
- `signal received`
- `signal injected successfully / failed`
- `cancelled`

这说明阶段 0 的“补观测”目标已经不是纸面完成，而是已经能支撑真实排障。

### 3.3 `signal channel closed` 已被成功复现和定位

测试日志中已经明确出现两类相关异常：

1. 刷新后重新启动 workflow，出现：
   - `Workflow error: General("Signal channel closed")`
2. refresh / cancel 后再进行 confirm broadcast，出现：
   - `Signal injection failed: Gateway error: channel closed, attempting recovery`

这满足了阶段 0 的关键验收目标之一：该问题现在是**可复现、可比较、可继续分析**的，而不再只是用户主观描述。

### 3.4 waiting 状态的结构化观察已经初步成立

从日志可直接看到：

- `awaiting_user`
- `awaiting_approval`
- `cancelled`
- `wait_reason=user_input`
- `wait_reason=approval`
- `signal_type=approval`
- `signal_type=stop`
- `signal_type=request_confirm_broadcast`

这说明后续阶段要做的“统一等待模型”和“统一 signal 入口”，已经有了可靠的现状基线。

---

## 4. 未完全达成项

阶段 0 虽然验收可通过，但并不意味着所有计划内交付都已收口。

### 4.1 日志格式在代码层已统一，测试日志展示受脱敏影响

`plan.md` 第 7.4 节建议统一格式为：

```text
[Workflow][session={session_id}][phase={phase}] message...
```

复核代码实现后，可以确认关键日志路径已经采用统一格式，例如：

- `commands/workflow.rs`
  - `phase=create`
  - `phase=start`
  - `phase=signal`
  - `phase=run_loop`
- `engine.rs`
  - `phase=wait`
  - `phase=state`
- `gateway.rs`
  - `phase=gateway`

测试日志中看起来像是：

- `session=***`
- 只有 `[Workflow]`
- 缺少部分结构字段

更大的原因不是代码没有统一，而是**日志记录器在输出测试日志时对敏感内容做了替换或裁剪**，导致展示结果不完全等同于原始日志模板。

因此，这一项的准确结论应调整为：

- **代码层的关键日志格式已经统一**
- **测试日志展示存在脱敏噪声，评审时不能仅凭脱敏后的文本判断格式未完成**

### 4.2 自动化测试仍未体现

`plan.md` 第 7.3 节要求：

> 若仓库已有测试基础，新增至少一个 workflow 恢复相关测试文件。

在当前提供材料中，没有看到新增测试文件或自动化测试执行结果。因此这一项目前应判定为：

- **未确认完成**

### 4.3 操作级日志仍不够完整

计划中明确要求补齐的路径包括：

- workflow create
- workflow start
- workflow resume
- workflow signal
- workflow stop
- approval request
- approval resume
- executor crash

从当前日志看：

- `create`、`start`、`signal`、等待态日志较清晰
- `stop`、`approval request`、`approval resume` 仍更多表现为信号层日志，而不是清晰的操作级事件日志
- `resume` 的语义也还不够统一

这不影响阶段 0 通过，但说明“观测补齐”仍有收口空间。

---

## 5. 关键发现与风险

### 5.1 已确认存在刷新后的信号通道生命周期问题

场景 2 ~ 4 已经表明：

- 页面刷新后，旧 channel 与新 UI 绑定之间存在脱节
- 某些情况下 workflow 还能继续走
- 某些情况下则会出现 `Signal channel closed`

这说明当前 session 生命周期与 UI 页面生命周期仍然耦合过深，和 `plan.md` 中的现状判断一致。

### 5.2 已确认存在取消后“后端继续跑、前端无输出”的严重异常

场景 5 是本次评审最重要的发现。

日志表明：

- session 处于 `cancelled`
- 后端仍然发起了模型请求
- 也拿到了 token usage
- 但前端没有任何内容输出

这不是单纯的日志问题，而是一个真实的行为异常，可能涉及：

- executor 生命周期管理不正确
- cancelled 状态下仍触发新的执行路径
- gateway sink 或 UI 订阅链路失效
- 前后端 session 绑定关系异常

该问题虽然不阻止阶段 0 验收通过，但必须被视为后续阶段的高优先级问题。

### 5.3 当前系统仍不能被视为“刷新可恢复”

从测试事实看：

- 刷新后“部分场景能继续”
- 但并非稳定恢复
- 且存在 channel closed、UI 不出内容、状态与行为不一致等异常

因此，不能把当前结果表述为“刷新恢复已完成”，只能表述为：

- **刷新相关问题已被成功复现和观测**

---

## 6. 对下一阶段的建议判定

### 6.1 是否允许进入阶段 1

**允许进入阶段 1。**

原因不是因为现有行为已经稳定，而是因为阶段 0 的目标本来就不是“修好恢复”，而是“把问题观察清楚”。

当前已经具备进入阶段 1 所需的条件：

- 关键链路可观测
- waiting 与 signal 路径可观察
- 核心异常可复现
- 状态保留对象已初步明确

### 6.2 进入阶段 1 时应显式继承的已知问题

建议把下面几项列为阶段 1 的已知输入，而不是遗忘在测试日志里：

1. 页面刷新后，signal channel 可能失效。
2. `request_confirm_broadcast` 在取消态或 channel closed 场景下存在异常分支。
3. 取消后重新发送消息或点击继续，后端可能执行但前端无输出。
4. 当前恢复能力仍依赖运行时对象和 UI 绑定，不具备真正的后端托管语义。

---

## 7. 建议补充的收口项

这些项不是阶段 0 的阻塞条件，但建议在进入阶段 1 前或阶段 1 过程中尽快补齐：

1. 为 `workflow stop`、`approval request`、`approval resume` 增加更明确的操作级日志。
2. 增加至少一个“waiting 恢复”相关自动化测试。
3. 在测试记录中显式标注每个场景对应 `plan.md` 的哪一条必测项。
4. 把场景 5 单独升级为已知缺陷条目，而不是仅放在测试日志末尾。

---

## 8. 最终评审意见

最终结论如下：

- **阶段 0 验收通过**
- **阶段目标已实现：运行基线和问题观测已建立**
- **当前系统行为仍不稳定，不能宣称刷新恢复或取消恢复已完成**
- **可以进入阶段 1，但必须把场景 5 以及 channel closed 问题作为后续高优先级输入**

如果需要一句适合放在阶段汇报里的结论，可以使用下面这句：

> 阶段 0 已完成“基线建立与观测补齐”的目标，关键链路可追踪，等待态与 signal 路径可观察，`signal channel closed` 与取消后 UI 无输出等核心问题已成功复现并定位；因此本阶段验收通过，可进入阶段 1，但不得将当前行为视为稳定恢复能力已完成。
