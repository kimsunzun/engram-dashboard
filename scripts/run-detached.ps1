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
#   ★프로세스 부재로 판정하지 말 것★ — 단 사유는 「래퍼 cmd 가 자식보다 먼저 사라진다」가 **아니다**(그 사유는 폐기했다 · 2026-09-22).
#     래퍼가 자식보다 먼저 빠지는 것은 한 번도 관측되지 않았다 — 마커가 찍힌 경우 그것은 언제나 명령 자신의 출력 **뒤**였다.
#     살아 있는 사유 둘:
#     ① ★**부재는 결과를 하나도 안 싣는다**★ — 실측된 반례가 있다. 대상이 배치인데 `call` 이 안 붙으면 **본문은 끝까지 돌고
#        프로세스도 사라지는데 `__EXIT` 줄만 없다**(실측 2026-09-22: `probe.cmd`·`npm --version` 둘 다 출력은 정상, 마커 0줄).
#        부재만 보면 그것이 「끝났다」로 읽히고, 그게 빈 로그를 통과로 읽는 false PASS 다. **종료코드를 나르는 것은 마커뿐이다.**
#     ② **PID 는 재사용된다** — 번호가 남에게 넘어가면 「아직 있다」도 그 명령에 대한 답이 아니다
#        (같은 성질 위에 선 판정 = `docs/reference/structure/agent-backend.md` 「세션 id 회수」).
#   ★호출자가 `call` 을 손으로 붙이지 않는다★ — 붙일지 말지는 이 스크립트가 대상의 실체를 보고 정한다
#     (사유 = 아래 「`call` 은 **대상이 배치 파일일 때만** 붙인다」 주석 블록. ★변수 이름으로 가리키지 말 것★ —
#      옛 포인터가 `$batBody` 를 가리켰는데 그 변수는 그 블록보다 한참 아래라 찾는 자리가 어긋났다).
#     손으로 붙이면 그 판정을 건너뛰어 인자 속 `^` 가 망가진다(첫 토큰이 `call` 이면 실물이 안 풀려 덧붙지는 않는다 — 실측).

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

# 명령줄의 첫 토큰(= 실행 대상)을 뽑는다. 따옴표로 감싼 경로도 받는다.
function Get-FirstToken([string]$line) {
  $s = $line.TrimStart()
  if ($s.Length -eq 0) { return '' }
  if ($s[0] -eq '"') {
    $end = $s.IndexOf('"', 1)
    if ($end -lt 1) { return $s.Substring(1) }
    return $s.Substring(1, $end - 1)
  }
  return ([regex]::Match($s, '^\S+')).Value
}

