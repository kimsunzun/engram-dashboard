#!/usr/bin/env pwsh
# scripts/build-release.ps1 — ADR-0100 릴리즈 패키징(portable-folder-only).
#
# ★ADR-0100 불변식 (이 스크립트가 소유한다) — co-location★
#   릴리즈는 인스톨러/사이드카가 아니라 "한 폴더에 필요한 것만" 형태로 배포한다. 세 exe 와 prompts/ 가
#   **반드시 같은 디렉토리(release/)에 동거**해야 런타임이 서로를 찾는다:
#     - 실행되는 앱(engram-dashboard.exe)은 형제 engram-dashboard-daemon.exe 를 spawn 한다.
#     - 데몬은 형제 engram-send.exe 를 locate_send_exe(current_exe().parent())로 찾는다(ADR-0086 F1).
#     - 데몬은 프라이밍 prompts/agent-priming[-cli].md 를 find_install_root(=릴리즈에선 exe 폴더) 기준
#       상대해석한다(ADR-0092). 릴리즈 폴더엔 .git·[workspace] 마커가 없으므로 install_root = exe 디렉토리.
#   따라서 manifest(이 스크립트의 EXPECTED_*)가 곧 "무엇을 배송하는가"의 단일 출처다. manifest 를 바꾸면
#   위 동거 불변식이 깨지지 않는지 반드시 재검토한다. tauri.conf.json 은 건드리지 않는다(정식 번들은 유예).
#
# 사용: 프로젝트 루트 어디서든 `pwsh scripts/build-release.ps1`. cwd 무관(경로는 스크립트 위치 기준).

$ErrorActionPreference = 'Stop'

# ── 경로 앵커 (cwd 불신 — 스크립트 자기 위치 기준으로 프로젝트 루트를 잡는다) ─────────────
$ScriptDir   = $PSScriptRoot                              # <root>/scripts
$ProjectRoot = Split-Path -Parent $ScriptDir             # <root>
$ReleaseDir  = Join-Path $ProjectRoot 'release'          # 조립 산출물(clean 재생성)
$PromptsSrc  = Join-Path $ProjectRoot 'prompts'
# $TargetRel(cargo 산출 release 디렉토리)은 하드코딩하지 않는다 — 아래에서 cargo metadata 로 실측 확정(FIX-2).

# ── manifest: 릴리즈 폴더에 들어가는 정확한 집합(단일 출처). exe = target/release 에서, prompt = prompts/ 에서. ──
$ExpectedExes    = @('engram-dashboard.exe', 'engram-dashboard-daemon.exe', 'engram-send.exe')
$ExpectedPrompts = @('agent-priming.md', 'agent-priming-cli.md')

function Fail([string]$msg) {
    Write-Host "[build-release] 실패: $msg" -ForegroundColor Red
    exit 1
}

# 빌드 단계마다 종료코드를 직접 확인한다($LASTEXITCODE — 네이티브 exe 는 $ErrorActionPreference 로 안 잡힘).
# FIX-4: $LASTEXITCODE 를 블록 진입 전에 0 으로 리셋한다. 이 게이트는 "블록이 네이티브 exe 로 끝난다"는
#   암묵 불변식에 의존한다 — 만약 미래에 어떤 step-block 이 PowerShell cmdlet 으로 끝나면 이전 네이티브
#   exit code 가 그대로 남아 오탐/누락이 생긴다. 리셋으로 stale exit code 계승을 원천봉쇄한다.
function Invoke-Step([string]$label, [scriptblock]$block) {
    Write-Host "[build-release] $label ..." -ForegroundColor Cyan
    $global:LASTEXITCODE = 0
    & $block
    if ($LASTEXITCODE -ne 0) { Fail "$label (exit $LASTEXITCODE)" }
}

Write-Host "[build-release] ADR-0100 portable release 조립 시작 — root=$ProjectRoot" -ForegroundColor Green

