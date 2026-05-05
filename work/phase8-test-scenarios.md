# 阶段8测试验收文档

> 测试目的：验证 Handoff 与焦点代理模型  
> 对应计划：`work/plan.md` 第 15 节和 15A 节

---

## 一、阶段测试边界

本阶段是控制权转移，不新增审核 Gate，不做任务账本 UI 扩展。

- 必测：`focused_agent_id`、输入路由、自动归还、强制回主控、恢复一致性
- 不测：阶段9展示增强

---

## 二、核心验证点

1. handoff 后用户输入路由到焦点代理
2. 专家任务完成后自动归还主控
3. 异常路径可强制回主控
4. `focused_agent_id` / `handoff_stack` 可持久化恢复
5. 焦点与运行代理不混淆

---

## 三、日志关键字对照

建议关注：

```log
workflow.handoff.enter
workflow.handoff.route_input
workflow.handoff.return_to_parent
workflow.handoff.force_return
workflow.handoff.restore
```

---

## 四、自动化测试最低要求

1. handoff 后输入路由断言
2. 自动归还主控断言
3. 强制回主控断言
4. 重启后焦点代理恢复断言

建议命令：

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

---

## 五、测试场景（自动化优先）

### 场景1：进入 handoff 并切换焦点代理（自动化）

建议实现：
1. 构造主代理触发 handoff 到专家代理
2. 断言 `focused_agent_id` 更新和 `handoff_stack` 入栈

验收：
- ✅ `focused_agent_id` 更新为专家代理
- ✅ `handoff_stack` 入栈

### 场景2：handoff 后输入路由正确（自动化 + 手工补充）

自动化建议：
1. handoff 活跃时注入用户输入
2. 断言输入被路由到焦点代理上下文

手工补充：
1. UI 连续输入多条消息
2. 验证展示层“当前焦点代理”与实际路由一致

验收：
- ✅ 输入被路由到焦点代理
- ✅ 主代理不误消费输入

### 场景3：专家完成后自动归还主控（自动化）

建议实现：
1. 专家子流程完成
2. 断言 `focused_agent_id` 回主代理且 `handoff_stack` 出栈

验收：
- ✅ 焦点自动回主代理
- ✅ `handoff_stack` 出栈

### 场景4：异常场景强制回主控（自动化）

建议实现：
1. 模拟专家代理异常/超时
2. 调用 force_return 逻辑
3. 断言后续输入路由回主代理

验收：
- ✅ 强制回主控成功
- ✅ 输入恢复到主代理

### 场景5：重启恢复焦点代理（自动化 + 手工补充）

自动化建议：
1. handoff 活跃时写 snapshot/event
2. 重建 executor（模拟重启）并 restore
3. 断言焦点代理和路由规则恢复

手工补充：
1. 真机重启应用后继续输入
2. 验证 UI 焦点提示和消息落点一致

验收：
- ✅ 焦点代理信息不丢失
- ✅ 输入路由仍正确

---

## 六、里程碑回归范围（阶段8）

阶段8为高风险控制权里程碑，建议执行：

1. 阶段7：Call 模型全量
2. 阶段6：分发层冒烟
3. 阶段5：恢复链冒烟
4. 阶段8：handoff 全量

---

## 七、完成定义检查

| 检查项 | 状态 |
|---|---|
| handoff 切换焦点成功 | ⬜ |
| 输入路由到焦点代理 | ⬜ |
| 自动归还主控可用 | ⬜ |
| 强制回主控可用 | ⬜ |
| 重启后焦点可恢复 | ⬜ |
| 未引入任务账本 UI 改造 | ⬜ |
