# ChatSpeed 项目开发与推广计划

## 摘要

本项目旨在通过增强核心工作流功能、整合并开发新的搜索工具，以及制定全面的发布与推广策略，来提升 ChatSpeed 的产品竞争力和市场影响力。

---

## 第一阶段：搜索工具整合与开发 (预估 3-5 天)

### 1.1 内置网页抓取工具 (Tauri Webview)

**目标：** 使用 Tauri 的 Webview 创建一个内置的网页内容抓取器，作为新的 `WebScraperTool`，以替代原先的 chp工具。

-   **[ ] 后端开发 (Rust + Tauri)**
    -   **[ ] 创建 Tauri 指令:** 在 `src-tauri/src/commands/` 下创建一个新指令，例如 `scrape_url(url: &str, selector: Option<&str>) -> Result<String>`。
    -   **[ ] Webview 实现:**
        -   该指令将创建一个隐藏的 `tauri::WebviewWindow`。
        -   控制 Webview 导航到指定的 `url`。
        -   待页面加载完成后，通过 `webview.eval()` 执行一段 JavaScript 代码。
        -   该 JS 代码将根据 `selector` 抓取页面内容（如果 `selector` 为空，则抓取 `body.innerText`），并将结果返回给 Rust 后端。
    -   **[ ] 创建新工具:** 在 `src-tauri/src/workflow/tools/` 中创建 `WebScraperTool`，它接收 `url` 和 `selector` 作为参数，并调用 `scrape_url` 指令。

### 1.2 统一搜索工具

**目标：** 将现有的 Google, Serper, Tavily 搜索整合为一个统一的、可在工作流中调用的搜索工具，并废弃旧的 chp 搜索。

-   **[ ] 后端重构 (Rust)**
    -   **[ ] 创建统一接口:** 在 `src-tauri/src/search/` 目录下创建一个新模块，例如 `unified_search.rs`。
    -   **[ ] 定义统一函数:** 实现一个 `search(query: &str, provider: SearchProvider) -> Result<String>` 函数，其中 `SearchProvider` 是一个包含 `Google`, `Serper`, `Tavily` 的枚举。
    -   **[ ] 逻辑迁移:** 将 `src-tauri/src/commands/chat_web_search.rs` 中分散的搜索逻辑重构并迁移到上述统一函数中。
    -   **[ ] 创建新工具:** 在 `src-tauri/src/workflow/tools/` 中创建一个新的 `SearchTool`，它内部调用 `unified_search`。这个工具可以接受 `provider` 作为参数，以增加灵活性。

-   **[ ] 前端集成 (Vue)**
    -   **[ ] 设置选项:** 在设置界面中，允许用户配置默认的搜索引擎，并保存该选项。

---

## 第二阶段：核心功能增强 (Workflow) (预估 5-7 天)

### 2.1 ReAct 自定义工作流

**目标：** 允许用户通过前端界面自定义 ReAct 工作流的 Prompt，实现高度灵活的自定义 Agent。

-   **[ ] 前端开发 (Vue + Tauri)**
    -   **[ ] UI 组件创建:** 在 `src/components/setting/` 目录下创建一个新的 Vue 组件，例如 `WorkflowReactSetting.vue`。
    -   **[ ] Prompt 编辑器:** 在新组件中，提供一个文本域（Textarea），允许用户输入和编辑 ReAct 的 Prompt 模板。
    -   **[ ] 状态管理 (Pinia):** 在 `src/stores/` 中创建或修改一个 store（例如 `settingStore`），用于管理自定义 Prompt 的状态。
    -   **[ ] 保存逻辑:**
        -   添加“保存”按钮，点击后调用 Tauri 后端指令。
        -   创建对应的 Tauri 指令，例如 `save_react_prompt`，并将其添加到 `src-tauri/src/commands/setting.rs` 中。

-   **[ ] 后端开发 (Rust)**
    -   **[ ] 数据持久化:**
        -   在 `src-tauri/src/db/config.rs` 中增加逻辑，将自定义 Prompt 保存到数据库中。
        -   确保应用启动时能从数据库加载已保存的 Prompt。
    -   **[ ] 工作流执行逻辑修改:**
        -   修改 `src-tauri/src/workflow/react/` 目录下的 ReAct 引擎。
        -   在执行工作流前，检查是否存在用户自定义的 Prompt。如果存在，则使用自定义 Prompt；否则，使用默认的内置 Prompt。

