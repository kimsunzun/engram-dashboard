#!/usr/bin/env pwsh
# scripts/build-release.ps1 — ADR-0100 release packaging (portable-folder-only).
#
# ★ADR-0100 invariant (this script owns it) — co-location★
#   A release ships not as an installer/sidecar but as "only what is needed in one folder". The three exes and prompts/
#   **must live together in the same directory (release/)** so that they can find each other at runtime:
#     - The running app (engram-dashboard.exe) spawns the sibling engram-dashboard-daemon.exe.
#     - The daemon finds the sibling engram.exe (control-plane CLI) with locate_send_exe(current_exe().parent()) (ADR-0086 F1).
#       ★The source of truth for that file name is the agent's CLI_EXE_NAME★ — the manifest below is a hand copy of that value.
#       A mismatch usually breaks loudly: if the [[bin]] is renamed, the --bin argument of the build step below fails, and if only the constant
#       drifts, CI's shipped-file-name guard test (crates/engram-dashboard-cli/tests/engram_cli.rs) goes red.
#       ★But there is one quiet hole — stale files in the cargo output directory★: this script does not
#       clean target/release, so if an exe with the old name is left over and you fix only the manifest below to that old name,
#       both the existence check and the tripwire pass and **that stale binary gets packaged as is** (even though the build did not
#       update that file). When you fix the manifest, check the output directory too.
#     - The daemon resolves the priming file prompts/agent-priming.md relative to find_install_root (ADR-0092).
#       That rule looks for .git·[workspace] markers among the exe's ancestors regardless of build mode — for a release folder assembled
#       outside the checkout there is no marker, so install_root = the exe directory, and for one inside a checkout like launch\release of launch\빌드.bat,
#       it is that checkout's root, so it reads the repository's prompts/ rather than the distribution folder's.
#     - The daemon reads prompts/engram-help.md, the body of the `engram help` screen, with the **same anchor** on every request and serves it as /control/help
#       (ADR-0092-family externalization · ADR-0285 — engram.exe only asks the daemon for the screen and does not read this file).
#       ★There is no copy★ — in a distribution missing this file, every help call is answered with a bounce (INTERNAL). The tripwire below is the
#       device that makes that failure happen before shipping, not in front of an agent after.
#   So the manifest (EXPECTED_* in this script) is the single source of "what do we ship". When you change the manifest,
#   always re-check that the co-location invariant above is not broken. tauri.conf.json is left untouched (formal bundling is deferred).
#
# Usage: `pwsh scripts/build-release.ps1` from anywhere in the project root. cwd does not matter (paths are relative to the script location).
#   If -OutDir sets the assembly location, output can go outside the repo too (for test convenience — used by the local\ launcher).

# param() must be the first executable statement of the script (comments and blank lines may precede it) — any statement placed above it fails parsing.
param([string]$OutDir = '')

$ErrorActionPreference = 'Stop'

# ── Path anchor (do not trust cwd — find the project root relative to the script's own location) ─────────────
$ScriptDir   = $PSScriptRoot                              # <root>/scripts
$ProjectRoot = Split-Path -Parent $ScriptDir             # <root>
# Assembly output (clean regeneration). Parameterized with -OutDir so assembly can also run outside the repo.
#   When unspecified, falls back to the old repo-root release/ (a bare call behaves as before).
#   The GetFullPath 2-arg overload makes a relative path absolute against the caller cwd — Resolve-Path throws on a path that does not exist yet,
#   and the release directory may not exist at this point, so it is not used.
if (-not [string]::IsNullOrWhiteSpace($OutDir)) {
    $ReleaseDir = [System.IO.Path]::GetFullPath($OutDir, (Get-Location).Path)
} else {
    $ReleaseDir = Join-Path $ProjectRoot 'release'
}
$PromptsSrc  = Join-Path $ProjectRoot 'prompts'
# $TargetRel (the cargo output release directory) is not hardcoded — it is fixed below by measuring with cargo metadata (FIX-2).

