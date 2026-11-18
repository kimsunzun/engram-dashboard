@echo off
REM launch\빌드.bat — reassembles the release into release\ of this folder (calls scripts\build-release.ps1).
REM %~dp0 = the folder this .bat sits in (= launch\ right under the repo root). Works by double-click regardless of the absolute location.
pwsh -NoProfile -File "%~dp0..\scripts\build-release.ps1" -OutDir "%~dp0release"
pause
