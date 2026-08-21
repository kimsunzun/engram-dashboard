@echo off
REM Engram Dashboard - debug launcher WITH tracing on (double-click).
REM
REM Same as rebuild-run-debug.bat in every respect (stale-daemon cleanup, daemon rebuild, client shell
REM build, vite, detached launch) - it just turns logging up. This file is deliberately a one-line
REM wrapper rather than a copy: the daemon-kill guards in that script are safety-critical (they refuse
REM to kill a pid whose image name or executable path is not THIS deployment's daemon), and a second
REM copy would drift away from them silently.
REM
REM ★What you get★: the app and the daemon it spawns both run at RUST_LOG=debug, so the daemon records
REM   what the agent actually wrote on stdout before decoding. That raw line is what tells you whether
REM   a turn ended and the UI missed it, or the turn never ended at all - at the default (warn) level
REM   it is not recorded and the question cannot be answered after the fact.
REM
REM ★The log file is NOT this window★ - the app is launched detached and its output goes to a file.
REM   The launcher prints `LOG=<path>` and `PID=<pid>`; use that path. A NEW file is created per launch.
REM   The daemon writes its own log under the deployment's data dir (repo-root .engram-data\logs\).
REM
REM ★Debug logging is verbose★ - use this while reproducing one specific thing, not as your daily
REM   launcher. Use rebuild-run-debug.bat otherwise.
setlocal
set "ENGRAM_RUST_LOG=debug"
call "%~dp0rebuild-run-debug.bat" %*
