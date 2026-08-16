# 앱을 현재 셸 트리 밖에서 띄운다 — 작업 스케줄러가 대신 프로세스를 만든다.
#
# ★왜 필요한가★: Bash 툴에서 그냥 실행하면 앱이 wezterm 자손(8단계 아래)이 되고, 앱 출력이
#   파이프를 타고 그 사슬을 거슬러 올라온다. 이 조합에서 wezterm-gui 가 반복 크래시했다
#   (주입된 보안 에이전트 DLL 안 스택 오버플로 — 2026-08-16 실측).
#   스케줄러 경로는 (a) 생성 주체가 서비스라 우리 트리 밖이고 (b) 출력이 파일로만 간다.
#
# 사용: launch-detached.ps1 -Exe <경로> [-WorkDir <경로>] [-LogFile <경로>] [-Env "K=V","K2=V2"]
# 반환: 시작된 프로세스의 pid (표준 출력 마지막 줄 "PID=<n>")

param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [string]$WorkDir = "",
  [string]$LogFile = "",
  [string[]]$EnvVars = @()
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Exe)) { throw "exe not found: $Exe" }
$Exe = (Resolve-Path $Exe).Path
if ($WorkDir -eq "") { $WorkDir = Split-Path $Exe -Parent }
if ($LogFile -eq "") { $LogFile = Join-Path $env:TEMP ("detached-" + [System.IO.Path]::GetFileNameWithoutExtension($Exe) + ".log") }

$imageName = [System.IO.Path]::GetFileName($Exe)
$before = @(Get-Process -Name ([System.IO.Path]::GetFileNameWithoutExtension($Exe)) -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })

# ★래퍼 .bat 를 거치는 이유★: schtasks /tr 의 인용 규칙이 까다로워 env 설정·리다이렉션을
#   한 줄에 밀어 넣으면 조용히 깨진다. 전부 .bat 에 담고 태스크는 그 경로만 가리킨다.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)
$bat = Join-Path $env:TEMP "engram-detach-$tag.bat"
$taskName = "EngramDetach_$tag"

$lines = @('@echo off', "cd /d `"$WorkDir`"")
foreach ($e in $EnvVars) { $lines += "set `"$e`"" }
$lines += "`"$Exe`" > `"$LogFile`" 2>&1"
Set-Content -LiteralPath $bat -Value $lines -Encoding ASCII

try {
  # ★schtasks 는 정상 동작 중에도 stderr 로 경고를 낸다★(예: /ST 가 과거 시각 — 우리는 즉시 /run
  #   으로 띄우므로 무해). ErrorActionPreference=Stop 이면 그 경고가 NativeCommandError 로 승격돼
  #   멀쩡한 실행이 죽는다. 그래서 이 구간만 Continue 로 낮추고 판정은 종료코드로만 한다.
  $ErrorActionPreference = 'Continue'
  # /it = 로그인한 사용자의 대화형 세션에서 실행(창이 실제 화면에 뜬다). 세션 0 이면 GUI 가 안 뜬다.
  $null = schtasks /create /tn $taskName /tr "`"$bat`"" /sc once /st 00:00 /it /f 2>&1
  if ($LASTEXITCODE -ne 0) { throw "schtasks create failed ($LASTEXITCODE)" }
  $null = schtasks /run /tn $taskName 2>&1
  if ($LASTEXITCODE -ne 0) { throw "schtasks run failed ($LASTEXITCODE)" }

  $newPid = $null
  for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $now = @(Get-Process -Name ([System.IO.Path]::GetFileNameWithoutExtension($Exe)) -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
    $diff = @($now | Where-Object { $before -notcontains $_ })
    if ($diff.Count -gt 0) { $newPid = $diff[0]; break }
  }
} finally {
  $null = schtasks /delete /tn $taskName /f 2>&1
  Start-Sleep -Milliseconds 300
  Remove-Item -LiteralPath $bat -Force -ErrorAction SilentlyContinue
}

if ($null -eq $newPid) {
  Write-Output "LAUNCH_FAILED (no new $imageName within 20s). log tail:"
  if (Test-Path $LogFile) { Get-Content $LogFile -Tail 20 | ForEach-Object { "  $_" } }
  exit 1
}

Write-Output "LOG=$LogFile"
Write-Output "PID=$newPid"
