# Step 타임라인 — Engram Dashboard

언제·무엇을·어떻게 했는지 시간순 기록. 산출 문서와 커밋을 매핑한다.
상세는 각 폴더 참조: `design/`(설계) · `reviews/`(검증) · `briefings/`(구현 지시) · `spec/`(요구사항) · `history/`(초안).

**검증 3-게이트:** 코더(dco23 Opus / dcs24 Sonnet) → LLD 리뷰(dr26 Fable) → QA(dq25, build/lint/test).

---

## S0 — View phase (이전 세션)
- **무엇:** 더미 데이터로 UI 골격 — 테마/폰트/레이아웃/슬롯/에이전트 트리/diff/팝업 (Step 1–10 + 슬롯 동적 분할)
- **결과:** Tauri 창에 더미 대시보드 동작 (백엔드 없이 UI 검증 완료)
- **문서:** `spec/view-spec.md` (+ `view-spec-gpt-review.md`), `spec/requirements.md`, `spec/research.md`

## S1 — 백엔드 설계 (이전 세션)
- **무엇:** 백엔드 아키텍처 → 상세설계(LLD) → 프론트 통합 설계
- **어떻게:** architecture 초안 → Gemini/GPT 리뷰 → final. LLD Stage 1 작성 → **fable/Gemini/GPT 3자 adversarial 검증** → GO. frontend-integration LLD도 동일 3자 검증.
- **결과:** 구현 계약서 확정 — `pty/` Tauri 격리(OutputSink/StatusSink), kill 6단계, C4 replay→live, AppState 단일 manager
- **문서:** `design/backend-architecture.md`, `design/backend-lld-stage1.md`, `design/frontend-integration-lld.md`, `reviews/*`

## S2 — Phase 0: Spike (2026-06-11)
- **무엇:** 본 구현 전 PTY kill 시퀀스를 Windows 실기기에서 실측
- **어떻게:** dco23 — `examples/spike.rs`로 portable-pty spawn → kill 6단계 → reader join 타이밍 측정
- **결과:** ✅ PASS. `master.take()`(ConPTY 종료)로 reader가 **17ms** 내 EOF. LLD 가정 그대로.
- **문서:** `briefings/phase0-spike.md`

## S3 — Phase 1: 백엔드 PTY 코어  · 커밋 `575e36d`
- **무엇:** `pty/` 6모듈 + `logging/` + headless 테스트
- **어떻게:** m1 types → m2 session(C4 subscribe) → m3 drain(lock-밖-send) → m4 platform/windows(JobObject) → m5 manager(kill 6단계) → m6 logging → headless. 각 모듈 3-게이트 통과.
- **결과:** ✅ headless PASS — spawn→write→resize→kill, 상태 Running→Exiting→Killed, kill **23ms**. `pty/` tauri import 0. dr26이 win_err 버그·transition race·poison 정책 등 실결함 포착.
- **문서:** `briefings/m1-types` ~ `m6b-headless`, `design/backend-lld-stage1.md`

