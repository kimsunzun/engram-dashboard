# Run build·test commands outside the current shell tree — the WMI process factory creates them on our behalf.
#
# ★Why it is needed★: output lands **only in a file**, so instead of the entire build log coming back up into the tool result,
#   you can pick and read only the lines needed for the decision. That is why this script is the default.
#   `start` · background jobs · `nohup` do not give the log file + `__EXIT` completion marker contract below, so they are not substitutes.
#
# ★Avoiding the crash is not this script's reason — the old comment that said so was a misdiagnosis★ (corrected 2026-08-19).
#   The terminal crash (`0xc00000fd`) that took the session down with it was `wezterm-gui`'s own bug (there was no cycle guard in
#   process-tree recursion) and detached running did not prevent it — it died even during tests run detached (measured).
#   Cause and evidence record = git history (the old qa binding 「분리 실행」). How to use = `.claude/skill-bindings/qa.md` 「분리 실행」.
#
# ★How it differs from `launch-detached.ps1`★: that one takes **only an exe path** to launch an app, and its `PID=` is the **app's** pid.
#   This script takes a **command line** to run builds·tests, and `PID=` is the pid of the **wrapper cmd** (both are WMI paths).
#   The uses are split, so do not merge them.
#
# Usage: run-detached.ps1 -Command "cargo test -p foo" -WorkDir <repo root> -LogFile <path>
# Returns: `PID=<n>` · `LOG=<경로>` · `BAT=<래퍼경로>`.
# Completion decision: a `__EXIT=<종료코드>` line is appended at the end of the log — only when that marker appears is it done.
#   ★Do not decide by process absence★ — but the reason is **not** 「래퍼 cmd 가 자식보다 먼저 사라진다」 (the wrapper cmd vanishes before its child) (that reason was discarded · 2026-09-22).
#     The wrapper leaving before the child has never once been observed — when the marker was printed, it was always **after** the command's own output.
#     Two living reasons:
#     ① ★**Absence carries no result at all**★ — there is a measured counterexample. If the target is a batch file and `call` is not attached, **the body runs to the end
#        and the process vanishes, but only the `__EXIT` line is missing** (measured 2026-09-22: `probe.cmd` · `npm --version` both with normal output, 0 marker lines).
#        Looking only at absence, that reads as 「끝났다」 (it is done), and that is a false PASS that reads an empty log as a pass. **Only the marker carries the exit code.**
#     ② **PIDs are reused** — once the number is handed to someone else, 「아직 있다」 (it is still there) is not an answer about that command either
#        (a decision standing on the same property = `docs/reference/structure/agent-backend.md` 「세션 id 회수」).
#   ★The caller does not attach `call` by hand★ — whether to attach it is decided by this script looking at what the target really is
#     (reason = the comment block below 「`call` 은 **대상이 배치 파일일 때만** 붙인다」 (call is attached **only when the target is a batch file**). ★Do not point to it by variable name★ —
#      the old pointer pointed to `$batBody`, but that variable was far below that block, so the place to look was off).
#     Attaching it by hand skips that decision and breaks `^` in the arguments (if the first token is `call` the real thing is not expanded, so nothing gets appended — measured).

param(
  [Parameter(Mandatory = $true)][string]$Command,
  [Parameter(Mandatory = $true)][string]$WorkDir,
  [Parameter(Mandatory = $true)][string]$LogFile
)

$ErrorActionPreference = 'Stop'

$WorkDir = (Resolve-Path $WorkDir).Path
# A relative-path log is made absolute against `$WorkDir` — the wrapper creates the file after `cd /d`, so
# unless it is made absolute, the path the caller specified and the actual file end up in different folders.
if (-not [System.IO.Path]::IsPathRooted($LogFile)) { $LogFile = Join-Path $WorkDir $LogFile }

# Put the command in the wrapper .bat — that way shell quoting does not have to pass through two layers.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)
$bat = Join-Path $env:TEMP ("detached-cmd-" + $tag + ".bat")

