# =================================================================
# Setup Environment Variables for Building
# =================================================================
# This script automatically configures the build environment by setting
# up the necessary environment variables for building the project.
#
# It dynamically detects and sets up:
# - Visual Studio installation path and version
# - Windows SDK location and version
# - VCPKG root directory
# - Include paths for headers
# - Library paths for linking
# - Binary paths for tools
#
# Features:
# - Automatic detection of latest Visual Studio installation
# - Dynamic Windows SDK version detection
# - Flexible VCPKG location discovery
# - Comprehensive error checking
# - Detailed progress reporting
#
# Usage:
#   Method 1 (Recommended):
#     .\build.bat
#
#   Method 2 (Direct PowerShell):
#     powershell -ExecutionPolicy Bypass -File setup-env.ps1
#
# Note: If you encounter execution policy restrictions, you can:
# 1. Use build.bat (recommended)
# 2. Run with -ExecutionPolicy Bypass
# 3. Set execution policy to RemoteSigned (requires admin):
#    Set-ExecutionPolicy RemoteSigned
# =================================================================

# Get Visual Studio installation path using vswhere
$vsWherePath = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vsWherePath)) {
    Write-Error "Visual Studio installer not found. Please install Visual Studio first."
    exit 1
}

# Get latest Visual Studio installation path
$vsInstallPath = & $vsWherePath -latest -property installationPath
if (-not $vsInstallPath) {
    Write-Error "Visual Studio installation not found."
    exit 1
}
$env:VSINSTALLDIR = $vsInstallPath

# Get VC Tools version from Visual Studio installation
$vcToolsVersionFile = Join-Path $vsInstallPath "VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt"
if (Test-Path $vcToolsVersionFile) {
    $env:VCToolsVersion = (Get-Content $vcToolsVersionFile).Trim()
} else {
    Write-Error "VC Tools version file not found."
    exit 1
}

# Get Windows SDK information from registry
$sdkRegPath = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0"
if (Test-Path $sdkRegPath) {
    $sdkProps = Get-ItemProperty $sdkRegPath
    $env:WindowsSdkDir = $sdkProps.InstallationFolder
    $env:WindowsSdkVersion = $sdkProps.ProductVersion + ".0"
} else {
    Write-Error "Windows SDK not found in registry."
    exit 1
}

