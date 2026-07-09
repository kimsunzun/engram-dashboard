# 핸드오프: DOM 모드 관측 렌더러 + RenderMode enum 커밋(1a4e76a) — 큰 미결=리로드 두절 정공 픽스(데몬 재-hydration) 결정 대기

## 한 줄 상태 · 다음 첫 액션
이번 세션은 원래 목표(리로드 두절 버그 픽스)를 **진단만** 하고, 대신 관측 도구(DOM 모드 렌더러)+렌더 추상화(RenderMode enum)를 깔아 커밋(1a4e76a)했다. **버그 자체는 미픽스** — 정공 픽스(데몬 재-hydration)가 코어 불변식(ADR-0040) 건드리는 설계 결정이라 사용자 결정 대기. **다음 첫 액션 = 사용자에게 리로드 픽스 방향 확인**(정공/좁은패치/보류) + 어디까지(설계·ADR 초안 vs 끝까지). 정공이면 CLAUDE.md 참조구현 원칙대로 OSS(tmux/mosh/ttyd re-attach replay) /research → ADR(ADR-0040 리로드 예외 개정) → 코더→review→qa.

## repo 상태
- **HEAD = `1a4e76a` (master), 미푸쉬.** 이번 세션 커밋 1개(front 7파일). 그 아래 `b006be6`/`1fcabd8`(딴 에이전트=track② research 스킬 리팩터, 동시 진행분). 작업 트리 clean.
- **선재 fmt drift(내 밖):** `crates/engram-dashboard-daemon/tests/ws_e2e.rs:23` import 그룹핑 → `cargo fmt --check` FAIL. 내 변경 아님(CRLF 아티팩트 의심). daemon 영역이라 안 건드림. 고치려면 `cargo fmt`(단 line-ending 설정 확인 — 이 체크아웃 CRLF 경고 다수).
- **실행 중(이 세션이 띄움):** tauri dev(RUST_LOG=debug, 포트 9223, 백그라운드 bash `bs47kls3m`) · 데몬 pid 21144(전 세션 바이너리, 그대로) · 테스트 JSON 에이전트(0482489b=QA용, 3068027b, 트리 leftover 다수). 다음 세션 시작 시 살아있을 수 있음 — 정리 필요하면 engram-dashboard-daemon만.

## 이번 세션 한 것
1. **커밋 1a4e76a:** ① 렌더 선택 boolean(`capabilities.output.structured`)→`RenderMode` enum(terminal/rich/dom), 슬롯별 `renderModeOverride` + `window.__engramLayout.setRenderMode/clearRenderMode`(§5). 비오버라이드 동작 byte-identical. ② `DomSlot`(신규): 출력 스트림을 `<pre data-dom-mode>`에 평문 → cdp eval/LLM이 읽음(xterm은 WebGL 캔버스라 안 읽힘). 읽기전용, TerminalSlot 구독 규율 미러. ③ review(code full) FIX 5건 반영.
2. **원래 버그 진단(핵심 — 아래 정정 필독).**

## ★ 핸드오프 정정 (지난 핸드오프가 틀렸음) ★
- 지난 핸드오프의 후보 1~3(러스트 replay enqueue silent-drop / 빈 slots_for_window / registry 가시성)은 **라이브 실측으로 전부 뒤집힘.** 다시 그거 쫓지 말 것.
- 지난 핸드오프 "재배정하면 전량 복원(유실 0)"은 **틀렸음** — 이번 실측에선 재배정으로도 pre-reload 출력 복원 안 됨.
- **실제 메커니즘:** 클라 뷰버퍼(core `OutputViewStore`)는 데몬 authoritative 데이터의 순수 캐시. 리로드/재구독 시 클라가 데몬에 from-oldest replay를 재요청 안 함 → 캐시가 비거나 낡으면 두절. 데몬은 재전송 능력 있음(`Subscribe{after_seq:None}`→`ReplayKind::FromOldest`, 확인함). 리로드 경로(`subscribe_output` fresh)는 클라 캐시만 replay.
- **미해결 조사 충돌(하지만 픽스엔 무관):** 라이브는 "리로드가 캐시를 비운다", 정적 트레이스는 "리로드 경로에 폐기 코드 없음"(폐기는 layout DropSlots/재연결 sweep에서만)로 어긋남. 성공 경로 무로그라 못 가림. **데몬 재-hydration 픽스는 이 충돌과 무관하게 옳음**(원본에서 재구축하니까).

