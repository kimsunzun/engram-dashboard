@echo off
REM Clean relaunch: kills the engram daemon + REBUILDS the backend, then runs.
REM Use this after ANY Rust/backend change, or if the screen looks wrong.
REM
REM ★WHY the rebuild (do not remove): `tauri dev` (beforeDevCommand=npm run dev) rebuilds
REM   only the CLIENT SHELL (engram-dashboard.exe), NOT the daemon binary. Agent I/O runs
REM   in the DAEMON process (ADR-0029), and ensure_daemon reuses a live compatible daemon,
REM   so without an explicit `cargo build` the app keeps connecting to a STALE daemon and
REM   your Rust changes silently have no effect. (does NOT touch claude.exe)
REM ★setlocal (do not remove)★: without it, DEV_DAEMON_EXE below survives into a SECOND run in the
REM   same cmd window. Combined with a failed Get-Process lookup that stale value makes the path check
REM   pass, so a pid we could NOT confirm gets killed - the exact opposite of what that check promises.
setlocal
cd /d "%~dp0.."
REM ★%CD%, not a %~dp0.. path (do not remove)★: this launcher lives in scripts\, so a %~dp0-relative
REM   path would carry a literal ".." segment. EXPECTED_DEV_DAEMON_EXE below is string-compared against
REM   the running process's own Path (canonical, no ".."), so the comparison would never match and
REM   every daemon kill would be silently skipped. Anchoring on %CD% after the cd gives a canonical root.
set "ROOT=%CD%"

