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

REM ADR-0137 - the debug shell must carry the DEV bundle identifier, not the release one.
REM ★Build through scripts\build-client-shell.mjs, NEVER a bare `cargo build -p engram-dashboard`
REM   (do not remove)★: that script injects the dev overlay and then re-reads the produced exe to
REM   confirm the dev identifier actually landed. A bare cargo build skips the Tauri CLI, so the
REM   overlay never applies and the exe is stamped with the RELEASE identifier (measured). The symptom
REM   never shows at build time: with the release app up, the dev app then exits instantly with no
REM   window and a 0-byte log, and this launcher dies at LAUNCH_FAILED (mechanism = the overlay file's
REM   own comment). The same script is what the /qa gate calls, so there is ONE implementation to fix.
REM ★Launch the path the script PRINTS, never a hardcoded target\debug (do not remove)★: CARGO_TARGET_DIR
REM   or .cargo\config.toml build.target-dir moves cargo's output, and a hardcoded path would then point
REM   at a STALE exe that still exists and still passes the identifier check. The script resolves the
REM   real directory (cargo metadata) and prints the exe path as its only stdout line - empty stdout
REM   means it failed, which is also our "do not build/launch" signal. Progress and errors go to stderr,
REM   so they still reach this window despite the capture.
echo [debug] Building client shell...
set "CLIENT_EXE="
for /f "usebackq delims=" %%E in (`node scripts\build-client-shell.mjs`) do set "CLIENT_EXE=%%E"
if not defined CLIENT_EXE ( echo [debug] CLIENT SHELL BUILD FAILED - not launching. & pause & exit /b 1 )
REM ★That script does NOT build the daemon (do not remove)★: engram-dashboard-daemon is a
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
REM ★The exe path travels via env var, never interpolated into the quoted literal (do not revert)★: it
REM   is resolved at runtime now, and a checkout path containing an apostrophe would break out of a
REM   single-quoted PowerShell string built by text substitution.
powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe $env:CLIENT_EXE -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223'"
if errorlevel 1 ( echo [debug] LAUNCH FAILED - see the log tail above. & pause & exit /b 1 )

echo.
echo [debug] Launched. The PID above is the app - use it if you need to force-kill.
echo [debug] First render after a fresh vite can take ~60-90s. Later launches are near-instant.
echo [debug] vite is still running in its own window; close that window to stop it.
pause