### 2.2 DAG 工作流配置文件执行

**目标：** 让用户可以通过加载配置文件来执行预设的 DAG 工作流。

-   **[ ] 配置文件格式定义**
    -   **[ ] 格式选择:** 选用 YAML 格式（例如 `my_workflow.yml`），因其对人类更友好。
    -   **[ ] 结构设计:** 定义 DAG 的结构，包括节点（`nodes`，定义工具和参数）和边（`edges`，定义节点间的数据流）。

-   **[ ] 后端开发 (Rust)**
    -   **[ ] 配置文件解析:**
        -   引入 `serde_yaml` crate 到 `src-tauri/Cargo.toml`。
        -   在 `src-tauri/src/workflow/dag/` 中创建模块，用于解析 YAML 配置文件并构建成内存中的 DAG 图结构。
    -   **[ ] Tauri 指令:**
        -   在 `src-tauri/src/commands/workflow.rs` 中创建一个指令，如 `execute_dag_from_file`，接收配置文件路径作为参数。
    -   **[ ] 执行器集成:**
        -   将解析后的 DAG 图结构传递给 `src-tauri/src/workflow/dag/executor/` 中的执行器来运行。

-   **[ ] 前端开发 (Vue)**
    -   **[ ] UI 交互:** 在界面上添加一个“从文件加载 DAG”的按钮。
    -   **[ ] 文件选择:** 调用 Tauri 的文件对话框 API，让用户选择本地的 `.yml` 配置文件。
    -   **[ ] 触发执行:** 用户选择文件后，调用后端的 `execute_dag_from_file` 指令。

---

## 第三阶段：发布与推广计划 (持续进行)

### 3.1 官方网站 (aidyou.ai)

**目标：** 创建一个专业、信息全面的官方网站，作为产品信息中心和用户下载入口。

-   **[ ] 技术选型:** 建议使用静态网站生成器（如 Astro, Hugo）或现代前端框架（如 Next.js, Nuxt.js），部署在 Vercel 或 Netlify 上以获得最佳性能和 CI/CD 体验。
-   **[ ] 网站内容规划:**
    -   **[ ] 首页:** 清晰的产品价值主张、核心功能亮点（GIF/视频演示）、醒目的下载按钮 (Call to Action)。
    -   **[ ] 功能介绍页:** 详细介绍 ReAct/DAG 工作流、统一搜索、网页抓取等功能。
    -   **[ ] 文档/教程页:** 提供快速上手指南、工作流配置教程等。
    -   **[ ] 博客:** 用于内容营销，定期发布与产品相关的技术文章和使用案例。
    -   **[ ] 关于我们/联系方式:** 增加项目可信度。

### 3.2 市场推广策略

**目标：** 在目标用户群体中建立知名度，吸引早期用户。

-   **[ ] 目标用户:** 开发者、AI爱好者、效率工具用户、研究人员。
-   **[ ] 推广渠道与行动:**
    -   **[ ] Product Hunt:**
        -   准备高质量的产品截图、GIF 和介绍视频。
        -   精心撰写产品描述和第一条评论，引导讨论。
        -   选择合适的时机发布（通常是周中）。
    -   **[ ] Hacker News (Show HN):**
        -   发布 "Show HN: ChatSpeed, a local-first AI workflow tool built with Rust + Tauri"。
        -   重点突出技术栈、本地优先、可定制化等开发者关心的特性。
    -   **[ ] Reddit:**
        -   在 r/rust, r/SideProject, r/software, r/AI 等相关社区发帖。
        -   根据不同社区的文化，定制化帖子内容，避免纯广告。
    -   **[ ] GitHub:**
        -   优化 `README.md`，使其图文并茂，逻辑清晰。
        -   使用 GitHub Releases 发布版本，并撰写详细的 `CHANGELOG`。
        -   积极响应 Issues 和 Pull Requests。
    -   **[ ] 内容营销 (Twitter/X, 博客):**
        -   在 Twitter/X 上持续分享开发进展、小技巧 (#rustlang, #AI, #devtool, #tauri)。
        -   在官网博客上发表文章，如“如何用 ChatSpeed 打造一个自动化研究助手”、“ReAct 与 DAG 工作流在 ChatSpeed 中的实践”等。