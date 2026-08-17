@echo off
REM Engram Dashboard - release launcher (double-click). Runs the ALREADY BUILT release exe.
REM   Nothing is compiled here. To build first, use rebuild-run-release.bat.
REM
REM ★The app is launched detached (scripts\launch-detached.ps1) - do NOT replace this with `start`
REM   (do not remove)★: launched from a terminal, the app becomes a DESCENDANT of that terminal and
REM   its output travels back up the pipe chain. That combination repeatedly crashed the terminal
REM   (measured 2026-08-16), taking the app down with it. The scheduler path fixes BOTH halves - the
REM   app is created by a service so it is outside our process tree, AND its output goes to a file
REM   only. `start` satisfies NEITHER.
REM
REM ★Closing this window does NOT stop the app★ - it is no longer our child. Close the app window.
REM
REM No vite needed: the release build embeds the frontend (tauri build bakes in frontendDist).
REM Release data dir (daemon.json) = data\ NEXT TO the app exe, i.e. target\release\data (ADR-0134).
setlocal
cd /d "%~dp0.."

if not exist "target\release\engram-dashboard.exe" (
  echo [release] target\release\engram-dashboard.exe not found - nothing built yet.
  echo [release] Run rebuild-run-release.bat first.
  pause
  exit /b 1
)

REM ★No daemon kill here (do not remove)★: this launcher builds nothing, so there is no freshly built
REM   binary that a running daemon could be shadowing - the reason rebuild-run-*.bat kill theirs
REM   (ADR-0139). A live daemon from this same deployment is REUSED on purpose; killing it would
REM   destroy in-progress agent work (agents are children of the daemon - measured, not hypothetical).

echo [release] Launching app detached...
REM ★`-Command`, not `-File` (do not remove)★: with -File, PowerShell takes every following argument
REM   as a literal string, so a comma-separated -EnvVars list collapses into ONE value. The debug port
REM   argument is then malformed, 9223 never opens, and the script still prints a PID - the failure is
REM   silent (measured 2026-08-17).
powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe 'target/release/engram-dashboard.exe' -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223'"
if errorlevel 1 ( echo [release] LAUNCH FAILED - see the log tail above. & pause & exit /b 1 )

echo.
echo [release] Launched. The PID above is the app - use it if you need to force-kill.
echo [release] This build's daemon.json -^> target\release\data\
pause
