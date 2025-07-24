# Windows PowerShell Debug Script for Tauri v2 + Vue3 Project
# This script helps coordinate frontend and backend debugging on Windows

param(
    [string]$Mode = "menu",
    [switch]$Help
)

# Colors for output
$Colors = @{
    Red    = "Red"
    Green  = "Green"
    Yellow = "Yellow"
    Blue   = "Cyan"
    NC     = "White"
}

function Write-ColorOutput {
    param(
        [string]$Message,
        [string]$Color = "White"
    )
    Write-Host $Message -ForegroundColor $Color
}

function Show-Header {
    Write-ColorOutput "🚀 Tauri Windows Debug Helper" $Colors.Blue
    Write-ColorOutput "===============================" $Colors.Blue
}

function Show-Help {
    Show-Header
    Write-Host @"
Usage: .\debug.ps1 [OPTIONS]

Options:
    -Mode <mode>    Specify debug mode directly
                    Values: fullstack, backend, frontend, clean
    -Help           Show this help message

Modes:
    menu           Show interactive menu (default)
    fullstack      Full Stack Debug (Frontend + Backend)
    backend        Backend Only Debug
    frontend       Frontend Only (Dev Server)
    clean          Clean ports and exit

Examples:
    .\debug.ps1                    # Show menu
    .\debug.ps1 -Mode fullstack    # Direct full stack debug
    .\debug.ps1 -Mode clean        # Clean ports
    .\debug.ps1 -Help              # Show help

"@
}

function Test-Port {
    param([int]$Port)

    try {
        $connection = Test-NetConnection -ComputerName "localhost" -Port $Port -WarningAction SilentlyContinue
        return $connection.TcpTestSucceeded
    }
    catch {
        return $false
    }
}

function Stop-ProcessOnPort {
    param([int]$Port)

    try {
        $processes = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue |
                    Select-Object -ExpandProperty OwningProcess |
                    Sort-Object -Unique

        if ($processes) {
            Write-ColorOutput "Killing processes on port $Port..." $Colors.Yellow
            foreach ($pid in $processes) {
                try {
                    $process = Get-Process -Id $pid -ErrorAction SilentlyContinue
                    if ($process) {
                        Write-ColorOutput "Stopping process: $($process.Name) (PID: $pid)" $Colors.Yellow
                        Stop-Process -Id $pid -Force
                        Start-Sleep -Seconds 1
                    }
                }
                catch {
                    Write-ColorOutput "Could not stop process with PID: $pid" $Colors.Red
                }
            }
        }
    }
    catch {
        Write-ColorOutput "Error managing port $Port : $($_.Exception.Message)" $Colors.Red
    }
}

function Start-FrontendServer {
    Write-ColorOutput "Starting frontend dev server..." $Colors.Green

    # Start yarn dev in a new PowerShell window
    $job = Start-Job -ScriptBlock {
        Set-Location $using:PWD
        yarn dev
    }

    Write-ColorOutput "Waiting for frontend server to start..." $Colors.Yellow
    $maxAttempts = 30
    $attempt = 0

    do {
        Start-Sleep -Seconds 1
        $attempt++
        Write-Host "." -NoNewline

        if (Test-Port -Port 1420) {
            Write-Host ""
            Write-ColorOutput "✅ Frontend server is running on http://localhost:1420" $Colors.Green
            return $job
        }
    } while ($attempt -lt $maxAttempts)

    Write-Host ""
    Write-ColorOutput "❌ Frontend server failed to start after 30 seconds" $Colors.Red
    Stop-Job -Job $job -PassThru | Remove-Job
    return $null
}

function Test-Dependencies {
    Write-ColorOutput "Checking dependencies..." $Colors.Blue

    $missing = @()

    # Check Node.js
    try {
        $nodeVersion = node --version 2>$null
        Write-ColorOutput "✅ Node.js: $nodeVersion" $Colors.Green
    }
    catch {
        $missing += "Node.js"
        Write-ColorOutput "❌ Node.js not found" $Colors.Red
    }

    # Check Yarn
    try {
        $yarnVersion = yarn --version 2>$null
        Write-ColorOutput "✅ Yarn: $yarnVersion" $Colors.Green
    }
    catch {
        $missing += "Yarn"
        Write-ColorOutput "❌ Yarn not found" $Colors.Red
    }

    # Check Rust
    try {
        $rustVersion = rustc --version 2>$null
        Write-ColorOutput "✅ Rust: $rustVersion" $Colors.Green
    }
    catch {
        $missing += "Rust"
        Write-ColorOutput "❌ Rust not found" $Colors.Red
    }

    # Check Cargo
    try {
        $cargoVersion = cargo --version 2>$null
        Write-ColorOutput "✅ Cargo: $cargoVersion" $Colors.Green
    }
    catch {
        $missing += "Cargo"
        Write-ColorOutput "❌ Cargo not found" $Colors.Red
    }

    if ($missing.Count -gt 0) {
        Write-ColorOutput "" $Colors.NC
        Write-ColorOutput "❌ Missing dependencies: $($missing -join ', ')" $Colors.Red
        Write-ColorOutput "Please install missing dependencies and try again." $Colors.Yellow
        Write-ColorOutput "" $Colors.NC
        Write-ColorOutput "Installation commands:" $Colors.Blue
        Write-ColorOutput "- Node.js: winget install OpenJS.NodeJS" $Colors.NC
        Write-ColorOutput "- Yarn: npm install -g yarn" $Colors.NC
        Write-ColorOutput "- Rust: winget install Rustlang.Rust" $Colors.NC
        return $false
    }

    return $true
}

