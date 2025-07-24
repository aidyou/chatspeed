# Tauri v2 + Vue3 调试指南

本指南介绍如何在 Zed 编辑器中调试 Tauri v2 应用程序。

## 🚀 快速开始

### 方法1：完整应用调试（推荐）

1. 在 Zed 中按 `F4` 或 `Cmd+Shift+P` → "debugger: start"
2. 选择 **"Debug Tauri App (Full Stack)"**
3. 这会自动启动前端开发服务器和后端调试

### 方法2：分步调试

如果方法1不工作，使用分步调试：

1. **启动前端开发服务器**：
   - 按 `Cmd+Shift+P` → "task: spawn"
   - 选择 **"Start Frontend Dev Server"**
   - 或在终端运行：`yarn dev`

2. **等待前端服务器启动**（通常在 http://localhost:1420）

3. **启动后端调试**：
   - 按 `F4` → "debugger: start"
   - 选择 **"Debug Backend (Dev Mode)"**

## 📋 可用的调试配置

### 1. Debug Tauri App (Full Stack)
- **用途**：完整的应用调试
- **特点**：自动启动前端和后端
- **推荐场景**：日常开发调试

### 2. Build & Debug Rust Backend
- **用途**：仅调试 Rust 后端
- **特点**：需要手动启动前端
- **推荐场景**：专注于后端逻辑调试

### 3. Debug Backend (Dev Mode)
- **用途**：开发模式下调试后端
- **特点**：禁用默认特性，连接到开发服务器
- **推荐场景**：前端已经在运行时使用

### 4. Debug with Custom Args
- **用途**：使用自定义参数调试
- **特点**：可传递命令行参数
- **推荐场景**：需要特定启动参数时

## 🛠 可用的任务

通过 `Cmd+Shift+P` → "task: spawn" 可以运行以下任务：

- **Start Frontend Dev Server**：启动 Vue3 开发服务器
- **Build Frontend**：构建前端资源
- **Tauri Dev (Full Stack)**：启动完整的 Tauri 开发环境
- **Tauri Build**：构建发布版本
- **Cargo Check (Tauri)**：检查 Rust 代码
- **Cargo Build (Tauri)**：构建 Rust 后端
- **Cargo Build (Dev Mode)**：以开发模式构建
- **Tauri Info**：显示环境信息
- **Clean All**：清理所有构建缓存

## 🔧 故障排除

### 问题：界面看不到，但程序正在运行

**症状**：程序启动正常，状态栏和 dock 有图标，但窗口不显示

**解决方案**：
1. 确保前端开发服务器在 http://localhost:1420 运行
2. 检查 `tauri.conf.json` 中的 `devUrl` 配置
3. 使用 "Debug Backend (Dev Mode)" 配置

### 问题：cargo tauri 命令不存在

**症状**：`error: no such command: 'tauri'`

**原因**：Tauri v2 CLI 通过 npm/yarn 安装，不是 cargo 子命令

**解决方案**：使用 `yarn tauri` 而不是 `cargo tauri`

### 问题：端口被占用

**症状**：前端服务器无法启动，端口 1420 被占用

**解决方案**：
```bash
# 查找占用端口的进程
lsof -ti:1420

# 杀死进程
kill -9 $(lsof -ti:1420)

# 或使用调试脚本
./debug.sh
```

### 问题：调试器无法附加

**症状**：调试器启动但无法设置断点

**解决方案**：
1. 确保使用 `CodeLLDB` 适配器
2. 检查是否在 Debug 模式下构建
3. 尝试重新构建：`cargo build --manifest-path src-tauri/Cargo.toml`

## 📁 文件结构

```
chatspeed/
├── .zed/
│   ├── debug.json          # 调试配置
│   └── tasks.json          # 任务配置
├── src-tauri/
│   ├── Cargo.toml          # Rust 项目配置
│   ├── tauri.conf.json     # Tauri 配置
│   └── src/
│       └── main.rs         # Rust 主程序
├── src/                    # Vue3 前端源码
├── dist/                   # 构建输出
├── debug.sh                # 调试辅助脚本
└── DEBUG_GUIDE.md          # 本指南
```

## 🎯 最佳实践

### 1. 开发流程
1. 在一个终端窗口保持 `yarn dev` 运行
2. 在 Zed 中使用 "Debug Backend (Dev Mode)" 进行后端调试
3. 使用浏览器开发者工具调试前端

### 2. 断点设置
- 在 Rust 代码中设置断点进行后端调试
- 在应用窗口右键 → "Inspect Element" 调试前端

### 3. 日志查看
- 后端日志：在 Zed 调试控制台查看
- 前端日志：在浏览器开发者工具 Console 查看
- 设置 `RUST_LOG=debug` 环境变量获取详细日志

### 4. 性能调试
- 使用 Release 模式构建进行性能测试
- 启用 Rust 的性能分析工具
- 使用浏览器性能工具分析前端

## 📞 获取帮助

如果遇到问题：

1. 检查 `yarn tauri info` 输出的环境信息
2. 查看 Zed 的调试输出面板
3. 检查终端中的错误信息
4. 参考 [Tauri 官方文档](https://tauri.app/v1/guides/debugging/application)
5. 使用 `./debug.sh` 脚本进行自动化调试

## 🔗 相关链接

- [Tauri v2 文档](https://v2.tauri.app/)
- [Zed 调试器文档](https://zed.dev/docs/debugger)
- [CodeLLDB 文档](https://github.com/vadimcn/codelldb)
- [Vue.js 调试指南](https://vuejs.org/guide/scaling-up/tooling.html#browser-devtools)