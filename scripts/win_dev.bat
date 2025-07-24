@echo off
powershell -ExecutionPolicy Bypass -Command ". '%~dp0setup-env.ps1'; cd '%~dp0..'; yarn tauri dev"
