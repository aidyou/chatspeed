[简体中文](./RELEASE.zh-CN.md) ｜ [English](./RELEASE.md)

# Release Notes

## [1.2.2]

### 🚀 New Features

- **Proxy Switcher Window**: Introduced a lightweight, dedicated window for quickly switching active proxy groups. Accessible via the system tray, titlebar menu, and global shortcuts.
- **Enhanced Proxy Stats UI**:
  - **Persistent Auto-Refresh**: Auto-refresh state is now persisted in local storage.
  - **Improved Interactivity**: Added sortable columns for Client Model, Backend Model, and various token counts.

### 🪄 Improvements

- **Intelligent Configuration Protection**: Optimized the database restoration process to automatically preserve machine-specific settings, including window positions, sizes, network proxy configurations, and backup paths. This ensures a seamless transition when moving data between different machines.
- **Fair Weight Rotation Logic**: Refactored the proxy rotation engine to group targets by provider. This prevents "weight explosion" where a provider with many keys or model aliases would unfairly dominate traffic distribution.
- **Robust Statistics Recording**: Introduced a RAII-based `StreamStatGuard` to ensure proxy usage statistics are accurately recorded even when streams are prematurely terminated or encounter errors.
- **Refined Proxy Rotator**: Simplified the internal architecture by removing redundant locks, improving performance and maintainability.
- **Cleaner Debug Logs**: Updated VS Code launch configurations to suppress noisy `OS_ACTIVITY_MODE` logs on macOS.

### 🐞 Bug Fixes

- **Startup Panic on Windows**: Fixed a critical issue where the application could panic during startup if window events (Resized/Moved) were triggered before the internal state was fully initialized.
- **Silent Process Execution on Windows**: Fixed an issue where starting MCP servers or environment detection would cause a terminal window (black box) to pop up on Windows. All background processes now run silently.
- **Stat Recording Leak**: Fixed a potential issue where statistics might not be recorded if a streaming response ended unexpectedly.

---

## [1.2.1]

### 🐞 Bug Fixes

- **Privacy Security Hardening (Attachment Sanitization)**: Resolved a critical vulnerability where sensitive data filtering was bypassed for multimodal (array format) messages. Text extracted from attachments (PDF, Word, or image analysis) is now rigorously sanitized before being sent to AI providers.
- **Gemini Protocol Link Calibration**:
  - **URL Generation Fix**: Fixed a logic bug where the protocol colon in `https://` was mistakenly identified as a suffix separator, ensuring valid backend request URLs.
  - **Auth Parameter Persistence**: Corrected an issue where the API Key query parameter was lost during embedding request forwarding.
- **Backend Provider Compatibility**: Fixed 400 errors from strict backends (e.g., Qwen/ModelScope) by ensuring `encoding_format: "float"` is always supplied and empty string parameters are stripped.

---

## [1.2.0]

### 🚀 New Features

- **Full Embedding Proxy Protocol Support**:
  - **Multi-Protocol Integration**: The proxy layer now natively supports embedding protocols for **OpenAI** (`/v1/embeddings`), **Gemini** (`:embedContent`), and **Ollama** (`/api/embeddings`).
  - **Cross-Protocol Auto-Conversion**: Implemented a comprehensive protocol conversion engine. Users can use any client SDK (e.g., OpenAI SDK) to call backend models configured with different protocols.
  - **Intelligent Parameter Alignment**: Added automatic cleaning and completion logic for optional parameters (like `encoding_format`) that certain backends validate strictly. Systems now automatically supply default values (e.g., `float`) when converting from Gemini/Ollama to OpenAI.
  - **Dedicated Rejection Path**: Created a standalone `/v1/claude/embeddings` path for the Claude protocol. Since Claude officially does not support embeddings, this path returns a clear 501 error, keeping the API structure aligned without causing router conflicts.
- **MCP Proxy Router Overhaul**:
  - **Path Unification**: Consolidated MCP-related SSE and HTTP routes under the `/mcp` namespace, introducing `/mcp/sse` and `/mcp/http` as standard endpoints.
  - **State Isolation**: Leveraged `nest_service` to decouple different application states (SharedState vs. ChatState), improving system stability under concurrent requests.
  - **Backward Compatibility**: Maintained the legacy `/sse` entry point (marked as deprecated) to ensure a smooth transition for older clients.

### 🪄 Improvements

