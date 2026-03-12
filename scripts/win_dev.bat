@echo off
powershell -ExecutionPolicy Bypass -Command ". '%~dp0setup-env.ps1'; cd '%~dp0..'; pnpm tauri dev"
