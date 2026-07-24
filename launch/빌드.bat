@echo off
REM launch\빌드.bat — 릴리즈를 이 폴더의 release\ 로 재조립한다(scripts\build-release.ps1 호출).
REM %~dp0 = 이 .bat 이 놓인 폴더(=repo 루트 바로 아래 launch\). 절대 위치와 무관하게 더블클릭으로 동작.
pwsh -NoProfile -File "%~dp0..\scripts\build-release.ps1" -OutDir "%~dp0release"
pause
