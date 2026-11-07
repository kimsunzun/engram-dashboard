# Launch the app outside the current shell tree — the WMI process factory (`Win32_Process.Create`) creates the process on our behalf.
#
# ★Why it is needed★: if run plainly from the Bash tool, the app becomes a descendant of the shell, and the app's output travels up that
#   chain through the pipe. Then the call hangs on the app's lifetime, and when the shell that launched the app ends, the app goes down with it.
#   The WMI path (a) is outside our tree because the creator is `WmiPrvSE.exe` (it also escapes the agent shell's Job —
#   `docs/process/step-log.md` spike #1) and (b) sends output only to a file.
#
# ★Do not go back to Task Scheduler (`schtasks`)★: the STARTUPINFO the scheduler gives the action process has no window-show
#   setting (dwFlags=0x80 · no USESHOWWINDOW — measured 2026-09-25) and the task definition has no field to give that value either.
#   So the wrapper cmd console came up as a normal window and stole focus (on machines where the default terminal is delegated to Windows Terminal,
#   that new window comes to the front). WMI gives that value through `Win32_ProcessStartup.ShowWindow` (see the comment below).
#
# ★Avoiding the crash is not this script's reason — the old comment that said so was a misdiagnosis★ (corrected 2026-08-19).
#   The `0xc00000fd` in `wezterm-gui` was that terminal's own bug (no cycle guard in its process-tree recursion), and
#   an injected security-software DLL was not the cause. Cause and evidence record = git history (the old qa binding 「분리 실행」).
#
# Usage: launch-detached.ps1 -Exe <path> [-WorkDir <path>] [-LogFile <path>] [-EnvVars "K=V","K2=V2"]
# The wrapper console always comes up minimized and inactive (exception = the ShowWindow=7 comment below). This script cannot decide the app window's first show state (see the `$appLine` comment below).
# ★If an app with the same bundle identifier is already running (release = `com.engram.dashboard` — e.g. a release the user has installed and uses),
#   the new app does not come up★ — the new instance, through the single-instance callback, makes the **existing** window show·unminimize·focus and ends itself
#   (`tauri_plugin_single_instance::init` in `src-tauri/src/lib.rs` → `show_main_ui` in `src-tauri/src/tray/actions.rs` ·
#   `std::process::exit(0)` in `platform_impl/windows.rs` of tauri-plugin-single-instance 2.4.2). Then the foreground window
#   changes, and the 20-second polling below catches the pid of that new instance that is about to vanish, or emits `LAUNCH_FAILED`.
#   Debug is the same — the dev identifier `com.engram.dashboard.dev` is the same for all worktrees (`src-tauri/tauri.dev.conf.json`), so
#   if another worktree's debug app is running, this worktree's debug launch is handed over to that app in the same way (observed 2026-09-24 — the `/qa` feedback item 「다른 워크트리의 dev 앱」).
# Returns: the pid of the started process (last line of standard output "PID=<n>")
# Log: if `-LogFile` is given, that path is used (a relative path is made absolute against `-WorkDir` — see the comment below).
#   If omitted, `%TEMP%\detached-<exe이름>-<이번 실행 태그>.log` — **a new file every run** (see the comment below).
#   The actual path is reported with the `LOG=` line on success and the `LAUNCH_FAILED` line on failure.
# Failure: `LAUNCH_FAILED (...)` + exit code 1.

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

# A tag that tells this run apart — the wrapper .bat · default log name · wrapper console title all use it.
$tag = [System.Guid]::NewGuid().ToString('N').Substring(0, 8)

