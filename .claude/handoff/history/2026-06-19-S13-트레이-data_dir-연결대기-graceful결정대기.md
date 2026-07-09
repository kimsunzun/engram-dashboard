# 핸드오프 — Engram Dashboard: S13 sub-step 1(트레이 격리) 완료 + sub-step 2 토대(discovery 분리·data_dir 로컬화) 완료, 트레이 실제 연결 + graceful 끄기 순서 결정 대기

작성 2026-06-19 (dashboard7 세션). 직전 핸드오프(`2026-06-18-S13-트레이-데몬-토폴로지-설계완료-단계1대기.md`) 후속. 본문(`docs/decisions/`·`docs/process/`·`CLAUDE.md`)이 항상 우선. **master HEAD=`00ad8c5`, working tree 깨끗(`.ccb/`만 untracked). origin push 아직 안 함 — 사용자 확인 후.**

## ★★ 다음 세션 첫 행동 (필독) ★★
1. **읽기:** 이 핸드오프 → `docs/decisions/0023~0025`(특히 0024 "제어 허락 모델" + 0025) → `docs/process/S13-tray-lifecycle/tray-topology-trd.md` §8 → 아래 "§3 대기 중인 결정"·"§4 다음 조각 스펙".
2. **만지는 영역 ADR을 반드시 먼저 깔 것.** ★이번 세션 사고: 트레이 graceful 끄기가 ADR-0024에 "WS+토큰으로 graceful StopDaemon, taskkill 폴백"으로 이미 박혀 있었는데, 그걸 놓치고 "트레이는 graceful 못 한다 → taskkill"로 거꾸로 제안했다가 사용자가 잡음.★ ADR이 막으려던 재론을 메인이 함. 코드 만지기 전 관련 ADR 재독.
3. **§3 결정(graceful 끄기 순서 a/b)을 사용자에게 받고**, 그 뒤 구현(코더→reviewer-deep→QA, 구현 실행 규약 강제). 코더 직전 사용자 보고.

## 0. 한 줄 요약
S13 트레이/데몬 구현 진행. **sub-step 1(순수-Rust 트레이 격리 crate) 완료·커밋.** **sub-step 2 토대 2조각 완료·커밋**: ① discovery(데몬 발견/spawn/stop) 공유 crate 분리, ② data_dir을 `%APPDATA%`→로컬 `.engram-data/`로 이전. **트레이를 실제 데몬에 연결하는 조각은 미착수** — graceful 끄기 구현 순서(a/b)만 사용자 결정 받으면 바로 코더.

## 1. 이번 세션 커밋 (master, HEAD=00ad8c5, 미push)
| 커밋 | 내용 |
|---|---|
| `a21d448` | **S13 sub-step 1 — engram-tray-host crate(순수 Rust, WebView 없음).** core(순수 로직, tao/tray_icon/image/windows import 0 — 단위테스트 15) + main.rs(tao 이벤트 루프 + tray-icon shell). seam=DaemonProbe/Launcher(둘 다 stub). 메뉴 6개(데몬 켜기/끄기·UI 열기/닫기·트레이 종료·완전 종료), 상태=회색 아이콘(컬러=활성/회색=비활성), 툴팁="Engram". 버그 2건 수정: 아이콘 미표시(tray-icon 0.24.1은 `StartCause::Init` 시점 생성해야 등록됨) + 시작 실패(Common Controls v6 매니페스트 없음→build.rs+embed-manifest 임베드). `windows_subsystem="windows"`(콘솔 없음). |
| `ab45cc5` | **discovery 공유 crate 분리.** `src-tauri/src/discovery.rs`(데몬 발견/spawn/stop 순수 로직, WMI) → 신규 `engram-dashboard-discovery`(git mv, 바이트 동일·회귀 0). tauri 래퍼 `commands/discovery.rs`는 잔류, lib.rs re-export로 호출부 무수정. |
| `9eb1229` | **data_dir `%APPDATA%`→로컬 `.engram-data/` 이전.** 단일 출처 `discovery::default_data_dir()`: ① `ENGRAM_DATA_DIR` override(테스트 격리 전용) > ② 디버그=current_exe walk-up→repo 루트(`.git`/`[workspace]`)의 `.engram-data/` > ③ 릴리즈=exe 폴더 옆 > cwd fallback. daemon resolve_data_dir·embedded(app_data_dir 4곳+FileProfileStore) 전부 이 함수로 교체. reviewer-deep 6건 수정(ws_e2e 격리 복원·WMI smoke 경로·릴리즈 헬퍼+테스트·미사용 _app 제거). |
| `00ad8c5` | **CLAUDE.md 컨벤션** — "숨은 의도·불변식은 코드에 자세히 주석(시그니처로 설명 안 되는 load-bearing 의미)". ENGRAM_DATA_DIR 오해 사고를 교훈으로 박음. |

