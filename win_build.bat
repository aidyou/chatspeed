@echo off
rem ================================================================
rem Windows Platform Build Script
rem ================================================================
rem This script automates the build process for Windows platform by:
rem 1. Setting up the required environment variables
rem 2. Building the Tauri application
rem
rem Prerequisites:
rem - Visual Studio 2022 with C++ build tools
rem - Windows SDK
rem - Node.js and Yarn
rem - Rust and Cargo
rem
rem This script will:
rem - Automatically detect and configure Visual Studio environment
rem - Set up Windows SDK paths
rem - Configure VCPKG if available
rem - Run the Tauri build process
rem
rem Note: This script is specifically designed for Windows platform.
rem For other platforms, please refer to the project documentation.
rem
rem Usage:
rem   .\build.bat
rem
rem The script will handle all necessary environment setup and build
rem steps automatically.
rem ================================================================

echo Setting up build environment and starting build...
powershell -ExecutionPolicy Bypass -Command ". '%~dp0setup-env.ps1'; yarn tauri build"
if errorlevel 1 (
    echo Build failed.
    exit /b 1
)

echo Build completed successfully!