REM ADR-0139 - launchers kill only their OWN deployment's daemon.
REM ★Kill ONLY THIS (dev) deployment's daemon - never go back to `taskkill /IM` (do not remove)★:
REM   by image name this killed EVERY daemon on the machine, including the release build's, destroying
REM   in-progress agent work in other deployments (agents are CHILDREN of the daemon - measured, not
REM   hypothetical). dev and release now carry separate Tauri identifiers (src-tauri\tauri.dev.conf.json)
REM   specifically so they can run SIMULTANEOUSLY, which makes this the normal case, not an edge case.
REM   The daemon that owns this dev deployment records its pid in the portfile below (repo-root
REM   `.engram-data\daemon.json` - discovery's debug-build default_data_dir, NOT target\release\data).
REM   We kill that pid only after tasklist confirms it is STILL an engram-dashboard-daemon.exe AND its
REM   executable path matches THIS deployment's own daemon binary (target\debug\) - pids get recycled,
REM   possibly by another deployment's daemon.exe, and killing a stranger's process would be far worse
REM   than the stale-daemon bug this step exists for.
REM   Missing/unreadable portfile, a missing/zero/non-numeric pid, a dead pid, a different image name,
REM   or a different executable path => kill nothing and say so. If the kill itself fails, the script
REM   stops instead of silently rebuilding/launching against a still-locked binary.
set "DEV_PORTFILE=%ROOT%\.engram-data\daemon.json"
set "EXPECTED_DEV_DAEMON_EXE=%ROOT%\target\debug\engram-dashboard-daemon.exe"
set "DEV_DAEMON_PID="
REM ★Pre-clear DEV_DAEMON_EXE too (do not remove)★: `for /f` does not run its body when the lookup
REM   fails, so an uninitialised variable keeps whatever was there before. The comment below promises
REM   "cannot confirm => stays empty => do not kill"; without this line that promise is false.
set "DEV_DAEMON_EXE="
if not exist "%DEV_PORTFILE%" goto :dev_daemon_kill_no_portfile
REM Portfile path travels to PowerShell via env var, never interpolated into a quoted literal - see
REM   rebuild-run-release.bat for why (apostrophe/injection risk in the checkout path).
REM The [int] cast is a guard, not decoration: a portfile carrying a non-numeric "pid" throws into the
REM   catch and prints nothing. A missing pid field casts to [int]$null = 0, caught by the same
REM   explicit "-eq 0" check as a literal 0, so neither slips past as a bare "0".
for /f "usebackq delims=" %%P in (`powershell -NoProfile -Command "try { $j = ConvertFrom-Json (Get-Content -Raw -LiteralPath $env:DEV_PORTFILE); $p = [int]$j.pid; if ($p -eq 0) { throw 'missing or zero pid' }; $p } catch { }"`) do set "DEV_DAEMON_PID=%%P"
if "%DEV_DAEMON_PID%"=="" goto :dev_daemon_kill_bad_pid
echo %DEV_DAEMON_PID%| findstr /R /C:"^[0-9][0-9]*$" >nul || goto :dev_daemon_kill_bad_pid
REM ★/FO CSV is load-bearing★: tasklist's default table truncates the image column to 25 chars
REM   ("engram-dashboard-daemon.e"), so matching the full name against it never hits and every kill
REM   would be skipped. CSV prints the name in full.
tasklist /FI "PID eq %DEV_DAEMON_PID%" /FI "IMAGENAME eq engram-dashboard-daemon.exe" /NH /FO CSV | findstr /I /C:"engram-dashboard-daemon.exe" >nul || goto :dev_daemon_kill_stale
REM Image-name match alone is not enough - a stale portfile whose pid was recycled by ANOTHER
REM   deployment's daemon.exe (e.g. release's) passes the pid+image-name filters above too. Confirm
REM   the running process's own executable path is THIS deployment's before killing it. Get-Process
REM   -Id throws access-denied for an elevated/foreign-session process; that is treated as "cannot
REM   confirm" (DEV_DAEMON_EXE stays empty), not as a pass. Residual TOCTOU (accepted, not closed
REM   here): the pid can be reassigned again in the window between this check and the taskkill below.
for /f "usebackq delims=" %%E in (`powershell -NoProfile -Command "try { (Get-Process -Id $env:DEV_DAEMON_PID -ErrorAction Stop).Path } catch { }"`) do set "DEV_DAEMON_EXE=%%E"
if /I not "%DEV_DAEMON_EXE%"=="%EXPECTED_DEV_DAEMON_EXE%" goto :dev_daemon_kill_path_mismatch
echo [clean] Stopping this deployment's daemon (pid %DEV_DAEMON_PID%) so the freshly built one is used...
taskkill /PID %DEV_DAEMON_PID% /F >nul 2>&1
REM Verified by re-checking tasklist, not by trusting taskkill's own exit code - if the kill silently
REM   fails, the OLD daemon keeps holding the dev data dir's lock and the freshly built binary below
REM   is launched pointing at a stale daemon, silently making your Rust changes have no effect (the
REM   exact failure mode the rebuild comment above already warns about, just from a different cause).
tasklist /FI "PID eq %DEV_DAEMON_PID%" /FI "IMAGENAME eq engram-dashboard-daemon.exe" /NH /FO CSV | findstr /I /C:"engram-dashboard-daemon.exe" >nul && goto :dev_daemon_kill_failed
goto :dev_daemon_kill_done
:dev_daemon_kill_no_portfile
echo [clean] No daemon pid recorded for this deployment (.engram-data\daemon.json missing) - killing nothing.
goto :dev_daemon_kill_done
:dev_daemon_kill_bad_pid
echo [clean] Portfile at %DEV_PORTFILE% has no usable pid (missing, zero, or non-numeric) - killing nothing.
goto :dev_daemon_kill_done
:dev_daemon_kill_stale
echo [clean] Recorded pid %DEV_DAEMON_PID% is not a live engram-dashboard-daemon.exe - killing nothing.
goto :dev_daemon_kill_done
:dev_daemon_kill_path_mismatch
echo [clean] Recorded pid %DEV_DAEMON_PID% is engram-dashboard-daemon.exe but not from this deployment (expected %EXPECTED_DEV_DAEMON_EXE%) - killing nothing.
goto :dev_daemon_kill_done
:dev_daemon_kill_failed
echo [clean] FAILED to stop the stale daemon (pid %DEV_DAEMON_PID% still running) - access denied or an elevated daemon. Not rebuilding/launching: it would silently keep using the stale daemon's lock.
pause
exit /b 1
:dev_daemon_kill_done

