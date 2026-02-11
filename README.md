[简体中文](./README.zh-CN.md) | English

> [!IMPORTANT]
> **Urgent Update Notice**: Due to a critical bug in version **1.2.5**, the automatic update feature is non-functional. If you are currently using version **1.2.5**, please [manually download and install the latest version](https://github.com/aidyou/chatspeed/releases/latest) from the Releases page to restore update capabilities and receive future fixes. We sincerely apologize for this oversight.

# ChatSpeed

**Forged in Rust, this open-source, high-performance AI assistant is your powerful programming companion and smart desktop hub.**

![Claude Code Integration Demo](assets/images/claude.gif)

## 🌟 What can Chatspeed do?

- **💼 Multi-functional Desktop Assistant**: You can use it for translation, creating mind maps, flowcharts, daily conversations, etc., quickly summoned with the shortcut ALT+Z.
- **🔌 Connect Any Dev Tool**: Beyond just [Claude Code](https://docs.chatspeed.aidyou.ai/ccproxy/claude-code.md), you can also connect models to almost any major AI development tool, including [Gemini CLI](https://docs.chatspeed.aidyou.ai/ccproxy/gemini.md), [Cline](https://docs.chatspeed.aidyou.ai/ccproxy/cline.md), [Roo Code](https://docs.chatspeed.aidyou.ai/ccproxy/roo-code.md), and [Zed](https://docs.chatspeed.aidyou.ai/ccproxy/zed.md).
- **💰 Use Claude Code for Free**: As a best practice, we provide a detailed tutorial on how to [use Claude Code for free](https://docs.chatspeed.aidyou.ai/posts/claude-code-free/).
- **🚀 MCP Hub**：Chatspeed's MCP proxy can provide its own `WebSearch` and `WebFetch` tools, along with any `MCP` tools you've installed, to external clients via the more stable `Streamable HTTP` protocol. Learn how to [centrally manage MCP](https://docs.chatspeed.aidyou.ai/mcp/).

> [!CAUTION]
> **⚠️ Deprecation Notice**: The `/mcp/sse` endpoint is now officially deprecated and scheduled for removal in **v1.3.0**. Due to inherent stability issues with SSE (such as unfixable 410 errors) and the removal of support in the upstream library, users are strongly encouraged to migrate to the `/mcp/http` (Streamable HTTP) protocol.

## 🚀 Core Engine: `CCProxy`

Chatspeed's power is driven by its core proxy engine, [CCProxy](https://docs.chatspeed.aidyou.ai/ccproxy/). It's a universal adapter built with Rust, focused on:

1. **Protocol Conversion**: Seamlessly convert between major protocols like OpenAI-compatible, Claude, Gemini, and Ollama.
2. **Capability Expansion**: Expands the capabilities of models that do not natively support tool calling through a tool compatibility mode.
3. **Model Enhancement**: CCProxy's prompt injection feature effectively improves the performance of non-Claude models when connected to Claude Code, transforming them from conversational models into professional code assistants.
4. **Reducing User Burden**: The tool compatibility mode frees users from worrying about whether a model supports native tool calling, significantly lowering the barrier to entry and mental load.
5. **Security Isolation**: CCProxy's keys effectively isolate clients from direct access to AI provider keys, enhancing key security.
6. **Group Management**: Supports a proxy grouping feature to restrict client access to specific model groups.
7. **Load Balancing**: Effectively mitigates model rate-limiting issues by globally rotating through all configured provider keys.
8. **Simplified Workflow**: Say goodbye to repeatedly configuring tools in different IDEs with a unified MCP entry point.

## Installation

### Windows

1.  Download the `.exe` installer from the [Releases page](https://github.com/aidyou/chatspeed/releases/latest).
2.  Run the installer and follow the on-screen instructions.
3.  You may see a Windows SmartScreen warning. Click "More info," then "Run anyway" to proceed.

### macOS

Download the `.dmg` file from the [Releases page](https://github.com/aidyou/chatspeed/releases/latest). For Apple Silicon, choose the file with the `_aarch64.dmg` suffix; for Intel chips, choose the `_x86.dmg` suffix.

**Important Note:** On recent versions of macOS, the Gatekeeper security feature may prevent the app from running and show a message that the file is "**damaged**". This is because the application has not yet been notarized by Apple.

Please use the following terminal command to resolve this issue:

1.  Drag the `.app` file from the mounted `.dmg` image to your "Applications" folder.
2.  Open the Terminal app.
3.  Execute the following command (you may need to enter your system password):

    ```sh
    sudo xattr -cr /Applications/Chatspeed.app
    ```

4.  After the command executes successfully, you can open the application normally.

### Linux

1.  Download the `.AppImage`, `.deb`, or `.rpm` file from the [Releases page](https://github.com/aidyou/chatspeed/releases/latest).
2.  For `.AppImage` files, first grant execute permissions (`chmod +x Chatspeed*.AppImage`), then run it directly.
3.  For `.deb` files, use your package manager to install, or install via the command `sudo dpkg -i Chatspeed*.deb`.
4.  For `.rpm` files, use your package manager to install, or install via the command `sudo rpm -ivh Chatspeed*.rpm`.


## 📚 Learn More

**We highly recommend starting with our [Official Documentation Website](https://docs.chatspeed.aidyou.ai/) for the best reading and learning experience.**

<details>
<summary>Or, click here to expand the detailed documentation index</summary>

- [Chatspeed](https://docs.chatspeed.aidyou.ai/)
- [Features Overview](https://docs.chatspeed.aidyou.ai/guide/features/overview.html)
- [Guide](https://docs.chatspeed.aidyou.ai/guide/)
  - [Quick Start](https://docs.chatspeed.aidyou.ai/guide/quickStart.html)
  - [Installation Guide](https://docs.chatspeed.aidyou.ai/guide/installation.html)
  - [Development Guide](https://docs.chatspeed.aidyou.ai/guide/development.html)
- [CCProxy Introduction](https://docs.chatspeed.aidyou.ai/ccproxy/)
  - [CCProxy Tool Compatibility Mode Explained](https://docs.chatspeed.aidyou.ai/posts/experience-sharing/why-compat-mode.html)
  - [CCProxy Configuration Guiden](https://docs.chatspeed.aidyou.ai/ccproxy/configuration.html)
  - [Claude Code Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/claude-code.html)
  - [Gemini CLI Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/gemini.html)
  - [Cline Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/cline.html)
  - [Crush Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/crush.html)
  - [Roo Code Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/roo-code.html)
  - [Zed Integration Guide](https://docs.chatspeed.aidyou.ai/ccproxy/zed.html)
  - [How to Access the CCProxy API](https://docs.chatspeed.aidyou.ai/api/)
- [MCP Hub](https://docs.chatspeed.aidyou.ai/mcp/)
  - [Connecting to Claude Code](https://docs.chatspeed.aidyou.ai/mcp/#claude-code)
  - [Connecting to Gemini CLI](https://docs.chatspeed.aidyou.ai/mcp/#gemini-cli)
  - [Connecting to VS Code](https://docs.chatspeed.aidyou.ai/mcp/#vs-code)
  - [Connecting to Cursor](https://docs.chatspeed.aidyou.ai/mcp/#cursor)
  - [Connecting to Trae CN](https://docs.chatspeed.aidyou.ai/mcp/#trae-cn)
  - [Connecting to Windsurf](https://docs.chatspeed.aidyou.ai/mcp/#windsurf)
  - [Connecting to Cline](https://docs.chatspeed.aidyou.ai/mcp/#cline)
  - [Connecting to Roo Code](https://docs.chatspeed.aidyou.ai/mcp/#roo-code)
  - [Connecting to Crush](https://docs.chatspeed.aidyou.ai/mcp/#crush)
- [Prompt Library — Enhancing Code Agents with Prompts](https://docs.chatspeed.aidyou.ai/prompt/)
  - [CCProxy Common Prompts](https://docs.chatspeed.aidyou.ai/prompt/common.html)
  - [Claude Code Enhancement Prompts (Native Tool Call)](https://docs.chatspeed.aidyou.ai/prompt/claude-code-prompt-enhance-native-tool-call.html)
  - [Claude Code Enhancement Prompts (Tool Compatibility Mode)](https://docs.chatspeed.aidyou.ai/prompt/claude-code-prompt-enhance.html)
- [Blog](https://docs.chatspeed.aidyou.ai/posts/)
  - [Using Claude Code for Free - Integrating ModelScope's qwen3-coder](https://docs.chatspeed.aidyou.ai/posts/claude-code-free/qwen3-coder.html)
  - [Free Claude Code Usage - Integrating Nvidia deepseek-v3.1](https://docs.chatspeed.aidyou.ai/posts/claude-code-free/deepseek-v3.1.html)
  - [Free Claude Code - Integrating grok-4-fast](https://docs.chatspeed.aidyou.ai/posts/claude-code-free/grok-4-fast.html)
  - [CCProxy Tool Compatibility Mode - Empowering Any AI Model with Tool Calling Capabilities](https://docs.chatspeed.aidyou.ai/posts/experience-sharing/streamable-http-vs-sse.html)

</details>
