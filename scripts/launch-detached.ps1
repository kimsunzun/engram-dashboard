# 앱을 현재 셸 트리 밖에서 띄운다 — WMI 프로세스 공장(`Win32_Process.Create`)이 대신 프로세스를 만든다.
#
# ★왜 필요한가★: Bash 툴에서 그냥 실행하면 앱이 셸의 자손이 되고, 앱 출력이 파이프를 타고 그 사슬을
#   거슬러 올라온다. 그러면 호출이 앱 수명에 매달리고, 앱을 띄운 셸이 끝나면 앱도 함께 내려간다.
#   WMI 경로는 (a) 생성 주체가 `WmiPrvSE.exe` 라 우리 트리 밖이고(에이전트 셸의 Job 에서도 빠진다 —
#   `docs/process/step-log.md` spike #1) (b) 출력이 파일로만 간다.
#
# ★작업 스케줄러(`schtasks`)로 되돌리지 말 것★: 스케줄러가 액션 프로세스에 주는 STARTUPINFO 에는 창 표시
#   지정이 없고(dwFlags=0x80 · USESHOWWINDOW 없음 — 실측 2026-09-25) 태스크 정의에도 그 값을 줄 칸이 없다.
#   그래서 래퍼 cmd 콘솔이 보통 창으로 떠 포커스를 뺏었다(기본 터미널이 Windows Terminal 로 위임되는 머신에선
#   그 새 창이 앞으로 나온다). WMI 는 `Win32_ProcessStartup.ShowWindow` 로 그 값을 준다(아래 주석).
#
# ★크래시 회피는 이 스크립트의 이유가 아니다 — 그렇게 적혀 있던 옛 주석은 오진이었다★(정정 2026-08-19).
#   `wezterm-gui` 의 `0xc00000fd` 는 그 터미널 자신의 버그였고(프로세스 트리 재귀에 순환 가드 부재)
#   주입된 보안 에이전트 DLL 은 원인이 아니었다. 원인·증거·버전 경계의 정본 = `/qa` 바인딩 「분리 실행」.
#
# 사용: launch-detached.ps1 -Exe <경로> [-WorkDir <경로>] [-LogFile <경로>] [-EnvVars "K=V","K2=V2"]
# 래퍼 콘솔은 늘 최소화·비활성으로 뜬다(예외 = 아래 ShowWindow=7 주석). 앱 창의 첫 표시 상태는 이 스크립트가 정할 수 없다(아래 `$appLine` 주석).
# ★같은 번들 identifier 의 앱이 이미 떠 있으면(릴리즈 = `com.engram.dashboard` — 예: 사용자가 설치해 쓰는 릴리즈)
#   새 앱이 뜨지 않는다★ — 새 인스턴스는 single-instance 콜백으로 **기존** 창을 show·unminimize·focus 시키고 스스로
#   끝난다(`src-tauri/src/lib.rs` 의 `tauri_plugin_single_instance::init` → `src-tauri/src/tray/actions.rs` `show_main_ui` ·
#   tauri-plugin-single-instance 2.4.2 `platform_impl/windows.rs` 의 `std::process::exit(0)`). 그러면 전경 창이
#   바뀌고, 아래 20초 폴링은 곧 사라질 그 새 인스턴스의 pid 를 잡거나 `LAUNCH_FAILED` 를 낸다.
#   debug 도 같다 — dev identifier `com.engram.dashboard.dev` 는 모든 워크트리가 같으므로(`src-tauri/tauri.dev.conf.json`)
#   다른 워크트리의 debug 앱이 떠 있으면 이 워크트리의 debug 기동이 같은 식으로 그 앱에 넘어간다(관측 2026-09-24 — `/qa` 피드백 「다른 워크트리의 dev 앱」 항목).
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

# 이번 실행을 가르는 태그 — 래퍼 .bat · 기본 로그 이름 · 래퍼 콘솔 제목이 전부 이걸 쓴다.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)

