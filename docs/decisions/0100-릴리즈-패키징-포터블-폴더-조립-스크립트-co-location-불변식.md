# ADR-0100: 릴리즈 패키징 — 포터블 폴더 조립 스크립트 (co-location 불변식)

- 상태: 확정 (2026-07-23, 근거: 사용자 결정 + S10 백엔드 조사 실측)
- 관련: CLAUDE.md "아키텍처 원칙" · ADR-0023(3-프로세스 토폴로지) · ADR-0024(데몬 생사·데이터 위치) · ADR-0086(듀얼 입구 CLI/MCP) · ADR-0092(프라이밍 파일) · ADR-0099(채널 capability 스위치·프라이밍 2파일) · `scripts/build-release.ps1` · `.gitignore`

## 맥락
릴리즈 빌드가 한 번도 검증된 적 없는 영역이었다. 실사용 확인("사용자가 대시보드에서 에이전트에게 명령")이 최초 관문인데, 현 `src-tauri/tauri.conf.json` 번들 설정에는 `resources`·`externalBin`·`sidecar`가 전혀 없어 `npm run tauri build`가 런타임 필수 동반물을 **하나도 포함하지 않는다**.

런타임 배치 규약(불변식)은 3개 exe와 프라이밍 폴더가 **같은 디렉토리에 co-located**되어야 성립한다:
- `engram-dashboard.exe`(UI 셸)가 데몬 exe를 형제로 찾음(`discovery::locate_daemon_exe` — current_exe sibling 우선).
- 데몬이 `engram-send.exe`를 형제로 찾아 `ENGRAM_SEND_EXE`로 주입(`daemon/src/lib.rs::locate_send_exe`).
- 데몬이 `prompts/agent-priming.md`(MCP-capable)·`agent-priming-cli.md`(비-MCP)를 exe-상대 install-root로 해석(`control/priming.rs` FilePrimingProvider, ADR-0092/0099).

특히 prompts 누락은 **에러 없이 조용히**(fail-open) 프라이밍 없는 에이전트를 스폰하고, engram-send 누락은 CLI 입구를 조용히 비활성화한다 — 둘 다 배포 시 사람이 눈치채기 어렵다. 따라서 배치를 자동·검증 가능하게 보장하는 패키징 절차가 필요했다.

## 결정
릴리즈를 **포터블 폴더**로 산출한다 — 빌드 후 조립 스크립트(`scripts/build-release.ps1`, Windows 전용)가 다음을 수행한다:
1. 프론트 빌드 + 릴리즈 바이너리 3종 빌드(`engram-dashboard.exe` · `engram-dashboard-daemon.exe` · `engram-send.exe`).
2. 깨끗한 `release/` 폴더(프로젝트 루트, `.gitignore`)에 **정확히 이 항목만** 복사: 위 3개 exe + `prompts/agent-priming.md` + `prompts/agent-priming-cli.md`.
3. 매니페스트 검증 — 산출 폴더에 기대 파일이 전부 있고 그 외 잡파일이 없는지 단언(불일치 시 실패).

co-location 불변식과 "무엇을 담나"의 단일 출처(SSOT)를 이 스크립트가 소유한다. 측정용 bin(`roundtrip-smoke` 등)은 `required-features=["test-harness"]`라 릴리즈 그래프에서 자동 제외된다. 런타임 데이터(`daemon.json`·`agents.json`·`sessions/`)는 `%APPDATA%\com.engram.dashboard`에 실행 시 생성되며 번들 대상이 아니다(ADR-0024).

## 거부한 대안
- **Tauri 정식 번들(externalBin/sidecar + resources → MSI/NSIS)** — 지금은 버림. (a) sidecar는 `engram-send-x86_64-pc-windows-msvc.exe` 식 target-triple 네이밍 처리가 필요해 마찰이 큼. (b) 산출물이 설치 프로그램이라 "폴더에 파일만 두고 실행" 요구와 형태가 다름. (c) 실사용 확인이라는 당면 목표엔 과함. 정식 배포 정식화 시점에 재검토(그때 이 ADR을 supersede/amend).
- **tauri.conf.json `resources`에 prompts만 추가 + 나머지(exe) 수동 복사** — 버림. 절반만 자동화되어 co-location 불변식이 tauri 설정과 수동 절차 두 곳에 쪼개져 rot한다. 배치의 단일 출처를 조립 스크립트 하나로 모으는 편이 낫다.
- **`target/release/` 폴더를 그대로 배포** — 버림. 빌드 중간물(`deps/`·`build/`·`.pdb`·기타 bin)이 섞여 "릴리즈 파일만" 요구를 위반하고, `prompts/`는 애초에 그 폴더에 없다.

## 근거
- 사용자 결정(2026-07-23): 패키징 방식 = 포터블 폴더만, 산출 위치 = `release/`. 두 대안(정식 설치본·둘 다)은 명시적으로 후순위.
- co-location 불변식·fail-open 동작은 S10 백엔드 추상화 조사에서 실코드로 확인(`locate_daemon_exe`·`locate_send_exe`·`FilePrimingProvider`).
- 측정 bin 제외는 `required-features` 게이트로 이미 보장됨(추가 조치 불필요).

## 영향 / 불변식
- **co-location 불변식(변경 금지):** `release/` 산출 폴더는 3개 exe + `prompts/` 2파일이 한 디렉토리에 함께 있어야 한다. 하나라도 빠지면 조용한 기능 저하(프라이밍 없는 스폰 / CLI 입구 사망 / 데몬 미발견)가 난다. 스크립트의 매니페스트 검증이 이 불변식의 게이트다.
- 새 런타임 동반물(추가 exe·리소스)이 생기면 이 스크립트의 매니페스트를 함께 갱신해야 한다 — 안 하면 검증이 실패해 시끄럽게 잡힌다(의도된 tripwire).
- `release/`는 `.gitignore` 대상(재빌드마다 재생성, repo 오염 방지).
- 정식 설치본이 필요해지면 이 ADR을 supersede/amend하고 tauri 번들 설정을 도입한다.