# ── cargo 산출 디렉토리 실측 (FIX-2: 가정 금지) ──────────────────────────────────────────────
#   `target/release` 는 기본값일 뿐 — CARGO_TARGET_DIR / .cargo/config.toml build.target-dir 이
#   출력을 딴 데로 돌릴 수 있다. 하드코딩하면 그런 환경에서 "가정 경로의 낡은 exe"가 존재 검사를 통과해
#   최신 빌드는 다른 곳에 떨어진 채 stale 바이너리가 조용히 배송된다. cargo metadata 는 이 리디렉션을
#   존중하므로 target_directory 를 실측해 확정한다.
$metaJson = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $ProjectRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { Fail "cargo metadata 실패 (exit $LASTEXITCODE)" }
try {
    $meta = $metaJson | ConvertFrom-Json
} catch {
    Fail "cargo metadata 출력 파싱 실패: $($_.Exception.Message)"
}
$TargetDir = $meta.target_directory
if ([string]::IsNullOrWhiteSpace($TargetDir)) { Fail 'cargo metadata 에 target_directory 가 없다' }

# CARGO_BUILD_TARGET(비-호스트 triple) 이 걸려 있으면 release 디렉토리가 target/<triple>/release 로 바뀐다.
#   metadata 의 target_directory 는 이 하위 triple 세그먼트를 포함하지 않으므로 조용히 stale 을 배송하지
#   않도록 명시적으로 끼워 넣는다. (--target 플래그로 넘기는 경로는 이 스크립트가 쓰지 않으므로 env 만 본다.)
$BuildTargetTriple = $env:CARGO_BUILD_TARGET
if (-not [string]::IsNullOrWhiteSpace($BuildTargetTriple)) {
    Write-Host "[build-release] CARGO_BUILD_TARGET=$BuildTargetTriple 감지 — release 경로에 triple 세그먼트 반영" -ForegroundColor Yellow
    $TargetRel = Join-Path (Join-Path $TargetDir $BuildTargetTriple) 'release'
} else {
    $TargetRel = Join-Path $TargetDir 'release'
}
Write-Host "[build-release] cargo target release dir = $TargetRel" -ForegroundColor Gray

# ── (a) UI 앱: Tauri CLI production 빌드로 dist/ 를 embed (FIX-5, ADR-0100) ──────────────────
#   ★프로덕션 컨텍스트 불변식★: 순수 `cargo build --release -p engram-dashboard`(구 Option A)는 Tauri 를
#   프로덕션 컨텍스트로 넣지 못한다 — 그 exe 는 frontendDist 를 embed 하지 않고 devUrl(http://localhost:1420)을
#   로드해, 릴리즈로 실행하면 dev 서버 부재로 "connection refused" 화면만 뜨고 데몬이 뜨지 않는다.
#   프로덕션 컨텍스트·frontendDist embed·beforeBuildCommand 실행은 오직 Tauri CLI(`tauri build`)만 한다.
#   그래서 Option B: `npm run tauri -- build --no-bundle`(= tauri build --no-bundle) 로 컴파일한다.
#     - tauri build 가 tauri.conf.json 의 beforeBuildCommand(`npm run build`)를 스스로 실행해 dist/ 를
#       먼저 굽고 그 뒤 frontendDist(../dist)를 exe 에 embed 한다 → 별도 npm run build 단계는 불필요(제거).
#     - --no-bundle: exe 만 컴파일하고 MSI/NSIS 인스톨러 번들링은 건너뛴다(portable-folder 배포이므로).
#   ★--all-targets 금지★(daemon 단계 참조)는 tauri build 엔 해당 없음 — CLI 가 UI 앱 crate 만 컴파일한다.
# FIX-1: npm 은 cargo 처럼 --manifest-path 앵커가 없어 CALLER cwd 에서 돌면 엉뚱한 패키지를 빌드한다.
#   Push/Pop-Location 으로 작업 디렉토리를 $ProjectRoot 에 고정해 tauri.conf.json 을 확실히 집게 한다.
Invoke-Step 'tauri build UI app (npm run tauri -- build --no-bundle)' {
    Push-Location $ProjectRoot
    try { & npm run tauri -- build --no-bundle } finally { Pop-Location }
}
# tauri build 가 beforeBuildCommand 로 프론트를 굽고 embed 했는지 sanity 확인 — dist/index.html 부재면
#   frontendDist embed 가 일어나지 않았다는 신호(빈 프론트 exe 배송 위험).
if (-not (Test-Path (Join-Path $ProjectRoot 'dist\index.html'))) {
    Fail 'dist/index.html 부재 — tauri build 의 beforeBuildCommand(프론트 빌드)가 산출물을 안 만들었다(embed 실패 위험)'
}

