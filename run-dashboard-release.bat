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
REM   only after tasklist confirms it is STILL an engram-dashboard-daemon.exe AND its executable path
REM   matches THIS deployment's own daemon binary - pids get recycled, possibly by another deployment's
REM   daemon.exe on the same machine (dev and release now run simultaneously, each with its own Tauri
REM   identifier), and killing a stranger's process would be far worse than the stale-daemon bug this
REM   step exists for.
REM   Missing/unreadable portfile, a missing/zero/non-numeric pid, a dead pid, a different image name,
REM   or a different executable path => kill nothing and say so. If the kill itself fails (e.g. an
REM   elevated daemon), the script stops instead of silently building/launching against a still-locked
REM   binary.
set "RELEASE_PORTFILE=%~dp0target\release\data\daemon.json"
set "EXPECTED_DAEMON_EXE=%~dp0target\release\engram-dashboard-daemon.exe"
set "RELEASE_DAEMON_PID="
if not exist "%RELEASE_PORTFILE%" goto :daemon_kill_no_portfile
REM ★Portfile path travels to PowerShell via env var, never interpolated into a quoted literal (do
REM   not revert)★: a checkout path containing an apostrophe (e.g. C:\O'Brien\Engram) would break out
REM   of a single-quoted -LiteralPath string built by text substitution, and a crafted path could
REM   inject PowerShell. $env:RELEASE_PORTFILE is read by PowerShell itself from the process
REM   environment, so cmd never places the raw path text inside the command string.
REM ★The [int] cast is a guard, not decoration★: whatever comes back is expanded UNQUOTED further
REM   down, so a portfile carrying a non-numeric "pid" would inject shell text - a failed cast throws
REM   into the catch and prints nothing. A missing pid field casts to [int]$null = 0, so it is caught
REM   by the SAME explicit "-eq 0" check as a literal 0 - both used to slip past as a bare "0" and
REM   reach the numeric gate below, producing the misleading "pid 0 is not a live daemon.exe" message.
for /f "usebackq delims=" %%P in (`powershell -NoProfile -Command "try { $j = ConvertFrom-Json (Get-Content -Raw -LiteralPath $env:RELEASE_PORTFILE); $p = [int]$j.pid; if ($p -eq 0) { throw 'missing or zero pid' }; $p } catch { }"`) do set "RELEASE_DAEMON_PID=%%P"
if "%RELEASE_DAEMON_PID%"=="" goto :daemon_kill_bad_pid
echo %RELEASE_DAEMON_PID%| findstr /R /C:"^[0-9][0-9]*$" >nul || goto :daemon_kill_bad_pid
REM ★/FO CSV is load-bearing★: tasklist's default table truncates the image column to 25 chars
REM   ("engram-dashboard-daemon.e"), so matching the full name against it never hits and every kill
REM   would be skipped. CSV prints the name in full.
tasklist /FI "PID eq %RELEASE_DAEMON_PID%" /FI "IMAGENAME eq engram-dashboard-daemon.exe" /NH /FO CSV | findstr /I /C:"engram-dashboard-daemon.exe" >nul || goto :daemon_kill_stale
REM ★Image-name match alone is not enough (do not remove)★: a stale portfile whose pid was recycled by
REM   ANOTHER deployment's daemon.exe on this machine (e.g. dev's, or a different checkout's) passes
REM   the pid+image-name filters above too. Confirm the running process's own executable path is THIS
REM   deployment's before killing it. Get-Process -Id throws access-denied for an elevated/foreign-
REM   session process; that is treated as "cannot confirm" (RELEASE_DAEMON_EXE stays empty), not as a
REM   pass. Residual TOCTOU (accepted, not closed here): the pid can be reassigned again in the window
REM   between this check and the taskkill call below.
for /f "usebackq delims=" %%E in (`powershell -NoProfile -Command "try { (Get-Process -Id $env:RELEASE_DAEMON_PID -ErrorAction Stop).Path } catch { }"`) do set "RELEASE_DAEMON_EXE=%%E"
if /I not "%RELEASE_DAEMON_EXE%"=="%EXPECTED_DAEMON_EXE%" goto :daemon_kill_path_mismatch
echo [release] Stopping this deployment's daemon (pid %RELEASE_DAEMON_PID%) so the freshly built one is used...
taskkill /PID %RELEASE_DAEMON_PID% /F >nul 2>&1
REM ★Verified by re-checking tasklist, not by trusting taskkill's own exit code (do not remove)★: if
REM   the kill silently fails (access denied, an elevated daemon), the OLD daemon keeps holding
REM   target\release's lock and the freshly built binary below is launched pointing at a stale daemon
REM   - the exact thing this whole step exists to prevent. Re-checking tasklist confirms the process
REM   is actually gone regardless of what taskkill itself reported.
tasklist /FI "PID eq %RELEASE_DAEMON_PID%" /FI "IMAGENAME eq engram-dashboard-daemon.exe" /NH /FO CSV | findstr /I /C:"engram-dashboard-daemon.exe" >nul && goto :daemon_kill_failed
goto :daemon_kill_done
:daemon_kill_no_portfile
echo [release] No daemon pid recorded for this deployment (target\release\data\daemon.json missing) - killing nothing.
goto :daemon_kill_done
:daemon_kill_bad_pid
echo [release] Portfile at %RELEASE_PORTFILE% has no usable pid (missing, zero, or non-numeric) - killing nothing.
goto :daemon_kill_done
:daemon_kill_stale
echo [release] Recorded pid %RELEASE_DAEMON_PID% is not a live engram-dashboard-daemon.exe - killing nothing.
goto :daemon_kill_done
:daemon_kill_path_mismatch
echo [release] Recorded pid %RELEASE_DAEMON_PID% is engram-dashboard-daemon.exe but not from this deployment (expected %EXPECTED_DAEMON_EXE%) - killing nothing.
goto :daemon_kill_done
:daemon_kill_failed
echo [release] FAILED to stop the stale daemon (pid %RELEASE_DAEMON_PID% still running) - access denied or an elevated daemon. Not building/launching: it would silently keep using the stale daemon's lock.
pause
exit /b 1
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
