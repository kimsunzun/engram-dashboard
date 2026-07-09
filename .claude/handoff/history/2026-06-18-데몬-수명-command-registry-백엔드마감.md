# 핸드오프 — Engram Dashboard: 데몬 수명(ADR-0021) + command registry 방향(ADR-0022) + 백엔드 잔여

작성 2026-06-18. 직전 핸드오프(`2026-06-17-ADR0020-클라이언트-경로-통합.md`) 후속. 본문(`docs/decisions/`·`docs/process/step-log.md`·`CLAUDE.md`)이 항상 우선. **origin push 완료(master=`8c29ace`).**

## 0. 한 줄 요약
ADR-0020(클라이언트 통합) 완료에 이어 **ADR-0021 #1(데몬 수명: on-demand·자동재시작 없음·kill 가능·windowless) + fix A(main 닫으면 앱 종료) 완료·push.** **ADR-0022(통합 command registry = palette+키바인딩+LLM 단일 출처)를 제안으로 박제**(여파 최소화 북극성). 백엔드는 사실상 마감 — 잔여는 소소(아래 §5). **다음은 프론트.**

## 1. 이번 세션 커밋 (master, push 완료, HEAD=8c29ace)
| 커밋 | 내용 |
|---|---|
| `4e98c91` | ADR rot 방지 규약 + 인덱스/0016 폐기표기 + docs/README 진행표 단일화 |
| `d4ddcab`~`41a4a64`·`c8fca48` | **ADR-0020 클라이언트 경로 통합** 4단계(ConnectionCore 추출→Tauri 어댑터→ProtocolClient→옛경로 삭제) + step-log |
| `616a471` | ADR-0021 데몬 수명 (on-demand 모델) |
| `b03ffa7` | **ADR-0021 #1 구현** — 데몬 lifecycle command + ensure/reconnect 분리 + windowless |
| `664d629` | **fix A** — main 닫으면 앱 종료(hidden 창 좀비 잔존 수정) |
| `8c29ace` | **ADR-0022 통합 command registry (제안)** |

author=repo local user.email(개인계정, 건드리지 말 것). 트레일러 `Co-Authored-By: Claude Opus 4.8 (1M context)`.

## 2. ★ADR-0021: 데몬 수명 (확정·구현됨) — 재론 금지★
**모델: tmux/wezterm식 on-demand + 자동재시작 없음.** (systemd on-failure/desired-state는 거부 — raw kill과 크래시 exit code 구분 불가 + engram은 restore_all로 다음 연결 시 복원되니 watchdog 무의미.)
- **ensure(spawn)=명시 시점만**(부팅 1회 `App.tsx bootstrapDaemonIfNeeded` / `daemon_start`) / **reconnect·명령 경로(ensureReady)=attach-only(spawn 금지).** → kill/크래시 시 키입력·리사이즈·명령으로 respawn 안 함, `daemon_start`로만 부활. (reviewer B-1 적출: 재연결뿐 아니라 ensureReady도 attach-only여야 — 안 그러면 다음 액션에 respawn.)
- **lifecycle command(§5):** `daemon_start/stop/status` (Rust discovery.rs) + `window.__ENGRAM_DAEMON__`(daemonControl.ts) + `AgentClient.stopDaemon`(StopDaemon AgentCommand). `daemon_stop`=graceful + disconnect(즉시 down) + still-alive일 때만 taskkill fallback.
- **windowless:** WMI `Win32_Process.Create`가 `CREATE_NO_WINDOW` 거부(RV=21) → **ProcessStartup 생략(RV=0, 비대화형이라 창 없음)**. `console:true`=CREATE_NEW_CONSOLE.
- **GUI 실측 PASS:** kill→respawn0 / daemon_start 부활 / daemon_stop 조용한 down / 부팅 windowless 자동기동(Window Title=N/A) / spawn·출력.

## 3. ★ADR-0022: 통합 command registry (제안) — 방향 고정★
§5 LLM 제어 + VS Code식 command palette + 커스텀 키바인딩이 **같은 한 가지**로 수렴: 모든 동작=`id+메타+handler`로 registry 등록 → palette·키·메뉴·트레이·LLM이 전부 그 소비자. **새 기능 = command 등록 1개 → 모든 표면 자동 노출 = 추가 여파 0**(사용자 핵심 목표=blast-radius 최소화). 구현은 나중. **지금부터 만드는 command는 registry 호환(안정 id+메타)으로 쌓을 것.** §5 UI/레이아웃 제어 표면 갭(프론트 Zustand 전용)을 이 registry로 흡수가 전제.

## 4. 플랫폼 추상화 현황 (사용자 확인 사항)
- **백엔드: Windows API가 개념 뒤에 가려짐(완료).** `platform/`(JobObjectHandle·pid_alive, `#[cfg(windows)]`) + trait(`Spawner`/`PidLiveness`/`ProcessKiller`/`Clock`/`DaemonReader`). raw winapi가 로직에 안 샘. 다른 OS=impl만 추가(seam 깔림). 단 non-windows 실구현은 stub(아직 크로스플랫폼 동작 X).
- **프론트: React=웹이라 가릴 OS API 없음(자동 멀티플랫폼).** 주의 2: OS 경로 가정을 프론트에 박지 말 것 / WebView 엔진 OS별 차이(quirk).