## S4 — Channel spike: tauri 핀 결정 (Phase 2 직전)
- **무엇:** tauri 버전 확정을 위한 Channel 무손실 실측 (LLD는 "2.5 금지"였으나 caret이라 2.11.2 resolve)
- **어떻게:** dco23 — 임시 `channel_spike` command로 1000회 연속 send, 프론트 수신 카운트
- **결과:** ✅ 1000/1000 무손실 (이슈 #11421은 Linux 특정, Windows WebView2 미재현). **최신 2.x(2.11.2) 유지 확정.**
- **문서:** `briefings/channel-spike.md`

## S5 — Phase 2: Tauri 연결 계층  · 커밋 `f959304`
- **무엇:** commands 8개 + `lib.rs`(AppState / ChannelOutputSink / TauriStatusSink / setup)
- **어떻게:** dcs24 — thin wrapper + OutputSink/StatusSink의 Tauri 구현. 3-게이트.
- **결과:** ✅ dead_code 전멸(코어가 command 통해 사용됨), 앱 기동. `RunEvent::ExitRequested → shutdown_all` graceful 종료.
- **문서:** `briefings/m7-commands-lib.md`

## S6 — 백엔드 마감  · 커밋 `26dc649`
- **무엇:** ① 로그 API키 마스킹(T-1) + ④ shutdown 병렬 kill(T-8). ② cwd 검증은 **claude 자체 권한과 중복이라 스킵**.
- **어떻게:** dcs24 마스킹(regex, 키 패턴 6종) / dco23 병렬 kill(`std::thread::scope`). 3-게이트.
- **결과:** ✅ debug 로깅 시 키 노출 차단, 앱 종료 worst case N×5s→5s.
- **문서:** `tracking.md` T-1 / T-6 / T-8

## S7 — Phase 3: 프론트 통합 3a–3c  · 커밋 `ca61cbd`
- **무엇:** API 레이어(types/ptyApi) → eventBus+store → TerminalSlot 실제 PTY 연결
- **어떻게:** dcs24 — 3a→3b→3c. C2 reset / T-2 seq dedup / G-1 cleanup / 입력가드 / resize debounce.
- **결과:** ✅ **첫 E2E** — 실제 창에서 claude 기동, **PTY ↔ Tauri ↔ React 전체 파이프라인 실증**. 3d(popup+monaco)는 보류.
- **문서:** `briefings/m8a-api-layer` ~ `m8d-popup`, `design/frontend-integration-lld.md`

## S8 — 문서 정리  · 커밋 `fdf6d06` + 진행 중
- **무엇:** core/dashboard 통합 → docs 트리 재편 → 과정/정설 분리 + 타임라인
- **결과:** ✅ 모든 자료 `apps/engram-dashboard` 일원화. `process/`(과정 기록) + `reference/`(정설, 추후) 분리, 이 `step-log.md` 작성.

## S9 — 세션 저장/복원 (코어 GO)  · 커밋 `8981cb9`~`7052bc2`
- **무엇:** claude 세션 무손실 복원 + 에이전트 프로필 영속화 + 약간의 추상화(claude.rs/profile.rs 분리). 자동재시작(restart_agent)은 **게이트**(설계만).
- **어떻게(H-4 순서, 매니저 직접 구현 + fable 리뷰 게이트):**
  1. `profile.rs` + ProfileRegistry(단일 소유자, sid 생성·갱신) + dunce — `8981cb9`
  2. `persistence/` atomic agents.json(tmp+sync_all+rename+parent fsync, schema_version, .corrupt 보존) — `d7f42f4`
  3. `session_tracker.rs` sid drift 폴링(best-effort, PID shim 우회 스캔, 단일 스레드+정지핸들, degraded 강등) — `0a59fa3`
  4. LLD 개정 a~g(backend §18 / frontend §11, Stage-1 보존+addendum) — `1b9499c`
  5. `claude.rs` 격리 + manager `spawn_agent(profile,mode)`/`restore_all`(백그라운드)/fallback(조기종료 윈도→fresh, 종점 Failed) — `f67476b`
  6. profile CRUD 커맨드 + 프론트 TS 미러(epoch 재구독) + **fable 리뷰 수정** — `7052bc2`
- **검증:** unit test 19, headless PASS, `cargo fmt`·`tsc` 클린, `pty/` tauri import 0.
- **fable 리뷰(조건부 GO→수정 완료):** C-1 remove_session drain 대기(stale Killed 경합 제거), M-1 resume 조기종료 code 무관 fallback, Mn-1 status_changed epoch 동봉, Mn-2 Started variant, Mn-5 단일 persist.
- **핵심 메커니즘:** spawn 시 `--session-id <uuid>`로 우리가 sid 통제 → 재시작 `--resume`로 무손실 복원. `/clear`로 sid 바뀌면 `sessions/<pid>.json` 폴링으로 따라잡아 즉시 persist. 복원 정확성은 우리 통제 sid에만 의존(추적 파일은 best-effort).
- **문서:** `S9-session-restore/session-restore-lld.md`, `-code-plan.md`(§H), `spike-results.md`, `s9-*-review-*.md`

## S10 — 백엔드 추상화 (AgentTransport/OutputEvent)  · 커밋 `60fe859`~`fb50917`
- **무엇:** 검증된 S9 PTY 코드를 `AgentManager → AgentSession(OutputCore) → dyn AgentTransport(PtyTransport/ApiTransport)` 구조로 재편. 멀티 백엔드(claude/codex/gemini 콘솔 + API) 통합 인터페이스. **회귀 0**이 목표(기능 추가 아닌 seam 추상화).
- **어떻게(9단계, 단계별 build/test/commit + fable 리뷰 게이트, 오케스트레이터=서브에이전트 디스패치):**
  1. 중립 타입/enum: `OutputEvent`/`InputEvent`(확장 enum) · `TerminalReason` · `Capabilities`(영역별) · `CommandSpec` · PtyChunk→`OutputChunk` — `60fe859`
  2. `output_core.rs` OutputCore: seq/replay/subscribers/status/finalize, emit(variant-agnostic)/finish(finalize 1회)/join_pump/enter_exiting/subscribe — `dbcde55`
  3. `transport/{mod,pty}.rs` AgentTransport trait + PtyTransport: spawn/kill 1~5단계/drain_loop+transition 흡수, pump 스레드, shutdown 멱등 — `cd3b048`
  4. `backend/{mod,claude,shell}.rs` AgentBackend + CommandSpec 산출 dispatch — `38d2fe7`
  5. `session.rs` AgentSession 합성: kill=shutdown+join_pump 2동사 — `7c68e31`
  6. `manager.rs` AgentManager(PtyManager 개명) 신경로 전환 + 옛 구조(PtySession/drain.rs/claude.rs) 제거 — `c954305`
  7/8. ApiTransport 껍데기 + codex/gemini stub(dispatch 미연결, best-guess) + interrupt_agent 커맨드 + AgentInfo.capabilities + TS 미러 — `fb50917`
- **검증:** unit test 38(S9 19→backend 이관·stub 추가), headless·transport_smoke·session_smoke PASS, full build(bin), `cargo fmt`·`tsc` 클린, `pty/` tauri import 0.
- **신경로 실측:** `examples/transport_smoke.rs`·`session_smoke.rs` 신설 — manager 없이 PtyTransport/AgentSession 직접 검증(shutdown→pump EOF→finish(Killed) 인과, hang 없음).
- **fable 리뷰(2/3/6 + 최종 게이트, 전부 GO):** 회귀 0 확인. B-1(enter_exiting/finish 알림 역전 창)은 S9 기존 동작=natural-exit race, agent-list-updated 완화로 회귀 아님. attach_pump race는 mpsc 무한버퍼로 무손실. 소유권 분할(transport/core/session) 깨끗, "교체 가능" 추상화 성립(ApiTransport가 같은 trait로 끼워짐).
- **단계화(후일):** OutputEvent API variant(TextDelta/Usage 등)·ApiTransport 내부 HTTP 스트림·codex/gemini CLI 플래그(spike 후 AgentCommand variant 추가)·semantic event log/replay 고도화.
- **문서:** `S10-backend-abstraction/agent-transport-design.md`, `impl-spec.md`(코더 공통 참조 — 구체 시그니처)

## S11 — 에이전트 트리 슬롯화 + capability 잠복 버그 수정 (2026-06-14)
- **무엇:** ① `AgentInfo.name` 백엔드 채움(ProfileRegistry 조회, AgentSession에 중복 필드 안 둠). ② 슬롯 콘텐츠 모델 객체 union(`{kind:'terminal',agentId}|{kind:'tree'}`) + `LayoutCommand`/`dispatch` 단일 제어 표면(§5, `window.__engramLayout` 노출=LLM 제어 경로). ③ 트리를 슬롯 콘텐츠로(`LayoutRenderer` kind 분기). ④ `AgentTree` 재작성: 클릭=selectOnly(자기파괴 루프 방지), 우클릭 메뉴(배치/종료/중단), interrupt는 `ControlCaps.interrupt` capability 분기, 이름 표시. ⑤ 슬롯 우클릭 트리/터미널 토글.
- **어떻게:** `/consult`(GPT+Gemini+opus judge 블라인드 교차검증) → 설계 보정(string enum→객체 union 확정, store 직접호출→dispatch, name=ProfileRegistry 조회가 옳음 판정). 코딩 → opus 리뷰([높음] AgentTree 우클릭 메뉴 바깥닫기 가드 누락 수정) → QA(test/tsc/GUI 실측).
- **★capability 잠복 버그★:** `src-tauri/capabilities/` 파일이 없어 core event listen이 막혀, `agent-list-updated`/`status-changed` 상태 브로드캐스트가 **프론트에 원래부터 안 닿던** 버그(PTY 출력은 Channel이라 권한 무관 → 여태 미노출). 증상: spawn해도 트리 미갱신 → 거기서 kill 불가. `capabilities/default.json`(`core:default`+`core:event:default`)로 해결. spawn→트리 추가 / kill→트리 제거 실시간 GUI 실측 PASS.
- **검증:** unit test 38/38, `tsc` 0, `cargo fmt`, `pty/` tauri import 0, GUI 실측(9223 cdp eval — 트리 토글·이름·spawn/kill 실시간 갱신·`__engramLayout` 제어표면) 전부 PASS.
- **보류:** tabs 구조(`content`→`tabs[]` 확장)·레이아웃 영속화·UI 동작의 완전 backend 이관(데몬화 때). `assignAgent`/`setSlotContent` 명령 중복은 편의로 유지.

## S12 — 데몬화 (설계 단계, 2026-06-14)  · 별도 repo 분리
- **repo 분리:** engram-dashboard를 Engram 모노레포에서 분리(54 커밋 history 보존, filter-repo) → `github.com/kimsunzun/engram-dashboard`(private) push. 모노레포는 추적 0(.gitignore), 폴더는 당분간 apps/engram-dashboard 잔류(나중에 이동). engram=고수준 LLM 워크스페이스 / engram-dashboard=LLM 운용 툴로 완전 분리.
- **데몬화 결정·설계(consult 2회 교차검증):** 라이브러리 실조사(턴키 없음, 커스텀 불가피, replay=데몬 in-process, 경로 B-direct, raw tokio-tungstenite) + 세부 설계 비판. 상세: `S12-daemonization/{ipc-library-consult,daemon-design}.md`, tracking D-8.
- **확정 핵심:** 단일 WS 연결(lane 분리 금지)·output hot path 커스텀 binary frame/control JSON·ring 잘림 truncated 분기·토큰 env/ACL portfile(arg 금지)·emit try_send=코어변경0·Job breakaway가 단일 장애점(spike #1). 두-모드 토글(Embedded/Daemon). Cargo workspace(core/protocol/daemon/tauri).
- **상태:** 설계 완료, 구현 보류(사용자 결정 7개 대기: idle-timeout·타입생성·truncated UX·로컬 transport·throttle 주체·version mismatch·다중 인스턴스). ★최우선 spike: Job breakaway.

---

## 다음 (미진행)
- **[원칙→구현] LLM 제어 표면** — CLAUDE.md §5 신설(모든 메뉴가 LLM 제어 가능, LLM이 메인/사용자 UI는 서브, 손발/두뇌 분리). 현재 백엔드만 invoke로 제어되고 UI/레이아웃(분할·저장·트리 추가 등)은 프론트 전용. UI 액션을 LLM·사람이 같이 부르는 단일 control surface(command 버스)로 모으는 작업 필요. 새 UI 기능마다 제어 경로 동반.
- **[입주 1단계-b] UI 레이아웃/창 영속화** — **저장위치 결정 완료(D-7): 프론트 localStorage**(백엔드 아님). 다중창(창별 독립 layout+theme+좌표, 멀티모니터)·창 id별 키·Tauri JS `WebviewWindow`로 부팅 복원. 현 conf.json 정적 3창→동적 창 생성 신규 기능. **데몬화 뒤로 보류**(2026-06-14, 데몬 우선 결정). 상세: tracking.md D-7.
- **[입주 2단계] 에이전트 데몬화 (Rust 서버 + React 소켓 클라, tmux 모델)** — UI 재시작이 에이전트를 안 죽이게(서버가 세션 보유, 클라가 attach). **시점 결정: 에이전트 안정화(자동재시작·복원) 끝난 직후.** 공수 ~1~2주, `ptyApi.ts`/`eventBus`/`OutputSink`가 길목이라 facade swap(갈아엎기 X). 로컬 속도 영향 0(이미 직렬화 중, loopback 한 홉). 원격/모바일과 동일 인프라. **전제: "메시지 패싱·무상태 클라" 규율 유지**(공유메모리·동기 가정 금지)로 double-stabilization 방지 — facade 한 곳만 재검증.
- **[큰 것] 원격(WS) 프론트 = 모바일 제어** — 데몬화(2단계)의 자연 연장. 에이전트는 데스크톱(데몬)에서 돌고 폰은 원격 I/O 프론트로 attach. 인증/TLS 추가. 보안 1급.
- **[아이디어] 모드 시스템 (터미널/클로드/코덱스/api)** — 슬롯/에이전트의 "mode" 라벨이 [AgentCommand variant + transport + 기본 렌더러]를 묶어 고름. 터미널=Shell, 클로드·코덱스=콘솔(PtyTransport), api=ApiTransport(비-터미널, capability.output로 렌더러 분기). 모드 추가 = variant+backend(+api는 transport)+렌더러 하나. S10 추상화 위 UI 표현.
- **[아이디어] data-driven 우클릭 메뉴 (§5 구체 사례)** — SlotContextMenu를 데이터 트리(`MenuItem{label,children?,action?}`, 중첩 2·3단)로. 각 잎 항목=핸들(모드 spawn/분할/저장 등). 그 메뉴 config(JSON)를 LLM이 읽고(정보화)·수정(제어) → 사람 클릭과 LLM이 같은 핸들. "클로드에게 부탁해 메뉴 커스터마이징" = LLM이 메뉴 데이터 편집.
- **[✅ 해결] claude 콘솔 spawn Windows 실패** — `ClaudeBackend` program="claude"가 ConPTY/CreateProcessW로 확장자 없는 npm shim 못 띄우던 버그(error 193). 수정 완료(커밋 `adf80d7`): `backend/mod.rs::console_command`가 Windows에서 `cmd.exe /c <prog> …`로 래핑(claude.cmd shim 해석, claude 종료 시 cmd도 종료=수명 유지, JobObject 트리 kill), 비Windows 직접. claude/codex/gemini 공용 헬퍼. 스파이크(2026-06-12)로 발견(UI에 spawn 버튼 없고 테스트가 셸뿐이라 잠복) → 라이브로 실제 claude TUI spawn + `--session-id`/`--resume` 무손실 복원까지 실측 PASS. ※codex/gemini stub도 라우팅 시 console_command 채택 필요(현재 미적용).
- **codex/gemini CLI spike** — 실제 CLI 구독 후 플래그 확정 → `AgentCommand`에 Codex/Gemini variant 추가 + `backend_for` 라우팅 연결(현재 stub은 best-guess+미연결).
- **[게이트] 자동 재시작** — `restart_agent` 전용 태스크(사다리 resume→fresh→정지, backoff). 코어 안정 후.
- **실제 claude 복원 E2E** — headless는 shell만 실증. claude `--session-id`/`--resume` + `sessions/<pid>.json` PID 일치를 실제 claude로 실측(spike) 필요.
- 메시지 시스템(에이전트 간 통신) — 백엔드 추가 설계.
- Phase 3d (popup URL 전달 + monaco) + 프론트 상세(복원 배너 UX).
- `reference/` 정설 문서 집필 (시스템 안정화 후)
- **[정리] `pty/` 폴더명·구성 재고** — S10 후 `pty/`가 PTY 전용이 아니라 에이전트 코어 전반(AgentManager/AgentSession/OutputCore/transport/backend) 보유. 폴더명이 내용과 불일치(에이전트 공용 매니저가 pty/ 안). 다른 모듈 배치도 같이 점검(사용자 지적 2026-06-14, 트리 슬롯화 작업 끝난 뒤).
