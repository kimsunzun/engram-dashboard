@echo off
REM Engram Dashboard launcher (double-click). Close this window to stop the app.
cd /d "%~dp0"
set "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223"
echo Starting Engram Dashboard... (first build may take a moment; close window to stop)
call npm run tauri dev
echo.
echo [stopped] Press any key to close.
pause >nul