# ── manifest: the exact set that goes into the release folder (single source). exe = from target/release, prompt = from prompts/, doc = from the step (0) output. ──
$ExpectedExes    = @('engram-dashboard.exe', 'engram-dashboard-daemon.exe', 'engram.exe')
$ExpectedPrompts = @('agent-priming.md', 'engram-help.md')
# ADR-0294 · ADR-0100: third-party license notices — the runtime does not read them, so they are unrelated to the co-location invariant above, and are only
#   shipped at the top level next to the exes (the manifest tripwire sees them together with the other items). Not kept in the repository — step (0) below regenerates them on every build.
$NoticeName      = 'THIRD_PARTY_LICENSES.txt'
$ExpectedDocs    = @($NoticeName)

function Fail([string]$msg) {
    Write-Host "[build-release] 실패: $msg" -ForegroundColor Red
    exit 1
}

# Check the exit code directly at every build step ($LASTEXITCODE — native exes are not caught by $ErrorActionPreference).
# FIX-4: reset $LASTEXITCODE to 0 before entering the block. This gate depends on the implicit invariant that "the block ends with a native exe" —
#   if some step-block ever ends with a PowerShell cmdlet, the previous native
#   exit code stays and causes false positives/misses. The reset shuts out inheriting a stale exit code at the source.
function Invoke-Step([string]$label, [scriptblock]$block) {
    Write-Host "[build-release] $label ..." -ForegroundColor Cyan
    $global:LASTEXITCODE = 0
    & $block
    if ($LASTEXITCODE -ne 0) { Fail "$label (exit $LASTEXITCODE)" }
}

Write-Host "[build-release] ADR-0100 portable release 조립 시작 — root=$ProjectRoot" -ForegroundColor Green
Write-Host "[build-release] 조립 위치(release dir) = $ReleaseDir" -ForegroundColor Green

# Safety guard: $ReleaseDir is deleted wholesale with Remove-Item -Recurse -Force in step (b) below — -OutDir is now a
#   caller-supplied path, so a mistake with a drive root or the project root would blow that folder away. Only catastrophic values (drive
#   root · ProjectRoot) are rejected and normal subfolders pass through untouched (no friction for trusted local use).
if ($ReleaseDir -eq [System.IO.Path]::GetPathRoot($ReleaseDir) -or $ReleaseDir -eq $ProjectRoot) {
    Fail "위험한 -OutDir: '$ReleaseDir' — 드라이브 루트/프로젝트 루트는 clean 재생성(재귀 삭제) 대상이 될 수 없다"
}

# ── Measure the cargo output directory (FIX-2: no assuming) ──────────────────────────────────────────────
#   `target/release` is only the default — CARGO_TARGET_DIR / .cargo/config.toml build.target-dir can
#   redirect the output elsewhere. If hardcoded, in such an environment a "stale exe at the assumed path" passes the existence check,
#   the latest build lands elsewhere, and the stale binary is shipped quietly. cargo metadata respects this redirection,
#   so target_directory is measured and fixed.
$metaJson = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $ProjectRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { Fail "cargo metadata 실패 (exit $LASTEXITCODE)" }
try {
    $meta = $metaJson | ConvertFrom-Json
} catch {
    Fail "cargo metadata 출력 파싱 실패: $($_.Exception.Message)"
}
$TargetDir = $meta.target_directory
if ([string]::IsNullOrWhiteSpace($TargetDir)) { Fail 'cargo metadata 에 target_directory 가 없다' }

# If CARGO_BUILD_TARGET (a non-host triple) is set, the release directory becomes target/<triple>/release.
#   The target_directory in metadata does not include this triple segment, so it is inserted explicitly so that a stale binary
#   is not quietly shipped. (This script does not pass a --target flag, so it looks only at the env.)
$BuildTargetTriple = $env:CARGO_BUILD_TARGET
if (-not [string]::IsNullOrWhiteSpace($BuildTargetTriple)) {
    Write-Host "[build-release] CARGO_BUILD_TARGET=$BuildTargetTriple 감지 — release 경로에 triple 세그먼트 반영" -ForegroundColor Yellow
    $TargetRel = Join-Path (Join-Path $TargetDir $BuildTargetTriple) 'release'
} else {
    $TargetRel = Join-Path $TargetDir 'release'
}
Write-Host "[build-release] cargo target release dir = $TargetRel" -ForegroundColor Gray

