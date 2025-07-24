# Cross-Platform Debugging Guide - Tauri v2 + Vue3

This guide provides complete solutions for debugging Tauri v2 + Vue3 applications on different operating systems.

## 🌍 Platform Support Overview

| Feature | Windows | macOS | Linux |
|---------|---------|-------|-------|
| VS Code Debugging | ✅ | ✅ | ✅ |
| Zed Debugging | ✅ | ✅ | ✅ |
| Frontend Debugging | ✅ | ✅ | ✅ |
| Backend Debugging | ✅ | ✅ | ✅ |
| Automation Scripts | ✅ | ✅ | ✅ |

## 🔧 Platform-Specific Configuration

### Windows Configuration

#### Required Tools
```powershell
# Install Rust
winget install Rustlang.Rust

# Install Node.js
winget install OpenJS.NodeJS

# Install Yarn
npm install -g yarn

# Install Visual Studio C++ Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools
```

#### VS Code Debugger Selection
- **Recommended**: `cppvsdbg` (Microsoft C++ Debugger)
- **Alternative**: `CodeLLDB` (requires additional configuration)

#### Special Configuration
```json
// .vscode/launch.json - Windows-specific configuration
{
  "type": "cppvsdbg",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed.exe",
  "console": "integratedTerminal"
}
```

### macOS Configuration

#### Required Tools
```bash
# Install Xcode Command Line Tools
xcode-select --install

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node.js (using Homebrew)
brew install node

# Install Yarn
npm install -g yarn
```

#### VS Code Debugger Selection
- **Recommended**: `lldb` (CodeLLDB)
- **Built-in Support**: Xcode debugging tools

#### Special Configuration
```json
// .vscode/launch.json - macOS-specific configuration
{
  "type": "lldb",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed",
  "sourceLanguages": ["rust"]
}
```

### Linux Configuration

#### Required Tools (Ubuntu/Debian)
```bash
# Update package manager
sudo apt update

# Install build tools
sudo apt install build-essential curl wget file

# Install system dependencies
sudo apt install libwebkit2gtk-4.0-dev \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node.js
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt install nodejs

# Install Yarn
npm install -g yarn
```

#### VS Code Debugger Selection
- **Recommended**: `gdb` (GNU Debugger)
- **Alternative**: `lldb` (CodeLLDB)

#### Special Configuration
```json
// .vscode/launch.json - Linux-specific configuration
{
  "type": "gdb",
  "program": "${workspaceFolder}/src-tauri/target/debug/chatspeed",
  "cwd": "${workspaceFolder}"
}
```

## 🚀 Platform-Specific Debugging Workflows

### Windows Debugging Workflow

1. **Environment Preparation**
```powershell
# Clone the project
git clone <repository-url>
cd chatspeed

# Install dependencies
yarn install

# Check environment
yarn tauri info
```

2. **VS Code Debugging**
- Open VS Code
- Select "🚀 Tauri Development (Windows MSVC)"
- Press F5 to start debugging

3. **Command Line Debugging**
```powershell
# Start frontend development server
yarn dev

# In another terminal, build and run Tauri
yarn tauri dev
```

### macOS Debugging Workflow

1. **Environment Preparation**
```bash
# Clone the project
git clone <repository-url>
cd chatspeed

# Install dependencies
yarn install

# Check environment
yarn tauri info
```

2. **VS Code Debugging**
- Open VS Code
- Select "🚀 Tauri Development (Full Stack)"
- Press F5 to start debugging

3. **Zed Debugging**
- Open Zed
- Press F4 to start the debugger
- Select "Debug Tauri App (macOS/Linux)"

### Linux Debugging Workflow

1. **Environment Preparation**
```bash
# Clone the project
git clone <repository-url>
cd chatspeed

# Install dependencies
yarn install

# Check environment
yarn tauri info
```

2. **VS Code Debugging**
- Open VS Code
- Select "🚀 Tauri Development (Linux GDB)"
- Press F5 to start debugging

3. **Command Line Debugging**
```bash
# Using GDB
cargo build --manifest-path src-tauri/Cargo.toml
gdb ./src-tauri/target/debug/chatspeed
```

## 🔍 Platform-Specific Troubleshooting

### Common Windows Issues