function Invoke-FullStackDebug {
    Write-ColorOutput "🔧 Starting Full Stack Debug Mode" $Colors.Blue

    # Check if frontend port is already in use
    if (Test-Port -Port 1420) {
        Write-ColorOutput "Port 1420 is already in use. Kill existing process? (y/n): " $Colors.Yellow -NoNewline
        $response = Read-Host
        if ($response -match '^[Yy]') {
            Stop-ProcessOnPort -Port 1420
        }
        else {
            Write-ColorOutput "Cannot proceed with port 1420 in use" $Colors.Red
            return
        }
    }

    # Start frontend
    $frontendJob = Start-FrontendServer
    if ($null -eq $frontendJob) {
        Write-ColorOutput "Failed to start frontend server" $Colors.Red
        return
    }

    Write-ColorOutput "🎯 Frontend is ready! Now start VS Code debugger with Windows MSVC configuration" $Colors.Green
    Write-ColorOutput "Press Ctrl+C to stop both frontend and backend" $Colors.Yellow

    try {
        # Keep the script running to maintain the frontend server
        Write-ColorOutput "Frontend server is running. Press Ctrl+C to stop..." $Colors.Blue
        while ($true) {
            Start-Sleep -Seconds 5
            if (-not (Test-Port -Port 1420)) {
                Write-ColorOutput "Frontend server appears to have stopped" $Colors.Red
                break
            }
        }
    }
    finally {
        if ($frontendJob) {
            Write-ColorOutput "Cleaning up frontend server..." $Colors.Yellow
            Stop-Job -Job $frontendJob -PassThru | Remove-Job
        }
        Stop-ProcessOnPort -Port 1420
    }
}

function Invoke-BackendDebug {
    Write-ColorOutput "🔧 Backend Only Debug Mode" $Colors.Blue
    Write-ColorOutput "Make sure frontend dev server is running separately!" $Colors.Yellow
    Write-ColorOutput "Now start VS Code debugger with 'Windows MSVC Backend' configuration" $Colors.Green
    Write-ColorOutput "To start frontend separately, run: yarn dev" $Colors.Blue
}

function Invoke-FrontendDebug {
    Write-ColorOutput "🔧 Frontend Only Mode" $Colors.Blue

    if (Test-Port -Port 1420) {
        Write-ColorOutput "Port 1420 is already in use. Kill existing process? (y/n): " $Colors.Yellow -NoNewline
        $response = Read-Host
        if ($response -match '^[Yy]') {
            Stop-ProcessOnPort -Port 1420
        }
        else {
            Write-ColorOutput "Cannot proceed with port 1420 in use" $Colors.Red
            return
        }
    }

    $frontendJob = Start-FrontendServer
    if ($frontendJob) {
        Write-ColorOutput "✅ Frontend dev server is running" $Colors.Green
        Write-ColorOutput "Press Ctrl+C to stop" $Colors.Yellow

        try {
            while ($true) {
                Start-Sleep -Seconds 5
                if (-not (Test-Port -Port 1420)) {
                    Write-ColorOutput "Frontend server appears to have stopped" $Colors.Red
                    break
                }
            }
        }
        finally {
            Stop-Job -Job $frontendJob -PassThru | Remove-Job
        }
    }
}

function Invoke-CleanPorts {
    Write-ColorOutput "🧹 Cleaning up ports" $Colors.Blue
    Stop-ProcessOnPort -Port 1420

    # Also try to kill any yarn or node processes
    try {
        Get-Process -Name "node" -ErrorAction SilentlyContinue | Stop-Process -Force
        Get-Process -Name "yarn" -ErrorAction SilentlyContinue | Stop-Process -Force
        Write-ColorOutput "✅ Ports and processes cleaned" $Colors.Green
    }
    catch {
        Write-ColorOutput "Some processes could not be stopped" $Colors.Yellow
    }
}

