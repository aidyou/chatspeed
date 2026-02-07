fix(windows): 修复Windows崩溃和字体图标不显示问题

## 问题描述
- 程序在Windows上启动后崩溃
- 字体图标显示为方框或乱码
- 日志目录不可见，无法诊断问题
- 错误信息不明确

## 主要修复

### 1. 字体加载修复
- 将CSS中的字体路径从Vite别名改为相对路径
- 文件: `src/components/icon/chatspeed.css`
- 原因: `@/components/icon/iconfont.woff2` 在生产环境中无法解析
- 修复: 使用 `./iconfont.woff2` 相对路径
- 新增: `font-display: swap` 优化加载性能

### 2. Icon组件样式强化
- 文件: `src/components/icon/Icon.vue`
- 在CSS和内联样式中都使用 `!important` 确保字体优先级
- 添加字体平滑渲染属性

### 3. 错误诊断增强
- 文件: `src-tauri/src/logger.rs`, `src-tauri/src/lib.rs`
- 添加详细的错误输出，包含:
  - 平台信息
  - 完整路径
  - 错误类型
  - 后续影响说明
- 使用醒目的分隔符标记关键错误

### 4. 资源打包配置
- 文件: `vite.config.js`
- 在 `assetsInclude` 中添加字体文件类型
- 确保 `.woff2`, `.woff`, `.ttf` 被正确打包

### 5. 诊断工具
新增文件:
- `scripts/windows-diagnostics.ps1` - Windows环境诊断脚本
- `scripts/check-windows-fixes.sh` - 发布前检查脚本
- `docs/Windows崩溃修复指南.md` - 用户修复指南
- `docs/WINDOWS_FIX_SUMMARY.md` - 技术细节总结
- `docs/WINDOWS_TROUBLESHOOTING.md` - 快速故障排查

## 测试验证
- ✓ 所有修改文件无编译错误
- ✓ check-windows-fixes.sh 检查通过 (9/9)
- ✓ 字体文件存在且路径正确
- ✓ Vite配置包含字体类型
- ✓ 错误处理代码完整

## Breaking Changes
无

## 影响范围
- Windows用户体验改善
- 错误诊断能力提升
- 不影响macOS和Linux

## 待测试
在Windows环境中验证:
1. [ ] 程序正常启动不崩溃
2. [ ] 字体图标正常显示
3. [ ] 日志文件正确生成
4. [ ] 诊断脚本可用

## 相关Issue
Closes #[issue-number] (如果有)

---
Co-authored-by: GitHub Copilot
