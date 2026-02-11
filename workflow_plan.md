# 工作流模块重构规划 (Vercel AI SDK 替换方案)

## 1. 目标
使用 **Vercel AI SDK (Core)** 彻底替换现有的手动 ReAct 引擎 (`engine.ts`, `llm.ts`, `stateMachine.ts`)。
*   **代码精简**: 预计缩减 60% 以上的样板代码。
*   **稳定性**: 利用 SDK 内置的 `maxSteps` 自动处理 ReAct 循环。
*   **类型安全**: 使用 **Zod** 替代手写的 JSON Schema 进行工具参数校验。
*   **高性能流式**: 使用 SDK 标准的流处理机制，提升 UI 响应速度。

## 2. 技术栈
*   **核心库**: `ai` (Vercel AI SDK Core)
*   **适配器**: `@ai-sdk/openai` (连接本地 `ccproxy` 11435 端口)
*   **校验**: `zod`

## 3. 核心重构策略

### A. 基础设施层
1.  **安装依赖**: `yarn add ai zod @ai-sdk/openai`
2.  **Provider 配置**: 在 `llm.ts` 或新文件中定义指向 `ccproxy` 的 OpenAI 兼容适配器。
    ```typescript
    import { createOpenAI } from '@ai-sdk/openai';
    const chatspeedProxy = createOpenAI({
      baseURL: 'http://localhost:11435/v1',
      apiKey: 'your-proxy-key',
    });
    ```

### B. 工具定义层 (`src/pkg/workflow/tools/`)
*   将现有的 `ToolDefinition` 转换为 AI SDK 的 `tool` 格式。
*   使用 `z.object({...})` 替换 `inputSchema`。
*   **示例**:
    ```typescript
    const getWeather = tool({
      description: 'Get weather for a location',
      parameters: z.object({ loc: z.string() }),
      execute: async ({ loc }) => { /* 调用现有 Rust 或 TS 逻辑 */ }
    });
    ```

### C. 引擎层 (`src/pkg/workflow/engine.ts`)
*   **废弃**: 手动的 `switch-case` 状态跳转逻辑。
*   **采用**: `streamText` 的 `maxSteps` 模式。
    ```typescript
    const result = await streamText({
      model: chatspeedProxy('model-id'),
      tools: { getWeather, ... },
      maxSteps: 10, // 自动 ReAct 循环
      onStepFinish: async ({ text, toolCalls, toolResults }) => {
        // 在每一步结束时，通过 api.ts 将消息同步到 Tauri 后端数据库
      },
      onFinish: async ({ text }) => {
        // 最终任务完成处理
      }
    });
    ```

### D. 状态管理层
*   简化 `stateMachine.ts`。由于 AI SDK 接管了中间步骤，状态机只需维护：`IDLE` -> `RUNNING` -> `PAUSED (等待审批)` -> `FINISHED` -> `ERROR`。

## 4. 实施步骤

### 第一阶段: 环境与基础
1.  安装 `ai`, `zod`, `@ai-sdk/openai`。
2.  在 `types.ts` 中更新类型定义，兼容 SDK 的 `LanguageModel` 接口。

### 第二阶段: 工具迁移 (关键)
1.  编写一个适配器，将现有的 TypeScript 工具封装为 SDK 可用的 `tool` 对象。
2.  重点重构 `todoList` 和 `webAnalytics` 两个复杂工具。

### 第三阶段: Engine V2 开发
1.  实现 `engine_v2.ts`，使用 `streamText` 替换原本的递归/循环调用。
2.  对接 `api.ts`，确保 SDK 产生的每一个 `step` 都能正确持久化到本地 SQLite。

### 第四阶段: 审批逻辑集成
1.  利用 SDK 的 `onStepFinish` 拦截需要人工审批的工具。
2.  实现工作流的暂停与恢复（利用 SDK 的 `initialMessages` 恢复上下文）。

## 5. 注意事项
*   **端口一致性**: 确保 SDK 始终连接 `localhost:11435`。
*   **Token 统计**: AI SDK 返回的 `usage` 信息非常精准，需将其同步到现有的统计模块。
*   **错误处理**: 针对网络超时或模型拒绝调用工具的情况，利用 SDK 内置的重试机制。

---
**准备就绪**: 您可以随时启动新会话，只需告诉下一个 AI 助手“按照根目录的 workflow_plan.md 开始重构工作流模块”即可。
