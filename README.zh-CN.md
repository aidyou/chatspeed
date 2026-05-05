简体中文 | [English](./README.md)

> [!IMPORTANT]
> **紧急更新提示**：由于 **1.2.5** 版本中存在一个严重的逻辑疏忽，导致自动更新功能失效。如果您当前正在使用 **1.2.5** 版本，请前往 Releases 页面 [**手动下载并安装最新版本**](https://github.com/aidyou/chatspeed/releases/latest)，以恢复自动更新能力并获取后续修复。对于给您带来的不便，我们深表歉意。

# ChatSpeed

**由 Rust 精心打造，一款开源、高性能的 AI 助手，是您强大的编程伴侣与智能桌面中枢。**

![Claude Code 接入演示](assets/images/claude.gif)

## 🌟 Chatspeed 能做啥？

- **💼 多功能桌面助手**：你可以用它翻译、制作脑图、制作流程图、日常对话等，通过快捷键 ALT+Z 快速呼叫
- **🔌 连接任何开发工具**：不仅是 [Claude Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/claude-code.md)，你还可以将模型接入 [Gemini CLI](https://docs.chatspeed.aidyou.ai/zh/ccproxy/gemini.md)、[Cline](https://docs.chatspeed.aidyou.ai/zh/ccproxy/cline.md)、[Roo Code](https://docs.chatspeed.aidyou.ai/zh/ccproxy/roo-code.md)、[Zed](https://docs.chatspeed.aidyou.ai/zh/ccproxy/zed.md) 等几乎所有主流 AI 开发工具。
- **💰 免费使用 Claude Code**：作为最佳实践，我们提供了详细的[免费使用 Claude Code](https://docs.chatspeed.aidyou.ai/zh/posts/claude-code-free/)教程。
- **🚀 MCP Hub**：Chatspeed 的 MCP 代理可以将自身的`WebSearch`和`WebFetch`工具连同您安装的 `MCP` 工具通过更稳定的 `Streamable HTTP` 协议提供给外部其他客户端使用，了解如何[集中管理 MCP](https://docs.chatspeed.aidyou.ai/zh/mcp/)

> [!CAUTION]

## 🚀 核心引擎: `CCProxy`

Chatspeed 的强大能力由其核心代理引擎 [CCProxy](https://docs.chatspeed.aidyou.ai/zh/ccproxy/) 驱动。它是一个用 Rust 实现的万能适配器，专注于：

1. **协议转换**：无缝转换 OpenAI 兼容协议、Claude、Gemini、Ollama 等主流协议。
2. **能力拓展**：通过工具兼容模式，为不支持原生工具调用功能的模型拓展了能力。
3. **模型能力增强**：CCProxy 的提示词注入功能有效提升非 Claude 模型接入 Claude Code 的表现，实现从对话模型到专业代码助手的转型。
4. **减轻用户负担**：工具兼容模式让用户无需关心模型是否支持原生工具调用，显著降低了使用门槛和心智负担。
5. **安全隔离**：CCProxy 的密钥可以有效隔离客户端对AI密钥的直接访问，提升密钥安全性。
6. **分组管理**：支持代理分组功能，将客户端访问权限限制在指定模型分组内。
7. **负载均衡**：通过全局轮询所有供应商配置的密钥，有效缓解模型调用频率限制。
8. **简化工作流**：通过统一的 MCP 入口，告别在不同 IDE 中重复配置工具的烦恼。

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
