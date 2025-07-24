# VS Code Tauri v2 + Vue3 Debugging Guide

This guide is specifically for debugging Tauri v2 + Vue3 applications in the VS Code editor.

## 🚀 Quick Start

### Recommended Debugging Workflow

1. **One-click Debugging**:
   - Press `F5` or `Ctrl+Shift+D` to open the debugging panel
   - Select **"🌟 Full Stack Debug (Recommended)"**
   - This will automatically start the frontend development server and backend debugging

2. **Step-by-Step Debugging** (if one-click has issues):
   - Select **"🔧 Tauri Backend Only"**
   - Manually start the frontend: run `yarn dev` in terminal

## 📋 Debug Configuration Details

### 1. 🚀 Tauri Development (Full Stack)
- **Purpose**: Complete full-stack development debugging
- **Features**:
  - Automatically starts frontend development server
  - Sets development environment variables
  - Complete error tracing
- **Use cases**: Daily development, preferred configuration

### 2. 🔧 Tauri Backend Only
- **Purpose**: Debug only the Rust backend
- **Features**:
  - Requires manually starting the frontend
  - Focuses on backend logic debugging
  - Faster startup time
- **Use cases**: Backend logic development, stable frontend

### 3. 🏗️ Tauri Production Debug
- **Purpose**: Production mode debugging
- **Features**:
  - Uses release build
  - Performance close to production environment
  - Optimized code debugging
- **Use cases**: Performance testing, production issue investigation

### 4. 🧪 Tauri Test Debug
- **Purpose**: Unit tests and integration tests
- **Features**:
  - Runs test suites
  - Complete error tracing
  - Test environment configuration
- **Use cases**: Test-driven development, bug fix verification

### 5. 🎯 Tauri with Custom Args
- **Purpose**: Debug with custom parameters
- **Features**:
  - Can pass command line arguments
  - Flexible startup configuration
  - Supports different running modes
- **Use cases**: Special scenario testing, feature verification

### 6. 🔗 Attach to Running Tauri Process
- **Purpose**: Attach to an already running process
- **Features**:
  - Doesn't restart the application
  - Debug running instance
  - Suitable for long-running scenarios
- **Use cases**: Production environment debugging, process analysis

## 🛠 Available Tasks

Through `Ctrl+Shift+P` → "Tasks: Run Task" you can run:

### Development Tasks
- **start-frontend-dev-server**: Start the Vue3 development server
- **prepare-debug**: Prepare the debug environment (language files, etc.)
- **tauri-dev**: Start the complete Tauri development environment

### Build Tasks
- **cargo-check-tauri**: Check Rust code syntax
- **cargo-build-tauri-debug**: Build Debug version
- **cargo-build-tauri-release**: Build Release version
- **build-frontend-production**: Build frontend production version

### Maintenance Tasks
- **clean-all**: Clean all build caches
- **kill-dev-server**: Stop the development server
- **install-dependencies**: Install project dependencies

## 🔧 Breakpoints and Debugging Tips

### Rust Backend Debugging
1. **Setting Breakpoints**:
   - Click to the left of line numbers in `.rs` files to set breakpoints
   - Use conditional breakpoints: right-click breakpoint → "Edit Breakpoint"

2. **Variable Inspection**:
   - Hover to view variable values
   - View scope variables in the "Variables" panel
   - Add expressions in the "Watch" panel

3. **Call Stack**:
   - "Call Stack" panel shows function call chain
   - Click on stack frames to switch context

### Vue3 Frontend Debugging
1. **Browser Developer Tools**:
   - Right-click in application window → "Inspect Element"
   - Use Vue DevTools extension

2. **Source Mapping**:
   - TypeScript/JavaScript breakpoints will auto-map
   - Set breakpoints in the Sources panel

## 🔍 Troubleshooting

### Issue: Frontend server fails to start
**Symptoms**: `start-frontend-dev-server` task fails

