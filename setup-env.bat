@echo off
setlocal enabledelayedexpansion

:: Try to find Visual Studio installation
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "!VSWHERE!" set "VSWHERE=%ProgramFiles%\Microsoft Visual Studio\Installer\vswhere.exe"

:: Check if VS2022 is installed
if exist "!VSWHERE!" (
    for /f "usebackq tokens=*" %%i in (`"!VSWHERE!" -latest -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath`) do (
        set VSINSTALLDIR=%%i
    )
)

if not defined VSINSTALLDIR (
    echo Visual Studio 2022 with ARM64 support not found
    echo Please install Visual Studio 2022 with the "Desktop development with C++" workload
    echo and the "MSVC v143 - VS 2022 C++ ARM64 build tools" component
    exit /b 1
)

:: Call vcvarsall.bat for complete environment setup
set "VCVARSALL=!VSINSTALLDIR!\VC\Auxiliary\Build\vcvarsall.bat"
if exist "!VCVARSALL!" (
    echo Setting up environment for ARM64 development...
    call "!VCVARSALL!" arm64
    if !ERRORLEVEL! neq 0 (
        echo Failed to setup environment
        exit /b !ERRORLEVEL!
    )
) else (
    echo vcvarsall.bat not found at expected location
    exit /b 1
)

echo Environment variables set for ARM64 development
echo You can now run your build commands

endlocal & set PATH=%PATH%