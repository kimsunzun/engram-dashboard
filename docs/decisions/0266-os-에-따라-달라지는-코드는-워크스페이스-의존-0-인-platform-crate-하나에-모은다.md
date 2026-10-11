# ADR-0266: OS 에 따라 달라지는 코드는 워크스페이스 의존 0 인 platform crate 하나에 모은다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-09-26 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 1) + 인터페이스 모양 · 「프로세스 그룹」 핸들 · 게이트 꼴은 메인 판단(사용자 위임) + 사용자 결정 2026-10-02 (로그 때문에 base 를 끌지 않는다 — 「거부한 대안」 둘째 항목) + 사용자 결정 2026-10-03 (platform 의 `tracing` 직접 의존 — 결정 9) + 원자적 쓰기 거처는 메인 판단 2026-10-03 (결정 8) + 현황 실측 master `a226f63` · 재실측 `d5ac725`) · 부분 폐기 by ADR-0275 (결정 6과 열린 것과 영향 불변식의 시한부 예외) · 부분 폐기 by ADR-0291 (결정 8 원자적 쓰기 거처와 둘째 항목 사유 및 사본 목록)
- 관련: Amends ADR-0175 (결정 1의 PID 헬퍼 거처와 결정 2의 Job Object 래퍼 거처와 거부한 대안 첫 항목 중 platform 쪽) · Amends ADR-0218 (결정 11의 플랫폼 질의 거처) · Amends ADR-0230 (결정 1의 OS 분기 자리를 crate 하나로 좁힘) · ADR-0262(`ProcessGroup` 중립 손잡이 · `platform/windows.rs` 잎 — 통째로 옮길 대상) · ADR-0151(crate 판정 기준) · ADR-0175 결정 6(lib 무게) · ADR-0004(백엔드 지식 격리) · ADR-0001(kill 인과) · ADR-0269(base 의 셋째 입주자 되열 조건에 답한다 · 원자적 쓰기는 base `file` 에서 빠져 여기로 — 결정 8) · ADR-0268(로그는 각 crate 가 `tracing` 을 직접 부른다 — 결정 9) · `src-tauri/src/fsutil.rs`(`write_atomic`) · `crates/engram-dashboard-base/src/platform.rs` · `crates/engram-dashboard-agent/src/platform/` · `crates/engram-dashboard-discovery/src/lib.rs`(`wmi_spawn` · `wmi_create_raw`) · `docs/refactoring/architecture-discussion-2026-09-26.md` §1 · step-log S21 · Amends ADR-0265 (결정 4의 원자적 쓰기 함수 자리) · Amended by ADR-0275 (결정 6과 열린 것과 영향 불변식의 시한부 예외) · Amended by ADR-0291 (결정 8 원자적 쓰기 거처와 둘째 항목 사유 및 사본 목록)

## 맥락

OS 에 따라 달라지는 코드가 여러 crate 에 흩어져 있다(실측 master `a226f63`, 테스트 전용 분기 제외).

| 무엇 | 지금 위치 |
|---|---|
| Windows API 래퍼 — PID 생존 · 생성 시각 · 자식 PID · Job Object · 파일을 연 프로세스 찾기 · 프로세스 트리 · WMI 로 터미널 트리 밖에 띄우기 | `base/src/platform.rs` · `agent/src/platform/` · `discovery/src/lib.rs`(`wmi_spawn` · `wmi_create_raw`) |
| OS 규칙 — `.exe` 붙이기 · 기본 셸 · 홈 디렉터리 · CLI 를 `cmd.exe /c` 로 감싸기 · `PATH` 대소문자 무시 | `discovery/src/lib.rs`(데몬 실행 파일 이름) · `daemon/src/lib.rs`(CLI 실행 파일 이름) · `agent/src/manager.rs`(`default_shell`) · `agent/src/backend/claude/mod.rs` · `agent/src/backend/mod.rs`(`console_command` · PATH 합치기) |
| 부르는 쪽에 샌 분기 — Job Object 핸들을 `#[cfg(windows)]` 로 들고 다님 | `agent/src/transport/pty.rs` · `agent/src/transport/stdio.rs` · `agent/src/backend/codex/transport.rs` |

