#!/bin/bash

# Chatspeed Release Script - 简化版
# 自动更新版本号并创建发布标签

set -e  # 遇到错误立即退出

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 打印带颜色的消息
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 验证版本号格式
validate_version() {
    local version=$1
    # 移除可能的 v 前缀
    version=${version#v}

    # 验证语义化版本格式
    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
        print_error "Invalid version format: $version"
        print_error "Expected format: x.y.z or x.y.z-prerelease (e.g., 1.0.0, 1.0.0-beta.1)"
        exit 1
    fi

    echo $version
}

# 获取当前版本
get_current_version() {
    if [[ -f "src-tauri/tauri.conf.json" ]]; then
        grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/'
    else
        echo "unknown"
    fi
}

# 更新 tauri.conf.json 中的版本
update_tauri_config() {
    local version=$1
    local file="src-tauri/tauri.conf.json"

    if [[ ! -f "$file" ]]; then
        print_error "File not found: $file"
        exit 1
    fi

    # 使用 sed 更新版本号
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        sed -i '' "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    else
        # Linux
        sed -i "s/\"version\": *\"[^\"]*\"/\"version\": \"$version\"/" "$file"
    fi

    print_success "Updated version in $file to $version"
}

# 更新 Cargo.toml 中的版本
update_cargo_toml() {
    local version=$1
    local file="src-tauri/Cargo.toml"

    if [[ ! -f "$file" ]]; then
        print_error "File not found: $file"
        exit 1
    fi

    # 使用 sed 更新版本号
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        sed -i '' "s/^version[[:space:]]*=[[:space:]]*\"[^\"]*\"/version     = \"$version\"/" "$file"
    else
        # Linux
        sed -i "s/^version[[:space:]]*=[[:space:]]*\"[^\"]*\"/version     = \"$version\"/" "$file"
    fi

    print_success "Updated version in $file to $version"
}

# 验证文件中的版本是否正确更新
verify_version_update() {
    local expected_version=$1

    local tauri_version=$(grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
    local cargo_version=$(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*"\([^"]*\)".*/\1/')

    if [[ "$tauri_version" != "$expected_version" ]]; then
        print_error "Version mismatch in tauri.conf.json: expected $expected_version, got $tauri_version"
        exit 1
    fi

    if [[ "$cargo_version" != "$expected_version" ]]; then
        print_error "Version mismatch in Cargo.toml: expected $expected_version, got $cargo_version"
        exit 1
    fi

    print_success "Version verification passed: $expected_version"
}

# 提交版本更改
commit_version_changes() {
    local version=$1

    # 检查是否有未提交的更改
    if ! git diff --quiet src-tauri/tauri.conf.json src-tauri/Cargo.toml; then
        print_info "Committing version changes..."

        # 添加版本文件到暂存区
        git add src-tauri/tauri.conf.json src-tauri/Cargo.toml

        # 提交更改
        git commit -m "chore: bump version to $version"

        print_success "Committed version changes"
    else
        print_info "No version changes to commit"
    fi
}

# 获取目标远程仓库（优先 GitHub）
get_target_remote() {
    # 优先查找名为 github 的远程仓库
    if git remote get-url github >/dev/null 2>&1; then
        echo "github"
        return
    fi

    # 检查 origin 是否指向 GitHub
    if git remote get-url origin >/dev/null 2>&1; then
        local origin_url=$(git remote get-url origin)
        if [[ "$origin_url" == *"github.com"* ]]; then
            echo "origin"
            return
        fi
    fi

    # 查找其他指向 GitHub 的远程仓库
    for remote in $(git remote); do
        local url=$(git remote get-url "$remote" 2>/dev/null || echo "")
        if [[ "$url" == *"github.com"* ]]; then
            echo "$remote"
            return
        fi
    done

    # 如果没找到 GitHub 远程仓库，使用 origin 作为默认
    echo "origin"
}

create_and_push_tag() {
    local version=$1
    local tag="v$version"
    local remote=$(get_target_remote)

    print_info "Target remote: $remote ($(git remote get-url $remote))"

    # 检查远程标签是否已存在
    if git ls-remote --tags "$remote" | grep -q "refs/tags/$tag$"; then
        print_warning "Tag $tag already exists on remote $remote"
        read -p "Do you want to delete and recreate it? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            # 删除远程标签
            print_info "Deleting remote tag $tag..."
            git push "$remote" ":refs/tags/$tag" || print_warning "Failed to delete remote tag (may not exist)"

            # 删除本地标签
            if git tag -l | grep -q "^$tag$"; then
                git tag -d "$tag"
                print_info "Deleted existing local tag $tag"
            fi
        else
            print_info "Skipping tag creation"
            return
        fi
    fi

    # 检查本地标签是否已存在
    if git tag -l | grep -q "^$tag$"; then
        print_warning "Tag $tag already exists locally"
        git tag -d "$tag"
        print_info "Deleted existing local tag $tag"
    fi

    # 推送版本提交到远程
    print_info "Pushing version commit to remote $remote..."
    if git push "$remote" HEAD; then
        print_success "Successfully pushed version commit to $remote"
    else
        print_error "Failed to push version commit to $remote"
        exit 1
    fi

    # 等待一下确保提交已经到达远程
    sleep 2

    # 创建标签（指向当前 HEAD）
    print_info "Creating tag $tag on current HEAD..."
    git tag "$tag" HEAD
    print_success "Created tag $tag"

    # 验证标签指向正确的提交
    local tag_commit=$(git rev-list -n 1 "$tag")
    local head_commit=$(git rev-parse HEAD)

    if [[ "$tag_commit" != "$head_commit" ]]; then
        print_error "Tag $tag does not point to current HEAD"
        print_error "Tag commit: $tag_commit"
        print_error "HEAD commit: $head_commit"
        exit 1
    fi

    print_info "Tag $tag points to commit: $tag_commit"

    # 推送标签
    print_info "Pushing tag $tag to remote $remote..."
    if git push "$remote" "$tag"; then
        print_success "Successfully pushed tag $tag to $remote"
    else
        print_error "Failed to push tag $tag to $remote"
        print_info "You can try pushing manually: git push $remote $tag"
        exit 1
    fi
}

# 显示使用说明
show_usage() {
    echo "Usage: $0 [version]"
    echo ""
    echo "Examples:"
    echo "  $0                    # Interactive mode"
    echo "  $0 1.0.0             # Release version 1.0.0"
    echo "  $0 v1.0.1-beta.1     # Release pre-release version"
    echo ""
    echo "The script will:"
    echo "  1. Update version in src-tauri/tauri.conf.json"
    echo "  2. Update version in src-tauri/Cargo.toml"
    echo "  3. Create a Git tag (v{version})"
    echo "  4. Push tag to remote repository"
}

# 主函数
main() {
    print_info "Chatspeed Release Script"
    print_info "========================"

    # 检查是否在项目根目录
    if [[ ! -f "src-tauri/tauri.conf.json" ]] || [[ ! -f "src-tauri/Cargo.toml" ]]; then
        print_error "Please run this script from the project root directory"
        exit 1
    fi

    # 检查 Git 仓库
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        print_error "Not a Git repository"
        exit 1
    fi

    local version=""
    local current_version=$(get_current_version)

    # 处理命令行参数
    if [[ $# -eq 0 ]]; then
        # 交互模式
        print_info "Current version: $current_version"
        echo -n "Enter new version (e.g., 1.0.0 or 1.0.0-beta.1): "
        read version

        if [[ -z "$version" ]]; then
            print_error "Version cannot be empty"
            exit 1
        fi
    elif [[ $# -eq 1 ]]; then
        if [[ "$1" == "-h" ]] || [[ "$1" == "--help" ]]; then
            show_usage
            exit 0
        fi
        version=$1
    else
        print_error "Too many arguments"
        show_usage
        exit 1
    fi

    # 验证版本号
    version=$(validate_version "$version")

    # 显示操作摘要
    echo ""
    print_info "Release Summary:"
    print_info "  Current version: $current_version"
    print_info "  New version: $version"
    print_info "  Tag: v$version"
    echo ""

    # 确认操作
    read -p "Do you want to proceed with the release? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_info "Release cancelled."
        exit 0
    fi

    # 执行发布流程
    print_info "Starting release process..."

    # 更新版本文件
    update_tauri_config "$version"
    update_cargo_toml "$version"

    # 验证更新
    verify_version_update "$version"

    # 提交版本更改
    commit_version_changes "$version"

    # 创建和推送标签
    create_and_push_tag "$version"

    echo ""
    print_success "🎉 Release $version completed successfully!"
    print_info "GitHub Actions will now build and create the release automatically."
    print_info "Check the progress at: https://github.com/aidyou/chatspeed/actions"
}

# 运行主函数
main "$@"