# Set VCPKG_ROOT based on user directory installation first
$userVcpkgPath = Join-Path $env:USERPROFILE "vcpkg"
if (Test-Path $userVcpkgPath) {
    # Use .NET Framework method to set environment variable, ensuring to overwrite existing value
    [System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', $userVcpkgPath, [System.EnvironmentVariableTarget]::User)
    $env:VCPKG_ROOT = $userVcpkgPath
    # Set default triplet for static linking
    [System.Environment]::SetEnvironmentVariable('VCPKG_DEFAULT_TRIPLET', 'x64-windows-static', [System.EnvironmentVariableTarget]::User)
    $env:VCPKG_DEFAULT_TRIPLET = "x64-windows-static"
} else {
    # Try Visual Studio installation
    $vcpkgPath = Join-Path $env:VSINSTALLDIR "VC\vcpkg"
    if (Test-Path $vcpkgPath) {
        [System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', $vcpkgPath, [System.EnvironmentVariableTarget]::User)
        $env:VCPKG_ROOT = $vcpkgPath
        # Set default triplet for static linking
        [System.Environment]::SetEnvironmentVariable('VCPKG_DEFAULT_TRIPLET', 'x64-windows-static', [System.EnvironmentVariableTarget]::User)
        $env:VCPKG_DEFAULT_TRIPLET = "x64-windows-static"
    } else {
        # Try other common locations
        $commonVcpkgPaths = @(
            "C:\vcpkg",
            "${env:LOCALAPPDATA}\vcpkg"
        )

        $foundVcpkg = $false
        foreach ($path in $commonVcpkgPaths) {
            if (Test-Path $path) {
                [System.Environment]::SetEnvironmentVariable('VCPKG_ROOT', $path, [System.EnvironmentVariableTarget]::User)
                $env:VCPKG_ROOT = $path
                # Set default triplet for static linking
                [System.Environment]::SetEnvironmentVariable('VCPKG_DEFAULT_TRIPLET', 'x64-windows-static', [System.EnvironmentVariableTarget]::User)
                $env:VCPKG_DEFAULT_TRIPLET = "x64-windows-static"
                $foundVcpkg = $true
                break
            }
        }

        if (-not $foundVcpkg) {
            Write-Warning "vcpkg not found. Please install vcpkg and set VCPKG_ROOT manually if needed."
        }
    }
}

Write-Host "VCPKG Root: $env:VCPKG_ROOT"

# Build paths
$VsPath = "$env:VSINSTALLDIR\VC\Tools\MSVC\$env:VCToolsVersion"
$SdkPath = "$env:WindowsSdkDir"

# Set VCINSTALLDIR
$env:VCINSTALLDIR = "$env:VSINSTALLDIR\VC\"

# Set include paths
$includePaths = @(
    "$VsPath\include",
    "$VsPath\atlmfc\include",
    "$SdkPath\Include\$env:WindowsSdkVersion\ucrt",
    "$SdkPath\Include\$env:WindowsSdkVersion\um",
    "$SdkPath\Include\$env:WindowsSdkVersion\shared"
)
[Environment]::SetEnvironmentVariable('INCLUDE', ($includePaths -join ";"), 'Process')
$env:INCLUDE = $includePaths -join ";"

# Set library paths
$libPaths = @(
    "$VsPath\lib\x64",
    "$SdkPath\Lib\$env:WindowsSdkVersion\um\x64",
    "$SdkPath\Lib\$env:WindowsSdkVersion\ucrt\x64"
)
[Environment]::SetEnvironmentVariable('LIB', ($libPaths -join ";"), 'Process')
$env:LIB = $libPaths -join ";"

# Set LIBPATH for .NET Framework
$libPaths = @(
    "$VsPath\lib\x64",
    "$VsPath\atlmfc\lib\x64"
)
[Environment]::SetEnvironmentVariable('LIBPATH', ($libPaths -join ";"), 'Process')
$env:LIBPATH = $libPaths -join ";"

# Set tool paths
$toolPaths = @(
    "$VsPath\bin\HostX64\x64",
    "$SdkPath\bin\$env:WindowsSdkVersion\x64",
    $env:Path
)
[Environment]::SetEnvironmentVariable('Path', ($toolPaths -join ";"), 'Process')
$env:Path = $toolPaths -join ";"

# Set additional VS environment variables
[Environment]::SetEnvironmentVariable('Platform', "x64", 'Process')
$env:Platform = "x64"
[Environment]::SetEnvironmentVariable('VSCMD_ARG_HOST_ARCH', "x64", 'Process')
$env:VSCMD_ARG_HOST_ARCH = "x64"
[Environment]::SetEnvironmentVariable('VSCMD_ARG_TGT_ARCH', "x64", 'Process')
$env:VSCMD_ARG_TGT_ARCH = "x64"
[Environment]::SetEnvironmentVariable('PreferredToolArchitecture', "x64", 'Process')
$env:PreferredToolArchitecture = "x64"

Write-Host "Environment variables have been set for Visual Studio and Windows SDK."
Write-Host "VS Installation: $env:VSINSTALLDIR"
Write-Host "VC Tools Version: $env:VCToolsVersion"
Write-Host "Windows SDK: $env:WindowsSdkDir"
Write-Host "Windows SDK Version: $env:WindowsSdkVersion"
if ($env:VCPKG_ROOT) {
    Write-Host "VCPKG Root: $env:VCPKG_ROOT"
}

Write-Host "LIB paths:"
$env:LIB -split ";" | ForEach-Object {
    if (Test-Path $_) {
        Write-Host "✓ $_"
    } else {
        Write-Host "✗ $_"
    }
}

# Test for kernel32.lib
$kernel32Paths = @(
    "$SdkPath\Lib\$env:WindowsSdkVersion\um\x64\kernel32.lib",
    "$VsPath\lib\x64\kernel32.lib"
)

Write-Host "`nChecking for kernel32.lib:"
$kernel32Paths | ForEach-Object {
    if (Test-Path $_) {
        Write-Host "✓ $_"
    } else {
        Write-Host "✗ $_"
    }
}
