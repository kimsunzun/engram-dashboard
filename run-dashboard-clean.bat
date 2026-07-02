@echo off
REM Clean relaunch: kills the engram daemon first (does NOT touch claude.exe), then runs.
REM Use this if the screen looks wrong (empty slot + cursor only / not showing).
cd /d "%~dp0"
echo [clean] Stopping engram daemon...
taskkill /IM engram-dashboard-daemon.exe /F >nul 2>&1
set "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223"
echo Starting Engram Dashboard... (close window to stop)
call npm run tauri dev
echo.
echo [stopped] Press any key to close.
pause >nul