- **API Documentation & UI Sync**: Updated the API endpoint help table in the Proxy Settings page, adding full path descriptions and placeholder replacement guides for Embedding services.

### 🐞 Bug Fixes

- **Router Conflict Hardening**: Resolved a critical issue where multiple protocols (OpenAI/Ollama/Claude) would attempt to register the same `/v1/embeddings` path, leading to application panics on startup.

---

## [1.1.26]

### 🚀 New Features

- **Refined Proxy Statistics**: Added "Today" (natural day) dimension to stats, complementing the "Last 24 Hours" view for a more intuitive perspective on daily usage.
- **Custom Header Defense System**:
  - **Full-Chain Filtering**: Built-in blacklist for restricted headers (e.g., `Host`, `Connection`, `Content-Length`) in `should_forward_header` to prevent proxy protocol conflicts.
  - **Source Validation**: Added real-time validation in the AI Engine settings UI. Users are now alerted with localized error messages when trying to save restricted headers, avoiding confusion over failed forwardings.
  - **Prefix-Aware Protection**: The interceptor intelligently strips the `cs-` prefix before performing safety checks, ensuring no restricted headers bypass the filter.
- **Model ID Robustness Enhancement**:
  - **Full-Chain Noise Reduction**: Implemented automatic `.trim()` handling in both backend forwarding logic (Body injection and URL splicing) and frontend model settings (Saving and Importing). This eliminates API authentication or matching failures caused by accidental leading/trailing spaces in configurations.
  - **Protocol-Aware Model Injection**: For OpenAI, Claude, and Ollama protocols, the proxy now forcibly inserts the correct backend model ID into the request body. This ensures the backend receives the expected ID even if the client omits the `model` field or passes an alias.

### 🪄 Improvements

- **Proxy Statistics Dashboard Optimization**:
  - **Real-time Auto-Refresh**: The dashboard now supports automatic data updates every 10 seconds. Auto-refreshes do not trigger full-screen loading or clear current views, ensuring a smooth and continuous observation experience.
  - **Parallel Data Fetching**: Optimized the statistics dashboard by switching from serial to parallel requests (`Promise.all`), significantly reducing load times.
- **Header Forwarding Architecture Overhaul (Sandwich Logic)**:
  - **Tiered Override Priority**: Refactored the header construction pipeline to establish a strict hierarchy: `Client Standard Headers -> Provider Config Headers -> Protocol Mandatory Headers -> Client cs- Overrides`. This prevents standard client headers from accidentally overriding provider-specific configurations (like custom User-Agents) while maintaining the explicit override power of `cs-` prefixed headers.
  - **Internal Metadata Passthrough**: Restored the forwarding of internal identification headers such as `X-Provider-Id`, `X-Model-Id`, and `X-Internal-Request` for better observability.
- **Token Estimation Calibration**: Fixed a major estimation bug in direct forward mode. The system now counts tokens only for **plain text content**, excluding massive counts caused by binary data like Base64 images.
- **Param Forwarding Strategy Optimization**: Refined the handling of sampling parameters like `stop`. If the configuration is empty or resolves to an empty list, the field is no longer sent, ensuring compatibility with providers that enforce strict validation against empty values.
- **Performance & Stability Enhancements**:
  - **Streaming Robustness**: Restored and optimized buffer flush logic for the Claude protocol, ensuring complete output delivery even during unexpected stream interruptions.

### 🐞 Bug Fixes

- **Assistant Window UX Fix**: Resolved an issue where the Assistant window failed to auto-hide on blur and lacked reliable input focus upon being shown. Optimized focus handling specifically for macOS to ensure a smoother transition.
- **Codebase Cleanup**: Eliminated multiple redundant header forwarding loops in `chat_handler.rs`, streamlining the backend execution path.
- **i18n Synchronization**: Completed translations for "Today" statistics and "Restricted Header" warnings across all supported language packs.

---

## [1.1.25] - 2026-01-28

### 🚀 New Features

- **New Proxy Statistics Feature**:
  - **Comprehensive Data Monitoring**: The system now automatically records all requests passing through the proxy module, including Client Model Alias, actual Backend Model ID, provider, protocol, and HTTP status code.
  - **Dual-Mode Persistence**: Statistics are captured for both protocol conversion and direct forward modes, including detailed error logs for failed requests.
  - **Precise Token Tracking**: Supports cumulative input, output, and cache token counts. Implemented **intelligent estimation logic** based on character counts as a fallback when providers do not return usage data.
