@echo off
REM Release build + run. Builds the RELEASE client shell AND the daemon binary (both required),
REM kills THIS deployment's stale daemon so the freshly built one is used, then launches the exe.
REM
REM ★WHY build the daemon separately (do not remove): `npm run tauri build` compiles ONLY the client
REM   shell (engram-dashboard.exe). engram-dashboard-daemon is a separate workspace member and only a
REM   dev-dependency of src-tauri, so `tauri build` does NOT produce it. locate_daemon_exe() looks for
REM   the daemon RIGHT NEXT TO the app exe (current_exe().parent()), so BOTH must land in target\release\.
REM   Without this the release app cannot spawn the daemon (ExeNotFound) and hosts no agents.
REM
REM Release data dir (daemon.json) = data\ NEXT TO the app exe, i.e. target\release\data
REM   (ADR-0134; NOT the dev .engram-data, and no longer %APPDATA%).
REM   scripts\engram.mjs finds that portfile on its own - just run `node scripts\engram.mjs list`.
REM ★Do NOT set ENGRAM_DATA_DIR to "point the CLI at" the release daemon (do not remove this warning):
REM   under ADR-0134 that variable is ALSO the single-instance scope, so setting it makes the next
REM   daemon claim a DIFFERENT folder - you silently get a second daemon with its own roster and no
REM   error. Let the CLI discover the portfile instead.
cd /d "%~dp0"

REM ★Kill ONLY THIS deployment's daemon - never go back to `taskkill /IM` (do not remove)★:
REM   by image name this killed EVERY daemon on the machine, including a dev one hosting live agents.
REM   Agents are CHILDREN of the daemon, so running this launcher while developing destroyed
REM   in-progress agent work (measured, not hypothetical).
REM   The daemon that owns this deployment records its pid in the portfile below. We kill that pid
REM   only after tasklist confirms it is STILL an engram-dashboard-daemon.exe - pids get recycled, and
REM   killing a stranger's process would be far worse than the stale-daemon bug this step exists for.
REM   Missing/unreadable portfile, dead pid, or a different image name => kill nothing and say so.
set "RELEASE_PORTFILE=%~dp0target\release\data\daemon.json"
set "RELEASE_DAEMON_PID="
if not exist "%RELEASE_PORTFILE%" goto :daemon_kill_skip
REM ★The [int] cast is a guard, not decoration★: whatever comes back is expanded UNQUOTED on the next
REM   line, so a portfile carrying a non-numeric "pid" would inject shell text. A failed cast throws
REM   into the catch and prints nothing, which lands us in :daemon_kill_skip.
for /f "usebackq delims=" %%P in (`powershell -NoProfile -Command "try { [int]((ConvertFrom-Json (Get-Content -Raw -LiteralPath '%RELEASE_PORTFILE%')).pid) } catch { }"`) do set "RELEASE_DAEMON_PID=%%P"
echo %RELEASE_DAEMON_PID%| findstr /R /C:"^[0-9][0-9]*$" >nul || goto :daemon_kill_skip
REM ★/FO CSV is load-bearing★: tasklist's default table truncates the image column to 25 chars
REM   ("engram-dashboard-daemon.e"), so matching the full name against it never hits and every kill
REM   would be skipped. CSV prints the name in full.
tasklist /FI "PID eq %RELEASE_DAEMON_PID%" /FI "IMAGENAME eq engram-dashboard-daemon.exe" /NH /FO CSV | findstr /I /C:"engram-dashboard-daemon.exe" >nul || goto :daemon_kill_stale
echo [release] Stopping this deployment's daemon (pid %RELEASE_DAEMON_PID%) so the freshly built one is used...
taskkill /PID %RELEASE_DAEMON_PID% /F >nul 2>&1
goto :daemon_kill_done
:daemon_kill_skip
echo [release] No daemon pid recorded for this deployment (target\release\data\daemon.json) - killing nothing.
goto :daemon_kill_done
:daemon_kill_stale
echo [release] Recorded pid %RELEASE_DAEMON_PID% is not a live engram-dashboard-daemon.exe - killing nothing.
:daemon_kill_done

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
echo [release] This build's daemon.json -^> target\release\data\
pause