PTY 자체는 `portable-pty` 가 이미 감춘다.

이 배치는 옛 기준의 결과다. ADR-0175 는 PID 헬퍼를 base 로 내리고 Job Object 래퍼는 「소비자가 `agent` 안뿐이라」 남겼다(결정 2). ADR-0218 은 `file_holders` · `process_tree` 를 「소비자가 하나뿐이라 입주 조건 ①을 못 채운다」로 agent 에 두었다(결정 11). platform 을 독립 crate 로 떼는 안은 ADR-0175 가 사용자 결정으로 기각했다(거부한 대안 첫 항목 — *"기능 추가할 때마다 모듈 만들 순 없다. 과해지면 그때 분리하자."*). 셋 다 기준이 **소비자 수**였다.

사용자(2026-09-26): 「인터페이스 호출로 하고 플랫폼적인 건 다 감추고」 · 「어차피 플랫폼 전용 기능은 의존 0」 · 「애초에 초반부터 잡고 갔어야 됐어」.

## 결정

1. **OS 에 따라 달라지는 코드를 워크스페이스 의존 0 인 독립 crate 하나로 모은다**(사용자 2026-09-26). 부르는 쪽은 인터페이스만 부르고 OS 분기(`#[cfg]`)는 그 crate 안에만 있다. 이미 OS 코드에 연결된 곳도 전부 이쪽으로 옮긴다 — 위 「맥락」 표의 세 줄과 아래 「근거」의 재실측분.
2. **판정 기준 = OS 의존 여부.** 소비자 수(ADR-0175 결정 2 · ADR-0218 결정 11)를 대신한다.
3. **도메인 지식은 남긴다.** 예: claude 설정 폴더는 「홈 디렉터리 찾기」만 옮기고 `.claude` 경로 지식은 backend 에 둔다(CLAUDE.md 「백엔드 확장」). 락 디렉터리 위치와 「파일 이름 = 스레드 id」 지식은 그대로 `backend/codex` 다(ADR-0218 결정 11 의 그 절반은 유지). 테스트를 Windows 에서만 돌리는 분기는 테스트 쪽에 남긴다.
4. **`logging` 은 base 에 그대로 둔다. base 의 역할 = 어디서든 쓰는 잎**(사용자 2026-09-26). 이 결정으로 base 에서 나가는 것은 PID 헬퍼(`platform` 모듈)뿐이다.
5. **인터페이스 모양(메인 판단 — 사용자 위임):** 함수 + 핸들 타입, 컴파일 시점 분기. 테스트용 가짜가 필요한 곳은 **쓰는 쪽이** 작은 트레이트를 둔다(discovery 의 주입 seam 이 선례).
6. **「프로세스 그룹」 핸들 = ADR-0262 의 `ProcessGroup` 과 Job Object 래퍼를 한 덩이로 옮긴 것이다**(메인 판단). 새 손잡이를 따로 짓지 않는다. pty · stdio · codex transport 가 `#[cfg(windows)]` 로 들고 다니는 Job 핸들은 이 손잡이 뒤로 들어간다. 근거 = 그 두 파일이 이미 그렇게 옮기도록 짜여 있다 — `platform/windows.rs` 를 잎으로 두고 중립 타입으로의 변환을 `process_group.rs` 가 하는 배치가 사용자 결정(2026-09-29 「windows crate 와 io 만 · 통째로 옮길 수 있게」)이다(ADR-0262 「[구현] 고른 것」).
   - ★**옮겨도 소유는 그대로다 — Job 의 강한 주인은 통로(transport)에 남는다**★(ADR-0262 영향 「소유권」 · CLAUDE.md 「핵심 불변식」 소유권 분할). 이 손잡이로 밖에 나가는 것은 약한 손잡이(`Weak<JobObjectHandle>` — `platform/process_group.rs:118-119`) · 읽기 전용 `RetiringSignal` · 한 번 붙이는 포트뿐이다. 옮기는 것은 타입의 거처이지 강한 참조의 주인이 아니다. kill 인과(ADR-0001 — `transport.shutdown()` 이 Job 을 끝낸다)는 바뀌지 않는다.