- **Brand New Visual Analytics Dashboard**:
  - **Advanced Charting**: Added Backend Model Distribution (Pie), Error Code Distribution (Pie), and Daily Token Trends (Stacked Column).
  - **Interactive Dashboard**: A new "Statistics" tab in Proxy Settings supports daily summaries and multiple time ranges (Last 24h, 7d, 30d, 90d).
  - **Detailed Drill-down**: Expand daily logs to view "Statistics Details," showing the mapping between model aliases and actual backend providers.
  - **Rapid Error Diagnostics**: Click on error counts to instantly view the distribution of error codes and specific failure reasons for that date.

### 🪄 Improvements

- **UI/UX Interaction Enhancements**:
  - **Instant Data Refresh**: Added a refresh button with `row-key` optimization to ensure statistical data and charts sync and re-render instantly.
  - **Refined Scrolling**: Disabled horizontal overscroll bounce effects for a crisper feel. Fixed the expansion column to the left and optimized column widths with text-overflow tooltips.
  - **Smart Unit Formatting**: Large token counts are now automatically converted to "Ten Thousand" (万) or "Hundred Million" (亿) with thousands separators.
- **System Architecture & Robustness**:
  - **Database Evolution**: Implemented V4 migration with the `ccproxy_stats` table and indexes on key fields like `request_at` and `provider` for high-performance queries.
  - **Enhanced Error Defense**: Backend SQL queries now use `COALESCE` protection to eliminate type-parsing errors caused by malformed or missing data.
  - **Memory Management**: Automatic destruction of chart instances upon component unmounting to prevent memory leaks.

---

## [1.1.24] - 2026-01-27

### 🚀 New Features

- **New Proxy Group Switcher Window**:
  - **Instant Summon**: Added a global shortcut (default `Alt+Shift+P`) to quickly open a lightweight switcher panel
  - **Native Interaction**: Supports Arrow keys (`↑`/`↓`) for navigation, `Enter` to activate, and `Esc` to close
  - **Seamless UX**: Automatically hides on blur and includes built-in drag support for effortless placement
  - **Full Support**: Comes with complete localized translations for all 8 languages and optimized startup shortcut registration
- **Enhanced Proxy Server List**:
  - **Accordion Layout**: Refactored the proxy list in Settings into an animated accordion view
  - **Focus Management**: Implemented "Single-Open" logic to maintain focus and improved usability with localized updates for template loading and tool modes
- **Enhanced Attachment Support**: Now supports sending images and various office documents in chat:
  - **Images**: Supported formats include JPG, PNG, GIF, WEBP, SVG, BMP. Supports direct pasting and drag-and-drop.
  - **Office Documents**: Local parsing for PDF, DOCX, XLSX, and XLS formats.
- **Local File Parsing**: All office document parsing (PDF/Word/Excel) is performed locally without relying on external networks or CDNs, ensuring stability even in offline or restricted network environments.
- **Intelligent Vision Model Integration**:
  - Automatically invokes the configured "Vision Model" to generate content descriptions or extract text when images/documents are sent.
  - Description results are automatically prepended to the user's question as context, significantly enhancing the AI's multimodality comprehension.
- **Tool Call Mode Configuration**: Added tool call mode settings in proxy groups with three modes:
  - **URL Setting**: Automatically determines mode based on request path (e.g., `/compat_mode/`), preserving original behavior
  - **Compat Mode**: Forces XML-based tool compatibility mode for models that don't support native tool calls
  - **Native Call**: Forces native tool call format for optimal performance and compatibility
- **Quick Mode Toggle**: Added tool mode toggle icon in proxy group list, supporting one-click cycling between three modes with real-time status display
- **Batch Update Template Loading**: Added "Load from Template" feature in batch update dialog, allowing quick configuration import from existing proxy groups

### 🪄 Improvements

- **Immersive UI/UX Optimizations**:
  - **Instant Feedback**: Clears the input area immediately upon sending and displays processing progress for a smoother interaction.
  - **Visible Progress**: Displays real-time processing steps (e.g., "Analyzing images...", "Generating response...") at the bottom of the chat.
  - **Auto-Scrolling**: The chat window now automatically scrolls to the bottom when progress steps are updated.
  - **Image Preview**: Images in chat bubbles now support click-to-zoom; redundant attachment filenames have been removed.
  - **Error Rollback**: If vision analysis or message storage fails, the system automatically restores backed-up text and attachments to the input area to prevent data loss.