## 5. ★백엔드 잔여 (실사용 확인 완료) — 다음에 칠 것★
**칠 것(소소):**
1. **죽은 필드 `restart_policy`/`restart_count`/`failed_reason`** (profile.rs) — 읽는 로직 0(확인). ADR-0019 자동재시작 폐기로 무의미. **단 protocol wire 미러(`restart_policy_to_wire`)+ts 바인딩+PROTOCOL_VERSION까지 걸려 제거 범위 넓음.** → **결정 대기: 완전 제거(버전 bump) vs "reserved·미사용" 주석만.** (사용자 미결.)
2. **`set_daemon_console` 런타임 토글** (ADR-0021 M-2) — 현재 spawn-time param만. 진짜 토글은 데몬이 AllocConsole/FreeConsole 처리(데몬측 작업). 편의, 선택.
3. **wsTransport `start()` in-flight race nit** — start 진입 시 cleanupSocket 선행. 싸다, 바로 가능.

**보류가 맞음(지금 무해/비해당):**
- **shutting_down 리셋** — 데몬에서 manager가 프로세스와 1:1(shutdown_all=데몬종료)이라 무해. "per-client 정리로 재용도화" 시에만 필요 → 그때 ADR.
- **watcher 단일화** — 세션당 스레드, 다중세션 확장성(장기). 동작 정상.
- **status Killed/Exited 오분류** — 자동재시작 폐기 + disposition intent 우선이라 영향 0. 비해당.
- ~~reaper 테스트 갭(ADR-0019 후속 #4)~~ — **이미 해소**(tests/reaper.rs `epoch_mismatch_does_not_reap_current_session`·`duplicate_reap_processes_exactly_once`, 커밋 5c6b58d). ADR 후속이 stale.

**외부 차단(보류):** codex/gemini CLI spike(stub 미연결, **사용자 codex 테스트 불가**) · ApiTransport 내부(API 모델 등장 시).

**검증(코드 아님):** ADR-0020 daemon 모드 GUI 재실측(통합 후 1회 권장).

## 6. 다음 방향 — 프론트
백엔드 마감(위 1~3 처리/결정 후) → **프론트.** 첫 후보:
- **#2 트레이** — ADR-0022 registry 패턴 첫 실증(트레이 항목=command 참조, 별 로직 0). Tauri v2 TrayIconBuilder, command 표면 깔려 있어 반나절급. close-to-tray 동작 결정 1개.
- **§5 UI/레이아웃 제어 표면** — 프론트 Zustand 액션을 command로 승격(ADR-0022 흡수 대상). 이게 진짜 여파 감소의 핵심.
- 기타 프론트: 깡통 라이브 갱신(ProfileListUpdated, daemon 모드는 됨/embedded는 registry fanout 후속), popup+monaco(3d).

## 7. 환경/주의
- **engram 프로세스 현재 0**(세션 끝에 정리함). 데몬 kill하려면 **대시보드(engram-dashboard.exe)부터** 종료(안 그러면 부팅 bootstrap이 다시 띄움) — 또는 `daemon_stop`.
- dev: `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` + `scripts/cdp.mjs`. localStorage `engram_client_mode`로 embedded/daemon 토글(잔류값 주의 — 직전 세션 값이 우선).
- 빌드 잠금(daemon.exe os error 5): `tasklist|grep -iE engram`→`taskkill //F //PID`.
- 서브에이전트 코더/리뷰어/QA 스폰으로 구현 실행 규약 준수(이번 세션 ADR-0020/0021 전부 coder→reviewer-deep→QA+GUI 실측 게이트 통과).

## 8. 핵심 불변식 (변경 금지 — 근거 docs/decisions/)
- 이전 핸드오프(ADR0020) §8 + reaper/kill 2동사/finalize 1회/epoch 재구독 동일.
- **ADR-0020:** 단일 프로토콜+dispatch, carrier만 교체 · ConnectionCore=daemon crate(코어 격리) · embedded 단일 command loop 직렬화 · 단일 writer 큐(R1) · carrier가 epoch 끝까지 전파 · lease 우회 금지.
- **ADR-0021:** ensure(spawn)=명시만 / reconnect·명령=attach-only(spawn 절대 금지 — 깨지면 "못 끄는" 버그 재발) · 자동재시작/watchdog/desired-state 없음 · 데몬 인프라 수명 ≠ 에이전트 자동재시작(0019 폐기).
- **ADR-0022(제안):** 새 command는 registry 호환(안정 id+메타)으로 쌓는다.

## 9. 시작 첫 행동 제안
1. `docs/decisions/0020·0021·0022` + step-log 맨 끝 정독.
2. **백엔드 마감:** §5의 1(제거 vs reserved 결정)·2(선택)·3(바로) 처리. 그 후 프론트.
3. 프론트 가면 **#2 트레이를 ADR-0022 registry 첫 사례로** 또는 §5 UI 제어 표면부터. 사용자와 범위 확인.
4. push 완료(origin/master=8c29ace) — 이후 작업분만 새로 커밋/push.