# ── (0) Generate third-party notices — before the long build (ADR-0294) ────────────────────────────────────
#   Failures such as a license outside the allowed set or a package with no license text stop here, not after a build of tens of minutes. The prerequisites (cargo-about and
#   node_modules after `npm ci`) and the failure conditions are defined in the generator's header. CI runs the same generator on every push in the gates job too.
#   The output goes under the cargo output directory (outside the repository · fresh on every build). ★Delete the previous edition first★ — so that even if generation ends
#   without having updated that file, a stale notice does not pass the existence check and ship (the same kind as the stale hole for exes below).
$DocsSrc    = Join-Path $TargetDir 'third-party-licenses'
$NoticeFile = Join-Path $DocsSrc $NoticeName
if (Test-Path -LiteralPath $NoticeFile) { Remove-Item -Force -LiteralPath $NoticeFile }
Invoke-Step 'generate third-party notice (gen-third-party-licenses.mjs)' {
    & node (Join-Path $ScriptDir 'gen-third-party-licenses.mjs') $NoticeFile
}

# ── (a) UI app: embed dist/ with a Tauri CLI production build (FIX-5, ADR-0100) ──────────────────
#   ★Production context invariant★: plain `cargo build --release -p engram-dashboard` (the old Option A) cannot put Tauri into
#   the production context — that exe does not embed frontendDist and loads devUrl (http://localhost:1420),
#   so when run as a release it shows only a "connection refused" screen for lack of a dev server, and the daemon does not start.
#   Only the Tauri CLI (`tauri build`) does the production context, the frontendDist embed and the beforeBuildCommand run.
#   So Option B: compile with `npm run tauri -- build --no-bundle` (= tauri build --no-bundle).
#     - tauri build itself runs the beforeBuildCommand (`npm run build`) from tauri.conf.json to bake dist/
#       first and then embeds frontendDist (../dist) into the exe → a separate npm run build step is unnecessary (removed).
#     - --no-bundle: compiles only the exe and skips MSI/NSIS installer bundling (because this is a portable-folder distribution).
#   ★--all-targets forbidden★ (see the daemon step) does not apply to tauri build — the CLI compiles only the UI app crate.
# FIX-1: unlike cargo, npm has no --manifest-path anchor, so run from the CALLER cwd it builds the wrong package.
#   Push/Pop-Location pins the working directory to $ProjectRoot so that tauri.conf.json is picked up for certain.
Invoke-Step 'tauri build UI app (npm run tauri -- build --no-bundle)' {
    Push-Location $ProjectRoot
    try { & npm run tauri -- build --no-bundle } finally { Pop-Location }
}
# Sanity check that tauri build baked and embedded the frontend via beforeBuildCommand — if dist/index.html is absent it is a sign that
#   the frontendDist embed did not happen (risk of shipping an exe with an empty frontend).
if (-not (Test-Path (Join-Path $ProjectRoot 'dist\index.html'))) {
    Fail 'dist/index.html 부재 — tauri build 의 beforeBuildCommand(프론트 빌드)가 산출물을 안 만들었다(embed 실패 위험)'
}

