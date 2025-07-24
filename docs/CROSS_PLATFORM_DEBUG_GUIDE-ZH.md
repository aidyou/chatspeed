# 跨平台调试指南 - Tauri v2 + Vue3

本指南提供在不同操作系统上调试 Tauri v2 + Vue3 应用程序的完整解决方案。

## 🌍 平台支持概览

| 功能 | Windows | macOS | Linux |
|------|---------|-------|-------|
| VS Code 调试 | ✅ | ✅ | ✅ |
| Zed 调试 | ✅ | ✅ | ✅ |
| 前端调试 | ✅ | ✅ | ✅ |
| 后端调试 | ✅ | ✅ | ✅ |
| 自动化脚本 | ✅ | ✅ | ✅ |

## 🔧 平台特定配置

### Windows 配置

#### 必需工具
```powershell
# 安装 Rust
winget install Rustlang.Rust

# 安装 Node.js
winget install OpenJS.NodeJS

# 安装 Yarn
npm install -g yarn

# 安装 Visual Studio C++ Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools
```

#### VS Code 调试器选择
- **推荐**：`cppvsdbg` (Microsoft C++ Debugger)
- **备选**：`CodeLLDB` (需要额外配置)

#### 特殊配置
```json
// .vscode/launch.json - Windows 特定配置
{
  "type": "cppvsdbg",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed.exe",
  "console": "integratedTerminal"
}
```

### macOS 配置

#### 必需工具
```bash
# 安装 Xcode Command Line Tools
xcode-select --install

# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 安装 Node.js (使用 Homebrew)
brew install node

# 安装 Yarn
npm install -g yarn
```

#### VS Code 调试器选择
- **推荐**：`lldb` (CodeLLDB)
- **内置支持**：Xcode 调试工具

#### 特殊配置
```json
// .vscode/launch.json - macOS 特定配置
{
  "type": "lldb",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed",
  "sourceLanguages": ["rust"]
}
```

### Linux 配置

#### 必需工具 (Ubuntu/Debian)
```bash
# 更新包管理器
sudo apt update

# 安装构建工具
sudo apt install build-essential curl wget file

# 安装系统依赖
sudo apt install libwebkit2gtk-4.0-dev \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev

# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 安装 Node.js
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt install nodejs

# 安装 Yarn
npm install -g yarn
```

#### VS Code 调试器选择
- **推荐**：`gdb` (GNU Debugger)
- **备选**：`lldb` (CodeLLDB)

#### 特殊配置
```json
// .vscode/launch.json - Linux 特定配置
{
  "type": "gdb",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed",
  "cwd": "${workspaceFolder}"
}
```

## 🚀 平台特定调试流程

### Windows 调试流程

1. **环境准备**
```powershell
# 克隆项目
git clone <repository-url>
cd chatspeed

# 安装依赖
yarn install

# 检查环境
yarn tauri info
```

2. **VS Code 调试**
- 打开 VS Code
- 选择 "🚀 Tauri Development (Windows MSVC)"
- 按 F5 开始调试

3. **命令行调试**
```powershell
# 启动前端开发服务器
yarn dev

# 在另一个终端中构建并运行 Tauri
yarn tauri dev
```

### macOS 调试流程

1. **环境准备**
```bash
# 克隆项目
git clone <repository-url>
cd chatspeed

# 安装依赖
yarn install

# 检查环境
yarn tauri info
```

2. **VS Code 调试**
- 打开 VS Code
- 选择 "🚀 Tauri Development (Full Stack)"
- 按 F5 开始调试

3. **Zed 调试**
- 打开 Zed
- 按 F4 启动调试器
- 选择 "Debug Tauri App (macOS/Linux)"

### Linux 调试流程

1. **环境准备**
```bash
# 克隆项目
git clone <repository-url>
cd chatspeed

# 安装依赖
yarn install

# 检查环境
yarn tauri info
```

2. **VS Code 调试**
- 打开 VS Code
- 选择 "🚀 Tauri Development (Linux GDB)"
- 按 F5 开始调试

3. **命令行调试**
```bash
# 使用 GDB
cargo build --manifest-path src-tauri/Cargo.toml
gdb ./src-tauri/target/debug/chatspeed
```

## 🔍 平台特定故障排除

### Windows 常见问题

#### 问题：构建失败 "link.exe not found"
**解决方案**：
```powershell
# 安装 Visual Studio Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# 或安装完整的 Visual Studio
winget install Microsoft.VisualStudio.2022.Community
```

#### 问题：PowerShell 执行策略限制
**解决方案**：
```powershell
# 设置执行策略（管理员权限）
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope LocalMachine
```

#### 问题：端口被占用
**解决方案**：
```powershell
# 查找占用端口的进程
netstat -ano | findstr :1420

# 结束进程
taskkill /PID <PID> /F
```

### macOS 常见问题

#### 问题：权限被拒绝
**解决方案**：
```bash
# 给予可执行权限
chmod +x debug.sh

# 检查 Gatekeeper 设置
sudo spctl --master-disable  # 不推荐，仅用于开发
```

#### 问题：Xcode 许可协议
**解决方案**：
```bash
# 接受 Xcode 许可
sudo xcodebuild -license accept
```