# 맨 이름(`npm`)이 실제로 어느 파일인지를 PATHEXT 로 푼다 — 래퍼가 `cd /d "$WorkDir"` 한 뒤에 돌므로
# 검색 시작점도 `$WorkDir` 다. 못 찾으면 `$null` 을 돌려주고, 그때 호출부는 `call` 을 **안** 붙인다.
function Resolve-CmdTarget([string]$token, [string]$searchCwd) {
  if ([string]::IsNullOrWhiteSpace($token)) { return $null }
  $exts = @(($env:PATHEXT -split ';') | Where-Object { $_ })
  $ext = [System.IO.Path]::GetExtension($token)
  $hasDir = $token.IndexOfAny([char[]]@('\', '/', ':')) -ge 0
  $cands = New-Object System.Collections.Generic.List[string]
  if ($hasDir) {
    $base = if ([System.IO.Path]::IsPathRooted($token)) { $token } else { Join-Path $searchCwd $token }
    if ($ext) { $cands.Add($base) } else { foreach ($e in $exts) { $cands.Add($base + $e) } }
  }
  else {
    $dirs = @($searchCwd) + @(($env:PATH -split ';') | Where-Object { $_ })
    foreach ($d in $dirs) {
      try {
        if ($ext) { $cands.Add([System.IO.Path]::Combine($d, $token)) }
        else { foreach ($e in $exts) { $cands.Add([System.IO.Path]::Combine($d, $token + $e)) } }
      }
      catch { }
    }
  }
  foreach ($c in $cands) {
    try { if (Test-Path -LiteralPath $c -PathType Leaf) { return $c } } catch { }
  }
  return $null
}

# ★`call` 은 **대상이 배치 파일(`.cmd`·`.bat`)일 때만** 붙인다 — 무조건 붙이지 않는다★(실측 2026-09-22).
#   ┌ 붙여야 하는 쪽: 배치 파일이 다른 배치 파일을 `call` 없이 부르면 **제어가 넘어간 채 돌아오지 않는다**(cmd 규칙).
#   │  그러면 아래 `echo __EXIT=` 줄이 아예 실행되지 않아 **마커가 안 찍힌다** — 마커로 완료를 판정하는 호출자는
#   │  그것을 「아직 안 끝났다」로 읽어 무한 대기하거나, 빈 로그를 「통과」로 읽는다(조용한 false PASS).
#   │  ★이 함정은 `npm`·`npx` 처럼 exe 로 보이는 것에도 걸린다★ — Windows 에서 그것들의 실체는 `npm.cmd`·`npx.cmd` 다.
#   │  실측: `probe.cmd`(exit 7) 와 `npm --version` 둘 다 `call` 없이는 본문만 돌고 마커 0줄 · 붙이면 `__EXIT=7`·`__EXIT=0`.
#   └ 붙이면 안 되는 쪽: ★`call` 은 그 줄을 **한 번 더 파싱**해 따옴표 안의 `^` 를 두 배로 늘린다★. 실측 —
#      `"^\s*use tauri"` → `"^^\s*use tauri"` · `"^(?:[^/]|/[^/])*?\btauri::"` → `"^^(?:[^^/]|/[^^/])*?\btauri::"`.
#      ★`[^/]` 가 `[^^/]` 이 되는 것은 **다른 문자 클래스**다★ — CLAUDE.md 「빌드·검증 명령」의 `replay_flight` 순수성 게이트가
#      정확히 그 두 구성물로 된 정규식이고, 그 게이트는 **컴파일러 backstop 이 없는 유일한 벽**이며 **PASS 조건이 0줄**이다.
#      망가진 정규식은 아무것도 못 물어 0줄을 내고 조용히 통과한다 — 이 스크립트가 없애려는 바로 그 false PASS 다.
#   ★그래서 대상의 실체를 **PowerShell 쪽에서 미리** 푼다★ — 호출자가 주는 것은 보통 맨 이름(`npm`)이라 `.cmd` 인지 `.exe` 인지가
#     PATHEXT 해석 결과에 달렸고, 배치 파일 안으로 들어간 뒤에는 그것을 물어볼 수단이 없다. 위 `Resolve-CmdTarget` 이 그 답을 만든다.
#   ★기본값이 「안 붙임」인 것은 두 오판의 대가가 다르기 때문이다★ — 배치를 못 알아보면 **마커가 사라져 시끄럽게** 걸리지만,
#     exe 를 배치로 잘못 보면 **정규식이 조용히 망가진다.** 그래서 `.cmd`·`.bat` 로 **확정될 때만** 붙인다.
#   ★「무조건 붙여도 안전하다」로 되돌리지 말 것★ — 그 옛 결론은 종료코드·`--` 이하 인자·따옴표만 재고 **`^` 를 안 재서** 나온 것이다.
#   ★**`%` 는 이 분기가 못 고친다 — 안 고쳐진 채 남는 한계다**★(실측 2026-09-22): 래퍼가 배치 파일이라 **`call` 이 없어도**
#     본문의 `%VAR%` 는 펴지고 홑 `%` 는 먹힌다(`"x%USERNAME%y"` → `xkimsunzuny` · `"50% done C:\pct%NOPE%path"` → `50\pctpath`,
#     `call` 유무 양쪽 동일). `call` 이 더하는 것은 파싱 한 판뿐이라 한 판을 견디는 `%%` 까지 먹는다(`"50%% done"` → 없으면
#     `50% done`, 있으면 `50 done`). 즉 **`%` 가 든 인자는 이 래퍼에 넘기지 않는 것이 규칙**이고, 그 규칙은 이 변경 전후로 같다.
$target = Resolve-CmdTarget (Get-FirstToken $Command) $WorkDir
$callPrefix = ''
if ($target) {
  $targetExt = ([System.IO.Path]::GetExtension($target)).ToLowerInvariant()
  if ($targetExt -eq '.cmd' -or $targetExt -eq '.bat') { $callPrefix = 'call ' }
}

$batBody = @"
@echo off
cd /d "$WorkDir"
$callPrefix$Command
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
