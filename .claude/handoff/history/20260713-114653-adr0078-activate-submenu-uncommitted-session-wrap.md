# 핸드오프: 활성화 서브메뉴(ADR-0078) **미커밋** — verify→review→ADR본문→커밋 남음. 그 외(오케스트레이션·드래그·피커·세션 resume/fallback)는 커밋+푸쉬 완료(origin `a4aac1a`)

## 한 줄 상태 · 다음 첫 액션
- **상태:** 이번 세션은 (1) 릴리즈/디버그 **에이전트 오케스트레이션**(에이전트끼리 메시지·앱 제어) 실증+문서화, (2) 트리 **드래그 재부모화** 실동작 fix, (3) 실행중-에이전트 **피커 표시명** fix, (4) **활성화 세션 처리**(ADR-0076 resume + ADR-0077 fresh-fallback) 까지 **커밋+푸쉬 완료**(origin/master = `a4aac1a`). 마지막 (5) **트리 우클릭 "활성화" → 서브메뉴(클로드 터미널/JSON) 기능(ADR-0078)** 은 **코딩+게이트 통과했으나 미커밋**.
- **다음 첫 액션(순서):** ADR-0078 기능을 ① **라이브 GUI 실측**(앱 재빌드·재시작 → 예약 노드 우클릭 "활성화" hover → flyout에서 터미널/JSON 클릭 → 각각 xterm/챗UI로 스폰 + 저장 프로필 output_format 불변 확인) → ② **/review code** → ③ **ADR-0078 본문 작성**(`/adr` — 현재 코드에 앵커만, 본문 파일 없음) → ④ **커밋 + 푸쉬**(사용자 승인 후).

## 커밋+푸쉬 완료분 (origin `a4aac1a`, 세션 중 여러 커밋)
- **오케스트레이션/데모 기반:** `scripts/engram.mjs`(데몬 CLI: list/spawn/spawn-claude/send/kill/reparent, 자기위치로 데몬 발견=cwd무관) · `scripts/cdp.mjs`(웹뷰 제어) · exe-서브커맨드 `src-tauri/src/cli.rs`(옆 에이전트 작업 — 릴리즈 exe가 데몬 CLI 겸용) · `AGENT-CONTROL-GUIDE.md`(에이전트 대면) · `ORCHESTRA-DEMO.md`(운영자 대면) · `SETUP-FRESH-PC.md` · `run-dashboard-release.bat`.
- **드래그 fix:** `tauri.conf.json` 양 window `"dragDropEnabled": false` — Tauri OS 드래그가로채기 꺼서 웹뷰 HTML5 DnD(react-arborist nest) 활성. **원인규명: dragstart는 뜨나 dragover/drop 0회(Tauri 가로채기)**. 실측으로 확정.
- **피커 fix:** `monitoringPickerFilter.ts` — 실행중-에이전트 피커가 cwd basename 대신 `profile.display_name` 표시(트리와 단일 출처, id 조인). ADR-0061 정합.
- **세션 처리(핵심):** ADR-0076(활성화=기존 세션 resume, Fresh는 새 sid mint) + **ADR-0077**(수동 활성화도 resume 조기종료 시 fresh-fallback — restore_one fallback을 `resume_with_fresh_fallback`로 추출·공유). **라이브 3케이스 검증됨**(대화有→Resume / 빈세션→fresh-fallback→Running / 신규→Fresh).

## 미커밋 (ADR-0078 활성화 서브메뉴 — working tree, 디스크에 안전)
- **기능:** 트리 우클릭 "활성화" → flyout 서브메뉴 [클로드 터미널 / 클로드 JSON]. **per-activation 렌더모드 오버라이드, 저장 프로필 불변**(코더가 함정 잡음: profile-clone override는 `upsert_preserving_hierarchy`로 영속누수 → 오버라이드를 `effective_command`로 threading, 저장은 원본 유지). §5: `agent.activateTerminal`/`agent.activateJson` 커맨드.
- **미커밋 16파일:** core `manager.rs`·`profile.rs`(+tests activation/headless/reaper) · daemon `connection_core.rs`(+ws_e2e) · protocol `messages.rs`(SpawnProfile에 `#[serde(default)] output_format: Option<ClaudeOutputFormat>`, additive·버전유지)·`bindings/AgentCommand.ts` · front `agentClient.ts`·`protocolClient.ts`·`agentCommands.ts`(+test)·`AgentList.tsx`(+test)·`i18n/ko.ts`.
- **ADR-0078 본문 없음** — 코드에 `// ADR-0078` 앵커만. `/adr`로 본문 작성 필요(거부 대안 = profile-clone override의 영속누수).

