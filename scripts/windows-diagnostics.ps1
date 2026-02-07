# Windows 诊断脚本
# 用于诊断 Chatspeed 在 Windows 上的问题

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Chatspeed Windows 诊断工具" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 检查 WebView2 运行时
Write-Host "检查 WebView2 运行时..." -ForegroundColor Yellow
$webview2Path = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
if (Test-Path $webview2Path) {
    $version = (Get-ItemProperty -Path $webview2Path).pv
    Write-Host "✓ WebView2 已安装, 版本: $version" -ForegroundColor Green
} else {
    Write-Host "✗ WebView2 未安装或无法检测" -ForegroundColor Red
    Write-Host "  请从以下地址下载安装: https://go.microsoft.com/fwlink/p/?LinkId=2124703" -ForegroundColor Yellow
}
Write-Host ""

# 检查应用数据目录
Write-Host "检查应用数据目录..." -ForegroundColor Yellow
$appDataDir = "$env:LOCALAPPDATA\ai.aidyou.chatspeed"
Write-Host "应用数据目录: $appDataDir" -ForegroundColor Gray

if (Test-Path $appDataDir) {
    Write-Host "✓ 应用数据目录存在" -ForegroundColor Green
    
    # 检查日志目录
    $logDir = Join-Path $appDataDir "logs"
    if (Test-Path $logDir) {
        Write-Host "✓ 日志目录存在: $logDir" -ForegroundColor Green
        
        # 列出日志文件
        $logFiles = Get-ChildItem -Path $logDir -Filter "*.log" -ErrorAction SilentlyContinue
        if ($logFiles) {
            Write-Host "  发现 $($logFiles.Count) 个日志文件:" -ForegroundColor Gray
            foreach ($file in $logFiles) {
                $sizeKB = [math]::Round($file.Length / 1KB, 2)
                Write-Host "    - $($file.Name) ($sizeKB KB, 修改时间: $($file.LastWriteTime))" -ForegroundColor Gray
            }
        } else {
            Write-Host "  ⚠ 日志目录为空" -ForegroundColor Yellow
        }
    } else {
        Write-Host "✗ 日志目录不存在" -ForegroundColor Red
    }
    
    # 检查数据库
    $dbFile = Join-Path $appDataDir "chatspeed.db"
    if (Test-Path $dbFile) {
        $dbSize = [math]::Round((Get-Item $dbFile).Length / 1KB, 2)
        Write-Host "✓ 数据库文件存在: $dbFile ($dbSize KB)" -ForegroundColor Green
    } else {
        Write-Host "✗ 数据库文件不存在" -ForegroundColor Red
    }
    
    # 检查 WebView 缓存
    $webviewCache = Join-Path $appDataDir "EBWebView"
    if (Test-Path $webviewCache) {
        Write-Host "✓ WebView 缓存目录存在" -ForegroundColor Green
    }
} else {
    Write-Host "✗ 应用数据目录不存在" -ForegroundColor Red
    Write-Host "  这可能是首次运行，或应用从未成功启动" -ForegroundColor Yellow
}
Write-Host ""

# 检查权限
Write-Host "检查目录权限..." -ForegroundColor Yellow
try {
    $acl = Get-Acl $env:LOCALAPPDATA
    Write-Host "✓ 可以访问 LocalAppData 目录" -ForegroundColor Green
} catch {
    Write-Host "✗ 无法访问 LocalAppData 目录" -ForegroundColor Red
    Write-Host "  错误: $($_.Exception.Message)" -ForegroundColor Red
}
Write-Host ""

# 检查.NET Framework (某些依赖可能需要)
Write-Host "检查 .NET Framework..." -ForegroundColor Yellow
$dotnet = Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\NET Framework Setup\NDP' -Recurse | 
    Get-ItemProperty -Name version -ErrorAction SilentlyContinue | 
    Where-Object { $_.PSChildName -match '^(?!S)\p{L}'}
if ($dotnet) {
    Write-Host "✓ .NET Framework 已安装" -ForegroundColor Green
} else {
    Write-Host "⚠ 无法检测 .NET Framework" -ForegroundColor Yellow
}
Write-Host ""

# 尝试查看最近的日志
Write-Host "尝试读取最新日志 (最后20行)..." -ForegroundColor Yellow
$chatspeedLog = Join-Path $logDir "chatspeed.log"
if (Test-Path $chatspeedLog) {
    Write-Host "来自 chatspeed.log:" -ForegroundColor Gray
    Write-Host "----------------------------------------" -ForegroundColor DarkGray
    Get-Content $chatspeedLog -Tail 20 | ForEach-Object { Write-Host $_ -ForegroundColor Gray }
    Write-Host "----------------------------------------" -ForegroundColor DarkGray
} else {
    Write-Host "✗ 找不到 chatspeed.log" -ForegroundColor Red
}
Write-Host ""

# 事件日志检查
Write-Host "检查 Windows 事件日志中的应用程序错误..." -ForegroundColor Yellow
try {
    $events = Get-EventLog -LogName Application -Source "*chatspeed*" -Newest 5 -ErrorAction SilentlyContinue
    if ($events) {
        Write-Host "发现 $($events.Count) 条相关事件:" -ForegroundColor Gray
        foreach ($event in $events) {
            $color = if ($event.EntryType -eq "Error") { "Red" } elseif ($event.EntryType -eq "Warning") { "Yellow" } else { "Gray" }
            Write-Host "  [$($event.EntryType)] $($event.TimeGenerated): $($event.Message.Substring(0, [Math]::Min(100, $event.Message.Length)))..." -ForegroundColor $color
        }
    } else {
        Write-Host "  未发现相关事件" -ForegroundColor Gray
    }
} catch {
    Write-Host "  无法读取事件日志或没有相关条目" -ForegroundColor Gray
}
Write-Host ""

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "诊断完成" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "如果发现问题，请将以上信息发送给开发者" -ForegroundColor Yellow
Write-Host ""
Write-Host "按任意键退出..." -ForegroundColor Gray
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