7. **다른 OS 구현은 이 분리로 생기지 않는다.** 빈 곳이 한 자리에 모여 보일 뿐이다. Windows 밖의 PID 판정이 자리채움(PID 가 0 만 아니면 살아 있다)인 것도 그대로 옮긴다.
8. **원자적 파일 쓰기(`write_atomic`)는 platform 으로 간다**(메인 판단 2026-10-03). 셸 `src-tauri/src/fsutil.rs` 의 `write_atomic` 은 rename 이 공유 위반 · 잠금 위반으로 실패하면 다시 시도하는 Windows 전용 분기(`cfg!(windows)` · `raw_os_error` 32/33 — `fsutil.rs:107`)를 품는다. base 는 platform 을 의존하지 않는다(ADR-0175 입주 조건 ② — 워크스페이스 의존 0 은 그대로) — 그래서 이 함수는 base `file`(ADR-0269)이 아니라 여기 둔다.
   - **여러 벌의 동작을 하나로 정하는 일도 이 단계(작업 순서 1-3)로 온다** — ADR-0269 가 1-1 에 두었던 것이다. platform 이 생긴 뒤에 합친다. 현재 사본 = 셸 `src-tauri/src/fsutil.rs`(`settings/store.rs` · `ui_settings.rs` 가 부른다) · daemon `usage_service/reject_store.rs` · agent `persistence/mod.rs` · `persistence/presets.rs`(재실측 `d5ac725` — ADR-0269 「영향」 목록). 합치면 ADR-0265 결정 4 의 「`write_atomic` 은 셸 공용 `fsutil.rs`」가 낡는다.
   - **손상 사본 치우기(`set_aside_corrupt`)는 base `file` 에 남는다** — rename 하나로 OS 분기가 없다(ADR-0269).
9. **platform 은 서드파티 `tracing`(facade)을 직접 의존할 수 있다**(사용자 2026-10-03). 워크스페이스 crate 의존 0 은 그대로다 — 아래 게이트 ①은 `engram-dashboard` 접두 crate 만 센다. 그래서 `RmSession` 의 `Drop` 경고(`crates/engram-dashboard-agent/src/platform/file_holders.rs` 의 `impl Drop for RmSession` — 「`Drop` 이라 돌려보낼 곳이 없으므로 남기는 것은 로그뿐」)는 `tracing::warn!` 그대로 옮겨 간다. platform → base 간선은 없다(ADR-0268 — 로그는 감싸지 않는다).

## 거부한 대안

- **소비자 수로 거처를 정한다(현행 — ADR-0175 결정 2 · ADR-0218 결정 11).** 기각 = 사용자 결정(「애초에 초반부터 잡고 갔어야 됐어」 · 「플랫폼적인 건 다 감추고」). 현황은 OS 코드가 여러 crate 에 갈려 있고 부르는 쪽에 `#[cfg(windows)]` 핸들이 샌 모양이다(위 「맥락」 표 — 코드). ★ADR-0175 가 이 대안의 반대편(platform 독립 crate)을 기각할 때 든 근거도 사용자 판단이었다(그 ADR 자평: 약함)★ — 이번 결정은 같은 축의 사용자 판단을 뒤집은 것이다.
- **platform 이 로그를 찍으려고 base 를 의존한다.** 기각 = 사용자 결정(2026-10-02 — 「로그 때문에 Base 전체를 그렇게 포함시키는게 말이 안 됨」). 로그는 facade(`tracing`) 하나로 찍힌다(결정 9 · ADR-0268).
- **platform 은 로그를 안 찍는다 — 실패는 결과값으로, 정리는 명시 `end() -> Result` + 조용한 `Drop`**(Rust API Guidelines C-DTOR-FAIL · std · wezterm `filedescriptor` 같은 가장 낮은 핸들 층의 형 — `docs/research/low-level-crate-logging-2026-10-02.md` 갈래 A). 기각 = 사용자 결정(2026-10-03 — 「로그 감싸지 마」로 facade 직접 의존이 열려 결정 9 가 섰다) + 그 형의 대가: `holders_of` 처럼 세션을 연 뒤 출구가 여럿인 함수(`return Err` 둘 · 루프 안의 이른 `return Ok` 하나 · 끝의 수렴 실패 `Err` — 넷)는 모든 경로에서 `end()` 를 불러야 실패가 보인다(코드 — `file_holders.rs` 의 `holders_of`). 한 층 위 OS 래퍼(portable-pty · alacritty · cargo `FileLock`)는 facade 로 찍는다(같은 보고서 §1 · §3).
- **트레이트 객체로 OS 를 고른다(런타임 분기).** 기각 = OS 는 실행 중에 안 바뀌어 런타임 분기가 얻는 게 없다(메인 판단). **기각 근거 자평: 약함**(서술).