# ── (a) Release binaries: daemon + CLI ─────────────────────────────────────────────────────────
#   The daemon and the CLI are **two packages** separate from the UI app (engram-dashboard-daemon · engram-dashboard-cli — ADR-0273)
#    — tauri build does not build them, so they are compiled separately.
#   Build only with an **explicit --bin** (the measurement-only bins (saturation-pilot/priming-smoke/roundtrip-smoke)
#    are already excluded from the release graph by required-features=test-harness, but an explicit --bin shuts out any chance of them flowing in).
#   ★--all-targets forbidden★: through the unification of the daemon crate's self-dev-dependency (test-harness · test-support), the
#    yield-seam hook can get baked into the operational binary, and the daemon lib of that call is built holding even the in-process test server
#    (Cargo.toml warning · ADR-0286). The commands below do not use --all-targets.
#   ★Do not put the two calls in one Invoke-Step block★: Invoke-Step looks at $LASTEXITCODE only once after the block ends
#    — the success of the later call covers up the failure of the earlier one, and the existence check below accepts a stale exe left by a previous build.
#    Nor are they merged into one cargo call (two -p) — the shipped engram.exe must be built with the CLI package's own graph (the one
#    that `-p engram-dashboard-cli` alone measures in tests), but with one call the features merge and it is built with the daemon's feature set. The two builds
#    write the same hash-less output file (target\release\engram.exe) — so rebuild-run-release.bat, which uses the target\release this script packages from,
#    is split the same way, and rebuild-run-debug.bat, which uses only target\debug (not packaged), is one call.
Invoke-Step 'cargo build daemon (engram-dashboard-daemon)' {
    & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') `
        -p engram-dashboard-daemon --bin engram-dashboard-daemon
}
Invoke-Step 'cargo build cli (engram-dashboard-cli)' {
    & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') `
        -p engram-dashboard-cli --bin engram
}

# ── Confirm output exe locations (no assuming — check that the actual files exist) ───────────────────────────────────
foreach ($exe in $ExpectedExes) {
    $p = Join-Path $TargetRel $exe
    if (-not (Test-Path $p)) { Fail "빌드 산출 exe 부재: $p (빌드가 이 바이너리를 안 만들었다)" }
}
foreach ($md in $ExpectedPrompts) {
    $p = Join-Path $PromptsSrc $md
    if (-not (Test-Path $p)) { Fail "프라이밍 원본 부재: $p" }
}
foreach ($doc in $ExpectedDocs) {
    $p = Join-Path $DocsSrc $doc
    if (-not (Test-Path $p)) { Fail "고지 산출 부재: $p ((0) 단계가 이 파일을 안 만들었다)" }
}

# ── (b) release/ clean regeneration ────────────────────────────────────────────────────────────
#
# ★Block before deleting (load-bearing)★: Remove-Item -Recurse **can stop midway**. If the app is
#   running from that folder, the daemon holds data\daemon\run\daemon.json (ADR-0135 — no delete sharing), so the deletion is
#   refused, but by measurement the registry had already been deleted before that (measured in the layout where the registry was data\agents.json —
#   not re-measured at the current location data\daemon\state\). Result = only the agent registry is lost, and the build cannot even
#   start, with a raw PowerShell error. `launch\빌드.bat` does not taskkill and `launch\실행.bat` launches the app from the same
#   folder, so "run it, then build again" is an ordinary entry path.
#   ★Do not fix this by quietly preserving data★: the clean regeneration deleting data with it is the behavior the ADR
#   intended. What is fixed here is only **stopping midway**, so it is refused before anything is touched.
if (Test-Path $ReleaseDir) {
    $locked = @()
    foreach ($f in @(Get-ChildItem -Force -Recurse -File -Path $ReleaseDir -ErrorAction SilentlyContinue)) {
        try {
            # Write access + no sharing = the closest question to "can this file be deleted".
            $h = [System.IO.File]::Open($f.FullName, 'Open', 'ReadWrite', 'None')
            $h.Close()
        } catch {
            # ★Count only sharing violations★: read-only attributes and permission denials (UnauthorizedAccessException) caught here are
            #   still deleted by Remove-Item -Force — reporting those as "another process is using it" blocks a perfectly good build
            #   for the wrong reason. A file that vanished during the check (FileNotFoundException) is no reason to block either.
            # ★InnerException must be unwrapped (measured)★: PowerShell wraps an exception thrown by a .NET **method** in
            #   MethodInvocationException, so `$_.Exception -is [IOException]` is false even for a sharing violation —
            #   without unwrapping, this guard catches nothing.
            $ex = $_.Exception
            while ($ex -and -not ($ex -is [System.IO.IOException]) -and $ex.InnerException) {
                $ex = $ex.InnerException
            }
            if (($ex -is [System.IO.IOException]) -and (Test-Path -LiteralPath $f.FullName)) {
                $locked += $f.FullName
            }
        }
    }
    if ($locked.Count -gt 0) {
        Write-Host "[build-release] 아래 파일을 다른 프로세스가 쓰고 있습니다:" -ForegroundColor Yellow
        foreach ($l in $locked) { Write-Host "    $l" -ForegroundColor Yellow }
        Fail "release 폴더를 통째로 지울 수 없습니다 — 앱(engram-dashboard.exe)과 데몬을 완전히 종료한 뒤 다시 실행하세요. (지우다 만 상태로 데이터만 날리지 않으려고 시작 전에 멈춥니다)"
    }
    Remove-Item -Recurse -Force $ReleaseDir
}
New-Item -ItemType Directory -Path $ReleaseDir | Out-Null
$ReleasePrompts = Join-Path $ReleaseDir 'prompts'
New-Item -ItemType Directory -Path $ReleasePrompts | Out-Null