#### Issue: Build fails with "link.exe not found"
**Solution**:
```powershell
# Install Visual Studio Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# Or install the full Visual Studio
winget install Microsoft.VisualStudio.2022.Community
```

#### Issue: PowerShell execution policy restrictions
**Solution**:
```powershell
# Set execution policy (admin privileges)
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope LocalMachine
```

#### Issue: Port in use
**Solution**:
```powershell
# Find process using the port
netstat -ano | findstr :1420

# Kill the process
taskkill /PID <PID> /F
```

### Common macOS Issues

#### Issue: Permission denied
**Solution**:
```bash
# Grant executable permission
chmod +x debug.sh

# Check Gatekeeper settings
sudo spctl --master-disable  # Not recommended, only for development
```

#### Issue: Xcode license agreement
**Solution**:
```bash
# Accept Xcode license
sudo xcodebuild -license accept
```

### Common Linux Issues

#### Issue: Missing system dependencies
**Solution**:
```bash
# Ubuntu/Debian
sudo apt install libwebkit2gtk-4.0-dev libssl-dev

# CentOS/RHEL/Fedora
sudo dnf install webkit2gtk3-devel openssl-devel

# Arch Linux
sudo pacman -S webkit2gtk openssl
```

#### Issue: Debugger permissions
**Solution**:
```bash
# Allow ptrace
echo 0 | sudo tee /proc/sys/kernel/yama/ptrace_scope

# Or add to group
sudo usermod -a -G gdb $USER
```

## 📁 Cross-Platform File Structure

```
chatspeed/
├── .vscode/
│   ├── launch.json          # Cross-platform debug configuration
│   ├── tasks.json           # Cross-platform task configuration
│   ├── settings.json        # Editor settings
│   └── extensions.json      # Recommended extensions
├── .zed/
│   ├── debug.json          # Zed debug configuration
│   └── tasks.json          # Zed task configuration
├── src-tauri/
│   ├── Cargo.toml          # Rust project configuration
│   ├── tauri.conf.json     # Tauri configuration
│   └── src/                # Rust source code
├── src/                    # Vue3 frontend source code
├── scripts/
│   ├── debug.sh           # Unix debug script
│   ├── debug.ps1          # Windows debug script
│   └── setup.sh           # Environment setup script
├── Makefile               # Unix build script
├── build.ps1              # Windows build script
└── docs/
    ├── CROSS_PLATFORM_DEBUG_GUIDE.md  # This guide
    ├── VSCODE_DEBUG_GUIDE.md          # VS Code specific guide
    └── DEBUG_GUIDE.md                 # Zed specific guide
```

## 🎯 Best Practices

### 1. Unified Development Environment
```json
// .vscode/settings.json cross-platform settings
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

### 2. Conditional Configuration
```json
// launch.json conditional configuration example
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

### 3. Environment Variable Management
```json
// Cross-platform environment variables
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

## 🔄 Continuous Integration Considerations

### GitHub Actions Example
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

## 📞 Getting Help

### Platform-Specific Resources

#### Windows
- [Rust Development Environment on Windows](https://forge.rust-lang.org/infra/channel-layout.html#windows)
- [Visual Studio Code on Windows](https://code.visualstudio.com/docs/setup/windows)

#### macOS
- [Rust Development Environment on macOS](https://forge.rust-lang.org/infra/channel-layout.html#macos)
- [Xcode and Development Tools](https://developer.apple.com/xcode/)

#### Linux
- [Rust Development Environment on Linux](https://forge.rust-lang.org/infra/channel-layout.html#linux)
- [WebKit Dependencies Installation Guide](https://tauri.app/v1/guides/getting-started/prerequisites#linux)

### General Resources
- [Tauri v2 Official Documentation](https://v2.tauri.app/)
- [Rust Official Learning Resources](https://www.rust-lang.org/learn)
- [Vue.js Official Documentation](https://vuejs.org/)

## 🚨 Important Reminders

1. **Path Separators**: Using `/` works in most modern tools
2. **Executable Extensions**: Windows requires `.exe`, Unix systems don't
3. **Permission Settings**: Linux and macOS may require additional permission configuration
4. **System Dependencies**: Each platform has specific system-level dependency requirements
5. **Debugger Differences**: Different platforms' debuggers may behave slightly differently

Last updated: 2024