# 핸드오프 — ADR-0026/0027 트레이 앱-통합 1·2단계 완료, 다음=스텝3(single-instance 폴더별 + mode-aware 트레이 + autostart + 데이터위치 모드분리)

작성 2026-06-21 (dashboard9 세션). 직전 핸드오프(`2026-06-19-S13-트레이-켜기끄기완료-UI열기대기.md`) 후속. 본문(`docs/decisions/`·`docs/process/`·`CLAUDE.md`)이 항상 우선. **master HEAD=`333cdfd`, working tree 깨끗(`.ccb/`만). push 안 함(이번 세션 4커밋 로컬만: `b55b0d3`·`2d082c3`·`7286ec2`·`333cdfd`).**

## ★★ 다음 세션 첫 행동 (필독) ★★
1. **읽기:** 이 핸드오프 → **ADR-0026**(트레이를 앱에 통합, 2프로세스 — ADR-0023 폐기) → **ADR-0027**(모드별 인스턴스 스코프+데이터 위치 — ADR-0024 데이터부분 폐기) → ADR-0024 나머지(detached/breakaway·C4)·0021·0025 → `docs/process/S13-tray-lifecycle/tray-app-merge-trd.md` → step-log dashboard9.
2. **CLAUDE.md 신규 규약 인지:** "조사·웹서칭·대량읽기도 서브에이전트 일임(컨텍스트 위생)" — 메인에서 직접 web search 하지 말 것.
3. **구현 실행 규약 그대로:** 비자명 변경 = 코더(opus)→reviewer-deep→QA(build/test+cdp). 게이트 통과 후 커밋.

## 0. 한 줄 요약
ADR-0023(별도 순수-Rust 트레이 3프로세스)을 **롤백** → ADR-0026(트레이를 Tauri 앱에 통합, 2프로세스). **1단계(crate 제거+로직 이관)·2단계(네이티브 트레이+메뉴+X=hide+command+아이콘 race수정) 완료·커밋.** + ADR-0027로 **모드별 인스턴스/데이터 위치 확정**. 다음 = **스텝3**(아래 §3).

## 1. 이번 세션 커밋 (master, push 안 함)
| 커밋 | 내용 |
|---|---|
| `b55b0d3` | **ADR-0026** — 트레이를 앱에 통합(2프로세스), ADR-0023 폐기. `/consult` 3종 전원 롤백 찬성(Gemini의 KILL_ON_JOB_CLOSE=불변식 위반 폐기, GPT의 "네이티브 트레이≠cdp DOM" 보정 채택). TRD·step-log. |
| `2d082c3` | **1단계** — `engram-tray-host` crate 외과적 제거. 순수 로직(MenuAction/IconState/to_grayscale_rgba+테스트)을 `src-tauri/src/tray/core.rs`로 이관. workspace member 제거. 동작 변경 0. |
| `7286ec2` | **2단계** — Tauri 네이티브 트레이 배선. 메뉴 5개(StartDaemon/StopDaemon/ShowUi/HideUi/QuitApp), X=hide(prevent_close+hide, main만), show/hide/quit command(§5, actions.rs 공유), 컬러/회색 아이콘(image-ico). **끄기 후 아이콘 컬러 고착 race 수정**(reviewer-deep Blocker → StopOutcome 분기 복원: DaemonClosed→회색 직접/probe우회, Timeout·NoTarget→probe폴백). |
| `333cdfd` | **ADR-0027** — 모드별 인스턴스 스코프+데이터 위치(ADR-0024 데이터부분 폐기) + CLAUDE.md 서브에이전트 일임 규약. |

## 2. 현재 동작 상태 (실측/주의)
- **현재 기본 = embedded 모드**(`src/api/clientFactory.ts` `resolveMode` 기본 'embedded'). 에이전트가 앱 Rust 안 in-process. **데몬은 에이전트 호스트 아님**(flip 전). 트레이 아이콘 회색=데몬 프로세스 안 떠있음(정상).
- **GUI 수동검증 미완(사용자 테스트 중단됨):** 트레이 아이콘 색(컬러/회색)·OS 타이틀바 X→hide·트레이 메뉴 클릭은 cdp가 못 봐 **사용자 수동 확인 필요**. cdp로는 창 show/hide·트레이 실재·quit 검증됨. (테스트: `npm run tauri dev` 또는 `target/debug/engram-dashboard.exe`+vite. 트레이 끄기→회색, X→작업표시줄에서 사라짐 확인.)
- **현재 single-instance 無** → 같은 폴더에서 2번 실행하면 앱 2개가 같은 `.engram-data/agents.json` 복원 충돌(스텝3가 막음).

