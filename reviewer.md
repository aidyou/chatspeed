你描述的现象很典型：**终审 gate 变成了全量审计，而且每轮都在重新扩展范围，导致父代理陷入“修复—再发现—再修复”的长链条。**  
我重点看了提示词里会放大“过严/漂移”的几处机制，以及如何在不降低核心把关能力的前提下收敛。

---

## 一、导致多轮审查的提示词放大器

### 1. 首轮要求“一次列出所有可发现的问题”，这本身就在制造多轮

提示词原文：

> On the first review, inspect the full relevant risk surface before deciding. Consolidate all currently discoverable blocking and major issues into one verdict; do not stop after finding the first defect.

这句话意图是避免挤牙膏，但实际效果很可能是：

- 终审会在首轮尽可能找出所有 `major`，包括边缘、低概率、非核心问题；
- 由于“任何 `major` 都必须拒绝”，父代理必须一轮内处理所有这些问题；
- 问题一多，父代理容易遗漏、修错或引入新问题，反而导致多轮。

**问题本质**：没有区分“必须现在修复的核心验收问题”和“可以记录但不阻塞的问题”。  
首轮不是应该尽量多报，而是应该报出足以阻止不完整/不安全交付的关键问题。

---

### 2. “major”的定义和 `Review focus` 列表共同导致审查维度过宽

`Review focus` 里列了：

> API, data model, configuration, migration, compatibility, security, performance, or UX risks.

这个列表本身没问题，但如果每个改动都要求检查所有维度，就会把简单改动审成全量审计。  
比如一个纯文案修改，也可能被套上 performance/UX/security 的检查，然后产生一个 `major`。

同时，`Approval rules` 说：

> If there is any `blocker` or `major` finding, `approved` must be `false`.

如果 `major` 的门槛不高，比如只是“这里最好加个缓存”“这个错误处理不够优雅”，就会持续拒绝，父代理被迫处理很多非阻塞问题。

**问题本质**：`major` 没有和“用户可见结果偏差/验收标准不满足”强绑定，容易变成“代码质量偏好”。

---

### 3. 状态/边界检查清单太模板化，对简单改动也要求全套验证

提示词要求：

> For stateful or boundary-sensitive changes, explicitly consider applicable success, failure, partial-failure, cleanup/rollback, concurrency/race, retry/idempotency, compatibility, and state-transition paths. Verify whether tests exercise the important boundaries rather than merely the happy path.

如果终审严格按这句话执行，任何一个修改都可能被要求检查：

- 失败路径
- 回滚路径
- 并发竞争
- 重试幂等
- 状态转换

即使该改动根本不涉及并发、不涉及状态机，也会因为“边界验证不足”而被打回。

**问题本质**：规则没有强制“仅适用于与 diff 相关的路径”，导致审查者为了满足清单而扩展范围。

---

### 4. Re-review 的新发现限制不够硬，容易形成“相邻行为类”链条

提示词说：

> A new blocking or major finding during re-review is appropriate only when it was introduced by the fixes, is in a directly adjacent behavior class that must be checked to validate the fixes, or would make the requested result unsafe.

其中：

> directly adjacent behavior class that must be checked to validate the fixes

这个条件非常容易被泛化。  
比如修复了一个函数 A，审查者可能认为 B、C、D 都“属于同一行为类”，于是每轮修复后都能找到新的“相邻问题”，导致审查持续扩散。

**问题本质**：缺少对“相邻”的严格操作化定义，导致 re-review 范围无法收敛。

---

### 5. 缺少“必须收敛”的机制，非阻塞问题也可能被反复追踪

提示词只定义了 severity 和 approval 规则，但没有说明：

- 如果连续两轮都没有新的 `blocker`，是否可以批准？
- 如果剩余问题只是 `minor`/`info`，是否必须继续？
- 是否有轮次上限或升级机制？

实际中，终审可能因为输出 `required_fixes` 不空而持续拒绝，即使核心验收已经满足。

**问题本质**：规则要求“required_fixes 必须完整”，但没有告诉终审何时应该停止，导致 minor 问题也被当成修复前置条件。

---

## 二、改进方向：从“全量审计”回到“验收 gate”

你的目标不是让终审变松，而是让终审聚焦于**本次任务是否完成、是否正确、是否安全**，把其他问题降级为记录，不再阻塞交付。

### 1. 首轮只抓核心阻塞，限制阻塞问题数量

建议把首轮规则改成：

- 首轮只列出与原始验收标准、核心正确性、安全性、数据完整性直接相关的 `blocker/major`；
- 其他问题一律记 `minor/info`，不作为拒绝理由；
- 如果 `blocker/major` 超过 N 个，说明审查范围可能失控，应缩小范围后再报。

例如：

