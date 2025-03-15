# ChatSpeed 归档代码 / ChatSpeed Archived Code

## 中文说明

本目录包含了 ChatSpeed 项目中暂时不使用但计划在未来版本中可能重新启用的代码模块。这些模块主要包括：

### 插件系统 (plugins)

原设计基于 Python 和 Deno 的插件系统，允许用户通过编写脚本扩展 ChatSpeed 的功能。该系统支持：
- Python 脚本插件：利用 Python 生态系统的丰富库和工具
- Deno JavaScript/TypeScript 插件：提供安全的 JavaScript 运行环境
- 插件管理界面：安装、配置和管理插件的用户界面

### 工作流系统 (workflow)

原设计的工作流系统，允许用户创建自动化流程，将多个 AI 操作和数据处理步骤连接起来。该系统支持：
- 可视化工作流编辑器
- 预定义工作流模板
- 工作流执行引擎
- 数据流和状态管理

## 归档原因

在 MVP (最小可行产品) 阶段，我们决定优先实现核心聊天功能，将插件和工作流系统推迟到后续版本。这些代码被归档以便：
1. 保持当前代码库的简洁性
2. 减少维护负担
3. 在未来需要时能够方便地重新集成这些功能

---

## English Description

This directory contains code modules from the ChatSpeed project that are temporarily not in use but may be reactivated in future versions. These modules include:

### Plugin System (plugins)

The originally designed plugin system based on Python and Deno, allowing users to extend ChatSpeed's functionality by writing scripts. This system supports:
- Python script plugins: Leveraging the rich libraries and tools in the Python ecosystem
- Deno JavaScript/TypeScript plugins: Providing a secure JavaScript runtime
- Plugin management interface: User interface for installing, configuring, and managing plugins

### Workflow System (workflow)

The originally designed workflow system, allowing users to create automated processes that connect multiple AI operations and data processing steps. This system supports:
- Visual workflow editor
- Predefined workflow templates
- Workflow execution engine
- Data flow and state management

## Reason for Archiving

During the MVP (Minimum Viable Product) phase, we decided to prioritize core chat functionality and postpone the plugin and workflow systems to subsequent versions. This code has been archived to:
1. Maintain the simplicity of the current codebase
2. Reduce maintenance burden
3. Allow for easy reintegration of these features when needed in the future
