# 핸드오프: ADR-0067 우클릭 컨텍스트 메뉴 에이전트 배치 + 검색 팝업 구현 완료(커밋) · click-to-focus·ADR-0066 개정 포함 · 25커밋 미푸시

## 한 줄 상태 · 다음 첫 액션
- **상태:** 슬롯 배치 UX 재설계 대화 세션. focus-then-place(ADR-0066 결정 2)의 **focus-steal 결함**(트리도 slot이라 트리 클릭이 포커스 뺏음) 발견 → **우클릭 컨텍스트 메뉴 배치(ADR-0067)**로 재설계·구현·GUI 실측 완료. **이번 세션 4커밋 + 이전 누적 = origin/master 대비 총 25커밋 미푸시.** 미커밋 = 이 핸드오프 파일뿐.
- **다음 첫 액션:** 큰 갈래 열림, **사용자 선택** — (a) **푸시**(25커밋 미푸시, 오래 밀림) · (b) **드래그앤드롭 배치**(ADR-0067 후속 sugar, 같은 assign 위) · (c) **slot geometry 노출**(ADR-0066 결정 3, LLM 공간 배치 — 프론트 getBoundingClientRect vs 백엔드 논리좌표 미결).

## 무엇이 됨 (이번 세션 4커밋)
- `9cc923a` docs(adr): ADR-0066 크로스-윈도우 focus active 타깃 해소(§결정 5, last-focused-wins 파생 모델) + step-log
- `dd8a02b` feat(layout): 슬롯 **click-to-focus**(ADR-0066 결정 1) — `set_focused_slot` + `focus_slot` command + `slot.focus` registry + 슬롯 onClick + data-slot-id
- `b803349` docs(adr): **ADR-0067** 우클릭 컨텍스트 메뉴 배치 (ADR-0066 결정 2·5 부분 폐기) + index + step-log
- `5e34178` feat(slot): 우클릭 **"에이전트 모니터링" 검색 팝업**으로 배치(ADR-0067) — AgentMonitoringPicker + monitoringPickerStore + 필터 + 슬롯 "생성" 제거

## 검증 상태 (쌍)
- **돌린 것:** 각 코드변경 `/implement standard`(코더 Opus → `/review code full`[doc-aware reviewer-deep + cross-family Codex] → `/qa`). 모니터링 배치 리뷰에서 Codex **FIX 1**(팝업 재열림 stale 상태) → **key-remount**(store `openId` → `<AgentMonitoringPicker key={openId}>`)로 수정 → 재검 PASS. 최종: `npx tsc --noEmit`=0 · `npm test`=**491** · 격리 `rg "use tauri" crates/.../src/`=주석1(실 import 0) · `cargo build --lib` OK · **GUI 실측 PASS**(CDP 9223): click-to-focus(클릭·`slot.focus`·65% 링 이동 ~30ms) / 모니터링(팝업 열림·필터 match "kim"→kimsunzun·no-match "검색 결과 없음"·**선택→슬롯 콘텐츠 `{type:agent}` 배치**·선택후 닫힘·hideOn 트리슬롯 모니터링 없음·트리 "에이전트 생성" 유지).
- **★do-not★:** bare `cargo test`·`cargo test -p engram-dashboard`/`--lib` = 0xc0000139(WebView2Loader 사망). member-scoped만(`-core`/`-protocol`). src-tauri 레이아웃 로직 = `cargo build` + GUI 실측이 정본.
- **검증 안 된 것:** 경로 2(트리 우클릭 "열기"=`openInFocusedSlot`)는 코드리뷰+unit만(변경 없음), GUI 재실측 안 함. "새 콘텐츠" 서브메뉴 flyout의 생성-제거는 unit+리뷰만(CDP synthetic hover로 flyout 확장 안 잡힘). 멀티창(팝업)에서 모니터링 배치 미실측.

