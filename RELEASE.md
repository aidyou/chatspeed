[简体中文](./RELEASE.zh-CN.md) ｜ English

# Chatspeed v1.1.9 Release Notes

**Version 1.1.9 focuses on critical bug fixes and core stability improvements, addressing a series of issues related to tool calling, MCP proxy, and cross-platform compatibility to make your AI interaction experience smoother and more reliable.**

### 🐞 Bug Fixes

-   **Fixed issue where tools become unavailable after MCP proxy restart**: Resolved a critical bug where the client's tool mapping was lost due to an MCP server restart, ensuring tool availability and session continuity after reconnection.
-   **Fixed multi-turn tool call failures**: Fixed an issue where tool calls failed in consecutive multi-turn conversations because the tool list was not passed correctly.
-   **Fixed Scraper freeze issue**: Resolved a problem where the built-in web scraper could become unresponsive after a period of use, improving long-term stability.
-   **Fixed tool call UI click event bug**: Fixed a bug where the click event on the tool call display component in the chat interface was not working.
-   **Fixed macOS window border issue**: Addressed an issue on the macOS platform where the application window would sometimes display an unexpected border, affecting aesthetics.
-   **Fixed tool call parameter type issue**: Fixed a bug in the proxy module's compatibility mode where integer types, after being parsed and stored, could be incorrectly converted to floating-point numbers on output.
-   **Resolved local Windows development build failures**: Fixed build failures in the local Windows (especially ARM64) development environment caused by `vcpkg` dependency linking issues. This unifies the build process across platforms and paves the way for CI/CD optimizations.
-   **Fixed Windows initialization pop-up**: Resolved an issue on Windows where the MCP module would unexpectedly open a command-line window during initialization due to executing an external command, optimizing the application's silent startup experience.

### ✨ Enhancements

-   **Unified Cross-Platform Visual Experience**: Refactored the window's rounded corners and shadow effects to ensure a consistent and polished visual appearance across Windows, macOS, and Linux.
-   **Optimized `ccproxy` module**: Further optimized the tool compatibility mode and calling instructions in `ccproxy`, significantly reducing token consumption while maintaining usability, resulting in a smoother and more cost-effective workflow.
-   **Dependency Updates**: Upgraded project dependencies, including the core `rmcp` library from v0.5 to v0.6, for better performance and stability.

---

# Chatspeed v1.1.8 Release Notes

This update focuses on **comprehensive user experience optimizations** and **core feature enhancements**, bringing you a smoother, smarter, and more personalized experience.

### ✨ UI & Interaction Optimizations

- **Unified Layout**: Redesigned the toolbars at the top of the main interface and below the input box for a more unified and convenient interactive experience.
- **MCP Switch**: Added a global switch for the Model Context Protocol (MCP) on the main interface to easily enable or disable it.
- **Customizable Send Key**: Added an option in Settings to switch the send message shortcut between `Enter` and `Shift+Enter` to suit different user habits.

### 🚀 Efficiency Improvements

- **New Conversation Shortcut**: Added a global `Cmd/Ctrl + N` shortcut to quickly start a new chat anytime.
- **Toggle Sidebar**: Added a `Cmd/Ctrl + B` shortcut to quickly expand or collapse the conversation list sidebar.

### 🧠 Core AI Enhancements

- **Global System Prompt**: Introduced a new system prompt to help the AI better understand your environment and intent, providing more intelligent responses.
- **API Compatibility Fix**: Resolved a format compatibility issue with Tool Calls in chat history that caused API request failures.
- **Markdown Parsing Fix**: Fixed a conflict between the reference format `(^id)` and standard Markdown syntax, ensuring correct link rendering.

---

# Chatspeed v1.1.7 Release Notes

v1.1.7 is an emergency hotfix release that resolves an issue preventing model addition due to simplified model configuration.

## 🔧 Emergency Fix

- **Fix Model Addition Issue**: Resolved a critical issue where users were unable to add new models due to changes in the model configuration structure.

---

# Chatspeed v1.1.6 Release Notes

v1.1.6 is a hotfix release that addresses front-end display issues and updates dependencies.

## 🔧 Bug Fixes & Optimizations

- **Fix Front-end Display Issue**: Resolved a bug in Assistant window when parsing AI data from the server, which caused any text to incorrectly trigger tool flags and result in unexpected line breaks in the frontend display
- **Dependency Update**: Updated element-plus dependency from v2.8.5 to v2.11.0

---

# Chatspeed v1.1.5 Release Notes

v1.1.5 is a hotfix release that incorporates all features planned for prior versions and resolves the build-breaking lifetime issue.

## ✨ Enhancements & New Features

- **Single-Instance Lock**: Implemented a single-instance lock to prevent multiple application instances from running simultaneously. This resolves an issue on macOS where the app would launch twice on system restart if "launch at login" was enabled.
- **Scraper Proxy for Windows**: The web scraper on Windows will now use the in-app proxy settings for non-authenticated HTTP proxies, improving network configuration consistency.

## 🔧 Bug Fixes & Optimizations

- **Fixed Build Error**: Resolved a Rust lifetime error (E0597) related to the state lock during the creation of the scraper webview, which was preventing previous versions from compiling.
- **Missing Language Options**: Added missing language options to the translation skill.
- **Scraper Reliability**: Improved the reliability of the generic web scraper by implementing a retry mechanism with exponential backoff for pages that load content dynamically.

---

# Chatspeed v1.1.2 Release Notes

Version 1.1.2 focuses on optimizing tool-calling functionality and user experience, enhancing stability and compatibility by refactoring core modules.

## ✨ New Features & Major Improvements

1.  **New Tool Call Architecture**
    - Refactored the chat module to unify all models through ccproxy, enabling tool-calling capabilities for all models.
    - Simplified model settings by removing the previous "Tool-Calling Capability" switch; all supported models now have tool-calling enabled by default.
    - Optimized the parsing and display of tool calls, showing them and their results in real-time during AI conversations.
    - Added support for displaying complete tool call records in chat history.

2.  **Improved UI**
    - Added a network control switch, allowing users to easily enable or disable web search functionality.
    - Improved chat interface interaction; tool calls are now displayed as a separate component that can be expanded or collapsed to view parameters and results.
    - Optimized the model addition process, fixing an issue where the "Import Model" button might not be displayed.

## 🔧 Technical Optimizations

- Refactored tool-calling related components to improve code maintainability.
- Optimized the ccproxy authentication mechanism to support internal request validation.
- Improved the tool call parsing logic to enhance support for complex parameters.
- Updated the display style for tool calls to provide a better visual experience.
- Optimized the tool compatibility mode in the ccproxy module to improve adaptability for different models.
- **Optimized Key Rotation Mechanism**: Refactored the proxy's key management, using atomic updates to ensure the stability and reliability of multi-key rotation and fixing potential race conditions.

## 🐞 Bug Fixes

- Fixed an issue where the "Import Model" button might not be displayed when adding a model.
- Fixed an issue where tool call results were not displayed completely.
- Optimized network request handling to improve stability.
- Fixed several UI display issues to enhance user experience.

---

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
