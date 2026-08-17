@echo off
REM Engram Dashboard - debug launcher (double-click). Builds the client shell, makes sure vite is
REM up, then launches the app DETACHED.
REM
REM ★The app is launched detached (scripts\launch-detached.ps1) - do NOT go back to `npm run tauri dev`
REM   (do not remove)★: launched from a terminal, `tauri dev` makes the app a DESCENDANT of that
REM   terminal and the app's output travels back up the pipe chain. That combination repeatedly
REM   crashed the terminal (measured 2026-08-16), taking the app down with it. The scheduler path
REM   fixes BOTH halves - the app is created by a service so it is outside our process tree, AND its
REM   output goes to a file only. `start` / background jobs satisfy NEITHER, so they are not a
REM   substitute.
REM
REM ★Closing this window does NOT stop the app★ - it is no longer our child. Close the app window.
setlocal
cd /d "%~dp0.."

echo [debug] Building client shell...
cargo build -p engram-dashboard
if errorlevel 1 ( echo [debug] BUILD FAILED - not launching. & pause & exit /b 1 )
REM ★`-p engram-dashboard` does NOT build the daemon (do not remove)★: engram-dashboard-daemon is a
REM   separate workspace member. If you changed backend/daemon code, use rebuild-run-debug.bat - this
REM   launcher would leave you talking to a STALE daemon and your Rust changes would silently do
REM   nothing (ADR-0029: agent I/O lives in the daemon process).

REM ★The debug build does NOT embed the frontend (do not remove)★: it loads devUrl
REM   (http://localhost:1420), so vite must be running or the window opens EMPTY. `tauri dev` used to
REM   start vite for us as a child process; a detached app cannot, so we start it here.
REM   Left running on purpose - a warm vite renders the next launch in ~0.2s instead of ~60-90s
REM   (measured 2026-08-17). Output goes to a file so it never travels up a terminal's pipe chain.
powershell -NoProfile -Command "try { $null = Invoke-WebRequest -Uri 'http://localhost:1420' -UseBasicParsing -TimeoutSec 2; exit 0 } catch { exit 1 }" >nul 2>&1
if errorlevel 1 (
  echo [debug] Starting vite dev server ^(log: %TEMP%\engram-vite.log^)...
  start "engram-vite" /MIN cmd /c "npm run dev > "%TEMP%\engram-vite.log" 2>&1"
  powershell -NoProfile -Command "for ($i=0; $i -lt 60; $i++) { try { $null = Invoke-WebRequest -Uri 'http://localhost:1420' -UseBasicParsing -TimeoutSec 2; exit 0 } catch { Start-Sleep -Seconds 1 } }; exit 1"
  if errorlevel 1 ( echo [debug] vite did not come up on 1420 - see %TEMP%\engram-vite.log & pause & exit /b 1 )
) else (
  echo [debug] vite already up on 1420 - reusing it.
)

echo [debug] Launching app detached...
REM ★`-Command`, not `-File` (do not remove)★: with -File, PowerShell takes every following argument
REM   as a literal string, so a comma-separated -EnvVars list collapses into ONE value. The debug port
REM   argument is then malformed, 9223 never opens, and the script still prints a PID - the failure is
REM   silent (measured 2026-08-17).
powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe 'target/debug/engram-dashboard.exe' -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223'"
if errorlevel 1 ( echo [debug] LAUNCH FAILED - see the log tail above. & pause & exit /b 1 )

echo.
echo [debug] Launched. The PID above is the app - use it if you need to force-kill.
echo [debug] First render after a fresh vite can take ~60-90s. Later launches are near-instant.
echo [debug] vite is still running in its own window; close that window to stop it.
pause
