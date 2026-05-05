# 阶段6测试验收文档

> 测试目的：验证 Dispatcher + 多 Sink 解耦与稳定性  
> 对应计划：`work/plan.md` 第 13 节和 13A 节

---

## 一、阶段测试边界

本阶段只验证输出分发层稳定性，不改变恢复语义与业务状态机语义。

- 必测：UI sink 故障隔离、DB sink 慢速隔离、最终状态不丢失、lag 可观测
- 不测：Handoff、子任务 Call、UI 高级面板

---

## 二、核心验证点

1. executor 输出通过 dispatcher 分发
2. UI sink 失败不影响 DB sink 持久化
3. DB sink 变慢不阻塞 executor 关键路径
4. 最终状态事件（completed/failed/cancelled）不丢失
5. sink 指标与告警日志可见

---

## 三、日志关键字对照

建议关注：

```log
workflow.dispatch.enqueue
workflow.dispatch.sink_lag
workflow.dispatch.dropped
workflow.sink.tauri.error
workflow.sink.db.error
```

可按实际实现命名微调，但必须满足“每 sink 可观测”。

---

## 四、自动化测试最低要求

1. UI sink 抛错时 DB sink 仍写入成功
2. DB sink 慢速时 executor 可继续推进并最终完成
3. 终态事件至少一次到达 DB sink
4. dropped/lag 计数可被测试读取

建议命令：

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

---

## 五、测试场景（自动化优先）

### 场景1：正常多 sink 分发（自动化）

建议实现：
1. 用 mock sink 运行 executor 到完成
2. 断言 UI sink 与 DB sink 都收到事件

验收：
- ✅ UI 正常收到状态
- ✅ DB 正常持久化状态和事件

### 场景2：UI sink 故障隔离（自动化）

建议实现：
1. 注入始终报错的 UI sink mock
2. 继续执行并断言 DB sink 写入不受影响

验收：
- ✅ UI sink 报错可见
- ✅ DB 侧数据持续写入
- ✅ workflow 最终可完成或按业务终止

### 场景3：DB sink 慢速隔离（自动化）

建议实现：
1. 注入带 sleep 的 DB sink mock
2. 断言 executor 主流程在时限内推进
3. 断言 lag 指标或日志被触发

验收：
- ✅ executor 不被长时间阻塞
- ✅ lag 日志出现
- ✅ 最终状态仍写入

### 场景4：终态保障（自动化）

建议实现：
1. 参数化测试分别触发 completed/failed/cancelled
2. 断言终态事件至少一次到达 DB sink

验收：
- ✅ 三类终态都不丢失
- ✅ 与 snapshot 终态一致

### 场景5：丢弃与告警可观测（自动化）

建议实现：
1. 构造高压事件流 + 受限队列
2. 断言 dropped/lag 计数增长且有日志输出

验收：
- ✅ 有明确计数日志
- ✅ 不出现 silent drop

### 保留手工项（必要）

1. UI 端实时渲染是否“体感卡顿/闪烁”类体验验证
2. 真机环境下系统资源压力（CPU/IO）对界面观感影响

---

## 六、回归范围说明

阶段6非里程碑，回归范围控制为：

1. 阶段6分发层能力全量
2. 阶段5恢复链冒烟（snapshot/replay 不被分发重构破坏）

---

## 七、完成定义检查

| 检查项 | 状态 |
|---|---|
| executor 输出已通过 dispatcher | ⬜ |
| UI sink 失败不影响 DB sink | ⬜ |
| DB sink 慢速不阻断 executor | ⬜ |
| 终态事件不丢失 | ⬜ |
| lag/dropped 指标可观测 | ⬜ |
