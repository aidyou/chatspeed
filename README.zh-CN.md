[English](./README.md) | 简体中文

# ChatSpeed

**由 Rust 精心打造，一款开源、高性能的 AI 助手，是您强大的编程伴侣与智能桌面中枢。**

![Claude Code 接入演示](assets/images/claude.gif)

## 🌟 Chatspeed 能做啥？

- **💼 多功能桌面助手**：你可以用它翻译、制作脑图、制作流程图、日常对话等，通过快捷键 ALT+Z 快速呼叫
- **🔌 连接任何开发工具**：不仅是 [Claude Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/claude-code.md)，你还可以将模型接入 [Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/ccproxy/gemini.md)、[Cline](https://docs.chatspeed.aidyou.ai/zh/ccproxy/cline.md)、[Roo Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/roo-code.md)、[Zed](https://docs.chatspeed.aidyou.ai/zh/ccproxy/zed.md) 等几乎所有主流 AI 开发工具。
- **💰 免费使用 Claude Code**：作为最佳实践，我们提供了详细的[免费使用 Claude Code](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/)教程。
- **🚀 MCP Hub**：Chatspeed 的 MCP 代理可以将自身的`WebSearch`和`WebFetch`工具连同您安装的 `MCP` 工具通过 `SSE` 协议提供给外部其他客户端使用，了解如何[集中管理 MCP](https://docs.chatspeed.aidyou.ai/zh/mcp/)

## 🚀 核心引擎: `CCProxy`

Chatspeed 的强大能力由其核心代理引擎 [CCProxy](https://docs.chatspeed.aidyou.ai/zh/ccproxy/) 驱动。它是一个用 Rust 实现的万能适配器，专注于：

1. **协议转换**：无缝转换 OpenAI 兼容协议、Claude、Gemini、Ollama 等主流协议。
2. **能力拓展**：通过工具兼容模式，为不支持原生工具调用功能的模型拓展了能力。
3. **减轻用户负担**：工具兼容模式让用户无需关心模型是否支持原生工具调用，显著降低了使用门槛和心智负担。
4. **安全隔离**：CCProxy 的密钥可以有效隔离客户端对AI密钥的直接访问，提升密钥安全性。
5. **分组管理**：支持代理分组功能，将客户端访问权限限制在指定模型分组内。
6. **负载均衡**：通过全局轮询所有供应商配置的密钥，有效缓解模型调用频率限制。
7. **简化工作流**：通过统一的 MCP 入口，告别在不同 IDE 中重复配置工具的烦恼。

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
- [CCProxy 简介](https://docs.chatspeed.aidyou.ai/zh/ccproxy/)
  - [CCProxy 工具兼容模式介绍](https://docs.chatspeed.aidyou.ai/zh/posts/experience-sharing/why-compat-mode.html)
  - [CCProxy 配置](https://docs.chatspeed.aidyou.ai/zh/ccproxy/configuration.html)
  - [接入 Claude Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/claude-code.html)
  - [接入 Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/ccproxy/gemini.html)
  - [接入 Cline](https://docs.chatspeed.aidyou.ai/zh/ccproxy/cline.html)
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
- [提示词库 —— 通过提示词增强 Code Agents](https://docs.chatspeed.aidyou.ai/zh/prompt/)
  - [CCProxy 通用提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/common.html)
  - [原生工具下 Claude Code 增强提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/claude-code-prompt-enhance-native-tool-call.html)
  - [工具兼容模式下的 Claude Code 增强提示词](https://docs.chatspeed.aidyou.ai/zh/prompt/claude-code-prompt-enhance.html)
- [博客](https://docs.chatspeed.aidyou.ai/zh/posts/)
  - [如何免费使用 Claude Code](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/post-1.html)

</details>
