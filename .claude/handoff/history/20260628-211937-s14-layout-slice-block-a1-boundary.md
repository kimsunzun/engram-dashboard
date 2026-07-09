# 핸드오프: S14 레이아웃 슬라이스 — 2차 BLOCK 미커밋 + a1 경계충돌 대기

## 한 줄 상태 + 다음 첫 액션
S14 모듈②(레이아웃 코어, 백엔드 done)를 프론트에 잇는 **첫 수직 슬라이스(split 커맨드 루프 + 부팅 기본뷰 표시)** 가 working tree에 있으나 **미커밋** — 2차 코드리뷰(Codex)에서 좁은 잔여 결함으로 BLOCK. 주요 결함(version 의미론 global-vs-per-view·비-active view 캔버스 탈취)은 캐시 모델로 **이미 닫힘**.
**다음 첫 액션 (코더 한 라운드 → 커밋·push):**
1. `viewStore.initFromBackend` **init race** — `list_views`→await→`get_view` 2스텝 중 도착한 emit을 늦게 끝난 stale init이 덮는 것 가드(프론트 토큰/superseded 플래그, 백엔드 무변경).
2. `subscribeViewEvents` **dispose-during-await 누수** — `disposed` 플래그를 `listen()` await 해소 후 확인해 즉시 unlisten.
3. 위 2개 **비-vacuous 테스트**(await 사이 emit 주입 / pending 중 dispose).
4. **`AppLayout.tsx:47` slotStore strand은 고치지 말 것 — 의도된 이주 경계**(오너 "옛 화면 필요없음" 결정). 주석만.
→ 그 후 `/qa`(GUI cdp 실측: 부팅 시 기본 View 1 빈 슬롯 1개 뜨는지) → 커밋·push.
(이 코더 스펙 초안은 직전 세션 막판에 작성됨 — 거의 그대로 재사용 가능. spawn 직전 오너가 "컨텍스트 교체" 지시로 중단.)

## repo 상태
- HEAD = master `6e1d307` (방금 **wip/a1→master 머지**: RichSlot lab·TerminalSlot fix(ADR-0036)·커맨드/메시징 OSS 리서치2). origin push 동기 여부는 확인 필요.
- **미커밋 = 레이아웃 슬라이스:**
  - MODIFIED: `src-tauri/src/commands/layout.rs`(`list_views` read-only 핸들러 추가) · `src-tauri/src/lib.rs`(등록) · `src/components/layout/AppLayout.tsx`(메인 캔버스=ViewLayoutRenderer 고정, slotStore 폴백 제거) · `src/store/eventBus.ts`(subscribe await 후 initFromBackend).
  - NEW: `src/store/viewStore.ts`(캐시 모델: `layouts: Record<view_id,{layout,focusedSlotId,version}>`, active만 렌더, 전역 단조 version 가드) · `src/store/viewStore.test.ts` · `src/components/layout/ViewLayoutRenderer.tsx` · `src/api/layoutTypes.ts`.
  - working tree 보존(안 날아감). green이지만 BLOCK이라 커밋 안 함.
- **이번 세션 커밋·push 완료(머지 전 origin/master 160c5eb까지):** T4 재연결+취소 + 하네스 auth-count 버그수정(`04f635b`) · step-log(`cc50053`) · test(c)=`supersede_connect_cancels_reconnect_before_discovery` multi_thread 재작성+게이트순서 버그수정 un-ignore(`49be0e9`) · step-log(`160c5eb`). 전체 스위트 **111 passed / 0 ignored**.
- **Engram repo(별도, `I:/Engram`) `739930e`:** `core/claude-global-shared/rules/global-rules.md`에 "~50% 컨텍스트 → 현 작업 마무리 + 핸드오프 제안" 룰 추가·push.

## 검증 상태 (쌍)
- **green·커밋됨:** T4 · test(c) — `cargo test -p engram-dashboard --lib` = 111 passed. 재실행 동일.
- **레이아웃 슬라이스(미커밋):** 코드 게이트 green — `npx tsc --noEmit` · `npm test`(vitest 106) · `cargo test -p engram-dashboard --lib`(111) · `cargo fmt --check`. **단 2차 Codex 재리뷰 = BLOCK**(아래). GUI cdp 실측 안 함.
- **검증 안 됨:** 슬라이스 GUI 실측(cdp) · 부팅 기본뷰 실제 렌더 · init/dispose 가드(아직 미수정).

## 2차 리뷰 BLOCK findings (다음 세션 닫을 것)
- `src/store/viewStore.ts:121` — `list_views` init pull에 staleness 가드 없어 stale init이 `activeViewId`를 옛 뷰로 되돌릴 수 있음.
- `src/store/viewStore.ts:122` — `get_view`가 stale `active_view_id`로 — init 중 view 전환 시 list/layout 불일치.
- `src/store/eventBus.ts:75` — `listen()` pending 중 dispose/HMR → 리스너 누수(disposer 미발급 창).
- `src/components/layout/AppLayout.tsx:47` — slotStore strand = **수용(의도된 이주 경계, fix 아님).** PopupPage/TreePage/SlotContextMenu는 아직 slotStore 사용 — 전체 이주는 별도 다음 슬라이스.
- `src/store/viewStore.test.ts:258,316` — 위 race를 실제로 커버 못 함 → 비-vacuous로 교체.

