简体中文 | [English](./README.md)

# ChatSpeed

**由 Rust 精心打造，一款开源、高性能的 AI 助手，是您强大的编程伴侣与智能桌面中枢。**

![Claude Code 接入演示](assets/images/claude.gif)

## 🌟 Chatspeed 能做啥？

- **⚡ 工具驱动的工作流引擎**：专为编程和长链路多步骤任务而设计，通过 `计划 -> 执行 -> 终审` 的主循环，尽可能减少任务中断，把任务真正推进到完成。
- **🤖 主代理 + 子代理**：由主代理统筹，并通过子代理隔离和并行处理子任务、审查任务和专项任务。
- **🧠 记忆 + Skills + MCP**：把持久化记忆、可复用技能和 MCP 工具扩展整合进同一套执行系统，而不是散落在不同产品能力里。
- **🧩 多模型架构**：规划、执行、工具、视觉可以由不同模型承担；支持 OpenAI 兼容、Gemini、Claude、Ollama 协议模型，并通过 `CCProxy` 扩展部分不支持原生工具调用的模型。
- **💼 多功能桌面助手**：你也可以用它翻译、制作脑图、制作流程图、日常对话等，通过快捷键 `ALT+Z` 快速呼叫。
- **🔌 连接任何开发工具**：不仅是 [Claude Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/claude-code.md)，你还可以将模型接入 [Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/ccproxy/gemini.md)、[Cline](https://docs.chatspeed.aidyou.ai/zh/ccproxy/cline.md)、[Roo Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/roo-code.md)、[Zed](https://docs.chatspeed.aidyou.ai/zh/ccproxy/zed.md) 等几乎所有主流 AI 开发工具。
- **💰 免费使用 Claude Code**：作为最佳实践，我们提供了详细的[免费使用 Claude Code](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/)教程。
- **🚀 MCP Hub**：Chatspeed 的 MCP 代理可以将自身的`WebSearch`和`WebFetch`工具连同您安装的 `MCP` 工具通过 `Streamable HTTP` 协议（自 v2.0 起已移除 SSE 协议）提供给外部其他客户端使用，了解如何[集中管理 MCP](https://docs.chatspeed.aidyou.ai/zh/mcp/)

> [!CAUTION]

## ⚡ 工作流优先的理念

Chatspeed 的工作流系统围绕一个很实际的理念构建：**严肃的编程任务，应该由工具驱动、由状态驱动，并且以完成任务为目标**。

我在实际使用很多编程类 AI 工具时，经常遇到的问题不是“回答得不够像人”，而是：

- 任务做到一半就中断
- 状态丢失后无法继续
- 只会分析，不会真正推进执行
- 多步骤任务在关键节点缺少约束和审查

所以 Chatspeed 选择了相反的路线：

- **用工具驱动执行，而不是停留在对话续写**
- **用结构化的 `计划 -> 执行 -> 终审` 主线，而不是一次性回答**
- **用显式审批、检查点和恢复机制，而不是隐式状态**
- **尽可能把任务推到完成，而不是只生成看起来不错的中间输出**

这也是 Chatspeed 工作流和许多同类工具最大的区别之一：它不是单纯的聊天外壳，而是一套面向真实工程任务完成率的执行系统。

## 🚀 核心引擎: `CCProxy`

Chatspeed 的强大能力由其核心代理引擎 [CCProxy](https://docs.chatspeed.aidyou.ai/zh/ccproxy/) 驱动。它是一个用 Rust 实现的万能适配器，专注于：

1. **协议转换**：无缝转换 OpenAI 兼容协议、Claude、Gemini、Ollama 等主流协议。
2. **OpenAI Responses 支持**：支持 OpenAI 兼容的 `POST /v1/responses` 接口；在上游支持时可直通 `/responses`，不支持时也可回退到统一 chat 流水线后再转换回 Responses 输出。
3. **能力拓展**：通过工具兼容模式，为不支持原生工具调用功能的模型拓展了能力。
4. **模型能力增强**：CCProxy 的提示词注入功能有效提升非 Claude 模型接入 Claude Code 的表现，实现从对话模型到专业代码助手的转型。
5. **减轻用户负担**：工具兼容模式让用户无需关心模型是否支持原生工具调用，显著降低了使用门槛和心智负担。
6. **安全隔离**：CCProxy 的密钥可以有效隔离客户端对 AI 密钥的直接访问，提升密钥安全性。
7. **分组与服务器级切换**：既支持代理分组隔离，也支持按代理服务器别名直接切换后端模型目标，更适合多供应商、多模型的日常调度。
8. **统计与趋势分析**：支持代理服务器级别的 token 统计与趋势查看，可聚合查看输入 token、输出 token、缓存 token、缓存命中率以及近期每日趋势。
9. **负载均衡**：通过全局轮询所有供应商配置的密钥，有效缓解模型调用频率限制。
10. **简化工作流**：通过统一的 MCP 入口，告别在不同 IDE 中重复配置工具的烦恼。

## ⚡ 工作流引擎

Chatspeed 从 v2.0 起引入了**工具驱动的工作流引擎**，专为编程、调试、重构、文档维护等复杂多步骤任务设计。核心能力包括：

- **`计划 -> 执行 -> 终审` 主循环**：规划不是一个提示词技巧，而是显式的工作流阶段，经过审批后再进入执行。
- **主代理 + 子代理编排**：由主代理统筹任务，子代理并行承担隔离的子任务或专项审查任务。
- **Skills 技能支持**：把可复用技能纳入工作流执行，而不是作为额外提示词散落使用。
- **MCP 扩展支持**：工作流可以直接利用 MCP 工具体系，同时保持与外部客户端接入的一致性。
- **持久化记忆系统**：同时支持项目级记忆与全局记忆。
- **多模型工作流分工**：不同模型可分别承担规划、执行、工具辅助、视觉处理等角色。
- **广泛模型兼容性**：支持 OpenAI 兼容、Gemini、Claude、Ollama 协议模型；`CCProxy` 还能为部分不支持原生工具调用的模型补齐能力。
- **审批、安全与终审控制**：自动审批、Shell 策略、PathGuard、AI 辅助审查和显式完成工具共同组成安全与收口机制。
- **恢复与续跑能力**：工作流状态、等待态、审批态、恢复态都被视为一等运行时问题，而不是单纯的 UI 现象。
- **高上下文缓存命中率**：在我对 `deepseek-v4` 的实际测试中，缓存命中率通常可达到 **90%+**，部分编程场景最高可达 **98%**。
- **专门的工作流界面**：审批弹窗、文件差异对比、任务账本、状态面板、多工作流切换等界面能力均已内建。

如果你关心的不是“AI 能不能聊”，而是“AI 能不能把多步骤任务做完”，那 Chatspeed 的工作流系统就是最值得关注的部分。

了解 [Workflow 工作流引擎](https://docs.chatspeed.aidyou.ai/zh/workflow/)。

## 安装

### Windows

1. 从 [Releases 页面](https://github.com/aidyou/chatspeed/releases/latest)下载 `.exe` 安装程序。
2. 运行安装程序并按照屏幕上的提示操作。
3. 您可能会看到 Windows SmartScreen 警告。请点击“更多信息”，然后点击“仍要运行”以继续。

### MacOS

从 [Releases 页面](https://github.com/aidyou/chatspeed/releases/latest)下载 `.dmg` 文件，对于苹果芯片请选择`_aarch64.dmg`后缀的文件，对于 Intel 芯片请选择`_x86.dmg`后缀的文件。

**重要提示：** 在较新版本的 MacOS 上，Gatekeeper 安全机制可能会阻止应用运行，并提示文件“**已损坏**”。这是因为应用尚未经过苹果公证。

请使用以下终端命令来解决此问题：

1. 将 `.app` 文件从挂载的 `.dmg` 镜像中拖拽到您的“应用程序”文件夹。
2. 打开“终端”应用 (Terminal)。
3. 执行以下命令 (可能需要输入您的系统密码):

   ```sh
   sudo xattr -cr /Applications/Chatspeed.app
   ```

4. 命令执行成功后，您就可以正常打开应用了。

### Linux

1. 从 [Releases 页面](https://github.com/aidyou/chatspeed/releases/latest)下载 `.AppImage`、`.deb` 或 `.rpm` 文件。
2. 对于 `.AppImage` 文件，请先为其添加可执行权限 (`chmod +x Chatspeed*.AppImage`)，然后直接运行。
3. 对于 `.deb` 文件，请使用您的包管理器进行安装，或通过命令 `sudo dpkg -i Chatspeed*.deb` 进行安装。
4. 对于 `.rpm` 文件，请使用您的包管理器进行安装，或通过命令 `sudo rpm -ivh Chatspeed*.rpm` 进行安装。

## 📚 了解更多

**我们强烈建议您从 [官方文档网站](https://docs.chatspeed.aidyou.ai/zh/) 开始，以获得最佳的阅读和学习体验。**

<details>
<summary>或者，点击此处展开详细的文档索引</summary>

- [Chatspeed](https://docs.chatspeed.aidyou.ai/zh/)
- [功能概览](https://docs.chatspeed.aidyou.ai/zh/guide/features/overview.html)
- [指南](https://docs.chatspeed.aidyou.ai/zh/guide/)
  - [快速开始](https://docs.chatspeed.aidyou.ai/zh/guide/quickStart.html)
  - [安装指南](https://docs.chatspeed.aidyou.ai/zh/guide/installation.html)
  - [开发指南](https://docs.chatspeed.aidyou.ai/zh/guide/development.html)
- [Workflow 工作流引擎](https://docs.chatspeed.aidyou.ai/zh/workflow/)
- [CCProxy 简介](https://docs.chatspeed.aidyou.ai/zh/ccproxy/)
  - [CCProxy 工具兼容模式介绍](https://docs.chatspeed.aidyou.ai/zh/posts/experience-sharing/why-compat-mode.html)
  - [CCProxy 配置](https://docs.chatspeed.aidyou.ai/zh/ccproxy/configuration.html)
  - [接入 Claude Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/claude-code.html)
  - [接入 Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/ccproxy/gemini.html)
  - [接入 Cline](https://docs.chatspeed.aidyou.ai/zh/ccproxy/cline.html)
  - [接入 Crush](https://docs.chatspeed.aidyou.ai/zh/ccproxy/crush.html)
  - [接入 Roo Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/roo-code.html)
  - [接入 Zed](https://docs.chatspeed.aidyou.ai/zh/ccproxy/zed.html)
  - [如何访问 CCProxy 的 API](https://docs.chatspeed.aidyou.ai/zh/api/)
- [MCP Hub](https://docs.chatspeed.aidyou.ai/zh/mcp/)
  - [接入 Claude Code](https://docs.chatspeed.aidyou.ai/zh/mcp/#claude-code)
  - [接入 Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/mcp/#gemini-cli)
  - [接入 VS Code](https://docs.chatspeed.aidyou.ai/zh/mcp/#vs-code)
  - [接入 Cursor](https://docs.chatspeed.aidyou.ai/zh/mcp/#cursor)
  - [接入 Trae CN](https://docs.chatspeed.aidyou.ai/zh/mcp/#trae-cn)
  - [接入 Windsurf](https://docs.chatspeed.aidyou.ai/zh/mcp/#windsurf)
  - [接入 Cline](https://docs.chatspeed.aidyou.ai/zh/mcp/#cline)
  - [接入 Roo Code](https://docs.chatspeed.aidyou.ai/zh/mcp/#roo-code)
  - [接入 Crush](https://docs.chatspeed.aidyou.ai/zh/mcp/#crush)
- [提示词库 —— 通过提示词增强 Code Agents](https://docs.chatspeed.aidyou.ai/zh/prompt/)
  - [CCProxy 通用提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/common.html)
  - [原生工具下 Claude Code 增强提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/claude-code-prompt-enhance-native-tool-call.html)
  - [工具兼容模式下的 Claude Code 增强提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/claude-code-prompt-enhance.html)
- [博客](https://docs.chatspeed.aidyou.ai/zh/posts/)
  - [免费使用 Claude Code：集成魔塔 qwen3-coder](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/qwen3-coder.html)
  - [免费使用 Claude Code：集成 Nvidia deepseek-v3.1](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/deepseek-v3.1.html)
  - [免费使用 Claude Code：集成 grok-4-fast](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/grok-4-fast.html)
  - [CCProxy 工具兼容模式 - 让任何AI模型都具备工具调用能力](https://docs.chatspeed.aidyou.ai/zh/posts/experience-sharing/why-compat-mode.html)

</details>