## 근거

- **사용자 결정 2026-09-26** — 위 「맥락」 인용 셋. logging 잔류도 같은 날(「base 의 역할 = 어디서든 쓰는 잎」).
- **사용자 결정 2026-10-02** — 「로그 때문에 Base 전체를 그렇게 포함시키는게 말이 안 됨」(「거부한 대안」 둘째 항목).
- **사용자 결정 2026-10-03** — platform 의 `tracing` 직접 의존(결정 9). 근거 조사 = `docs/research/low-level-crate-logging-2026-10-02.md`(낮은 crate 가 facade 를 직접 의존하는 피어 관행 · 서드파티 facade 는 워크스페이스 의존 0 을 깨지 않는다 — 그 보고서 「우리 제약과의 적합도」 갈래 C).
- **현황 실측(master `a226f63`)** — 「맥락」 표. 셸(`src-tauri/src`)에는 그 시점 OS 분기가 없었다.
- ★**재실측(2026-10-02 · `d5ac725`) — 위 표보다 넓다**★(`rg -l 'cfg!?\(windows\)|cfg\(not\(windows\)\)' crates src-tauri/src` 뒤 각 파일의 `#[cfg(test)]` 시작 줄로 운영 분기와 테스트 분기를 갈랐다):
  - **운영 분기 — 옮길 것에 더한다:** agent `platform/process_group.rs`(ADR-0262 중립 손잡이) · agent `usage/process.rs`(사용량 조회 자식 띄우기 · Job · 스레드 스냅숏) · agent `backend/codex/thread_lock.rs`(홈 디렉터리 찾기 — `backend/claude/mod.rs` 와 같은 OS 규칙의 둘째 사본) · net `src/instance.rs`(단일 인스턴스 잠금의 공유 모드 열기) · 셸 `src-tauri/src/fsutil.rs`(rename 재시도가 Windows 공유 위반 코드를 본다 — 함수 `write_atomic` 째 옮긴다, 결정 8) · discovery `src/lib.rs` 의 `TaskKiller::kill`(`#[cfg(windows)]` 판이 `taskkill /PID … /F /T` 를 부르고 그 밖 판은 「Windows 전용」 오류 — `lib.rs:914-931`). discovery 는 ADR-0271 로 나뉘므로(프로세스 끄기 = 셸 몫 — 그 결정 3) 이 분기도 platform 으로 옮긴다.
  - **테스트 분기뿐 — 결정 3 대로 남긴다:** agent `usage/scratch.rs` · `backend/claude/leftover.rs` · `backend/claude/usage_probe.rs` · `backend/codex/mod.rs` · `session.rs` · daemon `messaging_host.rs` · net `src/portfile.rs` · base `logging/mod.rs`(각 파일의 분기가 전부 그 파일 `#[cfg(test)]` 아래에 있다).
  - **시험 하네스 bin · `examples/` — 테스트와 같은 규칙으로 남긴다(메인 판단):** daemon `src/bin/roundtrip_smoke.rs`(CLI 실행 파일 이름에 `.exe` 붙이기 — `#[cfg(test)]` 위 운영 본문) · `src/bin/saturation_pilot.rs`(`claude --version` 을 `cmd /c` 로 감싸기). 둘 다 `required-features = ["test-harness"]` 라 운영 · 릴리스 빌드가 컴파일하지 않는다(데몬 `Cargo.toml`). agent `examples/`(`spike.rs` · `spike_breakaway.rs`)도 같다. 결정 3 의 테스트 예외를 이 둘로 넓혀 읽는 것이고, 운영 빌드에 들어가지 않는다는 점이 그 근거다.
  - 착수 때 같은 명령으로 다시 센다 — 1-3 전에 다른 단계가 코드를 옮긴다.

