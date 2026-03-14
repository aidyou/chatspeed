mcp 自动加载工具，类似 skill 在上下文只展示工具的描述，不展示工具的实现代码，引导 ai 需要使用 mcp 工具时通过本工具获取工具详细信息，可以减小上下文 token 浪费。mcp 工具应该可以在聊天和工作流使用，所以 scope 应该是 both。

主要工作：
1. @src-tauri/src/commands/chat.rs 聊天和 @src-tauri/src/commands/workflow.rs 工作流启动时，不再注入 mcp 的工具描述。
2. 聊天的系统提示词注入 mcp 工具的描述，注意只包含 description 不包含参数信息。
3. 加强聊天的 system 提示词引导（参考第五点）
4. 工作流的系统提示词注入 mcp 工具的描述，注意只包含 description 不包含参数信息。
5. 加强系统提示词：在系统核心提示词后面新增一条说明，如果用户安装了 mcp 工具，则可根据调用本工具加载详细调用信息。表达方式优化下，采用英文方式。