트레일러 `Co-Authored-By: Claude Opus 4.8 (1M context)`. author=repo local user(건드리지 말 것).

## 2. 현재 위치 (S13 구현 로드맵 4단계 中)
- **sub-step 1** 트레이 격리 crate(stub) ✅ 완료.
- **sub-step 2(연결)** 진행: 토대 ✅(discovery 분리 + data_dir 로컬화). **트레이 실제 배선 ⏳ 미착수**(다음).
- sub-step 3(데몬 견고화: lockfile generation·idle shutdown·정밀 graceful C4) / sub-step 4(lifecycle command 표면=ADR-0022) — 그대로.

## 3. ★대기 중인 결정 (다음 세션 시작점)★
**트레이 "데몬 끄기/완전 종료"의 graceful 구현 순서.** 끄기 메커니즘 자체는 **이미 결정됨**(ADR-0024 제어 허락 모델 + TRD §8): 트레이가 lockfile의 토큰·포트로 데몬 WS에 인증 접속 → `StopDaemon` graceful 전송 → 데몬이 자식(PTY) 정리 후 self-exit, **유예 타임아웃 후 taskkill 폴백**. (taskkill 단독은 ADR 위반 — 메인이 한 번 헷갈렸음, 재발 금지.)

남은 건 **순서뿐**(트레이엔 아직 WS/tokio 스택 없음 → graceful 보내려면 작은 "one-shot 접속기"가 필요):
- **(a)** 다음 조각에 one-shot 접속기까지 넣어 graceful 끄기 완성(조각 큼).
- **(b, 메인 추천)** 다음 조각=켜기+상태→아이콘만(접속기 불필요), graceful 끄기는 one-shot 접속기와 함께 그 다음 조각(작게).

**사용자에게 a/b 물어보고 진행.**

## 4. 다음 조각 스펙 (트레이 실제 데몬 연결)
**목표:** 트레이 stub Launcher/Probe → 실제 데몬 제어. "트레이=진짜 엔진 스위치" 첫 동작.

**(b 채택 시) 이번 범위 = 데몬 켜기 + 상태→아이콘:**
- `RealProbe.is_alive()` = `discovery::daemon_status(&data_dir).alive`. data_dir = `discovery::default_data_dir()`(시작 시 1회).
- `RealLauncher.ensure_daemon()` = `discovery::ensure_daemon(&data_dir, &exe, 5s, false)`(exe=`locate_daemon_exe()`). ★반드시 discovery(WMI)로만 spawn — `std::process::Command` 데몬 직접 spawn 금지(ADR-0024 C1 detached).★
- **비동기 필수:** 메뉴 클릭→워커 스레드에서 discovery 호출→완료시 `EventLoopProxy::send_event`로 메인 회수→probe 재확인→`TrayIcon::set_icon`(컬러/회색). tao 메인 루프 블록 금지.
- 초기 아이콘: Init 때 probe 1회.
- **stop_daemon/open_ui/close_ui/shutdown_all 의 끄기 부분 = graceful 조각으로(§3 결정 후).** 이번엔 stop은 stub 유지하거나 a/b에 따라.
- **주의: 중단된 코더가 tray-host Cargo.toml에 discovery 의존만 추가했다가 되돌림(미커밋 정리됨).** 실제 배선 시 그 의존 다시 추가(windows 전용, core는 import 안 함 — 격리 유지).

