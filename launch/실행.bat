@echo off
REM launch\실행.bat — 잔여 데몬/앱을 정리한 뒤 release\ 의 앱을 실행한다.
taskkill /IM engram-dashboard.exe /F 2>nul
taskkill /IM engram-dashboard-daemon.exe /F 2>nul
start "" "%~dp0release\engram-dashboard.exe"
