#!/bin/bash

# Windows崩溃问题检查清单
# 用于在发布前检查是否已修复所有已知问题

echo "========================================"
echo "Windows崩溃问题修复检查清单"
echo "========================================"
echo ""

# 颜色定义
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

check_pass=0
check_fail=0

# 函数：检查项通过
check_ok() {
    echo -e "${GREEN}✓${NC} $1"
    ((check_pass++))
}

# 函数：检查项失败
check_fail_msg() {
    echo -e "${RED}✗${NC} $1"
    echo -e "  ${YELLOW}→${NC} $2"
    ((check_fail++))
}

# 函数：警告信息
check_warn() {
    echo -e "${YELLOW}⚠${NC} $1"
}

echo "1. 检查字体文件..."
# 检查字体文件是否存在
if [ -f "src/components/icon/iconfont.woff2" ]; then
    check_ok "字体文件存在: iconfont.woff2"
else
    check_fail_msg "字体文件缺失" "请确保 src/components/icon/iconfont.woff2 存在"
fi

# 检查CSS中的字体路径
if grep -q "url('./iconfont.woff2')" "src/components/icon/chatspeed.css"; then
    check_ok "CSS使用相对路径加载字体"
elif grep -q "url('@/components/icon/iconfont.woff2')" "src/components/icon/chatspeed.css"; then
    check_fail_msg "CSS使用了Vite别名" "应该使用相对路径 './iconfont.woff2'"
else
    check_warn "无法确认CSS中的字体路径"
fi

echo ""
echo "2. 检查Vite配置..."
# 检查assetsInclude是否包含woff2
if grep -q "woff2" "vite.config.js"; then
    check_ok "Vite配置包含字体文件类型"
else
    check_fail_msg "Vite配置未包含.woff2" "在assetsInclude中添加 '**/*.woff2'"
fi

echo ""
echo "3. 检查错误处理..."
# 检查logger.rs是否有增强的错误输出
if grep -q "CRITICAL.*Failed to retrieve log directory" "src-tauri/src/logger.rs"; then
    check_ok "日志初始化有详细错误输出"
else
    check_warn "日志初始化可能缺少详细错误信息"
fi

# 检查lib.rs是否有增强的错误输出
if grep -q "CRITICAL.*Failed to create main store" "src-tauri/src/lib.rs"; then
    check_ok "数据库初始化有详细错误输出"
else
    check_warn "数据库初始化可能缺少详细错误信息"
fi

echo ""
echo "4. 检查诊断工具..."
# 检查Windows诊断脚本
if [ -f "scripts/windows-diagnostics.ps1" ]; then
    check_ok "Windows诊断脚本存在"
else
    check_fail_msg "诊断脚本缺失" "请创建 scripts/windows-diagnostics.ps1"
fi

# 检查文档
if [ -f "docs/Windows崩溃修复指南.md" ]; then
    check_ok "修复指南文档存在"
else
    check_warn "建议创建 Windows 修复指南文档"
fi

echo ""
echo "5. 检查Icon组件..."
# 检查Icon.vue中是否正确设置font-family
if grep -q "fontFamily.*chatspeed.*important" "src/components/icon/Icon.vue"; then
    check_ok "Icon组件样式使用!important确保字体加载"
else
    check_warn "建议在Icon组件中使用!important强制字体"
fi

# 检查CSS中的font-display
if grep -q "font-display" "src/components/icon/chatspeed.css"; then
    check_ok "字体CSS包含font-display优化"
else
    check_warn "建议在@font-face中添加 font-display: swap"
fi

echo ""
echo "========================================"
echo "检查结果汇总"
echo "========================================"
echo -e "通过: ${GREEN}${check_pass}${NC}"
echo -e "失败: ${RED}${check_fail}${NC}"
echo ""

if [ $check_fail -eq 0 ]; then
    echo -e "${GREEN}✓ 所有关键检查项已通过${NC}"
    echo ""
    echo "建议的测试步骤："
    echo "1. 在Windows上构建: yarn tauri build"
    echo "2. 在干净的Windows系统上测试安装包"
    echo "3. 检查日志目录是否创建: %LOCALAPPDATA%\\ai.aidyou.chatspeed\\logs"
    echo "4. 验证字体图标是否正常显示"
    echo "5. 如果出现问题，运行诊断脚本: .\\scripts\\windows-diagnostics.ps1"
else
    echo -e "${RED}✗ 发现 ${check_fail} 个问题需要修复${NC}"
    echo ""
    echo "请修复上述问题后再进行发布"
fi

echo ""
