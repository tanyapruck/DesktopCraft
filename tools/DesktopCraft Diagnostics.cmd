@echo off
setlocal
rem Read-only diagnostics. No administrator rights or installation required.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0DesktopCraft Diagnostics.ps1" -Executable "%~dp0DesktopCraft.exe"
if errorlevel 1 (
    echo Diagnostic collection failed. Check whether your Windows policy allows local PowerShell scripts.
)
echo.
echo No diagnostics are uploaded automatically. Review the JSON report before sharing it.
pause