- **Assistant Window Interaction Improvements**:
  - **Layout Refactoring**: Icons have been moved below the text box and arranged horizontally to fix crowding and tooltip overlap issues.
  - **Smart Pinning**: Automatically enables "Always on Top" temporarily when selecting attachments, restoring the original state once the dialog is closed.
- **Settings Optimization**: Added "Clear" buttons for Vision and Title Generation models and fixed a UI bug where cleared selections displayed as `0`.
- **Enhanced Context Restoration**: In multi-turn dialogues, the system now automatically restores vision analysis results from history using XML tags, ensuring the AI maintains full background context even after switching models.
- **Batch Update Dialog Optimization**: Increased dialog height and added scroll support to prevent form content from being obscured by buttons
- **UI Alignment Fix**: Fixed vertical alignment between labels and checkboxes in batch update dialog

### 🐞 Bug Fixes

- **Synchronous Request Fix**: Improved the backend OpenAI protocol handler for non-streaming (synchronous) requests, resolving parsing errors in vision analysis scenarios.
- **Resend Message Fix**: Resolved a logic error where resending a message could lead to sending empty content or losing context.
- **Code Robustness**: Fixed a reference error in `Index.vue` caused by a missing `SkillItem` component import.
- **Tooltip Opacity Fix**: Optimized global CSS variables to fix overlapping text caused by transparent Tooltip backgrounds.

---

## [1.1.23] - 2026-01-22

### 🚀 New Features

- **Model-Specific Custom Body Parameters**: Added support for configuring custom JSON parameters (e.g., `enable_thinking`) at the specific model ID level.
- **Dynamic Group Switching (`/switch`)**: Introduced a unified `/switch` route prefix that dynamically resolves requests based on the currently "Active" proxy group.
- **System Prompt Keyword Replacement**: Implemented KV-based replacement rules in proxy groups to automatically correct System Prompts before forwarding.
- **Proxy Group Batch Update**: Supports synchronizing prompt injection, text replacement, and tool filtering across multiple groups in one transaction.

### 🪄 Improvements

- **Refined Parameter Injection Hierarchy**: Established a strict "Model Patch > User UI Intent > Engine Default" priority system.
- **Smart Type Conversion**: Automatically converts string values like `"true"/"false"` to Booleans and numeric strings to Numbers for provider compatibility.
- **UI Refactoring**: Upgraded the Proxy Group dialog to a tabbed layout, matching the AI Engine settings style and fixing button visibility issues.
- **Enhanced Robustness**: `process_custom_params` now intelligently handles various nested data structures and edge cases.

### 🐞 Bug Fixes

- **ModelScope/Qwen Compatibility**: Automatically forces `enable_thinking` to `false` for non-streaming requests to prevent 400 Bad Request errors.
- **UI State Persistence**: Fixed an issue where internal chat might lose real-time parameters (e.g., temperature, stop sequences) during protocol conversion.
- **Element Plus Adaption**: Resolved deprecation warnings for `el-radio` component APIs.
- **Security Hardening**: Removed HTML tags from i18n messages to eliminate XSS warnings.

---

## [1.1.22] - 2026-01-21

# Chatspeed v1.1.21 Release Notes

Optimized window visibility logic; after the change, pressing the hotkey will only hide the window if it has focus, otherwise it will bring the window to the front.

## 🐞 Bug Fixes

- Fixed an issue in model settings where the `baseUrl` input field became uneditable when switching API protocols.

---

# Chatspeed v1.1.19 Release Notes

Chatspeed v1.1.19 is an important release focused on **system stability improvements** and **data security enhancements**. This update significantly improves troubleshooting efficiency through comprehensive optimization of error handling mechanisms. Additionally, it introduces streaming encryption technology to solve memory consumption issues when backing up large database files, ensuring stable system operation even when processing massive amounts of data. Furthermore, we have fixed multiple key issues affecting user experience, including chat interface parsing errors, new model compatibility, and cross-platform window display issues, providing you with a smoother and more reliable AI interaction experience.

## 🔧 Technical Optimizations

- **Error Serialization Optimization**: Unified and optimized error serialization mechanisms across all modules. Error messages now include specific message content for easier troubleshooting.
- **Window Management Enhancement**: Improved window management mechanisms with debounced shortcut support and better error handling.
- **Web Search Tool Enhancement**: Optimized the implementation of the WebSearch tool, improving the stability and performance of search functionality.
- **Data Backup Optimization**: Optimized database backup logic with new database backup encryption.