### Linux 常见问题

#### 问题：缺少系统依赖
**解决方案**：
```bash
# Ubuntu/Debian
sudo apt install libwebkit2gtk-4.0-dev libssl-dev

# CentOS/RHEL/Fedora
sudo dnf install webkit2gtk3-devel openssl-devel

# Arch Linux
sudo pacman -S webkit2gtk openssl
```

#### 问题：调试器权限
**解决方案**：
```bash
# 允许 ptrace
echo 0 | sudo tee /proc/sys/kernel/yama/ptrace_scope

# 或添加到组
sudo usermod -a -G gdb $USER
```

## 📁 跨平台文件结构

```
chatspeed/
├── .vscode/
│   ├── launch.json          # 跨平台调试配置
│   ├── tasks.json           # 跨平台任务配置
│   ├── settings.json        # 编辑器设置
│   └── extensions.json      # 推荐扩展
├── .zed/
│   ├── debug.json          # Zed 调试配置
│   └── tasks.json          # Zed 任务配置
├── src-tauri/
│   ├── Cargo.toml          # Rust 项目配置
│   ├── tauri.conf.json     # Tauri 配置
│   └── src/                # Rust 源码
├── src/                    # Vue3 前端源码
├── scripts/
│   ├── debug.sh           # Unix 调试脚本
│   ├── debug.ps1          # Windows 调试脚本
│   └── setup.sh           # 环境设置脚本
├── Makefile               # Unix 构建脚本
├── build.ps1              # Windows 构建脚本
└── 调试指南/
    ├── CROSS_PLATFORM_DEBUG_GUIDE-ZH.md  # 本指南中文版
    ├── VSCODE_DEBUG_GUIDE-ZH.md          # VS Code 专用指南中文版
    └── DEBUG_GUIDE-ZH.md                 # Zed 专用指南中文版
```

## 🎯 最佳实践

### 1. 统一开发环境
```json
// .vscode/settings.json 跨平台设置
{
  "terminal.integrated.profiles.windows": {
    "PowerShell": {
      "source": "PowerShell",
      "args": ["-NoProfile"]
    }
  },
  "terminal.integrated.profiles.osx": {
    "bash": {
      "path": "/bin/bash"
    }
  },
  "terminal.integrated.profiles.linux": {
    "bash": {
      "path": "/bin/bash"
    }
  }
}
```

### 2. 条件配置
```json
// launch.json 中的条件配置示例
{
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed${command:extension.commandvariables.file.fileAsKey.windows:.exe}",
  "windows": {
    "type": "cppvsdbg"
  },
  "osx": {
    "type": "lldb"
  },
  "linux": {
    "type": "gdb"
  }
}
```

### 3. 环境变量管理
```json
// 跨平台环境变量
{
  "env": {
    "RUST_BACKTRACE": "1",
    "RUST_LOG": "debug"
  },
  "windows": {
    "env": {
      "VCPKG_ROOT": "C:\\vcpkg"
    }
  },
  "osx": {
    "env": {
      "PKG_CONFIG_PATH": "/usr/local/lib/pkgconfig"
    }
  }
}
```

## 🔄 持续集成考虑

### GitHub Actions 示例
```yaml
name: Cross-platform Build and Test

on: [push, pull_request]

jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]

    runs-on: ${{ matrix.os }}

    steps:
    - uses: actions/checkout@v2

    - name: Setup Node.js
      uses: actions/setup-node@v2
      with:
        node-version: '18'

    - name: Setup Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable

    - name: Install system dependencies (Ubuntu)
      if: matrix.os == 'ubuntu-latest'
      run: |
        sudo apt update
        sudo apt install libwebkit2gtk-4.0-dev

    - name: Install dependencies
      run: yarn install

    - name: Build
      run: yarn tauri build
```

## 📞 获取帮助

### 平台特定资源

#### Windows
- [Windows 上的 Rust 开发环境](https://forge.rust-lang.org/infra/channel-layout.html#windows)
- [Visual Studio Code on Windows](https://code.visualstudio.com/docs/setup/windows)

#### macOS
- [macOS 上的 Rust 开发环境](https://forge.rust-lang.org/infra/channel-layout.html#macos)
- [Xcode 和开发工具](https://developer.apple.com/xcode/)

#### Linux
- [Linux 上的 Rust 开发环境](https://forge.rust-lang.org/infra/channel-layout.html#linux)
- [WebKit 依赖安装指南](https://tauri.app/v1/guides/getting-started/prerequisites#linux)

### 通用资源
- [Tauri v2 官方文档](https://v2.tauri.app/)
- [Rust 官方学习资源](https://www.rust-lang.org/learn)
- [Vue.js 官方文档](https://vuejs.org/)

## 🚨 重要提醒

1. **路径分隔符**：使用 `/` 在大多数现代工具中都能正常工作
2. **可执行文件扩展名**：Windows 需要 `.exe`，Unix 系统不需要
3. **权限设置**：Linux 和 macOS 可能需要额外的权限配置
4. **系统依赖**：每个平台都有特定的系统级依赖要求
5. **调试器差异**：不同平台的调试器行为可能略有不同

最后更新：2024年
