# 阶段5-9自动化任务清单（执行版）

> 目标：将 `work/phase5-test-scenarios.md` 到 `work/phase9-test-scenarios.md` 中可自动化场景尽快落地。  
> 原则：自动化优先；仅保留必须手工验证的人机交互与体验项。

---

## 一、优先级定义

- `P0`：阻塞阶段验收，必须先完成
- `P1`：里程碑建议完成，提升稳定性
- `P2`：增强项，放在迭代尾部

工时预估口径：
- `S`：0.5 天
- `M`：1-2 天
- `L`：2-4 天

---

## 二、P0 任务（先做）

1. 阶段5：snapshot/replay 恢复主链单测套件
- 优先级：`P0`
- 预估：`L`
- 类型：Rust 集成/单元测试
- 覆盖：
  - snapshot hit（不进入 replay）
  - snapshot miss -> replay fallback
  - version mismatch -> replay fallback
  - replay failed -> 安全失败态
  - reducer 字段完整性（含 `last_event_id`）
- 建议落点：`src-tauri/src/workflow/react/`（新增 `replay` 相关测试模块）
- 验收命令：
  - `cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture`

2. 阶段6：dispatcher/sink 故障隔离测试
- 优先级：`P0`
- 预估：`L`
- 类型：Rust 单测 + mock sink
- 覆盖：
  - UI sink error 不影响 DB sink
  - DB sink 慢速不阻塞 executor 主链
  - completed/failed/cancelled 终态至少一次到 DB sink
  - lag/dropped 指标可读
- 建议落点：`src-tauri/src/workflow/react/dispatcher*.rs`（如尚无可先补测试友好抽象）

3. 阶段7：Call 父子任务状态机测试
- 优先级：`P0`
- 预估：`L`
- 类型：Rust 单测/集成测试
- 覆盖：
  - 父任务进入 waiting_on_task_id
  - 子任务完成触发父任务恢复
  - 子任务失败路径可解释收敛
  - 父任务等待期间“重启恢复”模拟
  - 取消链路无孤儿任务
- 建议落点：`src-tauri/src/workflow/react/orchestrator.rs` 相关测试模块

---

## 三、P1 任务（里程碑增强）

1. 阶段8：handoff 路由与恢复自动化
- 优先级：`P1`
- 预估：`L`
- 类型：Rust 单测/集成测试
- 覆盖：
  - handoff enter（focus 切换 + stack 入栈）
  - route_input 到焦点代理
  - 自动归还主控
  - 强制回主控
  - 重启恢复焦点代理

2. 阶段9：前端结构化状态消费测试
- 优先级：`P1`
- 预估：`M`
- 类型：前端单测（Vitest）+ store 测试
- 覆盖：
  - TaskItem/Inspector 字段映射
  - 多 session 切换一致性
  - 禁止 transcript 推断任务态
- 建议落点：`src/stores/workflow.js`、`src/components/workflow/*` 对应测试文件

3. 阶段9：UI 异常隔离集成测试
- 优先级：`P1`
- 预估：`M`
- 类型：前端集成测试（含 error boundary/降级）
- 覆盖：
  - 面板渲染异常时 UI 降级
  - 后端 workflow 执行不受影响（联调桩）

---

## 四、P2 任务（增强与治理）

1. 测试数据工厂统一化
- 优先级：`P2`
- 预估：`M`
- 类型：测试基础设施
- 内容：
  - session/snapshot/events 构造器
  - 常见状态链路 fixture（waiting/completed/cancelled/handoff/call）

2. 日志断言辅助工具
- 优先级：`P2`
- 预估：`S`
- 类型：测试工具
- 内容：
  - 统一关键字断言：`workflow.restore.*`、`workflow.replay.*`、`workflow.dispatch.*`、`workflow.handoff.*`

3. CI 分层执行
- 优先级：`P2`
- 预估：`S`
- 类型：CI 配置
- 内容：
  - PR 快速集：P0 冒烟
  - Nightly 全量：P0+P1+P2

---

## 五、保留手工测试（必要最小集）

1. 审批交互体验
- 同意/拒绝/重试的用户路径、提示文案、节奏体验

2. Handoff 期间真实 UI 输入体感
- 连续输入时焦点显示与实际路由是否一致

3. 复杂页面异常时用户可感知降级质量
- 错误提示是否清晰、是否可继续操作

4. 真机重启后的首屏恢复体验
- 加载态与最终态切换是否误导用户

---

## 六、建议执行顺序（两周版本）

1. 第1周：
- 完成阶段5 P0
- 完成阶段6 P0

2. 第2周：
- 完成阶段7 P0
- 完成阶段8 P1（核心链路）
- 完成阶段9 P1（结构化状态消费）

3. 迭代尾：
- P2 测试基础设施与 CI 分层

---

## 七、交付标准（自动化收口）

1. 每个阶段至少有一组稳定自动化用例覆盖“核心验证点”
2. `work/phaseX-test-scenarios.md` 的可自动化场景均有对应测试入口
3. 手工测试仅保留“体验与交互”类最小集合
4. 新增自动化在本地和 CI 可重复通过