## 🐞 Bug Fixes

- Optimized chat interface: Fixed reference parsing errors and resolved redundant chat content caused by incorrect parsing of step information provided by some vendors.
- Added compatibility for claude-sonnet-4-5 model, supporting cases where `thinking.budget_tokens` is greater than or equal to `max_tokens`.
- Optimized the maximum height of settings window to prevent it from exceeding screen display range.
- Removed proxy settings for crawler webview on Windows platform. All platforms now rely on system proxy for webview creation, resolving potential issues with crawler windows on Windows 11.
- Optimized data backup logic, removed support for old databases, and added backup support for mcp_sessions, schema, shared, static, and other directories.

---

# Chatspeed v1.1.18 Release Notes

Chatspeed v1.1.18 further enhances application startup performance and user experience, especially on the Windows platform. By optimizing the environment initialization process, it resolves startup freezes and unexpected command line window pop-ups, and corrects the accuracy of shortcut descriptions in the release notes.

### 🔧 Technical Optimizations

- **Optimized Environment Initialization, Resolved Startup Freezes and Panic**: Refactored the environment initialization process to be asynchronous, resolving potential startup freezes and the "runtime within a runtime" panic on Windows.
- **Error Unification**: Unified the error handling approach across the application to ensure consistency and readability of error messages.

### 🐞 Bug Fixes

- **Fixed Windows Startup Command Line Popup**: Applied `CREATE_NO_WINDOW | DETACHED_PROCESS` flags to child processes executing shell commands during Windows environment initialization, completely eliminating unexpected command line window pop-ups during startup.
- **Corrected Shortcut Description in Release Notes**: Corrected the description of main window positioning shortcuts in the release notes, clarifying that they only support horizontal movement (left and right).
- **ccproxy Fix**: Fixed an issue in the ccproxy module where duplicate content might appear during streaming output in tool compatibility mode.
- **Conversation Fix**: Fixed an issue where conversation list tool call parameter parsing errors caused the list to fail to load.

---

# Chatspeed v1.1.17 Release Notes

Chatspeed v1.1.17 focuses on enhancing cross-platform stability and user experience by addressing critical bugs across Windows and Linux environments, optimizing environment variable loading, and resolving UI state management issues in key settings components. This release also incorporates all features and fixes from the unreleased v1.1.16, including new window positioning shortcuts and unified custom event formats.

### ✨ Enhancements

- **Added Main Window Positioning Shortcuts**: Added main window center, left and right positioning shortcuts to improve window management efficiency
- **Unified Custom Event Format**: Unified all custom events to start with `cs://`, standardizing the application's internal event system

### 🔧 Technical Optimizations

- **Global Shortcut Upgrade**: Upgraded main window positioning shortcuts (to left and right) to global shortcuts, allowing triggering without application being active

### 🐞 Bug Fixes

- **Fixed Windows ARM64 Crash Issue**: Resolved the issue where the Windows ARM64 version crashed after installation in a PD virtual machine, by explicitly specifying the `sqlite3` library path.
- **Fixed Windows Command Line Popup Issue**: Fixed the problem of an unexpected command line window popping up when starting the MCP service on the Windows platform.
- **Fixed Windows MCP Startup Error**: Corrected the MCP service startup error on Windows, where directly running the `npx` script instead of wrapping it with `cmd /c` led to a `%1 is not a valid Win32 application` error.
- **Optimized Log Visibility**: Improved the log output level for shell command failures (e.g., shell not found or internal command failure) when searching for `npx` and similar commands in Linux environments, making them visible in production.
- **Fixed Form State Overwrite Issue**: Addressed a bug in `Agent.vue` and `Skill.vue` components where editing an item and then adding a new one would overwrite the previous item due to un-reset form data.
- **Fixed Initial Language Setting Issue**: Fixed the issue where initial installation language was incorrectly set to Simplified Chinese; now it prioritizes the user's native language with fallback to English if unsupported
- **Fixed Shortcut Key Validation**: Fixed error notifications when shortcut keys are left empty during setting
- **Fixed Shortcut Key Binding Mechanism**: Fixed issue where unset shortcuts caused module shortcut binding failures
- **Fixed Settings Window Switching**: Fixed issue where switching between setting windows via menu fails after opening a settings window
- **Removed Unavailable Vendor**: Removed built-in `Pollinations` vendor as it was mostly unusable after testing
- **Fixed Ubuntu Window Display**: Fixed issue where window could not be shown via shortcut after being minimized on Ubuntu

