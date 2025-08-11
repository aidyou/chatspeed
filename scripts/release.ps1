# Chatspeed Release Script for Windows PowerShell
# 自动更新版本号并创建发布标签

param(
    [string]$Version = ""
)

# 设置错误处理
$ErrorActionPreference = "Stop"

# 颜色函数
function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Blue
}

function Write-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] $Message" -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARNING] $Message" -ForegroundColor Yellow
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

# 验证版本号格式
function Test-Version {
    param([string]$Version)

    # 移除可能的 v 前缀
    $Version = $Version -replace '^v', ''

    # 验证语义化版本格式
    if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$') {
        Write-Error "Invalid version format: $Version"
        Write-Error "Expected format: x.y.z or x.y.z-prerelease (e.g., 1.0.0, 1.0.0-beta.1)"
        exit 1
    }

    return $Version
}

# 获取当前版本
function Get-CurrentVersion {
    if (Test-Path "src-tauri/tauri.conf.json") {
        $content = Get-Content "src-tauri/tauri.conf.json" -Raw
        if ($content -match '"version":\s*"([^"]*)"') {
            return $matches[1]
        }
    }
    return "unknown"
}

# 更新 tauri.conf.json 中的版本
function Update-TauriConfig {
    param([string]$Version)

    $file = "src-tauri/tauri.conf.json"

    if (-not (Test-Path $file)) {
        Write-Error "File not found: $file"
        exit 1
    }

    $content = Get-Content $file -Raw
    $content = $content -replace '"version":\s*"[^"]*"', "`"version`": `"$Version`""
    Set-Content $file $content -NoNewline

    Write-Success "Updated version in $file to $Version"
}

# 更新 Cargo.toml 中的版本
function Update-CargoToml {
    param([string]$Version)

    $file = "src-tauri/Cargo.toml"

    if (-not (Test-Path $file)) {
        Write-Error "File not found: $file"
        exit 1
    }

    $content = Get-Content $file
    $content = $content -replace '^version\s*=\s*"[^"]*"', "version     = `"$Version`""
    Set-Content $file $content

    Write-Success "Updated version in $file to $Version"
}

# 验证文件中的版本是否正确更新
function Test-VersionUpdate {
    param([string]$ExpectedVersion)

    $tauriContent = Get-Content "src-tauri/tauri.conf.json" -Raw
    $cargoContent = Get-Content "src-tauri/Cargo.toml" -Raw

    if ($tauriContent -match '"version":\s*"([^"]*)"') {
        $tauriVersion = $matches[1]
    } else {
        Write-Error "Could not find version in tauri.conf.json"
        exit 1
    }

    if ($cargoContent -match '^version\s*=\s*"([^"]*)"') {
        $cargoVersion = $matches[1]
    } else {
        Write-Error "Could not find version in Cargo.toml"
        exit 1
    }

    if ($tauriVersion -ne $ExpectedVersion) {
        Write-Error "Version mismatch in tauri.conf.json: expected $ExpectedVersion, got $tauriVersion"
        exit 1
    }

    if ($cargoVersion -ne $ExpectedVersion) {
        Write-Error "Version mismatch in Cargo.toml: expected $ExpectedVersion, got $cargoVersion"
        exit 1
    }

    Write-Success "Version verification passed: $ExpectedVersion"
}

# 检查工作目录状态
function Test-GitStatus {
    $status = git status --porcelain
    if ($status) {
        Write-Warning "You have uncommitted changes in your working directory."
        $response = Read-Host "Do you want to continue? (y/N)"
        if ($response -notmatch '^[Yy]$') {
            Write-Info "Release cancelled."
            exit 0
        }
    }
}

# 创建 Git 提交和标签
function New-GitRelease {
    param([string]$Version)

    $tag = "v$Version"

    Write-Info "Creating Git commit and tag..."

    # 添加修改的文件
    git add src-tauri/tauri.conf.json src-tauri/Cargo.toml

    # 创建提交
    git commit -m "chore: release version $Version"
    Write-Success "Created commit for version $Version"

    # 创建标签
    git tag $tag
    Write-Success "Created tag $tag"

    # 推送到远程
    Write-Info "Pushing to remote repository..."
    git push origin HEAD
    git push origin $tag
    Write-Success "Pushed commit and tag to remote repository"
}

# 显示使用说明
function Show-Usage {
    Write-Host "Usage: .\release.ps1 [version]"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\release.ps1                    # Interactive mode"
    Write-Host "  .\release.ps1 1.0.0             # Release version 1.0.0"
    Write-Host "  .\release.ps1 v1.0.1-beta.1     # Release pre-release version"
    Write-Host ""
    Write-Host "The script will:"
    Write-Host "  1. Update version in src-tauri/tauri.conf.json"
    Write-Host "  2. Update version in src-tauri/Cargo.toml"
    Write-Host "  3. Create a Git commit"
    Write-Host "  4. Create a Git tag (v{version})"
    Write-Host "  5. Push commit and tag to remote repository"
}

# 主函数
function Main {
    Write-Info "Chatspeed Release Script"
    Write-Info "========================"

    # 检查是否在项目根目录
    if (-not (Test-Path "src-tauri/tauri.conf.json") -or -not (Test-Path "src-tauri/Cargo.toml")) {
        Write-Error "Please run this script from the project root directory"
        exit 1
    }

    # 检查 Git 仓库
    try {
        git rev-parse --git-dir | Out-Null
    } catch {
        Write-Error "Not a Git repository"
        exit 1
    }

    $currentVersion = Get-CurrentVersion

    # 处理命令行参数
    if (-not $Version) {
        # 交互模式
        Write-Info "Current version: $currentVersion"
        $Version = Read-Host "Enter new version (e.g., 1.0.0 or 1.0.0-beta.1)"

        if (-not $Version) {
            Write-Error "Version cannot be empty"
            exit 1
        }
    }

    if ($Version -eq "-h" -or $Version -eq "--help") {
        Show-Usage
        exit 0
    }

    # 验证版本号
    $Version = Test-Version $Version

    # 显示操作摘要
    Write-Host ""
    Write-Info "Release Summary:"
    Write-Info "  Current version: $currentVersion"
    Write-Info "  New version: $Version"
    Write-Info "  Tag: v$Version"
    Write-Host ""

    # 确认操作
    $response = Read-Host "Do you want to proceed with the release? (y/N)"
    if ($response -notmatch '^[Yy]$') {
        Write-Info "Release cancelled."
        exit 0
    }

    # 检查 Git 状态
    Test-GitStatus

    # 执行发布流程
    Write-Info "Starting release process..."

    try {
        # 更新版本文件
        Update-TauriConfig $Version
        Update-CargoToml $Version

        # 验证更新
        Test-VersionUpdate $Version

        # 创建 Git 发布
        New-GitRelease $Version

        Write-Host ""
        Write-Success "🎉 Release $Version completed successfully!"
        Write-Info "GitHub Actions will now build and create the release automatically."
        Write-Info "Check the progress at: https://github.com/aidyou/chatspeed/actions"
    } catch {
        Write-Error "Release failed: $($_.Exception.Message)"
        Write-Info "Please check the error and try again."
        exit 1
    }
}

# 运行主函数
Main