## 검증 상태
- **PASS(ADR-0078 게이트):** member-scoped `cargo test -p ...-core --lib`(186)·`-p daemon --lib`(39)·`-p protocol`(golden/ts-rs) · `cargo build`(워크스페이스 — daemon.exe만 실행중 파일락으로 link skip, 코드 컴파일 OK) · `cargo fmt --check` · 코어격리 0 · `npx tsc --noEmit`(0) · `npx vitest run`(**614**).
- **PASS(ADR-0077 라이브):** 데몬 재시작 후 재활성화 실측 — 대화有 `mode=Resume`+Running / 빈세션 `mode=Resume`→"no conversation found"→`resume 실패→fresh fallback`→epoch+1 Fresh→Running.
- **검증 안 된 것(중요):** ① **ADR-0078 라이브 GUI 실측 안 함**(서브메뉴 클릭→모드별 스폰 실제 화면 미확인 — 게이트만 통과) ② **ADR-0078 /review 안 함** ③ release 빌드에서 exe-서브커맨드 실측은 이번 세션 안 함(코드만).
- **재실행:** Rust=member-scoped만(`-p engram-dashboard-core --lib`·`-p engram-dashboard-daemon --lib`·`-p engram-dashboard-protocol`) · 프론트 `npx tsc --noEmit`+`npx vitest run` · GUI `node scripts/cdp.mjs eval`(dev 앱 실행 중, 포트 9223).

## do-not / 실패한 접근
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시.** member-scoped만.
- **실행중 데몬/클라 있으면 `cargo build` link 실패(exe 파일락)** — 정상. 재빌드 전 `taskkill /IM engram-dashboard-daemon.exe /F` + `engram-dashboard.exe`.
- **release exe엔 CDP 디버그 포트 없음** → UI 조종(팝업/분할/서브메뉴 실측)은 **dev(run-dashboard.bat)** 로. 스폰/메시지는 릴리즈도 가능.
- **데몬 끄면 에이전트 강제종료**(Job Object) — 재연결로 프로세스 못 살림, 재활성화가 세션 resume(ADR-0076/0077).
- ADR-0078 오버라이드를 profile-clone으로 넣지 말 것(`upsert_preserving_hierarchy` 영속누수 — 코더가 이미 파라미터 threading으로 회피).

## 정지 조건 (다음 세션)
- **커밋/푸쉬는 사용자 승인 후만.** 이번 세션 "다 푸쉬"는 ADR-0077 배치까지였고 **ADR-0078은 아직 커밋 승인 안 받음**.
- ADR-0078은 verify+review+ADR본문 전 커밋 금지(구현 실행 규약).

## 참조 (읽을 것)
- **미커밋 코드:** `src/components/agent/AgentList.tsx`(RowMenuRow flyout·activateReserved(id,fmt)) · `src/commands/agentCommands.ts`(agent.activateTerminal/Json) · `crates/engram-dashboard-protocol/src/messages.rs`(SpawnProfile.output_format) · `crates/engram-dashboard-daemon/src/connection_core.rs`(활성화 핸들러 override 매핑) · `crates/engram-dashboard-core/src/agent/manager.rs`(effective_command·activate_profile·resume_with_fresh_fallback) · `crates/engram-dashboard-core/src/agent/profile.rs`(with_output_format_override).
- **ADR:** ADR-0076(활성화 resume)·**0077**(fresh-fallback)·**0078**(본문 미작성 — 활성화 서브메뉴·비영속 output_format 오버라이드) · 0044(output_format 렌더) · 0072(트리) · §5(CLAUDE.md — LLM 제어).
- **데모/도구:** `ORCHESTRA-DEMO.md`·`AGENT-CONTROL-GUIDE.md`·`scripts/engram.mjs`·`scripts/cdp.mjs`.
- 앱 실행: `run-dashboard.bat`(dev, 디버그포트9223) — 데몬은 재빌드본(ADR-0077 포함) 자동 spawn. 현재 데몬·클라 종료 상태.
- **스크래치(gitignore, 무시):** `daemon-debug.log`·`*build.log`.
