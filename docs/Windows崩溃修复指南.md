# Windows 崩溃问题修复指南

## 问题描述

在 Windows 上运行 Chatspeed 时遇到以下问题：
1. 程序打开后直接崩溃
2. 只能看到 WebView 缓存，但看不到日志目录
3. 界面打开一会后崩溃
4. 字体图标不显示（乱码）

## 已实施的修复

### 1. 字体文件路径修复
- **问题**：CSS 中使用 Vite 别名 `@/components/icon/iconfont.woff2`，在 Windows 生产环境中无法正确解析
- **修复**：改用相对路径 `./iconfont.woff2`
- **文件**：`src/components/icon/chatspeed.css`

### 2. 增强错误诊断
- **问题**：错误信息只输出到 stderr，用户看不到详细错误
- **修复**：
  - 在日志初始化失败时输出详细的错误信息（包括平台、路径、错误类型）
  - 在数据库初始化失败时输出详细诊断信息
  - 所有关键错误都添加了醒目的分隔符和详细说明
- **文件**：
  - `src-tauri/src/logger.rs`
  - `src-tauri/src/lib.rs`

### 3. 字体文件打包配置
- **问题**：字体文件可能没有被正确打包
- **修复**：在 Vite 配置中显式添加字体文件类型到 `assetsInclude`
- **文件**：`vite.config.js`

### 4. Windows 诊断工具
- **新增**：PowerShell 诊断脚本，用于检测：
  - WebView2 运行时安装状态
  - 应用数据目录和日志文件
  - 目录权限
  - 最近的日志内容
  - Windows 事件日志中的错误
- **文件**：`scripts/windows-diagnostics.ps1`

## 使用诊断工具

### 方法 1：从项目目录运行
```powershell
cd path\to\chatspeed
.\scripts\windows-diagnostics.ps1
```

### 方法 2：复制脚本内容
1. 打开 PowerShell（以管理员身份运行）
2. 复制 `scripts/windows-diagnostics.ps1` 的内容
3. 粘贴并执行

### 方法 3：允许脚本执行
如果遇到"无法加载脚本"错误：
```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
.\scripts\windows-diagnostics.ps1
```

## 查看日志位置

Windows 上的日志文件位于：
```
C:\Users\<用户名>\AppData\Local\ai.aidyou.chatspeed\logs\
```

你可以直接打开这个目录查看：
1. 按 `Win + R`
2. 输入：`%LOCALAPPDATA%\ai.aidyou.chatspeed\logs`
3. 按回车

日志文件：
- `chatspeed.log` - 主应用日志
- `ccproxy.log` - 代理服务日志

## 常见问题排查

### 问题 1：WebView2 未安装
**症状**：程序无法启动，没有任何窗口
**解决**：
1. 下载 WebView2 运行时：https://go.microsoft.com/fwlink/p/?LinkId=2124703
2. 安装后重启应用

### 问题 2：权限问题
**症状**：日志显示"Permission denied"或"Access denied"
**解决**：
1. 以管理员身份运行程序
2. 检查防病毒软件是否阻止了程序
3. 检查 AppData 目录的权限

### 问题 3：数据库损坏
**症状**：日志显示数据库相关错误
**解决**：
1. 备份现有数据库：`%LOCALAPPDATA%\ai.aidyou.chatspeed\chatspeed.db`
2. 删除数据库文件
3. 重启程序（会自动创建新数据库）

### 问题 4：字体图标不显示
**症状**：界面中的图标显示为方框或乱码
**可能原因**：
1. 字体文件未正确加载
2. CSS 路径解析失败
3. 字体文件损坏

**解决**：
1. 检查构建输出的 `dist` 目录中是否包含 `iconfont.woff2`
2. 检查浏览器控制台（F12）是否有 404 错误
3. 重新安装程序

## 开发者调试

### 在开发模式下运行
```bash
# 确保所有依赖已安装
yarn install

# 启动开发模式
yarn tauri dev
```

### 查看控制台输出
开发模式下，所有日志会同时输出到：
1. 终端控制台
2. 日志文件
3. 浏览器开发者工具控制台（前端日志）

### 检查资源打包
```bash
# 构建生产版本
yarn tauri build

# 检查打包后的文件
# Windows: src-tauri\target\release\bundle\msi\
```

## 报告问题

如果问题仍然存在，请提供以下信息：

1. **运行诊断脚本的完整输出**
2. **日志文件内容**（如果存在）：
   - `chatspeed.log`
   - `ccproxy.log`
3. **Windows 版本**：
   ```powershell
   winver
   ```
4. **系统信息**：
   ```powershell
   systeminfo | findstr /C:"OS"
   ```
5. **错误截图**（如果有可见错误信息）

## 临时解决方案

如果程序完全无法启动：

1. **使用开发模式**：
   ```bash
   yarn tauri dev
   ```
   这会在终端显示详细日志

2. **清除应用数据**：
   ```powershell
   Remove-Item -Recurse -Force "$env:LOCALAPPDATA\ai.aidyou.chatspeed"
   ```
   ⚠️ 警告：这会删除所有数据和配置

3. **检查防火墙/杀毒软件**：
   某些安全软件可能会阻止程序运行

## 更新日志

- 2025-02-07：
  - 修复字体文件路径问题
  - 增强错误诊断输出
  - 添加 Windows 诊断工具
  - 改进日志初始化错误处理
