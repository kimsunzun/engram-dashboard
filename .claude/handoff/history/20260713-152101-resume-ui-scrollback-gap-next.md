# 핸드오프: 생성-시-모드 기능 커밋·푸쉬 완료(863f9ec). resume 규명 = **백엔드 컨텍스트 복원 OK / 프론트 스크롤백 미복원**(복원 UX 미구현)이 다음 과제. 터미널 모드 미검증. stash@{0} 잔존

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0078 재정의(렌더 모드=생성 시 결정·고정) 기능 커밋·푸쉬 완료(origin/master `863f9ec` + 핸드오프 `38b9580`). 이후 세션 **resume 동작을 라이브 규명** — 백엔드는 정상, **프론트 UI 대화 스크롤백 복원이 미구현**임을 확인.
- **다음 첫 액션:** **세션 resume 시 프론트 대화 스크롤백 복원(복원 UX)** 설계·구현. 굵은 설계라 옵션 제시 → **사용자 결정부터**(임의 확정 금지). 선택 축: transcript(`.jsonl`)를 ① 데몬이 읽어 구조화 OutputEvent로 replay vs ② 프론트가 직접 읽어 렌더 · 터미널 모드(xterm) 스크롤백 재현 방식 · 배너/부분로드 UX.

## 완료 (이 세션 · 커밋·푸쉬)
- **`863f9ec` feat(agent)** — pane 배경 "에이전트 생성"을 1단 서브메뉴 [클로드 터미널 생성 / 클로드 JSON 생성] container로. 렌더 모드는 **생성 시 고정·이후 불변**(per-activation 오버라이드 폐기). createReservedProfile 헬퍼·createTerminal/createJson/파라미터화 createAgent(coerceOutputFormat invalid→throw). ADR-0078 본문·인덱스·step-log 포함. 프론트 전용(crates/** 무변경).
- **`38b9580` docs(handoff).**
- 게이트 PASS: /review code full(doc-aware+cross-family Codex, FIX 반영), /qa(vitest 613·tsc 0·코어격리 0), 라이브 GUI 실측(서브메뉴 렌더+command+invalid throw).

## resume 규명 (다음 과제의 근거 — 핵심)
- **resume = 2층 분리 확인.**
  - ① **백엔드 컨텍스트 = 복원됨.** `claude --resume <sid>`가 `.jsonl` 전체를 모델 컨텍스트로 로드 → 과거 턴 전부 기억(라이브 확인: AgentD가 "PERSIST-TEST-ALPHA-777 토큰·데몬 재연결 테스트"를 회상). 데몬 강제 사이클(stop/start) 후 재활성화해도 sid·epoch 불변, `.jsonl` 이어붙음.
  - ② **프론트 스크롤백 = 미복원(다음 과제).** RichSlot(JSON 모드)은 라이브 출력 스트림으로 렌더하는데, resume는 과거 턴을 stdout에 **재방출 안 함** + engram replay 버퍼는 **프로세스 단위**(재시작 리셋) + `.jsonl`로 UI 재구성하는 경로 **없음** → 채팅창에 과거 대화가 안 뜸(빈 화면에 새 턴만). 사용자 체감 "이전 내용 못 불러옴"의 정체.
- **= 미구현 기능**(버그 아님). step-log "다음"의 *"프론트 상세(복원 배너 UX)"* 가 이 항목.
- **터미널 모드(xterm) = 미검증.** 메커니즘 다름(원시 PTY 바이트, `claude --resume` TUI가 화면 repaint하면 xterm에 뜰 수도/아닐 수도). **확인 안 함 — 검증 필요.**
- **라이브 실측 레시피(재현):** `node scripts/engram.mjs send <label> "<마커>"` 로 한 턴 → `.jsonl`(`~/.claude/projects/C--...-Filter-Library/<sid>.jsonl`)에 저장 확인 → `node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('daemon_stop')"` / `daemon_start` → `node scripts/engram.mjs raw '{"SpawnProfile":{"profile_id":"<id>","resume":false,"request_id":"<uuid>"}}'` 재활성화 → agents.json sid/epoch + GUI 스크롤백 확인.

## 부차 발견
- **auto_restore 미발동:** AgentD `auto_restore:true`인데 데몬 재시작 후 자동 복원 안 됨(○ 예약 유지 → 명시 재활성화해야 복귀). `restore_all`이 client-invoked `daemon_start` 경로에서 안 도는지 **별도 규명 가치**(사용자 "데몬 껐다 켜니 사라진" 체감의 한 축).
- **`run-dashboard-clean.bat` 빈틈:** 데몬만 taskkill하고 stale vite(포트 1420)/구 클라이언트는 안 잡음 → orphan vite 남아 있으면 `Port 1420 already in use`로 launch 실패(코드 에러 아님). 보강 여지(vite/클라도 정리 or 안내).

## 상태 / 주의
- 워킹트리 clean, origin/master 동기(`863f9ec`·`38b9580`·이 핸드오프).
- **`stash@{0}`** = per-activation ADR-0078 폐기 시도(16파일: 코어/데몬/프로토콜/프론트 오버라이드 배선). **미폐기** — 폐기 여부 **사용자 판단**(되돌리기 어려움). ADR-0078 거부-대안 근거.
- **런타임 흔적(gitignore·repo 아님):** AgentD 세션 `255fc7cf`에 테스트 마커 턴 존재, Filter Library 예약 노드 2개(사용자가 새 기능으로 생성). 실행 데몬은 `run-dashboard-clean.bat`이 HEAD로 재빌드한 것.
- **do-not:** bare `cargo test`·`-p engram-dashboard` = WebView2 크래시(member-scoped만). 실행 중 앱 있으면 `cargo build` 파일락(재빌드 전 taskkill daemon+client). release exe엔 CDP 포트 없음(dev만).

## 참조 (읽을 것)
- ADR-0078(생성 시 모드 고정) · ADR-0044(output_format 렌더) · ADR-0008(세션 복원 sid 통제) · ADR-0007(epoch 재구독) · §5(CLAUDE.md).
- 복원 UX 손댈 곳: RichSlot 렌더 · `OutputCore` replay(프로세스 단위) · transcript = `~/.claude/projects/<cwd-slug>/<sid>.jsonl` · `src/api/protocolClient.ts`(replay 경계·seq dedup) · 프론트 구독 effect(`[viewId, agentId, epoch]`).
- 앱 실행: `run-dashboard-clean.bat`(데몬 HEAD 재빌드 + dev, 디버그포트 9223) — stale vite 있으면 그 node 프로세스 먼저 kill.
