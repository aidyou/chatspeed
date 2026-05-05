# 阶段6验收报告：内存分发总线与多 Sink

> 验收时间：2026-04-07  
> 对应计划：`work/plan.md` 第13节、13A节  
> 验收范围：`dispatcher.rs`、`sinks.rs`、`engine.rs`、`gateway.rs` 及前端联动路径

---

## 1. 验收结论

阶段6 **验收通过（Pass）**，可进入阶段7开发。

本次结论基于两类证据：

1. 自动化测试证据（覆盖 13A.4 三项最低要求）
2. 手工回归证据（你本轮阶段6联调与稳定性测试）

---

## 2. 目标对照（Plan 13 / 13A）

### 2.1 关键约束对照（13.2）

- DB写入不依赖不可靠广播语义：**满足**
- UI sink可容忍中间态丢失但终态不可丢失：**满足**
- executor不被慢sink长时间阻塞：**满足**

### 2.2 主链接入判据（13.7.1）

- `engine -> dispatcher -> sinks` 运行主链已接入：**满足**
- UI sink故障不影响DB持续写入：**满足**
- DB sink慢速不产生级联阻塞：**满足**

### 2.3 自动化测试最低要求（13A.4）

- UI sink失败不影响DB sink：**通过**
- DB sink变慢不阻断executor完成：**通过**
- 最终状态事件不丢失：**通过**

---

## 3. 自动化验收证据（13.7.3 最小模板）

以下为本次执行命令与关键输出摘要。

### 3.1 证据A：UI sink报错 + DB链路仍可继续

执行：

```bash
cd src-tauri
cargo test workflow::react::dispatcher::tests::test_p0_ui_sink_failure_doesnt_affect_db_sink -- --nocapture
```

关键输出：

- `[Dispatcher] sink 'ui' failed: General error: mock failure`
- `test ...test_p0_ui_sink_failure_doesnt_affect_db_sink ... ok`
- `test result: ok. 1 passed; 0 failed`

结论：UI sink 故障被隔离，测试断言通过。

### 3.2 证据B：DB sink注入慢速 + 分发不阻塞

执行：

```bash
cd src-tauri
cargo test workflow::react::dispatcher::tests::test_p0_slow_db_sink_doesnt_block_executor -- --nocapture
```

关键输出：

- `test ...test_p0_slow_db_sink_doesnt_block_executor ... ok`
- `test result: ok. 1 passed; 0 failed`

结论：慢DB sink场景下，分发路径断言通过（不阻断主流程）。

### 3.3 证据C：终态事件进入DB的断言

执行：

```bash
cd src-tauri
cargo test workflow::react::dispatcher::tests::test_p0_terminal_events_reach_db_sink -- --nocapture
```

关键输出：

- `test ...test_p0_terminal_events_reach_db_sink ... ok`
- `test result: ok. 1 passed; 0 failed`

结论：终态事件（completed/failed/cancelled）进入DB的断言通过。

---

## 4. 手工回归验收（本轮）

结合本轮联调结果，已覆盖并确认：

- 刷新页面/重启后状态恢复稳定（阶段6核心痛点已显著改善）
- stop与继续路径体感恢复到可用，且不再出现明显错误串扰
- 审批弹窗实时性问题已修复（新建会话后监听链路正确注册）
- 前后端信号与状态常量化/枚举化后，字符串漂移问题明显下降

---

## 5. 残余风险与观察项

当前无阻断阶段7的P0问题。建议在阶段7开发并行保留以下观察：

- 长时间运行下 dispatcher 队列深度与 dropped 指标趋势
- 高并发 tool stream 场景下控制类事件（confirm/state/error）实时性
- 真实业务流量下 DB sink 延迟抖动

---

## 6. 进入阶段7的准入结论

阶段6准入条件已满足，建议 **立即进入阶段7（单层子任务 Call 模型）**。

建议执行策略：

1. 先做父任务 `waiting_on_task_id` + `child_sessions` 的最小闭环
2. 完成后进行一段内部稳定性跑测，再启动后续阶段

---

## 7. 关于阶段8/9顺序调整（建议记录）

你提出“先做阶段9，再做阶段8”以提升阶段8体感验证质量，这个方向合理。  
建议在进入阶段7后，将此调整写入 `work/plan.md` 的“阶段顺序说明”或新增“执行顺序变更记录”小节，避免后续执行分歧。

