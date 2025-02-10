@echo off
powershell -ExecutionPolicy Bypass -Command ". .\setup-env.ps1; yarn tauri dev"

