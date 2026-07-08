@echo off
REM Clean relaunch: kills the engram daemon + REBUILDS the backend, then runs.
REM Use this after ANY Rust/backend change, or if the screen looks wrong.
REM
REM ★WHY the rebuild (do not remove): `tauri dev` (beforeDevCommand=npm run dev) rebuilds
REM   only the CLIENT SHELL (engram-dashboard.exe), NOT the daemon binary. Agent I/O runs
REM   in the DAEMON process (ADR-0029), and ensure_daemon reuses a live compatible daemon,
REM   so without an explicit `cargo build` the app keeps connecting to a STALE daemon and
REM   your Rust changes silently have no effect. (does NOT touch claude.exe)
cd /d "%~dp0"
echo [clean] Stopping engram daemon...
taskkill /IM engram-dashboard-daemon.exe /F >nul 2>&1
echo [clean] Rebuilding backend daemon (first change may take ~15-30s)...
cargo build -p engram-dashboard-daemon
if errorlevel 1 (
  echo [clean] BUILD FAILED - see errors above. Not launching.
  pause
  exit /b 1
)
set "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223"
echo Starting Engram Dashboard... (close window to stop)
call npm run tauri dev
echo.
echo [stopped] Press any key to close.
pause >nul