# ★Put **this run's tag** in the default log name (do not go back to the exe name alone)★
#   The exe name cannot tell distributions apart — worktrees (`engram-dashboard`·`-wt2`·`-wt3`) and the same worktree's
#   debug/release are all `engram-dashboard.exe`. With the name alone they all gather in one file
#   (`%TEMP%\detached-engram-dashboard.log`), and since the app that came up first keeps that file open all session, the
#   next app's wrapper .bat cannot open the redirection and cmd dies with exit code 1 **before even reaching the exe**
#   (measured in the scheduler-path era: scheduler event 201 recorded return code 2147942401 = 0x80070001 in about 1 second,
#   with no app tracing log at all. The current WMI path has no such scheduler history, so only the symptom below remains).
#   That symptom shows up as `LAUNCH_FAILED (no new ... within 20s)` below + an empty log tail, so
#   **a launch failure reads as an app failure.** Since ADR-0137, concurrent runs are the normal state, so this is the default path, not an exception.
#   The tag is drawn fresh every run, so it **structurally never collides** — there is no need to consider path normalization or collision margin,
#   and no need to pre-check for locking (an open file cannot be picked again).
#   Cost: one log is left per run and nothing deletes them. The path is reported with the `LOG=` line.
# ★Make a relative `-LogFile` absolute against `$WorkDir` (do not remove this)★: otherwise the base splits three ways —
#   the cwd the caller sees · .NET's `[Environment]::CurrentDirectory` (can differ from PowerShell's cwd,
#   measured) · the cwd after the wrapper's `cd /d "$WorkDir"`. The file is actually created at the third, so unless
#   it is made absolute, the path the caller specified and the actual file end up in different folders.
if ($LogFile -eq "") {
  $stem = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
  $LogFile = Join-Path $env:TEMP ("detached-" + $stem + "-" + $tag + ".log")
} elseif (-not [System.IO.Path]::IsPathRooted($LogFile)) {
  $LogFile = Join-Path $WorkDir $LogFile
}

$imageName = [System.IO.Path]::GetFileName($Exe)

# ★Tell a new process apart by **exe path**, not by image name (do not go back to name comparison)★: with the name alone,
#   an app of a **different distribution** that came up inside the 20-second polling window below gets caught as ours (worktrees · debug/release are all
#   `engram-dashboard.exe`). Returning that pid as `PID=` makes the caller's teardown (`taskkill /PID <n> /T /F`)
#   kill someone else's distribution app — on the daemon side, ADR-0139 decision 2 blocked the same misidentification with a name+path check, and
#   this spot is what was left on the app side. With ADR-0137 making concurrent launch normal, this window really does open.
# ★If Path cannot be read, treat it as not ours★ — on insufficient permission (elevated · another session) access fails, and
#   including it would claim someone else's pid as ours. The worst case of excluding it is a false LAUNCH_FAILED report (harmless).
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

# ★Why we go through a wrapper .bat★: env setup · working folder · redirection are all put in the .bat rather than quoted and
#   pushed into one WMI command line. A process created by WMI does not inherit the caller's environment, so the only
#   way `-EnvVars` reaches the app is the `set` lines in this file.
$bat = Join-Path $env:TEMP "engram-detach-$tag.bat"

# ★This script cannot decide the first show state of the Engram app window — the app always comes up as a normal window★ (measured 2026-09-25, release app)
#   cmd does not pass the show setting it received (7 for the wrapper console) on to the program it runs directly (GUI probe STARTUPINFO = dwFlags 0).
#   ★Do not put `start "" /b /min` back★ — today it has no effect, and it breaks if tao changes: the app process
#   receives STARTF_USESHOWWINDOW + 7 (confirmed in the PEB), but Windows applies that value only to the process's **first** ShowWindow,
#   and tao creates the window without WS_VISIBLE, then applies the window flags and issues SW_HIDE as its first call (`tao-0.35.3`
#   gathers the flags at `src/platform_impl/windows/window.rs:1124-1149` and `window.rs:1320` `set_window_flags` →
#   `window_state.rs:420-423` `apply_diff` issues SW_HIDE). If that first call stops being SW_HIDE (e.g. if the window
#   comes to be created WS_VISIBLE), 7 would make the window iconic **before** the WebView exists and the breakage below (hidden page ·
#   skewed viewport · CDP screenshot hang) is expected (inferred from the iconic measured when forced from outside — this path itself is unmeasured). The following `set_visible(true)` is a plain SW_SHOW (`window_state.rs:325-335`), so the window
#   comes up normal, and it is activated unless the window is built unfocused (tao `MARKER_DONT_FOCUS = !focused` · Tauri default focus = true).
#   Measured: the main window was never iconic (watched at 30 ms intervals). A probe on the same path also gave iconic for CreateWindow→SW_SHOW and
#   normal for CreateWindow→SW_HIDE→SW_SHOW. Foreground change could not be measured in this run (session separation, so foreground = NULL).
# ★Do not force the window iconic from outside — **before** the WebView child exists★: the window is
#   parked at (-25600,-25600), the WebView is built at 175×0, the page is skewed to `visibilityState=hidden` · viewport 2051×1440, and
#   `cdp.mjs shot` hangs (force-killed at 20 seconds). Minimizing **after** the first show is harmless (viewport · visibility · screenshot normal).
#   Launching the app window minimized or unfocused needs support on the app side — not there yet.
$appLine = "`"$Exe`" > `"$LogFile`" 2>&1"