## 영향 / 불변식

- **불변식: `#[cfg(windows)]` · `cfg!(windows)` 는 platform crate 안에만 있다**(테스트 전용 분기 제외 — 시험 하네스 bin · `examples/` 도 같다, 「근거」 재실측). ADR-0230 결정 1(「맡은 함수 · 모듈 하나 안에서만」)이 이 crate 하나로 좁아진다. CLAUDE.md 「플랫폼 중립」의 선례 `console_command` 도 이 crate 로 간다.
- **게이트(메인 판단) — base 와 같은 꼴 둘:** ① 의존 상한 `cargo tree -p <platform> --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` → 정확히 1줄(자기 자신 — 서드파티 crate(`windows` · `tracing` facade)는 세지 않는다 · 결정 9) ② `rg "^\s*use tauri" crates/<platform>/src/` → 0줄(경로 존재부터 확인 — CLAUDE.md 「빌드·검증 명령」 코어 격리 게이트 항목). 새 crate 는 게이트를 세 곳(`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`)에 등록하고, 이름 접두 `engram-dashboard-` 를 지켜야 상한 게이트가 본다(ADR-0175 영향 · ADR-0151 「개명 함정」). 실 자식 프로세스를 띄우는 테스트(`child_pids` 등)가 따라오므로 그 crate 의 테스트 명령은 `-- --test-threads=4` 를 붙인다(CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」).
- **바뀌는 게이트:** net 의 base 심볼 allowlist(정확히 2줄 — PID 헬퍼 둘)가 platform crate 로 갈아탄다 · base 입주자 상호 참조 게이트(`rg "(crate|super)::(logging|platform)"`)에서 `platform` 이 빠진다 · `windows` crate feature 목록(CLAUDE.md 「의존성」)이 crate 별로 옮겨 간다.
- ★**kill 인과(ADR-0001)가 Job Object 를 지난다** — 그 래퍼를 옮기는 단계는 full QA 다(작업 순서 1-3).
- **작업은 망가지지 않는 단위로 묶는다**(사용자 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」). 1-3 안에서 단위마다 빌드 · 회귀가 초록인 채로 끊는다.
- **ADR-0175 「거부한 대안」 첫 항목 중 platform 쪽은 이 결정이 뒤집는다**(platform 은 독립 crate) — logging 쪽은 결정 4 대로 base 에 남는다. 그 항목의 되열 조건(「base 에 세 번째 입주자」)에는 ADR-0269 이 답한다.
- **CLAUDE.md 「백엔드 모듈 맵」 base · agent 항목을 고친다** — agent 항목의 「남은 `platform`은 Job Object 래퍼 하나뿐」은 이미 낡았다(지금 넷 — Job Object 래퍼 · `process_group` · `file_holders` · `process_tree`).
- **열린 것 — 1-3 착수 때 정한다:** `windows` crate feature 를 기능별 cargo feature 로 나눌지(`net` 은 PID 판정만 필요한데 Job Object · Restart Manager 바인딩까지 끌려온다).
- **코드 앵커 = `// ADR-0266`** — platform crate 헤더 · 「프로세스 그룹」 핸들.
