# GPUI 与 Vue 工作流：第二批差异清单

审计基线：`bfdd672a`。此页只记录下一批候选工作，不代表已实现。Vue 是功能与视觉基准；只有 Vue 本身有问题、原生平台有限制或原生方案确实更优时才保留差异，并记录原因和验证结果。

## 第二批：缺失功能与交互

| 模块 | 下一批对齐范围 | Vue 参考 |
| --- | --- | --- |
| 浮动状态面板 | 补独立的可折叠、拖动面板及折叠入口。对齐主代理的上下文用量、待办进度、待办和近期操作；子代理摘要；模型状态；效率分析及刷新、加载、失败和空态。不要把它与输入框上方的 `StatusNotifier` 混同。 | `src/components/workflow/StatusPanel.vue`；`src/views/Workflow.vue` 中的浮动挂载点 |
| 会话 | 补列表中的编辑、删除及相应确认和选中态恢复。 | `src/components/workflow/WorkflowSidebar.vue` |
| 自动化 | 补列表、搜索、编辑、启停、删除、立即运行、运行历史、每日多时点/间隔/单次排程与计划审核。 | `src/components/workflow/automation/WorkflowAutomationEditor.vue` |
| 文件工作区 | 补授权目录排序、刷新、复制及文件管理器入口；树节点 `@` 引用、Git 状态、文本编辑及图片、音视频等预览。新任务授权目录引导不等待本批。 | `src/components/workflow/FileTree.vue`、`src/components/workflow/WorkflowCodeEditor.vue` |
| 终端和右侧停靠区 | 终端、多标签及工作区状态；插件、ChatHub、技能等停靠视图。全局设置当前委托 Vue 桌面打开，后续讨论是否要做原生设置中心，不默认替换。 | `src/components/workflow/TerminalPanel.vue`、`src/components/workflow/DockedViews.vue`、`src/views/Settings.vue` |
| 配置面板能力 | 模型显式提供商/代理模式、筛选、温度、上下文、最大 token 和思考预算；技能搜索与保存/取消；MCP 实时状态；工具分栏与 Shell 规则；沙箱方案过滤与禁用。第一批的即时反馈不等同于补全这些功能。 | `src/components/workflow/WorkflowInputArea.vue`、`WorkflowModelSelector.vue`、`WorkflowSkillsSelector.vue` |
| 消息、滚动和样式 | 会话骨架、整条消息复制、复杂 Markdown 和外链、自然高度折叠、二次布局后的尾部跟随、输入区高度/渐变、带引号路径补全与剩余状态文案。 | `src/components/workflow/WorkflowMessageList.vue`、`MarkdownSimple.vue`、`StatusNotifier.vue`、`src/composables/workflow/useWorkflowScroll.js` |
| 窗口与偏好 | macOS 全屏标题栏行为、跨客户端侧栏偏好。安全的位置恢复是前轮剩余项，不丢弃；原生 Wayland 位置由合成器决定。 | `src/components/window/Titlebar.vue`、`src-tauri/src/window.rs` |

## 当前进度

- `6d673f10` 修复回退响应：GPUI 读取运行时返回对象中的 `deleted` 布尔字段，不再把整个对象解码为布尔值。
- 会话改名、删除及删除确认已接入；删除当前会话后尝试选中剩余列表的第一项。尚未做图形界面验收。
- 自动化已接入列表、标题搜索、启停、立即运行、删除确认、运行历史和三种排程。编辑支持名称、内联指令和排程，更新校验原始修订号并保留未编辑的字段。计划审核、Vue 表单细节及真实界面验收仍待补齐。
- 子代理独立窗格已接入实时订阅、Markdown、审批和问答；标题栏的子会话待办在子窗格打开。回放验证了重开隔离和传输目标；仍需验证真实界面回调、重复提交保护及失败恢复。
- 主窗格支持整条消息复制、已有会话的本地文本草稿和带引号路径补全。文件树支持 `@` 引用及 Git 状态标记。技能支持搜索、保存和取消。
- GPUI 发送图片前将本地图片编码为 `data:` URL，专用 runtime 接口负责选模型和分析，不经工具分发。已验证输入边界、鉴权和意外工具响应的拒绝逻辑；尚未用真实视觉模型完成端到端验收。图片附件尚未随草稿持久化。
- 按最新要求，插件停靠暂缓，ChatHub 放在最后，均不计为已完成。

## 不推迟到第二批

- 第一批：审批、发送/停止/规划限制、图片附件与草稿、设置响应、消息队列、子代理卡及其独立消息界面。子代理面板不应继续嵌在卡片下方。按最新反馈，GPUI 不显示 Shell 流式输出，仅保留命令下方的运行状态。
- 前轮未闭环：等待/运行徽章与思考旋转、最后一组活动工具扫光、置顶图标、窗口位置的跨平台校验。审批、停止和设置响应已归入第一批，不重复排期。
- 无图形环境时只可认定代码和测试结果，不能认定动效、显示器布局或像素级样式通过。
