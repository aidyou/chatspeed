# Notes: 富文本聊天支持 - 研究笔记

## 聊天界面相关文件

### 主要文件
- `src/views/Assistant.vue` - 聊天主界面，包含输入框和消息显示
  - 使用 `el-input type="textarea"` 作为输入框
  - 支持回车发送消息
  - 有模型选择器 `ModelSelector` 组件
  - 有敏感过滤、网络、MCP等开关

- `src/components/chat/Chatting.vue` - 聊天消息组件
- `src/components/chat/Markdown.vue` - Markdown渲染组件
- `src/components/chat/ModelSelector.vue` - 模型选择器

### 状态管理
- `src/stores/chat.js` - 聊天状态管理
  - 管理对话列表 (`conversations`)
  - 管理当前对话ID (`currentConversationId`)
  - 管理消息列表
  - 使用 `invokeWrapper` 调用Tauri命令

- `src/stores/setting.js` - 设置存储
  - 已有模型配置结构:
    - `conversationTitleGenModel` - 对话标题生成模型
    - `workflowReasoningModel` - 工作流推理模型
    - `workflowGeneralModel` - 工作流通用模型
    - `websearchModel` - 网络搜索模型
  - 可以添加 `visionModel` - 视觉模型

### 后端Tauri命令
- `src-tauri/src/commands/chat.rs` - AI聊天命令
  - `chat_completion` - 发送聊天消息
  - `stop_chat` - 停止聊天
  - 支持多种AI协议
  - 消息格式: `[{ role: 'user', content: '...' }]`

- `src-tauri/src/commands/fs.rs` - 文件系统命令
  - `image_preview` - 图片预览
  - 支持图片缩放和保存到临时目录
  - 返回HTTP服务器URL

## 关键发现

### 现有图片处理能力
- 已有 `image_preview` 命令可以处理图片
- 图片保存到临时目录并通过HTTP服务器访问
- 支持图片缩放（200x200px）

### 消息格式
- 当前消息格式: `[{ role: 'user', content: 'text' }]`
- 需要扩展支持图片: `[{ role: 'user', content: [{ type: 'text', text: '...' }, { type: 'image_url', image_url: { url: '...' } }] }]`

### 模型配置模式
- 现有模型配置使用 `{ id: '', model: '' }` 格式
- `id` - AI provider配置ID
- `model` - 具体模型名称

## 待探索的文件列表
1. src/components/chat/ - 聊天相关组件详情
2. src-tauri/src/ai/ - AI交互实现
3. src-tauri/src/libs/fs.rs - 文件系统工具函数