## 3. ★다음 작업 = 스텝3 (ADR-0026 §spike + ADR-0027 구현)★
1. **single-instance (embedded=폴더별, ADR-0027):** `tauri-plugin-single-instance` 추가(**플러그인 중 가장 먼저 등록**). 락 키 = `{identifier}+hash(작업폴더)`(폴더별 분할 — Tauri 기본은 전역 `{identifier}-sim`이라 키 커스텀 필요). 같은 폴더 2nd 실행 → 콜백 `|app,argv,cwd|`에서 main 창 `show()`→`unminimize()`→`set_focus()`(★Windows focus-stealing: set_focus만으론 작업표시줄 깜빡임만 — 안 되면 일시 always_on_top 토글). 다른 폴더 → 신규 인스턴스 허용.
2. **데이터 위치 모드분리 (ADR-0027, #4 해소됨):** `discovery::default_data_dir()` 모드 분기는 **release에서만** 필요. ① `ENGRAM_DATA_DIR`(격리, 불변) ② **dev(debug) = 두 모드 모두 싱글(embedded) 데이터 = 현 debug walk-up `.engram-data` 그대로**(daemon-dev도 동일 — 모드 스위칭 테스트). ③ **release만**: embedded=실행(작업) 폴더 / daemon=`%APPDATA%\com.engram.dashboard`. → dev는 기존 동작 유지라 구현 단순.
3. **★트레이/X=hide = daemon 모드 전용 (B안 확정)★:** **embedded = 평범한 창 앱**(트레이 없음, X=일반 닫기/종료, 재오픈 cold restore). **daemon = 트레이 상주**(전역 1개, X=hide). 현 스텝2는 트레이·X=hide를 **무조건** 켜므로 → **embedded 모드에선 트레이 미생성 + X=일반 닫기로 mode-gate**. (embedded엔 데몬 메뉴 orphaned·트레이 클러터 문제 자체가 사라짐.)
4. **autostart 토글:** `tauri-plugin-autostart`. **옵션(기본 OFF)**, 트레이 체크 항목 + **command 노출(§5 — LLM도 켜고/끔)**. 부팅 기동 시 `--hidden`(창 없이 트레이만) — 부팅 vs 사용자클릭 기동 인자 구분.
5. **spike 4종(ADR-0026, 구현 전/중 실측):** ① WebView2 hidden 메모리, ② hidden 창 raise(focus stealing), ③ updater 재시작 중 데몬 reconnect, ④ 앱 크래시 후 데몬 재발견.

## 4. 핵심 불변식 (변경 금지 — 근거 docs/decisions/)
- **데몬 spawn = discovery(WMI)만**(detached/breakaway, ADR-0024 C1). `std::process::Command`로 데몬 직접 spawn 금지(앱 Job 상속→동반 사살). ★consult가 적출한 Gemini의 KILL_ON_JOB_CLOSE 동반종료는 폐기된 안 — 절대 도입 금지.
- **트레이 = Rust 층(네이티브), React 아님.** 트레이 아이콘 갱신은 Rust가 `tray.set_icon` 직접(메인 스레드 — `run_on_main_thread`). 끄기 후 아이콘: **StopOutcome::DaemonClosed=연결닫힘=꺼짐확정→probe우회 회색**, Timeout/NoTarget→probe폴백(`tray/mod.rs` `icon_state_for_stop_outcome`). 이 분기 단순화로 지우지 말 것(race 재발).
- **§5:** 트레이 액션=Tauri command(actions.rs 공유, 중복0). 네이티브 트레이 팝업은 cdp DOM 아님 → cdp 검증은 command invoke로.
- **로컬 제어 평면(창/트레이/데몬 lifecycle) vs 원격 데이터 평면(WS) 분리**(ADR-0026). 로컬 제어를 원격 데몬으로 라우팅 금지.
- core.rs(`src-tauri/src/tray/core.rs`) 순수성 — tauri/discovery import 0.
- **데이터 모드분리(ADR-0027):** embedded=폴더-로컬, daemon=유저-global(release). 에이전트 모드 간 비공유(의도). ENGRAM_DATA_DIR 격리 유지.

## 5. 모듈 맵 (이번 세션 신규)
- `src-tauri/src/tray/core.rs` — 순수 로직(MenuAction 5개·IconState·icon_state_for·to_grayscale_rgba + 테스트).
- `src-tauri/src/tray/mod.rs` — Tauri 배선(build_tray·dispatch_menu·spawn_daemon_action·icon_state_for_stop_outcome).
- `src-tauri/src/tray/actions.rs` — 트레이/command 공유 부수효과(show/hide/quit·refresh_tray_icon·set_tray_icon_state).
- `src-tauri/src/commands/tray.rs` — show_main_ui/hide_main_ui/quit_app command.
- `src-tauri/Cargo.toml` — tauri features `tray-icon`·`image-ico` 추가.

## 6. 환경/주의
- **빌드 잠금(os error 5):** 살아있는 engram(앱/데몬) 프로세스가 exe 잡음 → 떠 있는 dev/exe 닫고 빌드. `taskkill //F //PID <pid>`(bash, `//` 주의 — 권한 분류기 막히면 사용자에게 `! taskkill`).
- **S13 끄기 변경 시:** `cargo test -p engram-dashboard-discovery --test stop_smoke -- --ignored --test-threads=1` 필수(분류 정확성 #[ignore] 회귀망).
- dev 흰화면 김 = Vite 콜드 로드(Monaco/xterm 즉석 변환) + restore_all + ts.worker.js optimizeDeps 경고(무해). release는 빠름.
- 서브에이전트: 코더(opus)·reviewer-deep·QA 분리. **조사/웹서칭도 서브에이전트 일임**(신규 규약).

## 7. 핸드오프 종료 체크리스트
1. 새 결정 → ADR-0026·0027 ✅
2. 번복 → ADR-0023 폐기(0026)·ADR-0024 데이터부분 폐기(0027) — *폐기당한* ADR에 표시 ✅
3. README 인덱스 갱신 ✅
4. step-log dashboard9 ✅
5. push 안 함(사용자 지시 대기)
6. working tree 깨끗(.ccb만) ✅
7. **잔여:** 스텝2 GUI 수동검증(트레이 색·X=hide) 미완 — 다음 세션 또는 사용자 확인.
