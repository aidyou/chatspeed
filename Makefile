# 定义变量
SHELL := /bin/bash
SOURCE_JSON := src-tauri/i18n/available_language.json
TARGET_JSON := src/i18n/do_not_edit/copy_from_rust_src_i18n.json

# 默认目标
.PHONY: all
all: help

# 帮助信息
.PHONY: help
help:
	@echo "Available commands:"
	@echo "  make dev         - Start development environment"
	@echo "  make build-all   - Build for all platforms"
	@echo "  make build-mac   - Build for macOS (Apple Silicon and Intel)"
	@echo "  make build-win   - Build for Windows"
	@echo "  make build-linux - Build for Linux"
	@echo "  make clean       - Clean build artifacts"

# 复制语言文件
.PHONY: copy-lang-file
copy-lang-file:
	@echo "Copying language file..."
	@mkdir -p $(dir $(TARGET_JSON))
	@cp $(SOURCE_JSON) $(TARGET_JSON)
	@echo "Language file copied successfully"

# 开发环境
.PHONY: dev
dev: copy-lang-file
	@echo "Starting development environment..."
	yarn dev

# 构建前的准备工作
.PHONY: prepare
prepare: copy-lang-file
	@echo "Installing dependencies..."
	yarn install
	@echo "Dependencies installed"

# 清理构建产物
.PHONY: clean
clean:
	@echo "Cleaning build artifacts..."
	rm -rf src-tauri/target
	rm -rf node_modules
	rm -f $(TARGET_JSON)

# macOS 构建
.PHONY: build-mac
build-mac: prepare
	@echo "Building for macOS..."
	yarn tauri build --target universal-apple-darwin

# Windows 构建
.PHONY: build-win
build-win: prepare
	@echo "Building for Windows..."
	yarn tauri build --target x86_64-pc-windows-msvc

# Linux 构建
.PHONY: build-linux
build-linux: prepare
	@echo "Building for Linux..."
	yarn tauri build --target x86_64-unknown-linux-gnu

# 构建所有平台
.PHONY: build-all
build-all: build-mac build-win build-linux

# 监听文件变化并同步
.PHONY: watch-lang-file
watch-lang-file:
	@echo "Watching language file changes..."
	@while true; do \
		if [ "$(SOURCE_JSON)" -nt "$(TARGET_JSON)" ]; then \
			$(MAKE) copy-lang-file; \
		fi; \
		sleep 2; \
	done

# 开发环境（带文件监听）
.PHONY: dev-watch
dev-watch:
	@echo "Starting development environment with file watching..."
	@$(MAKE) copy-lang-file
	@($(MAKE) watch-lang-file &)
	yarn dev

# 准备调试环境
.PHONY: prepare-debug
prepare-debug: copy-lang-file
	@echo "Preparing debug environment..."
	yarn install
	@echo "Debug environment ready. You can now start debugging in VSCode"