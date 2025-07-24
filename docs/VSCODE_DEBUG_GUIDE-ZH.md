# VS Code Tauri v2 + Vue3 调试指南

本指南专门针对 VS Code 编辑器中调试 Tauri v2 + Vue3 应用程序。

## 🚀 快速开始

### 推荐调试流程

1. **一键启动调试**：
   - 按 `F5` 或 `Ctrl+Shift+D` 打开调试面板
   - 选择 **"🌟 Full Stack Debug (Recommended)"**
   - 这会自动启动前端开发服务器和后端调试

2. **分步调试**（如果一键启动有问题）：
   - 选择 **"🔧 Tauri Backend Only"**
   - 手动启动前端：在终端运行 `yarn dev`

## 📋 调试配置说明

### 1. 🚀 Tauri Development (Full Stack)
- **用途**：完整的全栈开发调试
- **特点**：
  - 自动启动前端开发服务器
  - 设置开发环境变量
  - 完整的错误回溯
- **使用场景**：日常开发，首选配置

### 2. 🔧 Tauri Backend Only
- **用途**：仅调试 Rust 后端
- **特点**：
  - 需要手动启动前端
  - 专注于后端逻辑调试
  - 更快的启动速度
- **使用场景**：后端逻辑开发，前端已稳定

### 3. 🏗️ Tauri Production Debug
- **用途**：生产模式调试
- **特点**：
  - 使用 release 构建
  - 性能接近生产环境
  - 优化的代码调试
- **使用场景**：性能测试，生产问题排查

### 4. 🧪 Tauri Test Debug
- **用途**：单元测试和集成测试
- **特点**：
  - 运行测试套件
  - 完整的错误回溯
  - 测试环境配置
- **使用场景**：测试驱动开发，bug 修复验证

### 5. 🎯 Tauri with Custom Args
- **用途**：使用自定义参数调试
- **特点**：
  - 可传递命令行参数
  - 灵活的启动配置
  - 支持不同运行模式
- **使用场景**：特殊场景测试，功能验证

### 6. 🔗 Attach to Running Tauri Process
- **用途**：附加到已运行的进程
- **特点**：
  - 不重新启动应用
  - 调试正在运行的实例
  - 适合长时间运行场景
- **使用场景**：生产环境调试，进程分析

## 🛠 可用任务

通过 `Ctrl+Shift+P` → "Tasks: Run Task" 可以运行：

### 开发任务
- **start-frontend-dev-server**：启动 Vue3 开发服务器
- **prepare-debug**：准备调试环境（语言文件等）
- **tauri-dev**：启动完整 Tauri 开发环境

### 构建任务
- **cargo-check-tauri**：检查 Rust 代码语法
- **cargo-build-tauri-debug**：构建 Debug 版本
- **cargo-build-tauri-release**：构建 Release 版本
- **build-frontend-production**：构建前端生产版本

### 维护任务
- **clean-all**：清理所有构建缓存
- **kill-dev-server**：停止开发服务器
- **install-dependencies**：安装项目依赖

## 🔧 断点和调试技巧

### Rust 后端调试
1. **设置断点**：
   - 在 `.rs` 文件中点击行号左侧设置断点
   - 使用条件断点：右键断点 → "Edit Breakpoint"

2. **变量检查**：
   - 悬停查看变量值
   - 在 "Variables" 面板查看作用域变量
   - 在 "Watch" 面板添加表达式

3. **调用堆栈**：
   - "Call Stack" 面板显示函数调用链
   - 点击堆栈帧切换上下文

### Vue3 前端调试
1. **浏览器开发者工具**：
   - 在应用窗口右键 → "Inspect Element"
   - 使用 Vue DevTools 扩展

2. **源码映射**：
   - TypeScript/JavaScript 断点会自动映射
   - 在 Sources 面板设置断点

## 🔍 故障排除

### 问题：前端服务器启动失败
**症状**：`start-frontend-dev-server` 任务失败

**解决方案**：
```bash
# 检查端口占用
lsof -ti:1420

# 杀死占用进程
kill -9 $(lsof -ti:1420)

# 重新安装依赖
yarn install
```