# ── (c) Copy only the manifest (nothing else) ───────────────────────────────────────────
foreach ($exe in $ExpectedExes)    { Copy-Item (Join-Path $TargetRel $exe) $ReleaseDir }
foreach ($md in $ExpectedPrompts)  { Copy-Item (Join-Path $PromptsSrc $md) $ReleasePrompts }
foreach ($doc in $ExpectedDocs)    { Copy-Item (Join-Path $DocsSrc $doc) $ReleaseDir }

# ── (d) Verify the manifest (tripwire): whether release/ is exactly the expected set — both missing and extra fail ───────
$errors = @()

# Top level: exactly the $ExpectedExes + $ExpectedDocs files + one prompts/ directory. Any other file or directory is a stray.
# FIX-3: only enumerating hidden/system items too with -Force makes the tripwire's strictness actually hold (prevents evading it with a hidden stray).
$topFiles = @(Get-ChildItem -Force -File      -Path $ReleaseDir | ForEach-Object Name)
$topDirs  = @(Get-ChildItem -Force -Directory -Path $ReleaseDir | ForEach-Object Name)
$ExpectedTopFiles = @($ExpectedExes + $ExpectedDocs)

foreach ($top in $ExpectedTopFiles) {
    if ($topFiles -notcontains $top) { $errors += "누락(top): $top" }
}
foreach ($f in $topFiles) {
    if ($ExpectedTopFiles -notcontains $f) { $errors += "여분(top file): $f" }
}
foreach ($d in $topDirs) {
    if ($d -ne 'prompts') { $errors += "여분(top dir): $d" }
}
if ($topDirs -notcontains 'prompts') { $errors += '누락(top dir): prompts/' }

# prompts/: exactly the .md files of $ExpectedPrompts, no subdirectories. ★Do not count them here★ — a number that was counted
#   falls behind every time the manifest grows (it actually did). The source of truth is the $ExpectedPrompts array above itself.
if (Test-Path $ReleasePrompts) {
    $pFiles = @(Get-ChildItem -Force -File      -Path $ReleasePrompts | ForEach-Object Name)
    $pDirs  = @(Get-ChildItem -Force -Directory -Path $ReleasePrompts | ForEach-Object Name)
    foreach ($md in $ExpectedPrompts) {
        if ($pFiles -notcontains $md) { $errors += "누락(prompts): $md" }
    }
    foreach ($f in $pFiles) {
        if ($ExpectedPrompts -notcontains $f) { $errors += "여분(prompts file): $f" }
    }
    foreach ($d in $pDirs) { $errors += "여분(prompts dir): $d" }
}

if ($errors.Count -gt 0) {
    Write-Host "[build-release] manifest 불일치 — 배송 불가:" -ForegroundColor Red
    $errors | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}

# ── Success summary ────────────────────────────────────────────────────────────────────────────
Write-Host ''
Write-Host "[build-release] OK — release/ manifest 일치 (ADR-0100 co-location 유지)" -ForegroundColor Green
Write-Host "  위치: $ReleaseDir" -ForegroundColor Green
Get-ChildItem -Recurse -File -Path $ReleaseDir | ForEach-Object {
    $rel = $_.FullName.Substring($ReleaseDir.Length + 1)
    $kb  = [math]::Round($_.Length / 1KB, 1)
    Write-Host ("  {0,-40} {1,8} KB" -f $rel, $kb) -ForegroundColor Gray
}
exit 0
