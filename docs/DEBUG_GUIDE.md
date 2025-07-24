# Tauri v2 + Vue3 Debugging Guide

This guide explains how to debug Tauri v2 applications in the Zed editor.

## 🚀 Quick Start

### Method 1: Full Application Debugging (Recommended)

1. In Zed, press `F4` or `Cmd+Shift+P` → "debugger: start"
2. Select **"Debug Tauri App (Full Stack)"**
3. This will automatically start the frontend development server and backend debugging

### Method 2: Step-by-Step Debugging

If Method 1 doesn't work, use step-by-step debugging:

1. **Start the frontend development server**:
   - Press `Cmd+Shift+P` → "task: spawn"
   - Select **"Start Frontend Dev Server"**
   - Or run in terminal: `yarn dev`

2. **Wait for the frontend server to start** (usually at http://localhost:1420)

3. **Start backend debugging**:
   - Press `F4` → "debugger: start"
   - Select **"Debug Backend (Dev Mode)"**

## 📋 Available Debug Configurations

### 1. Debug Tauri App (Full Stack)
- **Purpose**: Complete application debugging
- **Features**: Automatically starts frontend and backend
- **Recommended for**: Daily development debugging

### 2. Build & Debug Rust Backend
- **Purpose**: Debug only the Rust backend
- **Features**: Requires manual frontend start
- **Recommended for**: Focusing on backend logic debugging

### 3. Debug Backend (Dev Mode)
- **Purpose**: Debug backend in development mode
- **Features**: Disables default features, connects to dev server
- **Recommended for**: When frontend is already running

### 4. Debug with Custom Args
- **Purpose**: Debug with custom parameters
- **Features**: Can pass command line arguments
- **Recommended for**: When specific startup parameters are needed

## 🛠 Available Tasks

Through `Cmd+Shift+P` → "task: spawn" you can run the following tasks:

- **Start Frontend Dev Server**: Start the Vue3 development server
- **Build Frontend**: Build frontend assets
- **Tauri Dev (Full Stack)**: Start the complete Tauri development environment
- **Tauri Build**: Build release version
- **Cargo Check (Tauri)**: Check Rust code
- **Cargo Build (Tauri)**: Build Rust backend
- **Cargo Build (Dev Mode)**: Build in development mode
- **Tauri Info**: Display environment information
- **Clean All**: Clean all build caches

## 🔧 Troubleshooting

### Issue: UI not visible, but program is running

**Symptoms**: Program starts normally, status bar and dock have icons, but window doesn't display

**Solution**:
1. Ensure the frontend development server is running at http://localhost:1420
2. Check the `devUrl` configuration in `tauri.conf.json`
3. Use the "Debug Backend (Dev Mode)" configuration

### Issue: cargo tauri command doesn't exist

**Symptoms**: `error: no such command: 'tauri'`

**Cause**: Tauri v2 CLI is installed via npm/yarn, not as a cargo subcommand

**Solution**: Use `yarn tauri` instead of `cargo tauri`

### Issue: Port in use

**Symptoms**: Frontend server can't start, port 1420 is in use

**Solution**:
```bash
# Find process using the port
lsof -ti:1420

# Kill the process
kill -9 $(lsof -ti:1420)

# Or use the debug script
./debug.sh
```

### Issue: Debugger can't attach

**Symptoms**: Debugger starts but can't set breakpoints

**Solution**:
1. Make sure you're using the `CodeLLDB` adapter
2. Check if built in Debug mode
3. Try rebuilding: `cargo build --manifest-path src-tauri/Cargo.toml`

## 📁 File Structure

```
chatspeed/
├── .zed/
│   ├── debug.json          # Debug configuration
│   └── tasks.json          # Task configuration
├── src-tauri/
│   ├── Cargo.toml          # Rust project configuration
│   ├── tauri.conf.json     # Tauri configuration
│   └── src/
│       └── main.rs         # Rust main program
├── src/                    # Vue3 frontend source code
├── dist/                   # Build output
├── scripts/
│   └── debug.sh            # Debug helper script
└── docs/
    └── DEBUG_GUIDE.md      # This guide
```

## 🎯 Best Practices

### 1. Development Workflow
1. Keep `yarn dev` running in one terminal window
2. Use "Debug Backend (Dev Mode)" in Zed for backend debugging
3. Use browser developer tools to debug the frontend

### 2. Setting Breakpoints
- Set breakpoints in Rust code for backend debugging
- Right-click in application window → "Inspect Element" to debug frontend

### 3. Viewing Logs
- Backend logs: View in Zed debug console
- Frontend logs: View in browser developer tools Console
- Set `RUST_LOG=debug` environment variable for detailed logs

### 4. Performance Debugging
- Build in Release mode for performance testing
- Enable Rust's profiling tools
- Use browser performance tools to analyze frontend

## 📞 Getting Help

If you encounter issues:

1. Check environment information with `yarn tauri info`
2. View Zed's debug output panel
3. Check for error messages in the terminal
4. Refer to [Tauri official documentation](https://tauri.app/v1/guides/debugging/application)
5. Use the `./debug.sh` script for automated debugging

## 🔗 Related Links

- [Tauri v2 Documentation](https://v2.tauri.app/)
- [Zed Debugger Documentation](https://zed.dev/docs/debugger)
- [CodeLLDB Documentation](https://github.com/vadimcn/codelldb)
- [Vue.js Debugging Guide](https://vuejs.org/guide/scaling-up/tooling.html#browser-devtools)