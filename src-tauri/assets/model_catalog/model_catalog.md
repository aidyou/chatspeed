# 模型目录配置说明

配置文件：`src-tauri/assets/model_catalog/model_catalog.json`

模型目录是 ChatSpeed 后端使用的模型能力与请求传输适配规则。Rust 会在编译时通过 `include_str!` 内嵌此文件，修改后需要重新构建应用才会生效。该文件不是供应商配置，也不是用户运行时配置；不要写入 API Key、密钥或其他敏感信息。

完整字段规范、通配符规则、优先级、冲突处理、Transport 适配器和维护要求请参阅仓库中的 `src-tauri/assets/model_catalog.md`。关键规则：`*` 匹配任意长度字符，`?` 匹配一个字符；无通配符时是完整匹配；数组 pattern 是“或”关系；数值越大的 `priority` 越优先；未知能力使用 `null` 或省略；`recommendedTemperature` 只用于新建/导入预填，不覆盖用户配置。