echo [clean] Rebuilding backend daemon (first change may take ~15-30s)...
cargo build -p engram-dashboard-daemon
if errorlevel 1 (
  echo [clean] BUILD FAILED - see errors above. Not launching.
  pause
  exit /b 1
)

REM ★The client shell must be built too (do not remove)★: `tauri dev` used to compile it for us as
REM   part of launching. We no longer call it (see the detached-launch note below), so nothing else
REM   builds engram-dashboard.exe - without this you would relaunch the PREVIOUS shell binary.
echo [clean] Rebuilding client shell...
cargo build -p engram-dashboard
if errorlevel 1 (
  echo [clean] BUILD FAILED - see errors above. Not launching.
  pause
  exit /b 1
)

REM ★The debug build does NOT embed the frontend (do not remove)★: it loads devUrl
REM   (http://localhost:1420), so vite must be running or the window opens EMPTY. `tauri dev` used to
REM   start vite for us as a child process; a detached app cannot, so we start it here.
REM   Left running on purpose - a warm vite renders the next launch in ~0.2s instead of ~60-90s
REM   (measured 2026-08-17). Output goes to a file so it never travels up a terminal's pipe chain.
powershell -NoProfile -Command "try { $null = Invoke-WebRequest -Uri 'http://localhost:1420' -UseBasicParsing -TimeoutSec 2; exit 0 } catch { exit 1 }" >nul 2>&1
if errorlevel 1 (
  echo [clean] Starting vite dev server ^(log: %TEMP%\engram-vite.log^)...
  start "engram-vite" /MIN cmd /c "npm run dev > "%TEMP%\engram-vite.log" 2>&1"
  powershell -NoProfile -Command "for ($i=0; $i -lt 60; $i++) { try { $null = Invoke-WebRequest -Uri 'http://localhost:1420' -UseBasicParsing -TimeoutSec 2; exit 0 } catch { Start-Sleep -Seconds 1 } }; exit 1"
  if errorlevel 1 ( echo [clean] vite did not come up on 1420 - see %TEMP%\engram-vite.log & pause & exit /b 1 )
) else (
  echo [clean] vite already up on 1420 - reusing it.
)

REM ★The app is launched detached (scripts\launch-detached.ps1) - do NOT go back to `npm run tauri dev`
REM   (do not remove)★: launched from a terminal, `tauri dev` makes the app a DESCENDANT of that
REM   terminal and the app's output travels back up the pipe chain. That combination repeatedly
REM   crashed the terminal (measured 2026-08-16), taking the app down with it. The scheduler path
REM   fixes BOTH halves - the app is created by a service so it is outside our process tree, AND its
REM   output goes to a file only. `start` / background jobs satisfy NEITHER.
REM ★`-Command`, not `-File` (do not remove)★: with -File, PowerShell takes every following argument
REM   as a literal string, so a comma-separated -EnvVars list collapses into ONE value. The debug port
REM   argument is then malformed, 9223 never opens, and the script still prints a PID - silent failure
REM   (measured 2026-08-17).
echo [clean] Launching app detached...
powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe 'target/debug/engram-dashboard.exe' -EnvVars 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223'"
if errorlevel 1 ( echo [clean] LAUNCH FAILED - see the log tail above. & pause & exit /b 1 )

echo.
echo [clean] Launched. The PID above is the app - use it if you need to force-kill.
echo [clean] NOTE: closing this window does NOT stop the app - it is no longer our child. Close the app window.
echo [clean] First render after a fresh vite can take ~60-90s. Later launches are near-instant.
pause