**Solution**:
```bash
# Check port usage
lsof -ti:1420

# Kill the process using the port
kill -9 $(lsof -ti:1420)

# Reinstall dependencies
yarn install
```

### Issue: Rust compilation errors
**Symptoms**: Build fails, red error messages

**Solution**:
1. Run the `cargo-check-tauri` task to see detailed errors
2. Check Rust code syntax
3. Ensure dependency versions are compatible

### Issue: Debugger can't attach
**Symptoms**: Breakpoints don't work, can't pause execution

**Solution**:
1. Ensure you're using Debug build (not Release)
2. Check `sourceLanguages` setting in `launch.json`
3. Restart VS Code and the debugging session

### Issue: Environment variables not taking effect
**Symptoms**: `RUST_LOG` and other environment variables are ineffective

**Solution**:
1. Check `env` configuration in `launch.json`
2. Confirm terminal environment variable settings
3. Restart VS Code to apply settings

## 📁 Project Structure and Configuration Files

```
chatspeed/
├── .vscode/
│   ├── launch.json          # Debug configuration
│   ├── tasks.json           # Task configuration
│   ├── settings.json        # Workspace settings
│   └── extensions.json      # Recommended extensions
├── src-tauri/
│   ├── Cargo.toml          # Rust project configuration
│   ├── tauri.conf.json     # Tauri configuration
│   └── src/                # Rust source code
├── src/                    # Vue3 frontend source code
├── Makefile               # Build script
└── docs/
    └── VSCODE_DEBUG_GUIDE.md  # This guide
```

## ⚙️ Advanced Configuration

### Custom Debug Configuration
Add a new configuration in `launch.json`:
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

### Rust-analyzer Optimization
Adjust in `settings.json`:
```json
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "rust-analyzer.cargo.allFeatures": true,
  "rust-analyzer.inlayHints.typeHints.enable": false
}
```

### Performance Debugging
1. **Memory Usage**:
```json
{
  "env": {
    "RUST_LOG": "debug",
    "RUST_BACKTRACE": "full"
  }
}
```

2. **Performance Analysis**:
```bash
# Using perf tools
cargo build --release
perf record target/release/chatspeed
perf report
```

## 🎯 Best Practices

### 1. Development Workflow
1. Always start with "🚀 Tauri Development (Full Stack)"
2. Switch to "🔧 Tauri Backend Only" if issues occur
3. Regularly run `cargo-check-tauri` to check code quality

### 2. Debugging Strategy
1. **Incremental Debugging**: Start with simple scenarios, gradually increase complexity
2. **Logging First**: Add `log::debug!()` statements at key locations
3. **Unit Testing**: Use "🧪 Tauri Test Debug" to validate individual functions

### 3. Performance Optimization
1. Use Debug builds during development
2. Use "🏗️ Tauri Production Debug" for performance testing
3. Monitor memory usage and CPU consumption

### 4. Team Collaboration
1. Standardize on the same VS Code configuration
2. Regularly update `.vscode/` configuration files
3. Document special debugging scenarios

## 📞 Getting Help

### Common Commands
```bash
# Check environment information
yarn tauri info

# Clean and restart
make clean && yarn install

# Check Rust toolchain
rustc --version && cargo --version
```

### Viewing Logs
- **Backend logs**: VS Code Debug Console
- **Frontend logs**: Browser developer tools Console
- **Build logs**: VS Code Terminal panel

### Related Resources
- [Tauri v2 Official Documentation](https://v2.tauri.app/)
- [VS Code Debugging Guide](https://code.visualstudio.com/docs/editor/debugging)
- [Rust-analyzer User Manual](https://rust-analyzer.github.io/manual.html)
- [Vue.js Developer Tools](https://devtools.vuejs.org/)

## 🔄 Configuration Version History

- **v1.0**: Basic debugging configuration
- **v1.1**: Added production mode debugging
- **v1.2**: Optimized task dependencies and error handling
- **v1.3**: Added test debugging and custom parameter support
- **v1.4**: Enhanced frontend debugging configuration and documentation

Last updated: 2024