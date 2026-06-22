[简体中文](./README.zh-CN.md) | English

# ChatSpeed

**Forged in Rust, this open-source, high-performance AI assistant is your powerful programming companion and smart desktop hub.**

![Claude Code Integration Demo](assets/images/claude.gif)

## 🌟 What can Chatspeed do?

- **⚡ Tool-Driven Workflow Engine**: Built for coding and long-running multi-step work. Chatspeed uses a tool-driven `Plan -> Execute -> Final Review` loop to reduce task interruption and push work toward completion instead of stopping at partial analysis.
- **🤖 Main Agent + Child Agents**: Split work across a parent agent and isolated sub-agents for parallel subtasks, review passes, and controlled delegation.
- **🧠 Memory + Skills + MCP**: Combine persistent memory, reusable skills, and MCP-based tool expansion into one execution system instead of scattering capabilities across separate products.
- **🧩 Multi-Model by Design**: Mix planning, execution, utility, and vision models. Supports OpenAI-compatible, Gemini, Claude, and Ollama models, while `CCProxy` extends tool access to models without native tool calling.
- **💼 Multi-functional Desktop Assistant**: You can also use it for translation, mind maps, flowcharts, daily conversations, and other desktop tasks, quickly summoned with `ALT+Z`.
- **🔌 Connect Any Dev Tool**: Beyond just [Claude Code](https://docs.chatspeed.aidyou.ai/ccproxy/claude-code.md), you can also connect models to almost any major AI development tool, including [Gemini CLI](https://docs.chatspeed.aidyou.ai/ccproxy/gemini.md), [Cline](https://docs.chatspeed.aidyou.ai/ccproxy/cline.md), [Roo Code](https://docs.chatspeed.aidyou.ai/ccproxy/roo-code.md), and [Zed](https://docs.chatspeed.aidyou.ai/ccproxy/zed.md).
- **💰 Use Claude Code for Free**: As a best practice, we provide a detailed tutorial on how to [use Claude Code for free](https://docs.chatspeed.aidyou.ai/posts/claude-code-free/).
- **🚀 MCP Hub**: Chatspeed's MCP proxy provides its own `WebSearch` and `WebFetch` tools, along with any `MCP` tools you've installed, to external clients via the stable `Streamable HTTP` protocol (SSE protocol has been removed from v2.0). Learn how to [centrally manage MCP](https://docs.chatspeed.aidyou.ai/mcp/).

> [!CAUTION]

## ⚡ Workflow-First Philosophy

Chatspeed's workflow system is built around one practical belief: **serious coding work should be tool-driven, stateful, and completion-oriented**.

Many AI coding tools fail in the middle of real work because they drift into chat, lose task state, or stop after partial reasoning. Chatspeed is designed to push in the opposite direction:

- **Tool-driven execution instead of chat-only continuation**
- **Structured `Plan -> Execute -> Final Review` instead of one-shot answers**
- **Explicit approvals, checkpoints, and recovery instead of hidden state**
- **High completion pressure for real tasks, not just good-looking intermediate output**

This is the main distinction of Chatspeed's workflow engine: it is not trying to be another generic chat wrapper. It is trying to be a reliable task completion system for real engineering work.

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

## ⚡ Workflow Engine

Chatspeed introduces a **tool-driven Workflow Engine** since v2.0, purpose-built for complex multi-step tasks like coding, debugging, refactoring, and documentation work. Key capabilities include:

- **`Plan -> Execute -> Final Review` Main Loop**: Planning is not just a prompt style. It is an explicit workflow stage with approval and transition into implementation.
- **Main Agent + Child-Agent Orchestration**: Delegate parallel or isolated work to sub-agents while preserving parent workflow control.
- **Skills Integration**: Reuse structured skills as part of workflow execution rather than bolting them on as separate prompts.
- **MCP Expansion**: Extend the workflow through MCP tools and use the same MCP ecosystem across Chatspeed and external clients.
- **Persistent Memory**: Keep both project memory and global memory across sessions.
- **Multi-Model Workflow Routing**: Assign different models to planning, execution, utility, and vision roles.
- **Broad Model Compatibility**: Supports OpenAI-compatible, Gemini, Claude, and Ollama protocol families. `CCProxy` can bridge some models that do not support native tool calling.
- **Approval, Safety, and Review Controls**: Auto-approve, shell policies, PathGuard, AI-assisted review, and explicit terminal completion tools.
- **Recovery and Resume**: Workflow state, approvals, waiting, and recovery are designed as first-class runtime concerns instead of best-effort UI behavior.
- **High Context-Cache Efficiency**: In real coding tests with `deepseek-v4`, context-cache hit rate is typically around **90%+**, and in some programming scenarios reached as high as **98%**.
- **Dedicated Workflow UI**: Approval dialogs, file diffs, task ledger, status panel, multi-workflow switching, and execution-state feedback are built in.

If you care about whether an AI coding system can **finish** multi-step work rather than merely talk about it, this is the part of Chatspeed to pay attention to.

Learn more about the [Workflow Engine](https://docs.chatspeed.aidyou.ai/workflow/).

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
- [Workflow Engine](https://docs.chatspeed.aidyou.ai/workflow/)
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
