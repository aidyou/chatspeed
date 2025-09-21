[简体中文](./README.zh-CN.md) | English

# ChatSpeed

**Forged in Rust, this open-source, high-performance AI assistant is your powerful programming companion and smart desktop hub.**

![Claude Code Integration Demo](assets/images/claude.gif)

## 🌟 What can Chatspeed do?

- **💼 Multi-functional Desktop Assistant**: You can use it for translation, creating mind maps, flowcharts, daily conversations, etc., quickly summoned with the shortcut ALT+Z.
- **🔌 Connect Any Dev Tool**: Beyond just [Claude Code](https://docs.chatspeed.aidyou.ai/en/ccproxy/claude-code.md), you can also connect models to almost any major AI development tool, including [Gemini CLI](https://docs.chatspeed.aidyou.ai/en/ccproxy/gemini.md), [Cline](https://docs.chatspeed.aidyou.ai/en/ccproxy/cline.md), [Roo Code](https://docs.chatspeed.aidyou.ai/en/ccproxy/roo-code.md), and [Zed](https://docs.chatspeed.aidyou.ai/en/ccproxy/zed.md).
- **💰 Use Claude Code for Free**: As a best practice, we provide a detailed tutorial on how to [use Claude Code for free](https://docs.chatspeed.aidyou.ai/en/posts/claude-code-free/).
- **🚀 MCP Hub**: Chatspeed's MCP proxy can provide its own `WebSearch` and `WebFetch` tools, along with any `MCP` tools you've installed, to external clients via the `SSE` protocol. Learn how to [centrally manage MCP](https://docs.chatspeed.aidyou.ai/en/mcp/).

## 🚀 Core Engine: `CCProxy`

Chatspeed's power is driven by its core proxy engine, [CCProxy](https://docs.chatspeed.aidyou.ai/en/ccproxy/). It's a universal adapter built with Rust, focused on:

1. **Protocol Conversion**: Seamlessly convert between major protocols like OpenAI-compatible, Claude, Gemini, and Ollama.
2. **Capability Expansion**: Expands the capabilities of models that do not natively support tool calling through a tool compatibility mode.
3. **Reducing User Burden**: The tool compatibility mode frees users from worrying about whether a model supports native tool calling, significantly lowering the barrier to entry and mental load.
4. **Security Isolation**: CCProxy's keys effectively isolate clients from direct access to AI provider keys, enhancing key security.
5. **Group Management**: Supports a proxy grouping feature to restrict client access to specific model groups.
6. **Load Balancing**: Effectively mitigates model rate-limiting issues by globally rotating through all configured provider keys.
7. **Simplified Workflow**: Say goodbye to repeatedly configuring tools in different IDEs with a unified MCP entry point.

## 📚 Learn More

**We highly recommend starting with our [Official Documentation Website](https://docs.chatspeed.aidyou.ai/) for the best reading and learning experience.**

<details>
<summary>Or, click here to expand the detailed documentation index</summary>

- [Chatspeed](https://docs.chatspeed.aidyou.ai/)
- [Features Overview](https://docs.chatspeed.aidyou.ai/en/guide/features/overview.html)
- [Guide](https://docs.chatspeed.aidyou.ai/en/guide/)
  - [Quick Start](https://docs.chatspeed.aidyou.ai/en/guide/quickStart.html)
  - [Installation Guide](https://docs.chatspeed.aidyou.ai/en/guide/installation.html)
  - [Development Guide](https://docs.chatspeed.aidyou.ai/en/guide/development.html)
- [CCProxy Introduction](https://docs.chatspeed.aidyou.ai/en/ccproxy/)
  - [CCProxy Tool Compatibility Mode Explained](https://docs.chatspeed.aidyou.ai/en/posts/experience-sharing/why-compat-mode.html)
  - [CCProxy Configuration](https://docs.chatspeed.aidyou.ai/en/ccproxy/configuration.html)
  - [Connecting to Claude Code](https://docs.chatspeed.aidyou.ai/en/ccproxy/claude-code.html)
  - [Connecting to Gemini CLI](https://docs.chatspeed.aidyou.ai/en/ccproxy/gemini.html)
  - [Connecting to Cline](https://docs.chatspeed.aidyou.ai/en/ccproxy/cline.html)
  - [Connecting to Roo Code](https://docs.chatspeed.aidyou.ai/en/ccproxy/roo-code.html)
  - [Connecting to Zed](https://docs.chatspeed.aidyou.ai/en/ccproxy/zed.html)
  - [How to Access the CCProxy API](https://docs.chatspeed.aidyou.ai/en/api/)
- [MCP Hub](https://docs.chatspeed.aidyou.ai/en/mcp/)
  - [Connecting to Claude Code](https://docs.chatspeed.aidyou.ai/en/mcp/#claude-code)
  - [Connecting to Gemini CLI](https://docs.chatspeed.aidyou.ai/en/mcp/#gemini-cli)
  - [Connecting to VS Code](https://docs.chatspeed.aidyou.ai/en/mcp/#vs-code)
  - [Connecting to Cursor](https://docs.chatspeed.aidyou.ai/en/mcp/#cursor)
  - [Connecting to Trae CN](https://docs.chatspeed.aidyou.ai/en/mcp/#trae-cn)
  - [Connecting to Windsurf](https://docs.chatspeed.aidyou.ai/en/mcp/#windsurf)
  - [Connecting to Cline](https://docs.chatspeed.aidyou.ai/en/mcp/#cline)
  - [Connecting to Roo Code](https://docs.chatspeed.aidyou.ai/en/mcp/#roo-code)
- [Prompt Library — Enhancing Code Agents with Prompts](https://docs.chatspeed.aidyou.ai/en/prompt/)
  - [CCProxy Common Prompts](https://docs.chatspeed.aidyou.ai/en/prompt/common.html)
  - [Claude Code Enhancement Prompts (Native Tool Call)](https://docs.chatspeed.aidyou.ai/en/prompt/claude-code-prompt-enhance-native-tool-call.html)
  - [Claude Code Enhancement Prompts (Tool Compatibility Mode)](https://docs.chatspeed.aidyou.ai/en/prompt/claude-code-prompt-enhance.html)
- [Blog](https://docs.chatspeed.aidyou.ai/en/posts/)
  - [How to Use Claude Code for Free](https://docs.chatspeed.aidyou.ai/en/posts/claude-code-free/post-1.html)

</details>