**확정된 자잘한 결정(코드 마커+여기 기록):**
- **아이콘 갱신 = 버튼 액션 직후만**(2026-06-19 사용자). 주기 폴링(b안: WaitUntil N초마다 probe→갱신)은 후속 — 외부에서 데몬 죽으면 다음 액션/트레이 재시작 전엔 아이콘 안 따라감(의도적 보류).
- **켜기/끄기 실패 시 사용자 피드백(토스트/풍선) = 후속.** 지금은 조용히 `tracing::error!`.
- **범위:** UI 열기/닫기(앱 spawn)·완전종료 UI부분·clientFactory 기본 daemon flip = 그 다음 조각들.

## 5. 이번 세션 설계 결정/기록
- **ADR-0025 신규** — UI 부팅 1회 데몬 ensure 유지(ADR-0024 C3 "UI ensure 금지" 폐기). 근거: "UI 켜졌는데 데몬 없음"은 사용자 의도 모순. 자동은 부팅 1회뿐(사망 시 자동부활 없음). 데몬 싱글톤이라 tray·UI 양쪽 ensure 안전.
- **ADR-0024 갱신** — C3줄 폐기 표기(Superseded by 0025) + 데이터위치 갱신(주입 대신 self-resolve `default_data_dir`, env=테스트격리, **배포 시 appdata는 추후 메모**).
- **ADR-0023 갱신** — 메뉴 v1 4개→**6개 확정**(데몬 켜기/끄기·UI 열기/닫기·트레이 종료·완전 종료). "트레이 종료=트레이만(데몬·UI는 detached 생존, 다음 시작 시 재발견)" vs "완전 종료=전부 graceful 후 트레이". README 인덱스·step-log 갱신.
- **data_dir 결정 경위(사용자):** appdata 아닌 로컬(팀원 머신 안 더럽힘) → 상대경로 검토 → "어디서 띄워도 한 폴더"라 **exe walk-up 채택**(데몬 cwd 불신). **디버그=repo 공유 / 릴리즈=exe 옆**(테스트 시 디버그/릴리즈 폴더 분리). `ENGRAM_DATA_DIR`은 "배포 노브"로 오해해 제거했다 **테스트 격리 수단**임이 reviewer-deep 적출로 복원.

## 6. prior-art 재확인 요약 (graceful 끄기 — ADR-0024 모델이 맞음을 재확인)
`/prior-art` 3에이전트(Docker/Ollama/Tailscale · Discord/Steam/OneDrive · LSP/systemd/표준) 조사 결과 = ADR-0024 결정 재확인 + 디테일 보강:
- **정설 = 데몬이 control 채널 노출 + 여러 클라(CLI·GUI·트레이) 같은 진입점으로 붙기**(Tailscale LocalAPI·Consul·Docker). 트레이 직접 kill은 종료 경로 2개로 갈라져 §5 단일 제어표면 위반. → "트레이도 StopDaemon 같은 채널로"가 맞음.
- **graceful은 "타임아웃+강제 폴백"과 한 쌍**(LSP shutdown/exit + 클라 timeout→kill; systemd SIGTERM→TimeoutStopSec→SIGKILL; launchd 20s; SCM 30s+wait-hint 연장). ADR-0024 C4와 이미 일치. ADR-0001 join_pump 5s와 정합되게 유예 잡기.
- **보안:** loopback+토큰이 교과서(loopback 멀티유저 약점을 토큰이 메움). 트레이도 lockfile 토큰 공유. 강제 kill 폴백은 토큰 없이도 동작(보안 실패 시에도 종료 보장).
- **graceful-with-live-work:** 데몬이 PTY 자식 보유 → graceful이 각 세션 kill 인과(transport.shutdown→join_pump, ADR-0001) 적용 후 self-exit. 강제 폴백 시 Job(KILL_ON_JOB_CLOSE)이 트리 정리. **데몬이 자식 보유라 graceful이 더 중요**(직접 kill하면 finalize·done 통지 순서 생략됨).
- 차용 후보: **Tailscale**(BSD-3, `client/tailscale/localclient.go`·`safesocket`)가 우리 토폴로지에 가장 정합·라이선스 자유. (패턴 차용이지 복붙 아님.)