### 🕰 In Progress

Part of the ReAct logic has been completed, and development of this workflow module will continue

# Chatspeed v1.1.15 Release Notes

v1.1.15 focuses on enhancing the user experience of tool calls and system stability, while optimizing the protocol conversion capabilities of the ccproxy module.

### ✨ Enhancements

1. Optimized the tool call display interface, where multiple tool calls within the same conversation turn are now displayed as a single session
2. Optimized message deletion, where deleting a message will remove all tool call information for the same conversation turn at once
3. Optimized the conversation entry point, where all conversation errors are now notified to the frontend
4. Optimized error messages, returning status codes for HTTP request-related errors to facilitate frontend adaptation based on the situation
5. Optimized Markdown parsing in conversations, using a streaming parsing mechanism which greatly improves parsing efficiency and user experience
6. Added `ALT + ←/→` hotkeys to the main window for quickly moving the window to the bottom-left and bottom-right corners

### 🔧 Technical Optimizations

1. Optimized the conversation history in the ccproxy module, preprocessing conversation records and tool call records before protocol conversion to make them as suitable as possible for various protocols
2. Optimized the ccproxy module, now supporting `*` and `?` wildcards by defining model injection conditions
3. Optimized database upgrade logic for better version update management in the future

### 🐞 Bug Fixes

1. Fixed tool call interruption issues caused by non-standard end flag sending in some models (e.g., gtp-oss), improving system compatibility
2. Fixed an issue in the ccproxy module where tool structures were incorrect during complex scenarios when converting from gemini protocol to claude protocol

### 🕰 In Progress

Part of the ReAct logic has been completed, and development of this workflow module will continue

---

# Chatspeed v1.1.14 Release Notes

This update primarily introduces **proxy alias wildcard support**, significantly enhancing the flexibility and convenience of model configuration. You can now use wildcards like `*` and `?` to define proxy aliases, adapting to changes in model names without frequent configuration modifications. Additionally, we have optimized the search scraper's filtering mechanism, fixed a bug where some tool calls in the chat interface could not be expanded, and optimized the model configuration process.

### ✨ Enhancements

- Optimized the search scraper to automatically filter multimedia websites and files, providing more precise results as requested. The `WebSearch` tool's default number of results has also been adjusted to 5, with a maximum of 30, offering more flexible search control.
- Added proxy alias wildcard support, now allowing `*` for multiple characters and `?` for single characters.

### 🔧 Technical Optimizations

- Adjusted API documentation to adapt to the new MCP proxy.
- Optimized the model configuration process by changing the import button from invisible to disabled when adjusting model settings, improving user experience.

### 🐞 Bug Fixes

- Fixed a bug where some tool calls in the chat interface could not be expanded.

---

# Chatspeed v1.1.13 Release Notes

This update focuses on improving the stability and feature expansion of the MCP proxy, while optimizing assistant features and content extraction capabilities. Regarding the MCP proxy server, we have introduced the Streamable HTTP protocol to solve session loss issues caused by network disconnections and computer hibernation. For the scraper module, we have fixed critical stability issues and adopted Readability to improve content area recognition accuracy, reducing unnecessary token consumption from junk information.

### ✨ Enhancements

- **Added openai reference information extension field**: Added an extension field for reference information to the OpenAI-compatible interface, providing richer reference data.
- **Optimized assistant quick send information feature**: Optimized the experience of quickly sending information with the assistant (default shortcut `ALT+Z`) to improve operational efficiency.
- **Added Streamable HTTP proxy server protocol**: Added the Streamable HTTP proxy server protocol with endpoint `/mcp/http`, providing a more flexible connection method.
- **Implemented Streamable HTTP Session Persistence and Automatic Recovery**: By introducing an advanced session management mechanism, we have achieved disk persistence and automatic recovery for Streamable HTTP sessions. This completely resolves the MCP service session ID loss issue (manifesting as 410 or 401 errors) caused by network disconnections, computer hibernation, or proxy server restarts, significantly enhancing connection stability and user experience.
- **Optimized scraper module content extraction**: Optimized the scraper module content extraction algorithm. The current version can accurately identify content areas when extracting web content, with a hit rate improved to 90% ~ 95%.
- **Refactored AI conversation mechanism**: Refactored the AI conversation mechanism, making the interface better able to display the tool calling process; optimized system prompts to make AI better able to use tools to complete daily conversation tasks.
- **Updated documentation links**: Updated the blog link and English documentation link in README to ensure users can access the latest documentation resources.

