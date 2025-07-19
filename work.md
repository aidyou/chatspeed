plan.md:13
```markdown
**目标：** 使用 Tauri 的 Webview 创建一个内置的网页内容抓取器，作为新的 `WebScraperTool`，以替代原先的 chp工具。

-   **[ ] 后端开发 (Rust + Tauri)**
    -   **[ ] 创建 Tauri 指令:** 在 `src-tauri/src/commands/` 下创建一个新指令，例如 `scrape_url(url: &str, selector: Option<&str>) -> Result<String>`。
    -   **[ ] Webview 实现:**
        -   该指令将创建一个隐藏的 `tauri::WebviewWindow`。
        -   控制 Webview 导航到指定的 `url`。
        -   待页面加载完成后，通过 `webview.eval()` 执行一段 JavaScript 代码。
        -   该 JS 代码将根据 `selector` 抓取页面内容（如果 `selector` 为空，则抓取 `body.innerText`），并将结果返回给 Rust 后端。
    -   **[ ] 创建新工具:** 在 `src-tauri/src/workflow/tools/` 中创建 `WebScraperTool`，它接收 `url` 和 `selector` 作为参数，并调用 `scrape_url` 指令。
```

我们要根据这里的计划，利用tauri开始构建爬虫，爬虫包含 2 部分，一部分是根据网址爬取数据，另外一部分是根据关键词在对应的搜索引擎爬取搜索结果，他们共同的规则文件存储在 @/src-tauri/assets/schema ，其中 `search` 目录是存储某些支持的搜索引擎的规则，而 `content` 目录则是存储了一些支持的网站的内容抽取规则。

规则文件处理：
@/src-tauri/assets/schema 目录下的规则文件应在启动时，根据程序数据目录的`schema`目录是否存在相同文件，如果存在则跳过不存在则创建并将内容写入。
提供定时更新规则文件和手动更新规则文件的功能，其中手动更新应通过前端发送 command 命令来执行。所以规则文件的我们应该有个配置文件存储在 github 或者其他可以方便下载的地方，通过这个经常性更新。

内容爬取规则：
- 查找 `content` 目录下是否有完整域名，如果存在则利用该域名配置的规则进行内容抽取，不存在完整域名规则则继续下列逻辑
- 查找 `content` 目录下是否有子域名，如果存在则利用该子域名配置的规则进行内容抽取，如果不存在则继续下面逻辑
- 采用通用抽取规则：
  - 抽取出 `body` 元素
  - 利用 `header`、`footer`、`nav` 等规则去除一些与内容无关的区域
  - 去除 `script`、`style`、`noscript` 等非内容区域块级元素
  - 去除常见的一些广告元素、空行（比如`<p></br></p>`）等
  - 转为 markdown 格式文本

搜索引擎爬取规则：
- 查找 `search` 的规则文件，如果规则文件不存在，则返回错误（不支持指定的搜索引擎）
- 根据规则文件提取内容返回通用的搜索结果