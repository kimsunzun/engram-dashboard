# 핸드오프: 슬롯 UX 다듬기 5커밋(메뉴 정리·팝업 일반화·분할플래시·포커스링) + ADR-0066 포커스/배치/geometry 설계 확정(구현 미착수)

## 한 줄 상태 · 다음 첫 액션
- **상태:** incremental UI/UX 다듬기 대화 세션. **5커밋 완료·게이트 통과**(master 로컬, **푸시 안 함**). 미커밋 = 이 핸드오프 파일뿐. 다음 큰 갈래 = **ADR-0066 구현**(포커스/배치/geometry 제어 표면 §5) — **설계만 확정, 코드 미착수**.
- **다음 첫 액션:** ADR-0066을 정본으로 **focus 제어 표면 첫 슬라이스** 구현 — 백엔드 `manager.set_focused_slot` + `focus_slot` command + 프론트 슬롯 div `onClick`→`viewStore.focusSlot`→invoke + `slot.focus` registry 등록. `/implement`로(backend Rust → 재빌드·앱 재시작은 **사용자 승인 후**).

## 무엇이 됨 (커밋 — e2e4309 위 5개)
- `a344349`+`fc3eb41` **빈 슬롯 메뉴 정리(ADR-0065, 0064 부분개정):** 콘텐츠-채움 3항목(트리/팔레트/생성)을 `새 콘텐츠 ▶` 1단 서브메뉴로, `slot.empty`/`slot.popout`에 `hideOn:['empty']`로 빈 슬롯에서 숨김. descriptor에 `hideOn?`/`children?` 추가, `buildSlotMenu`가 hideOn 필터(최상위+자식)·children 1단 resolve·빈-컨테이너 skip·1단 nesting/XOR shape 검증, `validateSlotMenuContributions` 부팅 shape 검증 재귀. fail-loud=console.error+skip(crash-free). 파일: `src/commands/{slotMenu,slotCommands,slotContentCommands}.ts`·`src/components/slot/SlotContextMenu.tsx`.
- `ae9c134` **팝업 분리 일반화(ADR-0064 백엔드 완성):** `move_slot_to_window`가 agent_id 아닌 SlotContent 전체 이동(`prepare_detached_view`=`set_in_tree`, `(ViewId,SlotContent)` 반환), still_ours 가드=`slot_content==src_content`(agent는 value-identical), 구독델타=`collect_agents`가 Agent만 라우팅해 non-agent는 empty-by-construction. Empty만 거부. 파일: `src-tauri/src/commands/popout.rs`·`layout/{manager,tree,types}.rs`.
- `7ae7d11` **분할 흰 플래시 제거 + 포커스 링 은은화:** `html,body,#root{background:var(--bg,#0a0a0a)}`(투명 pane이 WebView2 흰배경 비추던 플래시 차단) · 포커스 슬롯 = border 항상 `1px --border` + `inset box-shadow color-mix(accent 65%)`(레이아웃 이동 0). 파일: `src/index.css`·`src/components/layout/ViewLayoutRenderer.tsx`.
- `25834c7` **ADR-0066** 포커스/배치/geometry 제어 표면 설계 결정(구현 아님).

## 검증 상태 (쌍으로)
- **돌린 것:** 각 코드 변경 `/implement`(코더→`/review code`→`/qa`). review = doc-aware Opus(reviewer-deep) + cross-family blind Codex 2인(ae9c134·fc3eb41=full, polish=light). 최종 = `cargo build` 링크 OK · `npx tsc --noEmit`=0 · `npm test`=**482** · 코어 격리 `rg "use tauri" crates/.../src/`=0 · **GUI 스샷 실측**(빈 슬롯 메뉴 4항목+서브메뉴 flyout+우측 clamp / 팝업 agent_list·preset 새 창 렌더+**agent 팝업 회귀 없음**(스폰→팝업→라이브 터미널·구독 이관) / 포커스 링·플래시 bg computed 불투명). **재실행 = 그대로.**
- **★do-not(재실행 시)★:** bare `cargo test`·`cargo test -p engram-dashboard`/`--lib` = 0xc0000139(WebView2Loader 사망). **member-scoped만**(`cargo test -p engram-dashboard-core`/`-protocol`). **src-tauri 레이아웃/라우터 로직 단위테스트는 이 배리어로 실행 불가 → cargo build + GUI 실측이 정본.**
- **검증 안 된 것:** 포커스 링 **65% 값은 40% GUI실측 뒤 리뷰 후 값만 bump** → unit 통과+reasoning으로만, 65% 자체 스샷 재실측 안 함(시각 확인 = 다음 세션/사용자). 멀티창 focus/geometry 동작 미실측.