## 실패한 접근 / 주의 (do-not 재시도)
- **focus-then-place(ADR-0066 결정 2) = 폐기.** 트리도 slot → 트리 클릭이 포커스를 트리 slot으로 뺏어 "포커스 슬롯 배치"가 트리 자신 가리킴(focus-steal). ADR-0067이 우클릭 메뉴로 대체. 재론 금지.
- **크로스-윈도우 `last_focused_window` 백엔드 추적 = 안 만듦.** 우클릭이 target-explicit이라 불필요(ADR-0067 거부). ADR-0057이 제거한 전역 `active_view_id` 부활도 금지.
- **click-to-focus(결정 1)는 살아있음(폐기 아님)** — 시각 선택 지시자, 배치 역할만 벗음(ADR-0067 재해석). 65% 링·`set_focused_slot`·`slot.focus` 그대로 유효.

## 정지 조건
- **앱 재시작 = 이번 세션에서만 자유 승인**(사용자 standing grant "하지 말라 할 때까지"). **이 grant는 세션 한정 — 다음 세션은 만료, 재확인 필요.**
- 비자명 코드 = `/implement`(코더→review→qa) · 굵은 결정 = ADR(`/adr`) · 설계 서베이 = `/research`. 메인 직접 구현 금지. 레이아웃/시각 = eval 아닌 GUI 실측.
- **현재 앱 실행 중**(CDP 9223, background task `bpwja47gh`). 내 GUI 실측이 부팅 빈 슬롯에 agent `kimsunzun` 배치해둠(무해한 테스트 흔적). 데몬은 client 재시작에도 생존(kimsunzun 유지 중).

## 미결 / 다음 갈래
- **slot geometry(ADR-0066 결정 3, 미구현):** 각 슬롯 `{id,x,y,w,h}`를 control surface로 노출해 LLM이 "우하단" 등 공간지시를 slot id로 번역. **프론트 `getBoundingClientRect` vs 백엔드 논리좌표** 미결(구현 시 결정).
- 드래그앤드롭 배치(ADR-0067 후속, 같은 assign 위) · 방향 sugar 커맨드 · 키보드 방향 포커스 이동.
- **푸시**(25커밋 미푸시).

## 참조 (읽을 것만)
- **ADR:** **0067**(우클릭 배치 — 정본) · **0066**(결정1 click-to-focus 생존 · 결정2/5 부분폐기 · 결정3 geometry 미구현) · 0064/0065(슬롯 메뉴 단일 기여 API·hideOn/children) · 0011(assign_agent) · 0060(SlotContent) · 0057(창별 tab·active) · 0035(레이아웃 권위). CLAUDE.md §5(LLM-우선 제어).
- **코드 포인터:** 배치 = `src/components/slot/AgentMonitoringPicker.tsx`·`monitoringPickerFilter.ts`·`src/store/monitoringPickerStore.ts`·`src/commands/slotContentCommands.ts`(`slot.assignRunningAgent` + 메뉴 기여·생성 제거)·`src/components/layout/WindowLayout.tsx`(picker 마운트)·`AgentList.tsx:151`(openInFocusedSlot=경로2). focus = `src-tauri/src/layout/manager.rs`(`set_focused_slot`)·`commands/layout.rs`(`focus_slot`)·`src/store/viewStore.ts`(`focusSlot`/`assignAgent`).
- **step-log:** 최근 3항목 — "슬롯 click-to-focus" / "크로스-윈도우 해소" / "슬롯 콘텐츠 배치 재설계(ADR-0067)".
- **GUI 검증:** `scripts/cdp.mjs`(포트 9223) — eval(DOM/invoke)+shot. 기동 = `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev`(백그라운드).
- **skill feedback(이번 세션 기록):** `/adr` 부분폐기가 폐기 마커를 폐기당한 ADR 상태줄 아닌 관련줄/index에만 박음(CLAUDE.md rot 규칙과 미세 어긋남 — adr feedback.md 2026-07-11 항목, 상태줄 수동 보강함).