### 🔧 Technical Optimizations

- **Cleaned up invalid i18n**: Cleaned up a large amount of invalid internationalization resources, reducing application size and improving performance.
- **Assistant window size adjustment**: Adjusted the assistant window size and implemented automatic restoration based on user size adjustments, enhancing user experience.
- **Archived Rust-implemented ReAct module**: Based on workflow module adjustments, archived the original Rust-implemented ReAct module to prepare for the new workflow architecture.
- **Adopted Readability to improve content recognition accuracy**: The scraper module adopted the Readability algorithm to improve content area recognition accuracy, reducing unnecessary token consumption from junk information.

### 🐞 Bug Fixes

- **Fixed scraper module event listener crash issue**: Fixed the issue where scraper module event listeners could cause the scraper module to crash, significantly improving scraper functionality stability.
- **Removed unused imports and dependencies**: Cleaned up unused imports and dependencies in the project, optimizing project structure and build efficiency.

---

[Here starts the original content from the file]

# Chatspeed v1.1.12 Release Notes

v1.1.12 is a release focused on core stability and key user experience optimizations. We have fixed underlying issues that could cause application crashes and improved the assistant and application startup workflows to be smoother and more intuitive.

### ✨ Enhancements

- **Optimized Assistant Shortcut Workflow**: The assistant shortcut (default `Alt+Z`) now directly sends text from the clipboard for processing, eliminating the need for an extra confirmation step (like pressing Enter) and significantly improving operational efficiency.
- **Improved First-Launch Experience**: To provide a more welcoming initial interface, the application now automatically loads the default conversation on first launch instead of displaying a blank page.

### 🐞 Bug Fixes

- **Fixed Assistant Shortcut Double-Trigger Issue**: Resolved a bug where the assistant shortcut could trigger twice in quick succession under certain conditions, causing the same message to be sent to the AI repeatedly.
- **Fixed Scraper Module Crash**: Addressed a critical issue where improper handling by an event listener in the scraper module could cause the application to panic, enhancing the stability of the web content fetching feature.

### 🔧 Technical Optimizations

- **Optimized Database Initialization Logic**: Removed the hardcoded creation time for the initial default conversation. The timestamp is now dynamically generated when the database is first created, making the logic more robust.

---

# Chatspeed v1.1.11 Release Notes

This update focuses on optimizing the stability and accuracy of AI tool calls. By improving system prompts and parsing logic, we've made it more reliable for the AI to use tools to answer user questions.

### ✨ Enhancements

- Optimized chat system prompts to enable the AI to better utilize tools for answering user questions.
- Improved the usage guidelines for tool compatibility mode, providing clearer examples to help the AI better understand and adhere to tool XML specifications.
- Enhanced the stability of the `WebFetch` tool by preventing it from downloading un-parseable files (like PDF, ZIP), improving the reliability of web scraping.

### 🐞 Bug Fixes

- Fixed a bug in tool compatibility mode where the tool call parser would incorrectly interpret code like `Result<String, String>` as HTML, causing tool call failures.

---

# Chatspeed v1.1.10 Release Notes

This update is dedicated to enhancing the visual experience and AI interaction efficiency. By adding fullscreen display support for SVG charts, we've optimized visual presentation; concurrently, we've streamlined tool compatibility mode prompts, significantly reducing token consumption, and introduced intelligent token estimation, providing you with a smoother, more economical, and transparent AI experience.

### ✨ Efficiency & Experience Improvements

1. Added fullscreen display support for SVG charts, optimizing the visual experience.
2. Optimized tool compatibility mode prompts, removing irrelevant information to significantly reduce token consumption.
3. Introduced estimated values for input and output tokens when AI servers do not provide usage data, primarily to enhance user perception.

---

# Chatspeed v1.1.9 Release Notes

**Version 1.1.9 focuses on critical bug fixes and core stability improvements, addressing a series of issues related to tool calling, MCP proxy, and cross-platform compatibility to make your AI interaction experience smoother and more reliable.**

### 🐞 Bug Fixes

