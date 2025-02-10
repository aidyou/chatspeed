@echo off
rem ================================================================
rem Setup Environment Variables for Building
rem ================================================================
rem This script is maintained for compatibility with older systems or
rem environments where PowerShell might not be available.
rem
rem It sets up the necessary environment variables for:
rem - Visual Studio Build Tools
rem - Windows SDK
rem - VCPKG
rem
rem NOTE: It is recommended to use setup-env.ps1 instead of this script
rem as the PowerShell version provides better error handling and is
rem more maintainable.
rem
rem Usage:
rem   setup-env.bat
rem ================================================================

setlocal enabledelayedexpansion

rem Find Visual Studio installation using vswhere
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo Visual Studio installer not found. Please install Visual Studio first.
    exit /b 1
)

rem Get latest Visual Studio path
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -property installationPath`) do (
    set VSINSTALLDIR=%%i
)

if not defined VSINSTALLDIR (
    echo Visual Studio installation not found.
    exit /b 1
)

rem Get VC Tools version
set "VCVERSION_FILE=%VSINSTALLDIR%\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt"
if exist "%VCVERSION_FILE%" (
    for /f "tokens=* usebackq" %%i in ("%VCVERSION_FILE%") do (
        set VCToolsVersion=%%i
    )
) else (
    echo VC Tools version file not found.
    exit /b 1
)

rem Get Windows SDK version from registry
for /f "tokens=3" %%a in ('reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0" /v ProductVersion') do (
    set WindowsSdkVersion=%%a.0
)

rem Get Windows SDK path from registry
for /f "tokens=2*" %%a in ('reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0" /v InstallationFolder') do (
    set WindowsSdkDir=%%b
)

if not defined WindowsSdkDir (
    echo Windows SDK not found.
    exit /b 1
)

rem Set Windows SDK paths
set WindowsLibPath=%WindowsSdkDir%UnionMetadata\%WindowsSdkVersion%;%WindowsSdkDir%References\%WindowsSdkVersion%

rem Set include paths
set INCLUDE=%VSINSTALLDIR%\VC\Tools\MSVC\%VCToolsVersion%\include
set INCLUDE=%INCLUDE%;%VSINSTALLDIR%\VC\Tools\MSVC\%VCToolsVersion%\atlmfc\include
set INCLUDE=%INCLUDE%;%WindowsSdkDir%Include\%WindowsSdkVersion%\ucrt
set INCLUDE=%INCLUDE%;%WindowsSdkDir%Include\%WindowsSdkVersion%\um
set INCLUDE=%INCLUDE%;%WindowsSdkDir%Include\%WindowsSdkVersion%\shared

rem Set library paths
set LIB=%VSINSTALLDIR%\VC\Tools\MSVC\%VCToolsVersion%\lib\x64
set LIB=%LIB%;%WindowsSdkDir%Lib\%WindowsSdkVersion%\um\x64
set LIB=%LIB%;%WindowsSdkDir%Lib\%WindowsSdkVersion%\ucrt\x64

rem Set path
set Path=%VSINSTALLDIR%\VC\Tools\MSVC\%VCToolsVersion%\bin\HostX64\x64;%Path%
set Path=%WindowsSdkDir%bin\%WindowsSdkVersion%\x64;%Path%

rem Try to find vcpkg in user profile first
if exist "%USERPROFILE%\vcpkg" (
    set VCPKG_ROOT=%USERPROFILE%\vcpkg
    set VCPKG_DEFAULT_TRIPLET=x64-windows-static
) else if exist "%VSINSTALLDIR%\VC\vcpkg" (
    set VCPKG_ROOT=%VSINSTALLDIR%\VC\vcpkg
    set VCPKG_DEFAULT_TRIPLET=x64-windows-static
) else if exist "C:\vcpkg" (
    set VCPKG_ROOT=C:\vcpkg
    set VCPKG_DEFAULT_TRIPLET=x64-windows-static
) else if exist "%LOCALAPPDATA%\vcpkg" (
    set VCPKG_ROOT=%LOCALAPPDATA%\vcpkg
    set VCPKG_DEFAULT_TRIPLET=x64-windows-static
) else (
    echo Warning: vcpkg not found in common locations.
)

echo Environment variables have been set for Visual Studio and Windows SDK.
echo VS Installation: %VSINSTALLDIR%
echo VC Tools Version: %VCToolsVersion%
echo Windows SDK: %WindowsSdkDir%
echo Windows SDK Version: %WindowsSdkVersion%
if defined VCPKG_ROOT echo VCPKG Root: %VCPKG_ROOT%

endlocal & (
    set "VSINSTALLDIR=%VSINSTALLDIR%"
    set "WindowsSdkDir=%WindowsSdkDir%"
    set "WindowsSdkVersion=%WindowsSdkVersion%"
    set "VCToolsVersion=%VCToolsVersion%"
    set "INCLUDE=%INCLUDE%"
    set "LIB=%LIB%"
    set "Path=%Path%"
    if defined VCPKG_ROOT set "VCPKG_ROOT=%VCPKG_ROOT%"
    if defined VCPKG_DEFAULT_TRIPLET set "VCPKG_DEFAULT_TRIPLET=%VCPKG_DEFAULT_TRIPLET%"
)
