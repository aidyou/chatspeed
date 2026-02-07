# Windows 故障排查快速指南

## 🚨 遇到问题？

如果Chatspeed在Windows上崩溃或字体图标不显示，请按照以下步骤操作：

## 第一步：运行诊断工具

打开PowerShell，运行诊断脚本：

```powershell
# 进入Chatspeed目录
cd C:\path\to\chatspeed

# 如果遇到执行策略限制，先运行：
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser

# 运行诊断
.\scripts\windows-diagnostics.ps1
```

诊断脚本会检查：
- ✓ WebView2运行时
- ✓ 应用数据目录
- ✓ 日志文件
- ✓ 数据库状态
- ✓ 权限问题

## 第二步：查看日志

按 `Win + R`，输入：
```
%LOCALAPPDATA%\ai.aidyou.chatspeed\logs
```

查看以下日志文件：
- `chatspeed.log` - 主应用日志
- `ccproxy.log` - 代理服务日志

## 常见问题快速解决

### ❌ 程序无法启动

**可能原因**：WebView2未安装

**解决方案**：
1. 下载WebView2：https://go.microsoft.com/fwlink/p/?LinkId=2124703
2. 安装后重启程序

---

### ❌ 字体图标显示为方框

**可能原因**：字体文件加载失败

**解决方案**：
1. 检查是否是旧版本，更新到最新版
2. 重新安装程序
3. 按F12打开开发者工具，查看是否有404错误

---

### ❌ 看不到日志文件

**可能原因**：日志目录创建失败

**解决方案**：
1. 检查是否以管理员身份运行
2. 运行诊断脚本查看详细信息
3. 检查防病毒软件是否阻止了程序

---

### ❌ 数据丢失

**可能原因**：使用了内存数据库（fallback模式）

**解决方案**：
1. 检查日志中是否有"in-memory database"字样
2. 确保数据库目录有写权限
3. 重新安装到有完整权限的目录

## 报告问题

如果以上方法都无效，请创建GitHub Issue并附上：

1. **诊断脚本输出**（完整复制）
2. **日志文件内容**（如果存在）
3. **Windows版本**：运行 `winver` 查看
4. **错误截图**（如果有）

GitHub Issues: https://github.com/aidyou/chatspeed/issues

## 开发者选项

如果你是开发者，可以在开发模式下运行以查看详细日志：

```bash
yarn install
yarn tauri dev
```

这会在终端显示所有日志输出。

## 更多信息

详细的修复说明和技术细节，请参阅：
- `docs/Windows崩溃修复指南.md` - 完整修复指南
- `docs/WINDOWS_FIX_SUMMARY.md` - 技术细节和修复总结

---

**最后更新**：2025-02-07
**适用版本**：v1.2.5+