# ── (a) 릴리즈 바이너리: 데몬 + send ─────────────────────────────────────────────────────────
#   데몬+send 는 UI 앱과 **별개 crate 의 두 [[bin]]** — tauri build 는 이들을 빌드하지 않으므로 별도 컴파일한다.
#   **명시 --bin** 으로만 빌드(측정 전용 bin(saturation-pilot/priming-smoke/roundtrip-smoke)은
#    required-features=test-harness 로 이미 릴리즈 그래프에서 제외되지만, 명시 --bin 으로 유입 가능성 봉쇄).
#   ★--all-targets 금지★: daemon crate 의 self-dev-dependency(test-harness) 유니피케이션으로 yield-seam hook
#    이 운영 바이너리에 박힐 수 있다(Cargo.toml 경고). 아래 명령은 --all-targets 를 쓰지 않는다.
Invoke-Step 'cargo build daemon + send (engram-dashboard-daemon)' {
    & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') `
        -p engram-dashboard-daemon --bin engram-dashboard-daemon --bin engram-send
}

# ── 산출 exe 위치 확정(가정 금지 — 실제 파일 존재 확인) ───────────────────────────────────
foreach ($exe in $ExpectedExes) {
    $p = Join-Path $TargetRel $exe
    if (-not (Test-Path $p)) { Fail "빌드 산출 exe 부재: $p (빌드가 이 바이너리를 안 만들었다)" }
}
foreach ($md in $ExpectedPrompts) {
    $p = Join-Path $PromptsSrc $md
    if (-not (Test-Path $p)) { Fail "프라이밍 원본 부재: $p" }
}

# ── (b) release/ clean 재생성 ────────────────────────────────────────────────────────────
if (Test-Path $ReleaseDir) { Remove-Item -Recurse -Force $ReleaseDir }
New-Item -ItemType Directory -Path $ReleaseDir | Out-Null
$ReleasePrompts = Join-Path $ReleaseDir 'prompts'
New-Item -ItemType Directory -Path $ReleasePrompts | Out-Null

# ── (c) manifest 그대로만 복사(그 외 아무것도) ───────────────────────────────────────────
foreach ($exe in $ExpectedExes)    { Copy-Item (Join-Path $TargetRel $exe) $ReleaseDir }
foreach ($md in $ExpectedPrompts)  { Copy-Item (Join-Path $PromptsSrc $md) $ReleasePrompts }

# ── (d) manifest 검증(tripwire): release/ 가 정확히 기대 집합인지 — 부족/여분 모두 실패 ───────
$errors = @()

# 최상위: 정확히 3 exe + prompts/ 디렉토리 하나. 그 외 파일·디렉토리는 stray.
# FIX-3: -Force 로 숨김/시스템 항목까지 열거해야 tripwire 의 엄격성이 실제로 성립한다(숨은 stray 회피 방지).
$topFiles = @(Get-ChildItem -Force -File      -Path $ReleaseDir | ForEach-Object Name)
$topDirs  = @(Get-ChildItem -Force -Directory -Path $ReleaseDir | ForEach-Object Name)

foreach ($exe in $ExpectedExes) {
    if ($topFiles -notcontains $exe) { $errors += "누락(top): $exe" }
}
foreach ($f in $topFiles) {
    if ($ExpectedExes -notcontains $f) { $errors += "여분(top file): $f" }
}
foreach ($d in $topDirs) {
    if ($d -ne 'prompts') { $errors += "여분(top dir): $d" }
}
if ($topDirs -notcontains 'prompts') { $errors += '누락(top dir): prompts/' }

# prompts/: 정확히 두 .md, 하위 디렉토리 없음.
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

# ── 성공 요약 ────────────────────────────────────────────────────────────────────────────
Write-Host ''
Write-Host "[build-release] OK — release/ manifest 일치 (ADR-0100 co-location 유지)" -ForegroundColor Green
Write-Host "  위치: $ReleaseDir" -ForegroundColor Green
Get-ChildItem -Recurse -File -Path $ReleaseDir | ForEach-Object {
    $rel = $_.FullName.Substring($ReleaseDir.Length + 1)
    $kb  = [math]::Round($_.Length / 1KB, 1)
    Write-Host ("  {0,-40} {1,8} KB" -f $rel, $kb) -ForegroundColor Gray
}
exit 0