## 실패한 접근 / do-not
- **Codex 모델명:** `gpt-5.2-codex`는 ChatGPT 계정서 미지원 → `mcp__codex__codex` 호출 시 `model` 생략(기본 모델).
- **cargo 동시 실행 금지:** 중복 백그라운드 cargo/test → 빌드락 데드락 + hung 테스트 바이너리. **한 번에 하나.** hung kill = PowerShell `cmd /c "taskkill /F /T /PID <id> ..."`(bash는 `/F`를 `F:/`로 오인). 후 `target/debug/.cargo-lock` 제거.
- **coder spawn "거절"이 실제 실행된 사례 있음** → 결과는 working tree로 직접 검증, 자기보고 불신.
- **over-loop 경계:** 이 슬라이스 코더 3라운드 + 리뷰 2라운드(BLOCK 2회) 소요. 잔여는 좁음 — 마지막 라운드로 닫고 커밋, 그 이상 갈지 말 것(handoff 교훈 #5).
- **green ≠ correct:** 첫 version-guard 테스트가 *틀린 불변식*을 박아 green이었음(리뷰가 잡음). 테스트 통과가 정답 아님.

## 결정된 것 (이번 세션)
- **공용 용어(통일):** **Server**(① 데몬/에이전트 호스트) / **Client**(② src-tauri Rust, 레이아웃 권위+invoke 핸들) / **webview**(③ React UI). "셸" 폐기. (Tauri 공식 ②="Core"=우리 "Client".) **코드·문서 일괄 적용은 미실시** — 별도 저우선 작업.
- **부팅 뷰 = 옵션 (a)/option-3:** 부팅 시 `ViewLayoutRenderer`로 백엔드 기본 View 1 표시(`list_views` pull). 옛 slotStore 메인 UI 폐기("옛 화면 필요없음").
- **글로벌 룰 추가:** ~50% 컨텍스트 → 현 단위 마무리 + 핸드오프 제안.
- **park(저우선, "중요하지 않다"):** 부팅 빈화면 튜토리얼/우클릭 안내, 배포판 CLAUDE.md 사용가이드.

## a1 트랙 (dashboard1, worktree `engram-dashboard-a1`, 브랜치 wip/a1 — 만지지 말 것)
- a1이 메시징 data-plane PRD + ADR-0014 작업. PRD 초안 `d9ed173`(`docs/research/messaging-data-plane-prd-draft-2026-06-28.md`; wip/a1→master 머지로 master에도 있을 수 있음).
- **a1 PRD = /review prd BLOCK + a1이 구조적 경계 escalate (2차 경계충돌):**
  - ① 응답인식(PTY stdout→reply 상관)을 비목표로 미뤘으나 Handoff/Approval/command_rpc가 전부 의존 → 빈 껍데기.
  - ② command_rpc는 control-plane인데 a1이 data-plane으로 묶음 = control(protocol/메인)↔data(messaging/a1) 경계가 같은 개념을 두 군데서 자름.
  - 뿌리: a1 design이 메인 소유 코어(protocol/ViewManager) 위에 강결합 → 독립 설계가 막힘. (1차 충돌 = 레이아웃 커맨드 vs ViewManager/ADR-0035.)
- **상태:** 메인이 standby ack 보냄(orch pane 10, `⟁ds11!`). 오너가 "db1 놔두고 할거하자" → 경계 세션 **연기**. a1은 PRD 추가 보류·현 초안 유지.
- **미결(오너 결정, 연기):** (a) control(protocol)↔data(messaging) 경계선 위치, (b) 응답인식 소유자(현재 무주공산).

## 멀티트랙 메타 (다음 세션 인지)
양 트랙이 같은 세션에 BLOCK: 메인 슬라이스(slotStore 이주 *경계* 미설정으로 strand)·a1(control/data *경계* 미설정). **공통 뿌리 = 경계 안 긋고 구현 먼저.** db1 제안 = "경계부터 같이 긋자"(오너 주도). **권장 다음 순서:** ① 메인 슬라이스 좁은 fix 마무리·커밋(독립, 경계와 무관) → ② 오너 주도 경계 세션(control↔data + slotStore 이주 + 응답인식 소유).

## 참조 (읽을 것만)
- 슬라이스 파일 8개(위). 핵심 = `src/store/viewStore.ts`(캐시 모델·가드) · `src/store/eventBus.ts`(subscribe/init 순서).
- 로드맵: `docs/process/S14-multi-page-layout/module1-transport-spike.md`(T5 OutputRouter / T6 invoke / T7 TauriTransport), `trd.md`. ADR-0035/0036/0037(레이아웃 권위=src-tauri) · ADR-0006(락) · ADR-0011(agentClient seam=에이전트 명령 전용; 레이아웃은 직접 invoke).
- 메시징/제어 리서치: `docs/research/control-surface-and-fleet.md` · `…/agent-messaging-survey-2026-06-28.md` · `…/llm-control-surface-message-command-scope-2026-06-28.md`(wip/a1 또는 머지된 master).
