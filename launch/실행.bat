@echo off
REM launch\실행.bat — cleans up a leftover daemon/app, then runs the app in release\.
taskkill /IM engram-dashboard.exe /F 2>nul
taskkill /IM engram-dashboard-daemon.exe /F 2>nul
start "" "%~dp0release\engram-dashboard.exe"