# Extract the first token of the command line (= the execution target). A path wrapped in quotes is accepted too.
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

# Resolve with PATHEXT which file a bare name (`npm`) actually is — the wrapper runs after `cd /d "$WorkDir"`, so
# the search starting point is `$WorkDir` too. If not found, return `$null`, and then the caller does **not** attach `call`.
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

# ★Attach `call` **only when the target is a batch file (`.cmd` · `.bat`)** — not unconditionally★ (measured 2026-09-22).
#   ┌ Side where it must be attached: if a batch file calls another batch file without `call`, **control is handed over and never returns** (cmd rule).
#   │  Then the `echo __EXIT=` line below is never executed at all, so **the marker is not printed** — a caller that decides completion by the marker
#   │  reads that as 「아직 안 끝났다」 (not finished yet) and waits forever, or reads an empty log as 「통과」 (pass) (a quiet false PASS).
#   │  ★This trap catches even things that look like exes, such as `npm` · `npx`★ — on Windows their real forms are `npm.cmd` · `npx.cmd`.
#   │  Measured: both `probe.cmd` (exit 7) and `npm --version` run only the body without `call` with 0 marker lines · with it they give `__EXIT=7` · `__EXIT=0`.
#   └ Side where it must not be attached: ★`call` **parses the line once more** and doubles `^` inside quotes★. Measured —
#      `"^\s*use tauri"` → `"^^\s*use tauri"` · `"^(?:[^/]|/[^/])*?\btauri::"` → `"^^(?:[^^/]|/[^^/])*?\btauri::"`.
#      ★`[^/]` becoming `[^^/]` is a **different character class**★ — the `replay_flight` purity gate in `ci.yml` is a regex made of
#      exactly those two constructs, and that gate is **the only wall with no compiler backstop** and its **PASS condition is 0 lines**.
#      A broken regex bites nothing, emits 0 lines, and passes quietly — the very false PASS this script exists to remove.
#   ★So the target's real form is resolved **beforehand on the PowerShell side**★ — what the caller gives is usually a bare name (`npm`), so whether it is `.cmd` or `.exe`
#     depends on the PATHEXT resolution result, and once inside the batch file there is no means to ask it. `Resolve-CmdTarget` above produces that answer.
#   ★The default being 「안 붙임」 (do not attach) is because the two misjudgments cost differently★ — failing to recognize a batch file gets caught **loudly, as the marker disappears**, but
#     mistaking an exe for a batch file **quietly breaks the regex.** So attach only when **confirmed** as `.cmd` · `.bat`.
#   ★Do not go back to 「무조건 붙여도 안전하다」 (it is safe to attach unconditionally)★ — that old conclusion came from measuring only exit codes · arguments after `--` · quotes and **not measuring `^`**.
#   ★**`%` is something this branch cannot fix — a limitation that remains unfixed**★ (measured 2026-09-22): because the wrapper is a batch file, **even without `call`**
#     `%VAR%` in the body is expanded and a lone `%` is eaten (`"x%USERNAME%y"` → `x<user>y` · `"50% done C:\pct%NOPE%path"` → `50\pctpath`,
#     the same with or without `call`). What `call` adds is only one more parsing pass, so it eats even `%%`, which survives one pass (`"50%% done"` → without it
#     `50% done`, with it `50 done`). So **the rule is not to pass arguments containing `%` to this wrapper**, and that rule is the same before and after this change.
$target = Resolve-CmdTarget (Get-FirstToken $Command) $WorkDir
$callPrefix = ''
if ($target) {
  $targetExt = ([System.IO.Path]::GetExtension($target)).ToLowerInvariant()
  if ($targetExt -eq '.cmd' -or $targetExt -eq '.bat') { $callPrefix = 'call ' }
}

