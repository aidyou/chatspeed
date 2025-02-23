#!/bin/bash

# gen_icon.sh - 生成 Tauri 应用所需的各种尺寸图标
# 依赖：imagemagick (convert 命令) 和 macOS 的 iconutil 工具

# 获取脚本所在目录的绝对路径
SCRIPT_DIR=$(dirname "$(realpath "$0")")
# 项目根目录（假设脚本位于 tools/ 目录下）
PROJECT_ROOT=$(dirname "$SCRIPT_DIR")
# 目标目录
TARGET_DIR="$PROJECT_ROOT/src-tauri/icons"
# 源文件路径
SOURCE="$TARGET_DIR/cs_logo.png"
TRAY_ICON_SOURCE="$TARGET_DIR/cs_icon.png"

# 检查依赖是否安装
if ! command -v convert &>/dev/null; then
    echo "错误：ImageMagick (convert) 未安装，请先安装！"
    exit 1
fi
if ! command -v iconutil &>/dev/null; then
    echo "错误：iconutil 未找到，请确保在 macOS 上运行此脚本！"
    exit 1
fi

# 检查源文件是否存在
if [[ ! -f "$SOURCE" ]]; then
    echo "错误：源文件 $SOURCE 不存在！"
    exit 1
fi
if [[ ! -f "$TRAY_ICON_SOURCE" ]]; then
    echo "错误：状态栏图标源文件 $TRAY_ICON_SOURCE 不存在！"
    exit 1
fi

# 检查目标目录是否存在
if [[ ! -d "$TARGET_DIR" ]]; then
    echo "错误：目标目录 $TARGET_DIR 不存在！"
    exit 1
fi

# 生成图标函数
generate_icon() {
    local source_file=$1
    local size=$2
    local output_file=$3
    echo "生成 $output_file ($size)..."
    if ! magick "$source_file" -resize "$size" "$output_file"; then
        echo "错误：生成 $output_file 失败！"
        exit 1
    fi
}

# 生成 macOS 图标（.icns）
generate_icns() {
    echo "生成 macOS 图标（.icns）..."
    # 创建临时目录
    TEMP_DIR=$(mktemp -d)
    ICONSET_DIR="$TEMP_DIR/icon.iconset"
    mkdir -p "$ICONSET_DIR"
    # 生成不同尺寸的图标
    generate_icon "$TRAY_ICON_SOURCE" "16x16" "$ICONSET_DIR/icon_16x16.png"
    generate_icon "$TRAY_ICON_SOURCE" "32x32" "$ICONSET_DIR/icon_16x16@2x.png"
    generate_icon "$TRAY_ICON_SOURCE" "32x32" "$ICONSET_DIR/icon_32x32.png"
    generate_icon "$TRAY_ICON_SOURCE" "64x64" "$ICONSET_DIR/icon_32x32@2x.png"
    generate_icon "$SOURCE" "128x128" "$ICONSET_DIR/icon_128x128.png"
    generate_icon "$SOURCE" "256x256" "$ICONSET_DIR/icon_128x128@2x.png"
    generate_icon "$SOURCE" "256x256" "$ICONSET_DIR/icon_256x256.png"
    generate_icon "$SOURCE" "512x512" "$ICONSET_DIR/icon_256x256@2x.png"
    generate_icon "$SOURCE" "512x512" "$ICONSET_DIR/icon_512x512.png"
    generate_icon "$SOURCE" "1024x1024" "$ICONSET_DIR/icon_512x512@2x.png"
    # 使用 iconutil 合并为 .icns 文件
    if ! iconutil -c icns -o "$TARGET_DIR/icon.icns" "$ICONSET_DIR"; then
        echo "错误：生成 .icns 文件失败！"
        rm -rf "$TEMP_DIR"
        exit 1
    fi
    # 清理临时目录
    rm -rf "$TEMP_DIR"
}

# 生成程序图标
echo "开始生成程序图标..."
generate_icon "$SOURCE" "128x128" "$TARGET_DIR/128x128.png"
generate_icon "$SOURCE" "256x256" "$TARGET_DIR/128x128@2x.png"
generate_icon "$SOURCE" "128x128" "$TARGET_DIR/icon.png"
generate_icon "$SOURCE" "256x256" "$TARGET_DIR/icon.ico"
generate_icon "$SOURCE" "512x512" "$TARGET_DIR/icon.png"
generate_icon "$SOURCE" "150x150" "$TARGET_DIR/Square150x150Logo.png"
generate_icon "$SOURCE" "284x284" "$TARGET_DIR/Square284x284Logo.png"
generate_icon "$SOURCE" "310x310" "$TARGET_DIR/Square310x310Logo.png"
generate_icon "$SOURCE" "50x50" "$TARGET_DIR/StoreLogo.png"
echo "程序图标生成完成！"

# 生成状态栏图标
echo "开始生成状态栏图标..."
generate_icon "$TRAY_ICON_SOURCE" "32x32" "$TARGET_DIR/32x32.png"
generate_icon "$TRAY_ICON_SOURCE" "32x32" "$TARGET_DIR/tray-icon.png"
generate_icon "$TRAY_ICON_SOURCE" "30x30" "$TARGET_DIR/Square30x30Logo.png"
generate_icon "$TRAY_ICON_SOURCE" "44x44" "$TARGET_DIR/Square44x44Logo.png"
generate_icon "$TRAY_ICON_SOURCE" "71x71" "$TARGET_DIR/Square71x71Logo.png"
generate_icon "$TRAY_ICON_SOURCE" "89x89" "$TARGET_DIR/Square89x89Logo.png"
generate_icon "$TRAY_ICON_SOURCE" "107x107" "$TARGET_DIR/Square107x107Logo.png"
generate_icon "$TRAY_ICON_SOURCE" "142x142" "$TARGET_DIR/Square142x142Logo.png"
echo "状态栏图标生成完成！"

# 生成 macOS 图标
generate_icns
echo "macOS 图标生成完成！"

echo "所有图标生成完成！"
