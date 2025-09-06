[简体中文](./RELEASE.zh-CN.md) ｜ English

# ChatSpeed v1.1.1 Release Notes

v1.1.1 is a maintenance release focused on improving stability and fixing issues. We have deeply optimized the internal logic of the `ccproxy` proxy module, especially in protocol compatibility and tool handling, to ensure more reliable and precise AI interaction.

## ✨ Enhancements

1.  **Refactored the Core Tool-Parsing Engine**
    We have adjusted and introduced a new tool-parsing mechanism. The new engine is more compatible and robust, allowing it to better understand and process diverse output formats from the AI. This not only improves the parsing success rate but also lowers the "difficulty" for the AI to use tools.

2.  **Unified & Enhanced Tool Compatibility Mode Across All Protocols**
    Tool Compatibility Mode now covers all protocols, including Gemini and Claude. Regardless of the backend model used, you can now benefit from the smoother workflows and higher goal completion rates provided by our enhanced guidance.

## 🐞 Bug Fixes

- **Fixed "Ghost Model" Issue in Proxy Configuration:** Resolved an issue where deleted backend models would remain in a proxy configuration and could not be removed during editing.
- **Fixed Missing Tool List in Compatibility Mode:** Addressed a bug where the tool list would fail to load into the prompt in certain scenarios when compatibility mode was enabled, ensuring the AI can see and use all available tools.
- **Fixed Imbalanced Global Key Rotation:** Corrected a flaw in the key rotation algorithm that caused some keys to be used too frequently and exceed rate limits, ensuring balanced key usage and service stability.

---

# ChatSpeed v1.1.0 Release Notes

Hello everyone! We are thrilled to announce the release of ChatSpeed v1.1.0. We've jumped directly from v1.0.4 to v1.1.0 because this version includes a series of milestone updates, especially regarding the **proxy module's Tool Compatibility Mode** and **search engine support**, which will significantly enhance the AI's capabilities and stability.

## 🚀 New Features

1.  **Powerful Search Engine Integration**
    - Now supports integration with major search engines via API keys, including **Google Search API**, **Tavily API**, and **Serper API**.
    - Additionally, built-in support for **Bing, DuckDuckGo, So.com, and Sougou** is included, providing you with diverse information sources.

2.  **Web Content Scraping & Analysis**
    - A new built-in web scraper has been added, capable of extracting text or Markdown content from a given URL, allowing the AI to fetch and analyze real-time web information.

3.  **Flexible Prompt Injection**
    - To better accommodate different models (especially those with weaker support for the `system` role), you can now choose to inject enhancement prompts into either the `system` role or the last `user` message, greatly improving compatibility.

## ✨ Enhancements

The core of this update is a deep refactoring and optimization of the **proxy module's "Tool Compatibility Mode,"** aiming for a more stable, compatible, and fluid intelligent experience.

### 1. More Stable Tool Invocation

By refactoring the underlying tool compatibility mode, we have significantly improved the stability and accuracy of AI tool execution. Optimized tool definitions (with required parameters listed first and labeled `(required)`/`(optional)`) and stricter formatting rules provide a clear set of "guardrails" for the AI, better driving different models (even weaker ones) to execute tasks stably and accurately.

### 2. Customizable Cross-Model Compatibility

We've introduced a flexible prompt injection mechanism. You can now choose to inject enhancement prompts into the `system` or `user` message. This feature allows for optimization based on the characteristics of different models, solving issues where some models respond poorly to the `system` role, and significantly improving the performance of various models in clients like `claude code`.

### 3. Workflow Optimization via Prompt Guidance

We have greatly optimized the prompts to assist the AI in forming a more robust and less interruptible workflow. This includes guiding the AI in task planning (prioritizing tools like `TodoWrite`) and implementing intelligent fault tolerance (providing a detailed "correction guide" when tool call parsing fails), thereby significantly increasing the success rate of complex tasks.

## 🐞 Bug Fixes

- Fixed a state management bug in the stream processor that caused tool call parsing to fail.
- Fixed an issue where an XML parameter value would incorrectly become `0` or `false` if it could not be parsed as the specified type (e.g., number). It now safely falls back to preserving the original string, preventing data corruption.

---

We believe v1.1.0 will bring you a more powerful and reliable AI proxy experience. We have already felt the significant improvements ourselves while using the proxy module to connect clients like `claude code`, `zed`, and `crush`.
