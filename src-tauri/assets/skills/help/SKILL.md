---
name: help
description: Displays ChatSpeed help and documentation index. Use this skill when users want help with or have questions about ChatSpeed features.
---

# ChatSpeed Help Index

This skill provides an index to ChatSpeed's official documentation.

## AI Assistant Instructions

When a user asks a question about ChatSpeed:

1. **Check the Index**: Scan the "Documentation Index" below to find the most relevant topic.
2. **Fetch Details**: If you need to provide a detailed explanation or troubleshooting steps, you **MUST** use the `web_fetch` tool to retrieve the actual content from the corresponding URL (Base URL: `https://docs.chatspeed.aidyou.ai/`).
3. **Synthesize**: After fetching the content, provide a concise and helpful answer based on the official documentation. Always include the source link at the end of your response.
4. **Fallback**: If the user's question is not covered by the topics in the index, or if fetching fails, direct them to the main documentation site: `https://docs.chatspeed.aidyou.ai/`.

## Documentation Index

Below are the key topics. All links are relative to `https://docs.chatspeed.aidyou.ai/`.

### 🚀 Getting Started
- **Quick Start Guide**: `/guide/quickStart.html`
- **Installation**: `/guide/installation.html`
- **Features Overview**: `/guide/features/overview.html`

### 🛠️ Configuration & Core Engine (CCProxy)
- **Introduction to CCProxy**: `/ccproxy/`
- **Configuration Guide**: `/ccproxy/configuration.html`
- **Client Integrations**:
  - **Claude Code**: `/ccproxy/claude-code.html`
  - **Cline / Roo-Code**: `/ccproxy/cline.html` & `/ccproxy/roo-code.html`
  - **Zed Editor**: `/ccproxy/zed.html`
  - **Gemini**: `/ccproxy/gemini.html`
  - **Crush**: `/ccproxy/crush.html`

### 🔌 Model Context Protocol (MCP)
- **MCP Hub**: `/mcp/`

### 🧠 Prompt Engineering & Enhancement
- **Prompt Enhancement Overview**: `/prompt/`
- **Claude Code Enhancement**: `/prompt/claude-code-prompt-enhance.html`
- **Native Tool Call Support**: `/prompt/claude-code-prompt-enhance-native-tool-call.html`
- **Common Prompts**: `/prompt/common.html`

### 💻 Development & API
- **Developer Guide**: `/guide/development.html`
- **API Reference**: `/api/`

## Additional Resources
- **GitHub**: [https://github.com/chatspeed-ai/chatspeed](https://github.com/chatspeed-ai/chatspeed)
- **Official Website**: [https://chatspeed.aidyou.ai/](https://chatspeed.aidyou.ai/)