function Show-Menu {
    Write-ColorOutput "Choose debug mode:" $Colors.Blue
    Write-Host "1) Full Stack Debug (Frontend + Backend)"
    Write-Host "2) Backend Only Debug"
    Write-Host "3) Frontend Only (Dev Server)"
    Write-Host "4) Clean ports and exit"
    Write-Host "5) Check dependencies"
    Write-Host "6) Show environment info"
    Write-Host "0) Exit"
    Write-Host ""
}

function Show-EnvironmentInfo {
    Write-ColorOutput "🔍 Environment Information" $Colors.Blue
    Write-ColorOutput "=========================" $Colors.Blue

    # System info
    Write-ColorOutput "OS Version: $(Get-ComputerInfo | Select-Object -ExpandProperty WindowsProductName)" $Colors.NC
    Write-ColorOutput "PowerShell Version: $($PSVersionTable.PSVersion)" $Colors.NC

    # Development tools
    Test-Dependencies

    # Project info
    if (Test-Path "package.json") {
        Write-ColorOutput "" $Colors.NC
        Write-ColorOutput "Project Dependencies:" $Colors.Blue
        try {
            $package = Get-Content "package.json" | ConvertFrom-Json
            Write-ColorOutput "Project: $($package.name) v$($package.version)" $Colors.NC
        }
        catch {
            Write-ColorOutput "Could not read package.json" $Colors.Red
        }
    }

    # Check Tauri info
    try {
        Write-ColorOutput "" $Colors.NC
        Write-ColorOutput "Running yarn tauri info..." $Colors.Blue
        yarn tauri info
    }
    catch {
        Write-ColorOutput "Could not run 'yarn tauri info'" $Colors.Red
    }
}

# Main script logic
function Main {
    # Handle help
    if ($Help) {
        Show-Help
        return
    }

    Show-Header

    # Get the directory where this script is located and change to project root
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $projectRoot = Split-Path -Parent $scriptDir
    Set-Location $projectRoot

    # Check if we're in the right directory
    if (-not (Test-Path "package.json") -or -not (Test-Path "src-tauri")) {
        Write-ColorOutput "❌ This doesn't appear to be a Tauri project directory" $Colors.Red
        Write-ColorOutput "Script location: $scriptDir" $Colors.Yellow
        Write-ColorOutput "Expected project root: $projectRoot" $Colors.Yellow
        return
    }

    Write-ColorOutput "✅ Running from project root: $projectRoot" $Colors.Green

    # Handle direct mode
    if ($Mode -ne "menu") {
        switch ($Mode.ToLower()) {
            "fullstack" {
                if (Test-Dependencies) { Invoke-FullStackDebug }
            }
            "backend" { Invoke-BackendDebug }
            "frontend" {
                if (Test-Dependencies) { Invoke-FrontendDebug }
            }
            "clean" { Invoke-CleanPorts }
            default {
                Write-ColorOutput "Invalid mode: $Mode" $Colors.Red
                Show-Help
            }
        }
        return
    }

    # Interactive menu
    do {
        Show-Menu
        $choice = Read-Host "Enter your choice (0-6)"

        switch ($choice) {
            "1" {
                if (Test-Dependencies) {
                    Invoke-FullStackDebug
                }
                break
            }
            "2" {
                Invoke-BackendDebug
                Read-Host "Press Enter to continue..."
                break
            }
            "3" {
                if (Test-Dependencies) {
                    Invoke-FrontendDebug
                }
                break
            }
            "4" {
                Invoke-CleanPorts
                Read-Host "Press Enter to continue..."
                break
            }
            "5" {
                Test-Dependencies | Out-Null
                Read-Host "Press Enter to continue..."
                break
            }
            "6" {
                Show-EnvironmentInfo
                Read-Host "Press Enter to continue..."
                break
            }
            "0" {
                Write-ColorOutput "Goodbye!" $Colors.Green
                return
            }
            default {
                Write-ColorOutput "Invalid choice. Please try again." $Colors.Red
                Start-Sleep -Seconds 1
            }
        }

        Clear-Host
        Show-Header

    } while ($choice -ne "0")
}

# Trap Ctrl+C to clean up
$null = Register-EngineEvent -SourceIdentifier PowerShell.Exiting -Action {
    Write-Host "Cleaning up..."
    Stop-ProcessOnPort -Port 1420
}

# Run main function with error handling
try {
    Main
}
catch {
    Write-ColorOutput "An error occurred: $($_.Exception.Message)" $Colors.Red
    Write-ColorOutput "Stack Trace: $($_.ScriptStackTrace)" $Colors.Red
}
finally {
    # Clean up any jobs
    Get-Job | Stop-Job -PassThru | Remove-Job
}