## 7. 환경/주의
- **Codex 2번째 리뷰어 — 공식 플러그인 확정·CLI 설치 완료(2026-06-19):** `openai/codex-plugin-cc`(공식, Claude Code **네이티브 플러그인**, MCP 아님). `/codex:review`(read-only)·`/codex:adversarial-review`·`/codex:rescue`·`/codex:status`/`result`/`cancel` 제공 + `codex:codex-rescue` 서브에이전트. **Codex CLI 설치함(`codex-cli 0.141.0`, npm -g).** 전제: Node 18.18+(있음 v24) + ChatGPT 구독(**Free 포함** — Pro면 한도↑) 또는 API 키.
  - **남은 건 사용자 대화형 단계(에이전트 실행 불가):** ① `/plugin marketplace add openai/codex-plugin-cc` ② `/plugin install codex@openai-codex` ③ `/reload-plugins` ④ `/codex:setup`(Codex ready 확인) ⑤ `!codex login`(ChatGPT 로그인 — 사용자 계정/구독). 끝나면 `/codex:review` + `/agents`에 codex-rescue 보임.
  - **이후 검증 게이트 = reviewer-deep(Claude) + `/codex:review`(Codex) 2인.** 다음 조각(트레이 실제 배선)부터 적용.
  - (옛 비공식 `ask codex` CCB 브리지는 활성 세션 필요 — 공식 플러그인으로 대체 권장.)
- **data_dir:** 디버그 = `<repo>/.engram-data/`(=`I:\Engram\apps\engram-dashboard\.engram-data\`). `ENGRAM_DATA_DIR`=테스트 격리 override(직접-spawn만 상속, **WMI-spawn 데몬은 env 미상속**). `.engram-data/` gitignore됨.
- **빌드 잠금(os error 5):** 살아있는 engram 프로세스가 exe 잡음 → `taskkill //F //PID`. **kill은 권한 분류기에 막힐 수 있음 — 사용자에게 `! taskkill //F //PID <pid>` 요청/확인.**
- **검증 하네스:** `#[ignore]` 실프로세스 테스트(ws_e2e 3·discovery WMI smoke 2)는 게이트 밖. 수동(`-- --ignored`) 시 운영 `.engram-data` 건드릴 수 있음(ws_e2e는 ENGRAM_DATA_DIR로 격리, WMI smoke는 default 경로+백업/복원).
- 서브에이전트: 코더(opus)·reviewer-deep·QA 분리 스폰(구현 실행 규약). 이번 세션 전부 이 흐름.

## 8. 핵심 불변식 (변경 금지 — 근거 docs/decisions/)
- 직전 핸드오프 §7 + ADR-0001(kill 2동사)·0005(finalize 1회)·0007(epoch 재구독)·0020(단일 프로토콜+carrier)·0021(on-demand·무재시작)·0023·0024·0025 동일.
- **data_dir 단일 출처 = `discovery::default_data_dir()`** — daemon·embedded·tray-host가 같은 빌드모드에서 같은 폴더(어긋나면 두 모드가 다른 agents.json 봐서 데이터 갈라짐).
- **트레이 데몬 spawn = discovery(WMI)만**(detached, ADR-0024 C1). 직접 `Command::spawn` 금지.
- **트레이 끄기 = graceful StopDaemon(WS+토큰) → 유예 → taskkill 폴백.** taskkill 단독 금지(ADR-0024).
- **core(tray-host) 순수성:** core.rs에 tao/tray_icon/image/windows/discovery import 0.

## 9. 핸드오프 종료 체크리스트 (이번 세션)
1. 새 결정 → ADR-0025 작성 ✅ / ADR-0023·0024 갱신 ✅
2. 번복 → ADR-0024 C3 `폐기(Superseded by ADR-0025)` 표기 ✅
3. `docs/decisions/README.md` 인덱스 갱신(0025 추가) ✅
4. `docs/process/step-log.md` sub-step 1·2 항목 추가 ✅
5. working tree 정리(중단 코더의 tray-host dep 미커밋 되돌림) ✅