# ★기본 로그 이름에 **이번 실행 태그**를 넣는다(exe 이름만으로 되돌리지 마라)★
#   exe 이름은 배포판을 가르지 못한다 — 워크트리(`engram-dashboard`·`-wt2`·`-wt3`)도 같은 워크트리의
#   debug/release 도 전부 `engram-dashboard.exe` 다. 이름만 쓰면 모두 한 파일
#   (`%TEMP%\detached-engram-dashboard.log`)로 모이는데, 먼저 뜬 앱이 그 파일을 세션 내내 열어 두므로
#   다음 앱의 래퍼 .bat 는 리다이렉션을 못 열고 **exe 에 닿기도 전에** cmd 가 종료코드 1 로 죽는다
#   (실측은 스케줄러 경로 시절: 스케줄러 이벤트 201 이 return code 2147942401 = 0x80070001 을 약 1초 만에
#   기록, 앱 tracing 로그는 아예 없음. 지금의 WMI 경로엔 그런 스케줄러 이력이 없어 아래 증상만 남는다).
#   그 증상이 아래 `LAUNCH_FAILED (no new ... within 20s)` + 빈 로그 꼬리로 나와
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

# ★래퍼 .bat 를 거치는 이유★: env 설정·작업 폴더·리다이렉션을 WMI 명령줄 한 줄에 인용해 밀어 넣지 않고
#   전부 .bat 에 담는다. WMI 가 만든 프로세스는 호출자 환경을 물려받지 않으므로 `-EnvVars` 가 앱에 닿는
#   길도 이 파일의 `set` 줄뿐이다.
$bat = Join-Path $env:TEMP "engram-detach-$tag.bat"

# ★이 스크립트는 Engram 앱 창의 첫 표시 상태를 정할 수 없다 — 앱은 늘 보통 창으로 뜬다★(실측 2026-09-25, 릴리즈 앱)
#   cmd 는 자기가 받은 표시 지정(래퍼 콘솔의 7)을 직접 실행한 프로그램에 넘기지 않는다(GUI 탐침 STARTUPINFO = dwFlags 0).
#   ★`start "" /b /min` 을 다시 넣지 말 것★ — 오늘은 효과가 없고, tao 가 바뀌면 깨진다: 앱 프로세스는
#   STARTF_USESHOWWINDOW + 7 을 받지만(PEB 에서 확인) Windows 는 그 값을 프로세스의 **첫** ShowWindow 에만 쓰고,
#   tao 는 창을 WS_VISIBLE 없이 만든 뒤 창 플래그를 적용하며 SW_HIDE 를 첫 호출로 낸다(`tao-0.35.3`
#   `src/platform_impl/windows/window.rs:1124-1149` 에서 플래그를 모으고 `window.rs:1320` `set_window_flags` →
#   `window_state.rs:420-423` `apply_diff` 가 SW_HIDE 를 낸다). 그 첫 호출이 SW_HIDE 가 아니게 되면(예: 창을
#   WS_VISIBLE 로 만들게 되면) 7 이 WebView 가 생기기 **전**에 창을 iconic 으로 만들어 아래의 깨짐(숨은 페이지·
#   어긋난 뷰포트·CDP 스크린샷 멈춤)이 날 것으로 본다(밖에서 강제한 iconic 실측에서 유추 — 이 경로 자체는 미실측). 뒤이은 `set_visible(true)` 는 평범한 SW_SHOW 라(`window_state.rs:325-335`) 창이
#   보통으로 뜨고, 창을 unfocused 로 짓지 않는 한 활성화된다(tao `MARKER_DONT_FOCUS = !focused` · Tauri 기본 focus = true).
#   실측: 메인 창은 한 번도 iconic 이 아니었다(30 ms 간격 감시). 같은 경로의 탐침도 CreateWindow→SW_SHOW 는
#   iconic, CreateWindow→SW_HIDE→SW_SHOW 는 보통이었다. 이 실행에선 전경 변화는 못 쟀다(세션 분리로 전경 = NULL).
#   ★밖에서 창을 강제로 iconic 으로 만들지 말 것 — WebView 자식이 생기기 **전**에는★: 창이 (-25600,-25600) 에
#   주차된 채 WebView 가 175×0 으로 만들어져 페이지가 `visibilityState=hidden`·뷰포트 2051×1440 로 어긋나고
#   `cdp.mjs shot` 이 멈춘다(20초에 강제 종료). 첫 표시 **뒤**의 최소화는 무해하다(뷰포트·가시성·스크린샷 정상).
#   앱 창을 최소화·비포커스로 띄우려면 앱 쪽 지원이 필요하다 — 아직 없다.
$appLine = "`"$Exe`" > `"$LogFile`" 2>&1"