## 실패한 접근 / 주의 (do-not 재시도)
- **팝업 MOVE가 전역 뷰(agent_list/preset_palette)에선 main을 비운다** — 부팅 `Split{트리|빈슬롯}`에서 트리 팝업 시 형제(빈슬롯) 승격 → main 전체가 빈 슬롯. 사용자가 "이상하다" 제보 → **접어둠(미해결)**. 사용자 제안 "close+팝업 따로"는 현재 MOVE와 동일 동작이라 해결 아님. 진짜 해법 후보 = **전역 뷰는 COPY**(원본 유지+새 창, agent만 MOVE) — 미결(ae9c134=MOVE 커밋됨). 재론 시 이 맥락 깔 것.
- ADR-0065 구현 때 코더가 무관한 `// ADR-0065` 앵커를 flash/focus 코드에 넣음 → 리뷰가 잡아 제거. **비-ADR급 폴리시엔 ADR 앵커 넣지 말 것.**

## 정지 조건 (stop conditions)
- **ADR-0066 구현 = backend Rust 포함 → 재빌드 필요. 앱/데몬 exe 락 → 종료·재시작은 사용자 승인 후**(이번 세션 사용자가 직접 종료·재시작 — 과거 승인 미래로 확장 안 됨, 다시 확인). **현재 QA가 앱을 다시 띄웠고 테스트 에이전트 1개 + 팝업 창 여럿 열어둔 상태**(CDP 9223, 정리 안 됨).
- 비자명 코드 = `/implement`(코더→`/review code`→`/qa`), 메인 직접 구현 금지. 굵은 결정 = ADR. 설계 서베이 = `/research`.
- **레이아웃/시각 변경은 eval 치수 아닌 스크린샷으로 검증.**
- **geometry 노출 설계 디테일(ADR-0066 미결):** 백엔드는 논리 트리(비율/분할)만 알고 픽셀 rect는 프론트 Allotment 렌더 소관 → 슬롯 geometry `{id,x,y,w,h}`를 **프론트 getBoundingClientRect로 노출할지 백엔드 논리좌표로 계산할지 구현 시 결정**.

## 참조 (읽을 것만)
- **ADR:** **0066**(포커스/배치/geometry §5 — 다음 세션 정본, 거부 대안·근거 有) · 0065(메뉴 descriptor hideOn/children) · 0064(슬롯 메뉴 단일 기여 API, 0065가 부분개정) · 0057(팝업 MOVE/detach) · 0063(set_slot_content) · 0035(레이아웃 백엔드 권위·낙관갱신 금지) · 0011(assign) · 0022/0055(command registry). CLAUDE.md §5(LLM-우선 제어).
- **step-log:** `docs/process/step-log.md` — "빈 슬롯 메뉴 정리" / "팝업 분리 일반화" / "슬롯 UI 폴리시" / "슬롯 포커스·배치 제어 표면" 항목.
- **코드 포인터(ADR-0066 구현 길목):** 포커스 = `src-tauri/src/layout/manager.rs`(`focused_slot_id`·`fixup_focus`; **`set_focused_slot` 신설**) · `src-tauri/src/commands/`(**`focus_slot` command 신설** + `lib.rs` generate_handler 등록) · `src/components/layout/ViewLayoutRenderer.tsx`(슬롯 div **onClick 신설**·`isFocused`=52·border/boxShadow=84~) · `src/store/viewStore.ts`(**`focusSlot` 신설**) · `src/store/eventBus.ts`(`__engramLayout` 노출) · `src/commands/slotCommands.ts`(**`slot.focus` 등록**). 배치 = 트리 항목 활성화→`assignAgent`(ADR-0011). geometry = 위 설계 디테일.
- **GUI 검증:** `scripts/cdp.mjs`(포트 9223) — eval(DOM/invoke)+shot. 앱 기동 = `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev`(백그라운드), 콜드+Rust빌드 ~2분.
