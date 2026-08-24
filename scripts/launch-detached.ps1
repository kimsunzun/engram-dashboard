# 앱을 현재 셸 트리 밖에서 띄운다 — 작업 스케줄러가 대신 프로세스를 만든다.
#
# ★왜 필요한가★: Bash 툴에서 그냥 실행하면 앱이 셸의 자손이 되고, 앱 출력이 파이프를 타고 그 사슬을
#   거슬러 올라온다. 그러면 호출이 앱 수명에 매달리고, 앱을 띄운 셸이 끝나면 앱도 함께 내려간다.
#   스케줄러 경로는 (a) 생성 주체가 서비스라 우리 트리 밖이고 (b) 출력이 파일로만 간다.
#
# ★크래시 회피는 이 스크립트의 이유가 아니다 — 그렇게 적혀 있던 옛 주석은 오진이었다★(정정 2026-08-19).
#   `wezterm-gui` 의 `0xc00000fd` 는 그 터미널 자신의 버그였고(프로세스 트리 재귀에 순환 가드 부재)
#   주입된 보안 에이전트 DLL 은 원인이 아니었다. 원인·증거·버전 경계의 정본 = `/qa` 바인딩 「분리 실행」.
#
# 사용: launch-detached.ps1 -Exe <경로> [-WorkDir <경로>] [-LogFile <경로>] [-Env "K=V","K2=V2"]
# 반환: 시작된 프로세스의 pid (표준 출력 마지막 줄 "PID=<n>")
# 로그: `-LogFile` 을 주면 그 경로가 쓰인다(상대 경로면 `-WorkDir` 기준으로 절대화한다 — 아래 주석).
#   생략하면 `%TEMP%\detached-<exe이름>-<이번 실행 태그>.log` — **실행마다 새 파일**이다(아래 주석).
#   실제 경로는 성공 시 `LOG=` 줄로, 실패 시 `LAUNCH_FAILED` 줄로 알린다.
# 실패: `LAUNCH_FAILED (...)` + 종료코드 1.

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

# 이번 실행을 가르는 태그 — 래퍼 .bat · 태스크 이름 · 기본 로그 이름이 전부 이걸 쓴다.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)

# ★기본 로그 이름에 **이번 실행 태그**를 넣는다(exe 이름만으로 되돌리지 마라)★
#   exe 이름은 배포판을 가르지 못한다 — 워크트리(`engram-dashboard`·`-wt2`·`-wt3`)도 같은 워크트리의
#   debug/release 도 전부 `engram-dashboard.exe` 다. 이름만 쓰면 모두 한 파일
#   (`%TEMP%\detached-engram-dashboard.log`)로 모이는데, 먼저 뜬 앱이 그 파일을 세션 내내 열어 두므로
#   다음 앱의 래퍼 .bat 는 리다이렉션을 못 열고 **exe 에 닿기도 전에** cmd 가 종료코드 1 로 죽는다
#   (실측: 스케줄러 이벤트 201 이 return code 2147942401 = 0x80070001 을 약 1초 만에 기록, 앱 tracing
#   로그는 아예 없음). 그 증상이 아래 `LAUNCH_FAILED (no new ... within 20s)` + 빈 로그 꼬리로 나와
#   **기동 실패가 앱 실패로 읽힌다.** ADR-0137 이후 동시 실행이 정상 상태라 예외가 아니라 기본 경로다.
#   태그는 실행마다 새로 뽑으므로 **구조적으로 충돌하지 않는다** — 경로 정규화·충돌 여유를 따질 필요가
#   없고, 잠김을 사전 검사할 필요도 없다(열려 있는 파일을 다시 고를 수가 없다).
#   대가: 실행마다 로그가 하나씩 남고 지우는 주체가 없다. 경로는 `LOG=` 줄로 알린다.
# ★상대 경로 `-LogFile` 은 `$WorkDir` 기준으로 절대화한다(빼지 마라)★: 안 그러면 기준이 셋으로 갈린다 —
#   호출자가 보는 cwd · .NET 의 `[Environment]::CurrentDirectory`(PowerShell 의 cwd 와 다를 수 있다,
#   실측) · 래퍼가 `cd /d "$WorkDir"` 한 뒤의 cwd. 파일이 실제로 만들어지는 곳은 셋째라, 절대화해 두지
#   않으면 호출자가 지정한 경로와 실제 파일이 다른 폴더에 생긴다.
if ($LogFile -eq "") {
  $stem = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
  $LogFile = Join-Path $env:TEMP ("detached-" + $stem + "-" + $tag + ".log")
} elseif (-not [System.IO.Path]::IsPathRooted($LogFile)) {
  $LogFile = Join-Path $WorkDir $LogFile
}

$imageName = [System.IO.Path]::GetFileName($Exe)

# ★새 프로세스는 이미지 이름이 아니라 **exe 경로**로 가른다(이름 비교로 되돌리지 마라)★: 이름만 보면
#   아래 20초 폴링 창 안에 뜬 **다른 배포판**의 앱이 우리 것으로 잡힌다(워크트리·debug/release 가 전부
#   `engram-dashboard.exe`). 그 pid 를 `PID=` 로 돌려주면 호출자의 teardown(`taskkill /PID <n> /T /F`)이
#   남의 배포판 앱을 죽인다 — 데몬 쪽에서 같은 오식별을 ADR-0139 결정 2 가 이름+경로 대조로 막았고,
#   앱 쪽에 남아 있던 것이 이 자리다. ADR-0137 로 동시 기동이 정상이 되면서 이 창은 실제로 열린다.
# ★Path 를 못 읽으면 우리 것이 아닌 것으로 친다★ — 권한 부족(승격·타 세션)이면 접근이 실패하는데,
#   포함시키면 남의 pid 를 우리 것으로 주장하게 된다. 빼는 쪽의 최악은 LAUNCH_FAILED 오보(무해)다.
function Get-DeploymentPids([string]$exePath) {
  $stem = [System.IO.Path]::GetFileNameWithoutExtension($exePath)
  $ids = @()
  foreach ($proc in @(Get-Process -Name $stem -ErrorAction SilentlyContinue)) {
    try {
      if ($proc.Path -eq $exePath) { $ids += $proc.Id }
    } catch {
    }
  }
  return $ids
}

$before = @(Get-DeploymentPids $Exe)

# ★래퍼 .bat 를 거치는 이유★: schtasks /tr 의 인용 규칙이 까다로워 env 설정·리다이렉션을
#   한 줄에 밀어 넣으면 조용히 깨진다. 전부 .bat 에 담고 태스크는 그 경로만 가리킨다.
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
    $now = @(Get-DeploymentPids $Exe)
    $diff = @($now | Where-Object { $before -notcontains $_ })
    if ($diff.Count -gt 0) { $newPid = $diff[0]; break }
  }
} finally {
  $null = schtasks /delete /tn $taskName /f 2>&1
  Start-Sleep -Milliseconds 300
  Remove-Item -LiteralPath $bat -Force -ErrorAction SilentlyContinue
}

if ($null -eq $newPid) {
  # 경로를 함께 찍는다 — 로그 이름이 실행마다 달라 호출자가 이 줄 없이는 어느 파일을 볼지 모른다.
  Write-Output "LAUNCH_FAILED (no new $imageName within 20s). log: $LogFile"
  Write-Output "log tail:"
  if (Test-Path $LogFile) { Get-Content $LogFile -Tail 20 | ForEach-Object { "  $_" } }
  exit 1
}

Write-Output "LOG=$LogFile"
Write-Output "PID=$newPid"
