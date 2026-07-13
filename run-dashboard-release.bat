@echo off
REM Release build + run. Builds the RELEASE client shell AND the daemon binary (both required),
REM kills any stale daemon so the freshly built one is used, then launches the standalone exe.
REM
REM ★WHY build the daemon separately (do not remove): `npm run tauri build` compiles ONLY the client
REM   shell (engram-dashboard.exe). engram-dashboard-daemon is a separate workspace member and only a
REM   dev-dependency of src-tauri, so `tauri build` does NOT produce it. locate_daemon_exe() looks for
REM   the daemon RIGHT NEXT TO the app exe (current_exe().parent()), so BOTH must land in target\release\.
REM   Without this the release app cannot spawn the daemon (ExeNotFound) and hosts no agents.
REM
REM Release data dir (daemon.json) = %APPDATA%\com.engram.dashboard  (NOT the dev .engram-data).
REM   To drive this release app with scripts\engram.mjs, point the CLI at that portfile first, e.g.:
REM     set "ENGRAM_DATA_DIR=%APPDATA%\com.engram.dashboard"  &&  node scripts\engram.mjs list
cd /d "%~dp0"

echo [release] Stopping any running daemon (so the freshly built one is used)...
taskkill /IM engram-dashboard-daemon.exe /F >nul 2>&1

echo [release] Building client shell (release, --no-bundle for speed)...
call npm run tauri build -- --no-bundle
if errorlevel 1 ( echo [release] TAURI BUILD FAILED - not launching. & pause & exit /b 1 )

echo [release] Building daemon binary (release, lands next to the app exe)...
cargo build --release -p engram-dashboard-daemon
if errorlevel 1 ( echo [release] DAEMON BUILD FAILED - not launching. & pause & exit /b 1 )

echo [release] Launching target\release\engram-dashboard.exe ...
start "" "target\release\engram-dashboard.exe"
echo.
echo [release] Launched. Full installers (msi/nsis): run "npm run tauri build" WITHOUT --no-bundle.
echo [release] This build's daemon.json -^> %%APPDATA%%\com.engram.dashboard
pause