### 问题：Rust 编译错误
**症状**：构建失败，红色错误提示

**解决方案**：
1. 运行 `cargo-check-tauri` 任务查看详细错误
2. 检查 Rust 代码语法
3. 确保依赖版本兼容

### 问题：调试器无法附加
**症状**：断点不生效，无法暂停执行

**解决方案**：
1. 确保使用 Debug 构建（不是 Release）
2. 检查 `launch.json` 中的 `sourceLanguages` 设置
3. 重新启动 VS Code 和调试会话

### 问题：环境变量不生效
**症状**：`RUST_LOG` 等环境变量无效

**解决方案**：
1. 检查 `launch.json` 中的 `env` 配置
2. 确认终端环境变量设置
3. 重启 VS Code 使设置生效

## 📁 项目结构和配置文件

```
chatspeed/
├── .vscode/
│   ├── launch.json          # 调试配置
│   ├── tasks.json           # 任务配置
│   ├── settings.json        # 工作区设置
│   └── extensions.json      # 推荐扩展
├── src-tauri/
│   ├── Cargo.toml          # Rust 项目配置
│   ├── tauri.conf.json     # Tauri 配置
│   └── src/                # Rust 源码
├── src/                    # Vue3 前端源码
├── Makefile               # 构建脚本
└── VSCODE_DEBUG_GUIDE.md  # 本指南
```

## ⚙️ 高级配置

### 自定义调试配置
在 `launch.json` 中添加新配置：
```json
{
  "type": "lldb",
  "request": "launch",
  "name": "My Custom Debug",
  "cargo": {
    "args": ["build", "--manifest-path=./src-tauri/Cargo.toml", "--features", "my-feature"]
  },
  "env": {
    "MY_ENV_VAR": "value"
  }
}
```

### Rust-analyzer 优化
在 `settings.json` 中调整：
```json
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "rust-analyzer.cargo.allFeatures": true,
  "rust-analyzer.inlayHints.typeHints.enable": false
}
```

### 性能调试
1. **内存使用**：
```json
{
  "env": {
    "RUST_LOG": "debug",
    "RUST_BACKTRACE": "full"
  }
}
```

2. **性能分析**：
```bash
# 使用 perf 工具
cargo build --release
perf record target/release/chatspeed
perf report
```

## 🎯 最佳实践

### 1. 开发工作流
1. 始终使用 "🚀 Tauri Development (Full Stack)" 开始
2. 遇到问题时切换到 "🔧 Tauri Backend Only"
3. 定期运行 `cargo-check-tauri` 检查代码质量

### 2. 调试策略
1. **逐步调试**：从简单场景开始，逐步增加复杂度
2. **日志先行**：在关键位置添加 `log::debug!()` 语句  
3. **单元测试**：使用 "🧪 Tauri Test Debug" 验证单个功能

### 3. 性能优化
1. 开发时使用 Debug 构建
2. 性能测试使用 "🏗️ Tauri Production Debug"
3. 监控内存使用和 CPU 占用

### 4. 团队协作
1. 统一使用相同的 VS Code 配置
2. 定期更新 `.vscode/` 配置文件
3. 文档化特殊调试场景

## 📞 获取帮助

### 常用命令
```bash
# 检查环境信息
yarn tauri info

# 清理并重新开始
make clean && yarn install

# 检查 Rust 工具链
rustc --version && cargo --version
```

### 日志查看
- **后端日志**：VS Code Debug Console
- **前端日志**：浏览器开发者工具 Console
- **构建日志**：VS Code Terminal 面板

### 相关资源
- [Tauri v2 官方文档](https://v2.tauri.app/)
- [VS Code 调试指南](https://code.visualstudio.com/docs/editor/debugging)
- [Rust-analyzer 用户手册](https://rust-analyzer.github.io/manual.html)
- [Vue.js 开发者工具](https://devtools.vuejs.org/)

## 🔄 配置版本历史

- **v1.0**：基础调试配置
- **v1.1**：添加生产模式调试
- **v1.2**：优化任务依赖和错误处理
- **v1.3**：添加测试调试和自定义参数支持
- **v1.4**：完善前端调试配置和文档

最后更新：2024年