# ★Put only the tag and fixed wording in the console title — do not splice in the command line or path★: the title line is parsed by cmd too, so
#   `& | < > ^ %` in it breaks the batch. The reasons for giving a title = tell what the minimized console's taskbar button is, and
#   warn that closing that window ends the attached console process via CTRL_CLOSE and aborts the run (console probe measured 2026-09-25).
#   A console child can overwrite the title (`npx --version` changes it to `npm` — measured).
$batBody = @"
@echo off
title engram run-detached $tag - closing this window aborts the run
cd /d "$WorkDir"
$callPrefix$Command
echo __EXIT=%ERRORLEVEL%
"@
Set-Content -LiteralPath $bat -Value $batBody -Encoding ASCII

if (Test-Path -LiteralPath $LogFile) { Remove-Item -LiteralPath $LogFile -Force }

$launch = 'cmd.exe /c ""' + $bat + '" > "' + $LogFile + '" 2>&1"'
# ★Launch the wrapper console 「활성화 없이 최소화」 (minimized without activation) (`ShowWindow = 7` = SW_SHOWMINNOACTIVE)★ — if created without a show setting, the
#   new console comes up as a normal window and steals focus (on machines where the default terminal is delegated to Windows Terminal, that new window comes to
#   the front). With 7, conhost does not delegate and only brings up its own window minimized, so the foreground window does not change (measured 2026-09-25).
#   Console children (cargo · rustc · test binaries · node) share this console, so they do not open a window separately.
#   ★Do not change it to 0 (SW_HIDE)★ — keeping the taskbar button is the requirement (user decision 2026-09-25).
#   ★With no foreground window at all (foreground = null), this console takes the foreground even with 7★ — conhost skips
#   `SetActiveWindow` when 7 (`ActivateAndShow` in `microsoft/terminal` `src/interactivity/win32/window.cpp`),
#   but the first show in that state seems to be activated by the window manager (the mechanism is a guess — the observation that it takes the foreground is measured).
#   When there is an ordinary foreground window it does not take it (even launching five wrappers in a row)
#   (foreground probe measured 2026-09-25 — wrapper console as the target).
#   Alternatives reviewed and discarded (keeping minimized — user decision 2026-09-25): SW_HIDE was dropped because it has no taskbar button, and
#   launching hidden and then calling `ShowWindow(7)` from another process was not chosen because it needs a compiled helper.
#   ★Do not put CREATE_NO_WINDOW (0x08000000) in `CreateFlags`★ — WMI rejects it with ReturnValue 21
#   (measured source of truth = the `spawn_outside_job` comment in `crates/engram-dashboard-platform/src/spawn.rs` · `real_wmi_spawn_flag_matrix` in the shell's `src-tauri/src/discovery/tests.rs`).
$startup = ([WMIClass]"\\.\root\cimv2:Win32_ProcessStartup").CreateInstance()
$startup.ShowWindow = 7
# ★Do not remove CREATE_BREAKAWAY_FROM_JOB (0x01000000)★ — Microsoft docs (`Win32_Process.Create` Remarks):
#   "Processes created with the Win32_Process.Create method are limited by the job object unless the
#   CREATE_BREAKAWAY_FROM_JOB flag is specified" (provider host quota = `__ProviderHostQuotaConfiguration`).
#   On this machine the new process belonged to no Job regardless of the flag, and RV=0 (measured 2026-09-25) —
#   it is a defense for environments where that Job applies. If that Job forbids breakaway, Create fails and shows up as `LAUNCH_FAILED` (documentation-based, unverified).
$startup.CreateFlags = 0x01000000
$res = ([WMIClass]"\\.\root\cimv2:Win32_Process").Create($launch, $null, $startup)

if ($res.ReturnValue -ne 0) {
  Write-Output ("LAUNCH_FAILED (Win32_Process.Create returned " + $res.ReturnValue + ")")
  exit 1
}

Write-Output ("PID=" + $res.ProcessId)
Write-Output ("LOG=" + $LogFile)
Write-Output ("BAT=" + $bat)