# ★Put only the tag and fixed wording in the console title — do not splice in a path★: the title line is parsed by cmd too, so
#   `& | < > ^ %` in it breaks the batch. What the title warns about = a debug build, a console-subsystem app, shares this console, so
#   closing the window kills it too via CTRL_CLOSE — regardless of whether a handler exists (HandlerRoutine docs: CTRL_CLOSE terminates by any path).
#   A release (GUI subsystem) app is not attached to the console, so only the wrapper ends (measured with console and GUI probes 2026-09-25).
$lines = @('@echo off', "title engram launch-detached $tag - closing this window kills a debug-build app", "cd /d `"$WorkDir`"")
foreach ($e in $EnvVars) { $lines += "set `"$e`"" }
$lines += $appLine
Set-Content -LiteralPath $bat -Value $lines -Encoding ASCII

$newPid = $null
$launchError = $null
try {
  # ★Launch the wrapper cmd console 「활성화 없이 최소화」 (minimized without activation) (`ShowWindow = 7` = SW_SHOWMINNOACTIVE)★ —
  #   the taskbar button stays and the foreground window does not change. With this value conhost does not delegate to the default terminal either (measured 2026-09-25).
  #   ★Do not change it to 0 (SW_HIDE)★ — keeping the taskbar button is the requirement (user decision 2026-09-25).
  #   The exception where 7 takes the foreground when there is no foreground window, and the alternatives reviewed and discarded = the ShowWindow=7 comment in `run-detached.ps1`.
  #   ★Do not put CREATE_NO_WINDOW (0x08000000) in `CreateFlags`★ — WMI rejects it with ReturnValue 21
  #   (measured source of truth = the `spawn_outside_job` comment in `crates/engram-dashboard-platform/src/spawn.rs` · `real_wmi_spawn_flag_matrix` in the shell's `src-tauri/src/discovery/tests.rs`).
  #   Unverified: the new process is assumed to follow the caller's token and session — called from outside the interactive desktop (SSH etc.) the window may not
  #   be visible, and called from an elevated shell the app also comes up elevated.
  $startup = ([WMIClass]"\\.\root\cimv2:Win32_ProcessStartup").CreateInstance()
  $startup.ShowWindow = 7
  # ★Do not remove CREATE_BREAKAWAY_FROM_JOB (0x01000000)★ — rationale · measurement = the CREATE_BREAKAWAY_FROM_JOB comment in `run-detached.ps1`
  #   (it keeps the app apart from the WMI provider host's Job quota).
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
  # Print the path too — the log name differs every run, so without this line the caller does not know which file to look at.
  Write-Output "LAUNCH_FAILED (no new $imageName within 20s). log: $LogFile"
  Write-Output "log tail:"
  if (Test-Path $LogFile) { Get-Content $LogFile -Tail 20 | ForEach-Object { "  $_" } }
  exit 1
}

Write-Output "LOG=$LogFile"
Write-Output "PID=$newPid"