## 큰 미결 = 리로드 두절 정공 픽스 (결정 대기)
- **왜 중요:** dev(HMR) 성가심 아님 — "데몬=영속 원본, 클라=attach/re-attach"(ADR-0029) 구조에서 **re-attach 시 출력 복원은 근본 기능**(tmux attach 스크롤백처럼)인데 깨져 있음. 웹뷰 리로드·클라 재기동 둘 다 물림.
- **권장 픽스:** 리로드/재등록(`subscribe_output` fresh) 시 클라 캐시만 replay하지 말고 **데몬에 `Subscribe{after_seq:None}`(from-oldest) 보내 뷰버퍼 재구축.** 재구독 전 캐시 클리어(중복 배달 방지) + replay→live 순서 보존(ADR-0043).
- **바꾸는 불변식:** ADR-0040(데몬 재구독=tail-only, axis A)를 **리로드 케이스 한해 from-oldest 예외** → 새 ADR(0040 부분 개정).
- **거부한 대안:** 좁은 패치(캐시 폐기 방지) = 클라 재기동 미커버 + 폐기 경로 미확정이라 불안정.
- **리스크:** 데몬 구독 semantics(중복구독·순서역전) 미묘 → 솔로로 끝까지 짜면 위험. OSS(tmux/mosh/ttyd re-attach) 조사 선행 권장(ADR-0038).

## 검증 상태 (쌍)
**돌린 것(재실행 명령):**
- `npx tsc --noEmit` PASS · `npm test`(vitest) **194** PASS · `cargo test` **195** PASS · 코어 격리 `rg "use tauri" crates/engram-dashboard-core/src/` 0줄 PASS.
- GUI 실측(cdp, 포트 9223): DOM 모드 토글→마운트 · 라이브 입력("QAOK") DomSlot 렌더 · 토글 복귀 · 무효 mode 거부(FIX-4) 전부 PASS.

**검증 안 된 것(오신뢰 금지):**
- **리로드 두절 픽스 = 미구현**(설계 결정 대기).
- DOM 모드 backfill-on-swap = **의도적 없음**(라이브-forward만). 라이브 에이전트에 DOM 토글 시 스왑 전 출력 안 보임 — known-limitation(커밋 메시지·DomSlot 헤더 명시). 같은 뿌리=리로드 replay.
- `cargo fmt --check` FAIL(선재, ws_e2e.rs, 내 밖). `cargo build` daemon 링크 실패=실행 중 exe 잠금(인프라, 코드 무관).

## 실패한 접근 (do-not)
- 후보 1~3 재추적 금지(위 정정).
- 라이브-정적 폐기경로 충돌 해소하려 삽질 금지 — 재-hydration 픽스가 우회함.
- **SendMessage 툴 이 하네스 없음** — 조사 에이전트 이어받기 불가(매번 새로 스폰).
- **`/implement` 스킬 디스크에서 제거됨**(세션 목록엔 잔상만). 수동 코더→review→qa로 태울 것.
- daemon 파일(ws_e2e.rs 등) 리로드 픽스 아니면 손대지 말 것(딴 에이전트/영역).

## 참조 (읽을 것만)
- 커밋물: `src/components/slot/DomSlot.tsx` · `renderMode.ts` · `components/layout/ViewLayoutRenderer.tsx` · `store/viewStore.ts`.
- 리로드 픽스 트레이스 대상: `src-tauri/src/commands/agent.rs`(subscribe_output→replay_slots fresh) · `crates/engram-dashboard-core/src/daemon_client/mod.rs`(replay_slots/try_enqueue) · `connection.rs`(ReplaySlots arm) · `output_view_store.rs`(캐시·resubscribe_slot) · protocol `messages.rs`(Subscribe after_seq) · core `output_core.rs`(subscribe_from→FromOldest).
- ADR-0040(axis A tail-only) · 0041(구독 소유권) · 0043(mount-replay) · 0029(daemon-only) · 0038(결함 OSS 선조사).
- cdp: `window.__engramLayout.setRenderMode(nodeId,'dom'|'rich'|'terminal')` / `disableDomMode(nodeId)` · 입력 `invoke('forward_daemon_command',{cmd:{WriteStdin:{agent_id,data:[bytes],request_id}}})`.
