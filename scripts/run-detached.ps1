# 빌드·테스트 명령을 현재 셸 트리 밖에서 돌린다 — WMI 프로세스 공장이 대신 만들어 준다.
#
# ★왜 필요한가★: 출력이 **파일로만** 떨어져서, 빌드 로그 전체가 도구 결과로 거슬러 올라오는 대신
#   판정에 필요한 줄만 골라 읽을 수 있다. 이게 이 스크립트를 기본값으로 쓰는 이유다.
#   `start`·백그라운드 잡·`nohup` 은 로그 파일 + 아래 `__EXIT` 완료 마커 계약을 주지 않으므로 대체재가 아니다.
#
# ★크래시 회피는 이 스크립트의 이유가 아니다 — 그렇게 적혀 있던 옛 주석은 오진이었다★(정정 2026-08-19).
#   세션까지 데려가던 터미널 크래시(`0xc00000fd`)는 `wezterm-gui` 자신의 버그였고(프로세스 트리 재귀에
#   순환 가드가 없었다) 분리 실행으로 막히지 않았다 — 분리 실행으로 돌린 테스트 중에도 죽었다(실측).
#   원인·증거·버전 경계·적용된 해법의 정본 = `/qa` 바인딩(`.claude/skill-bindings/qa.md`) 「분리 실행」.
#
# ★`launch-detached.ps1` 과 다른 점★: 그쪽은 **exe 경로**만 받아 앱을 띄운다(작업 스케줄러 경로).
#   이 스크립트는 **명령줄**을 받아 빌드·테스트를 돌린다. 용도가 갈려 있으니 합치지 말 것.
#
# 사용: run-detached.ps1 -Command "cargo test -p foo" -WorkDir <repo루트> -LogFile <경로>
# 반환: `PID=<n>` · `LOG=<경로>` · `BAT=<래퍼경로>`.
# 완료 판정: 로그 마지막에 `__EXIT=<종료코드>` 줄이 붙는다 — 그 마커가 나타나야 끝난 것이다.
#   (프로세스 부재로 판정하지 말 것 — 래퍼 cmd 는 자식보다 먼저 사라질 수 있다.)

param(
  [Parameter(Mandatory = $true)][string]$Command,
  [Parameter(Mandatory = $true)][string]$WorkDir,
  [Parameter(Mandatory = $true)][string]$LogFile
)

$ErrorActionPreference = 'Stop'

$WorkDir = (Resolve-Path $WorkDir).Path
# 상대 경로 로그는 `$WorkDir` 기준으로 절대화한다 — 래퍼가 `cd /d` 한 뒤에 파일을 만들기 때문에,
# 절대화하지 않으면 호출자가 지정한 경로와 실제 파일이 다른 폴더에 생긴다.
if (-not [System.IO.Path]::IsPathRooted($LogFile)) { $LogFile = Join-Path $WorkDir $LogFile }

# 명령을 래퍼 .bat 에 담는다 — 그래야 셸 인용을 두 겹 통과시키지 않아도 된다.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)
$bat = Join-Path $env:TEMP ("detached-cmd-" + $tag + ".bat")

$batBody = @"
@echo off
cd /d "$WorkDir"
$Command
echo __EXIT=%ERRORLEVEL%
"@
Set-Content -LiteralPath $bat -Value $batBody -Encoding ASCII

if (Test-Path -LiteralPath $LogFile) { Remove-Item -LiteralPath $LogFile -Force }

$launch = 'cmd.exe /c ""' + $bat + '" > "' + $LogFile + '" 2>&1"'
$res = ([WMIClass]"\\.\root\cimv2:Win32_Process").Create($launch)

if ($res.ReturnValue -ne 0) {
  Write-Output ("LAUNCH_FAILED (Win32_Process.Create returned " + $res.ReturnValue + ")")
  exit 1
}

Write-Output ("PID=" + $res.ProcessId)
Write-Output ("LOG=" + $LogFile)
Write-Output ("BAT=" + $bat)