- **Fixed issue where tools become unavailable after MCP proxy restart**: Resolved a critical bug where the client's tool mapping was lost due to an MCP server restart, ensuring tool availability and session continuity after reconnection.
- **Fixed multi-turn tool call failures**: Fixed an issue where tool calls failed in consecutive multi-turn conversations because the tool list was not passed correctly.
- **Fixed Scraper freeze issue**: Resolved a problem where the built-in web scraper could become unresponsive after a period of use, improving long-term stability.
- **Fixed tool call UI click event bug**: Fixed a bug where the click event on the tool call display component in the chat interface was not working.
- **Fixed macOS window border issue**: Addressed an issue on the macOS platform where the application window would sometimes display an unexpected border, affecting aesthetics.
- **Fixed tool call parameter type issue**: Fixed a bug in the proxy module's compatibility mode where integer types, after being parsed and stored, could be incorrectly converted to floating-point numbers on output.
- **Resolved local Windows development build failures**: Fixed build failures in the local Windows (especially ARM64) development environment caused by `vcpkg` dependency linking issues. This unifies the build process across platforms and paves the way for CI/CD optimizations.
- **Fixed Windows initialization pop-up**: Resolved an issue on Windows where the MCP module would unexpectedly open a command-line window during initialization due to executing an external command, optimizing the application's silent startup experience.

### ✨ Enhancements

- **Unified Cross-Platform Visual Experience**: Refactored the window's rounded corners and shadow effects to ensure a consistent and polished visual appearance across Windows, macOS, and Linux.
- **Optimized `ccproxy` module**: Further optimized the tool compatibility mode and calling instructions in `ccproxy`, significantly reducing token consumption while maintaining usability, resulting in a smoother and more cost-effective workflow.
- **Dependency Updates**: Upgraded project dependencies, including the core `rmcp` library from v0.5 to v0.6, for better performance and stability.

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

v1.1.15 is a hotfix release that incorporates all features planned for prior versions and resolves the build-breaking lifetime issue.

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

1. **New Tool Call Architecture**
    - Refactored the chat module to unify all models through ccproxy, enabling tool-calling capabilities for all models.
    - Simplified model settings by removing the previous "Tool-Calling Capability" switch; all supported models now have tool-calling enabled by default.
    - Optimized the parsing and display of tool calls, showing them and their results in real-time during AI conversations.
    - Added support for displaying complete tool call records in chat history.

2. **Improved UI**
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

1. **Refactored the Core Tool-Parsing Engine**
    We have adjusted and introduced a new tool-parsing mechanism. The new engine is more compatible and robust, allowing it to better understand and process diverse output formats from the AI. This not only improves the parsing success rate but also lowers the "difficulty" for the AI to use tools.

2. **Unified & Enhanced Tool Compatibility Mode Across All Protocols**
    Tool Compatibility Mode now covers all protocols, including Gemini and Claude. Regardless of the backend model used, you can now benefit from the smoother workflows and higher goal completion rates provided by our enhanced guidance.

## 🐞 Bug Fixes

- **Fixed "Ghost Model" Issue in Proxy Configuration:** Resolved an issue where deleted backend models would remain in a proxy configuration and could not be removed during editing.
- **Fixed Missing Tool List in Compatibility Mode:** Addressed a bug where the tool list would fail to load into the prompt in certain scenarios when compatibility mode was enabled, ensuring the AI can see and use all available tools.
- **Fixed Imbalanced Global Key Rotation:** Corrected a flaw in the key rotation algorithm that caused some keys to be used too frequently and exceed rate limits, ensuring balanced key usage and service stability.

---

# ChatSpeed v1.1.0 Release Notes

Hello everyone! We are thrilled to announce the release of ChatSpeed v1.1.0. We've jumped directly from v1.0.4 to v1.1.0 because this version includes a series of milestone updates, especially regarding the **proxy module's Tool Compatibility Mode** and **search engine support**, which will significantly enhance the AI's capabilities and stability.

## 🚀 New Features

1. **Powerful Search Engine Integration**
    - Now supports integration with major search engines via API keys, including **Google Search API**, **Tavily API**, and **Serper API**.
    - Additionally, built-in support for **Bing, DuckDuckGo, So.com, and Sougou** is included, providing you with diverse information sources.

2. **Web Content Scraping & Analysis**
    - A new built-in web scraper has been added, capable of extracting text or Markdown content from a given URL, allowing the AI to fetch and analyze real-time web information.

3. **Flexible Prompt Injection**
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