# ★콘솔 제목엔 태그와 고정 문구만 넣는다 — 경로를 끼워 넣지 말 것★: title 줄도 cmd 가 파싱하므로
#   `& | < > ^ %` 가 들어가면 배치가 깨진다. 제목이 경고하는 것 = 콘솔 서브시스템인 debug 빌드는 이 콘솔을
#   함께 쓰므로 창을 닫으면 CTRL_CLOSE 로 함께 죽는다 — 처리기 유무와 무관하다(HandlerRoutine 문서: CTRL_CLOSE 는 어느 경로든 종료).
#   release(GUI 서브시스템) 앱은 콘솔에 안 붙어 있어 래퍼만 끝난다(콘솔·GUI 탐침으로 실측 2026-09-25).
$lines = @('@echo off', "title engram launch-detached $tag - closing this window kills a debug-build app", "cd /d `"$WorkDir`"")
foreach ($e in $EnvVars) { $lines += "set `"$e`"" }
$lines += $appLine
Set-Content -LiteralPath $bat -Value $lines -Encoding ASCII

$newPid = $null
$launchError = $null
try {
  # ★래퍼 cmd 콘솔은 「활성화 없이 최소화」(`ShowWindow = 7` = SW_SHOWMINNOACTIVE)로 띄운다★ —
  #   작업 표시줄 단추는 남고 전경 창은 안 바뀐다. 이 값이면 conhost 가 기본 터미널로 위임하지도 않는다(실측 2026-09-25).
  #   ★0(SW_HIDE)으로 바꾸지 말 것★ — 작업 표시줄 단추를 남기는 것이 요구다(사용자 결정 2026-09-25).
  #   전경 창이 없을 때 7 이 전경을 가져가는 예외와 검토했다 버린 대안 = `run-detached.ps1` 의 ShowWindow=7 주석.
  #   ★`CreateFlags` 에 CREATE_NO_WINDOW(0x08000000)를 넣지 말 것★ — WMI 가 ReturnValue 21 로 거부한다
  #   (실측 정본 = `crates/engram-dashboard-discovery/src/lib.rs` `wmi_spawn` 주석 · `real_wmi_spawn_flag_matrix`).
  #   미검증: 새 프로세스는 호출자의 토큰·세션을 따른다고 본다 — 대화형 데스크톱 밖(SSH 등)에서 부르면 창이 안
  #   보일 수 있고, 승격된 셸에서 부르면 앱도 승격된 채 뜬다.
  $startup = ([WMIClass]"\\.\root\cimv2:Win32_ProcessStartup").CreateInstance()
  $startup.ShowWindow = 7
  # ★CREATE_BREAKAWAY_FROM_JOB(0x01000000)을 빼지 말 것★ — 사유·실측 = `run-detached.ps1` 의 CREATE_BREAKAWAY_FROM_JOB 주석
  #   (WMI 공급자 호스트 Job 쿼터에서 앱을 떼어 둔다).
  $startup.CreateFlags = 0x01000000
  $res = ([WMIClass]"\\.\root\cimv2:Win32_Process").Create('cmd.exe /c ""' + $bat + '""', $null, $startup)
  if ($res.ReturnValue -ne 0) {
    $launchError = "Win32_Process.Create returned " + $res.ReturnValue
  } else {
    for ($i = 0; $i -lt 40; $i++) {
      Start-Sleep -Milliseconds 500
      $now = @(Get-DeploymentPids $Exe)
      $diff = @($now | Where-Object { $before -notcontains $_ })
      if ($diff.Count -gt 0) { $newPid = $diff[0]; break }
    }
  }
} finally {
  Remove-Item -LiteralPath $bat -Force -ErrorAction SilentlyContinue
}

if ($launchError) {
  Write-Output "LAUNCH_FAILED ($launchError). log: $LogFile"
  exit 1
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