> On first review, focus on blocking issues that directly violate the original acceptance criteria or make the delivered result incorrect, incomplete, unsafe, or uninspectable. You may note other risks as minor/info, but do not block on them. If the number of blocking findings exceeds 3–5, re-scope your review before filing.

这样父代理一轮内只需要处理少量核心问题，而不是面对一长串。

---

### 2. 收紧 `major` 定义，使其与用户可见结果强绑定

建议明确：

- `blocker`：交付结果错误、不完整、不安全、无法验证，或与原始请求明显不符；
- `major`：交付结果基本可用，但存在直接影响用户可见行为的缺陷，且该缺陷由本次改动引入；
- `minor/info`：代码质量、可维护性、性能优化建议、非本次引入的旧问题等。

例如：

> A `major` finding must be a defect introduced by the change that materially affects the requested result from the user’s perspective. Code quality preferences, non-blocking performance improvements, or pre-existing issues must be `minor` or `info`.

这样可以避免“这里最好用缓存”之类的偏好被打成 `major`。

---

### 3. 审查维度按风险裁剪，不要求全维度覆盖

把 `Review focus` 从“必须全部检查”改成“按本次变更类型选择相关维度”。

例如：

> From the list below, select only the dimensions that are relevant to the change and the risk it introduces. Do not perform a full audit across all dimensions for every change.

可以给一个简单的映射，比如：

- 纯文档/注释：不审查性能/安全；
- 简单函数修复：只审查正确性、边界；
- 新增 API：审查 API 设计、兼容性、错误处理；
- 涉及状态/事务：审查并发、幂等、回滚。

这样终审不会再因为“UX 没有明确验收标准”而卡住。

---

### 4. 状态/边界检查限定为“与 diff 相关的路径”

建议在状态/边界规则前加一个前置条件：

> For stateful or boundary-sensitive changes, **only consider the paths that are directly touched by the diff or directly required to validate the change.** Do not require exhaustive coverage of unrelated state transitions, concurrency scenarios, or rollback paths.

并明确：

> Static inspection or existing tests are sufficient for low-risk paths; do not require new tests for every boundary unless the boundary is central to the change.

---

### 5. 给 re-review 加硬收敛规则，防止“相邻行为类”扩散

建议把 re-review 新问题条件收窄为：

- 由本次修复直接引入；
- 或者会使交付结果不安全/数据丢失；
- 其他新发现只能记 `info`，不得阻塞。

同时删除或严格定义“directly adjacent behavior class”。  
例如：

> A new blocking finding during re-review is appropriate only if it was introduced by the fix or is a safety/data-loss issue directly caused by the fix. Other newly discovered issues, even if related, must be recorded as `info` and must not block approval.

这样每轮之后，审查范围会逐步收缩，而不是继续向外扩展。

---

### 6. 增加“两轮无新 blocker 即收敛”的机制

建议在 approval 规则后加一段：

> If, after two consecutive reviews, no new `blocker` has been identified and the prior `blocker/major` findings have been resolved, approve even if `minor`/`info` findings remain. Residual risks must be reported in the verdict, but they must not prolong the workflow.

或者更激进一点：

> After the third review round, if the only remaining findings are `minor`/`info`, approve and report them as residual risks.

这样可以从机制上避免无限轮次。

---

### 7. 验证要求与风险等级挂钩

当前规则：

> If required verification is missing or only claimed without evidence, `approved` must be `false`.

但“required verification”没有被定义，容易变成所有改动都要求新增测试、跑完整 CI。

建议改为：

> Required verification must be proportional to the risk introduced by the change. Low-risk changes may be verified by code reading, existing tests, or static inspection. High-risk changes require targeted tests or logs that exercise the changed behavior.

这样终审不会对简单改动要求一套完整验证。

---

### 8. 输出中增加 `review_scope`，让范围可见并防止漂移

虽然不是强制，但建议让终审在 `summary` 或额外字段中写明：

> 本次审查了哪些文件、哪些路径、哪些风险维度，以及未审查范围的说明。

这样父代理和系统可以判断终审是否漂移，必要时可以干预。

---

## 三、总结

你的终审现在的问题不是“标准太松”，而是：

1. **首轮被迫全量枚举**，导致一次抛出过多问题；
2. **`major` 门槛过低**，把质量偏好当阻塞项；
3. **审查维度与边界检查没有按风险裁剪**，造成范围扩张；
4. **re-review 缺少硬收敛**，新问题可以不断从“相邻行为类”中冒出来；
5. **没有明确的轮次停止机制**，minor/info 也被持续追踪。

核心改进方向是：

> 把终审从“全量审计员”拉回“验收 gate”：只对**原始验收、核心正确性、安全、数据完整性**行使否决权，其他问题记录但不阻塞，并且让 re-review 范围逐轮收缩，而不是逐轮扩大。

如果你愿意，我可以基于这些点，直接给你一版修改后的终审提示词草稿。
