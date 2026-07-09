# 핸드오프: 슬롯 팝업창 분리 기능 — 구현+deep리뷰까지, 미커밋 known-bad(4결함 수정 필요)

## 한 줄 상태 · 다음 첫 액션
- **상태:** B(레이아웃) 재설계 중. **Brick 1 커밋 완료**(`297796e`). 그 위에 **"슬롯을 런타임 팝업 OS창으로 분리" 기능을 구현했으나 `/review code deep`이 실결함 4개 적출 → 미커밋 known-bad WIP.** 사전조사(/research)로 기술 실현성은 확인됨(하드 블로커 없음).
- **다음 첫 액션:** **아래 "팝업 WIP 4결함"을 코더로 재작업**(`/implement` 이어가기 — 이미 critical 강도, 루프 1/2 남음) → `/review code deep` 재리뷰 → **GUI 실측(데몬 재시작+리빌드 필요)** → 커밋. 워킹트리에 팝업 코드 그대로 있으니 **처음부터 짜지 말고 그 위에서 4결함만 수정.**

## ★★ 팝업 WIP — 미커밋 known-bad (유실 금지 · 이어서 수정) ★★
`297796e`(Brick 1) 위에 얹힌 **미커밋 변경**. 접근법은 deep리뷰 2인 모두 건전 확인(build()가 락 밖=데드락/ADR-0006 OK · 카운터 atomic · 코어 격리 · capability 정합 · Destroyed 멱등). **아래 4결함만 고치면 됨.**

**미커밋 파일:** 신규 `src-tauri/src/commands/popout.rs`·`src-tauri/capabilities/popup.json`·`src/pages/PopoutPage.tsx` / 수정 `src-tauri/src/{lib.rs,layout/manager.rs,commands/mod.rs,gen/schemas/capabilities.json}`·`src/{App.tsx,store/viewStore.ts,store/eventBus.ts,components/slot/SlotContextMenu.tsx,components/layout/ViewLayoutRenderer.test.tsx}`·`docs/process/step-log.md`(코더가 추가 — **완성처럼 서술됐을 수 있음, known-bad로 정정 필요**).

### 고쳐야 할 4결함 (deep리뷰 = doc-aware Opus FIX + cross-family Codex BLOCK 취합)
1. **★비원자 MOVE 경쟁 (Codex, 심각)★** — `popout.rs`: 창 `build()` 동안 락을 놓은 뒤 `close_slot(view_id, slot_id)`를 무검증 실행(≈line 156). build 중 원 슬롯이 재할당되면 **엉뚱한 에이전트 삭제.** 같은 슬롯 동시 pop-out 2회 = 같은 에이전트 팝업 2개. **수정:** close 전 그 슬롯이 여전히 같은 agent를 들고 있는지 재검증(두번째 락 안에서), 아니면 skip.
2. **★팝업 View 누수 (Codex #2 + doc-aware F1)★** — 창 닫을 때 `cleanup_popup_window`(`popout.rs`≈215)가 window_bindings·channel은 지우지만 **생성한 `new_view_id` View를 안 지움** → orphan "Popup N" View 누적, `switch_view`로 되살아나면 main에 에이전트 재등장. + 탭바(Brick 2) 생기면 유령 탭. **수정:** cleanup이 `close_view(bound view)`도 수행 + `view:list-updated` emit. **추가로** `view_metas`/탭목록이 `window_bindings.values()`(바인딩된 View)를 제외하게(라벨-agnostic 필터) — Brick 2가 유령탭 물려받지 않게.
3. **팝업 내 우클릭 메뉴가 main View를 건드림 (Codex #3)** — `PopoutPage.tsx`가 `ViewLayoutRenderer`에 자기 `viewId`를 안 넘겨서, 내부 `SlotContextMenu`가 global `viewStore.activeViewId`(=main) 사용(`SlotContextMenu.tsx:24`) → 팝업 안 분할/닫기/분리가 **엉뚱한 View 타격(SlotNotFound/오변형).** **수정:** 팝업은 자기 URL viewId를 명시 전달(ViewLayoutRenderer/SlotContextMenu가 viewId override 받게).
4. **`bind_window`가 무방비 공개 커맨드 (Codex #4)** — 임의 label+view_id 바인딩 허용(invoke_handler 노출, `lib.rs`). `{label:"main", 임의뷰}`로 라우팅 오염 가능. **수정:** `bind_window`를 **내부화**(pop_out_slot만 호출, invoke_handler에서 제거) — §5 노출은 `popOutSlot`만. (또는 label prefix 검증.)
- **(선택) F2 대칭 갭:** 팝업 backing View가 `close_view`(LLM)로 닫히면 팝업 창이 stale로 남음 → PopoutPage가 자기 view 사라지면 self-close, 또는 close_view가 바인딩된 View 거부. 결함2 수정과 함께 고려.
- **F3 (경미):** create_view→assign_agent 사이 실패 시 orphan view(현재 assign 실패 불가라 무해) — 가드 주석 한 줄.

### 검증된 NON-이슈 (다시 파지 말 것 — deep리뷰가 확인)
build() 락 밖(ADR-0006 OK, 데드락 없음) · PopupCounter atomic(중복 라벨 경쟁 없음) · `daemon_close`는 unload에 안 걸림(팝업 닫아도 공유 DaemonClient 안 죽음) · capability `popup.json`(`slot-popup-*` glob, core:default) 정합·과광 아님 · Destroyed 멱등·이중정리 안전 · agent-tree/main은 Destroyed arm이 안 잡음(`is_popup_label` = slot-popup- prefix) · 코어 crate tauri-import 0 · 프론트/Rust 테스트 non-vacuous.

### 미검증 (반드시 GUI 실측)
compose-flow(create→assign→bind→build→close)+cleanup은 **자동 커버리지 0**(src-tauri lib 테스트는 선재 0xC0000139 로더 이슈로 실행불가). **데몬 락 해제 + fresh 빌드 후 cdp로:** 팝업 분리→새 창에 에이전트 출력→3개+ 공존→창 닫기 시 window_bindings/channel/View 정리(누수 0) + main/agent-tree 라우팅 무손상.

## 사전조사 결론 (재조사 불요 — /research medium 완료, 근거 있음)
Tauri v2 런타임 동적 창 **실현 가능, 하드 블로커 없음.** 확정 사실:
- 창 생성은 **반드시 `async` 커맨드**(Windows sync 데드락, docs.rs 확인).
- 동적 창 라벨은 **capability `"windows":["slot-popup-*"]` glob** 필요(IPC 접근).
- 우리 라우팅(`OutputRouter`/`WindowChannelRegistry`)은 **label-agnostic** → `subscribe_output`(자동주입 `window.label()`)로 동적 창 오늘도 붙음. 유일 신규 = window_bindings 채우는 배선.
- 정리 = `WindowEvent::Destroyed`(라벨 단위). 정상 닫기 커버, 강제 process-kill은 상태 동반사망(무해).
- **empirical(문서 미보장, 구현서 확인):** (a) dev 해시라우트 로드 = **라이브 PASS**. (b) CDP가 동적 창 열거 = 메커니즘 확인(agent-tree 창 이미 열거)이나 **라이브 미검**(구 바이너리+데몬락) — GUI 실측 때 확인, 안 보이면 cdp.mjs가 URL/view로 타깃 선택하게 수정.

## B(레이아웃) 로드맵 (사용자 승인)
- **밀스톤 종착 시나리오:** 튜토리얼 클로드 생성 → 채팅으로 C드라이브 A/B/C/D 스폰 → A는 현재 창·C/D는 **새 팝업 창** → A→C 메시지. (이번 팝업 기능이 "새 창" 실현.)
- Brick 1 ✅ 커밋 · **팝업창 분리(현재 WIP, 4결함 수정 중)** · 탭바(나중, "여지만") · 튜토리얼 클로드(cwd=exe) · 오케스트레이션.
- **"새 창" 결정 = 동적 다중 창(3개+)** 확정(사용자). detach="이동"(원슬롯 main서 제거, 미러 아님).

## repo / 앱 상태
- 브랜치 master. top=`297796e`. **미푸시 9커밋**(push는 하네스가 master 직접 차단 — `! git push origin master`/승인 필요).
- **워킹트리 = 팝업 WIP 미커밋**(위 파일들, known-bad). 커밋 말 것(게이트 미통과) — 4결함 수정 후 커밋.
- 앱 dev 실행 중(background 태스크 `bopmfqe39`, CDP 9223) **단 구 바이너리(Brick 1, pop_out_slot 없음).** 팝업 GUI 실측하려면 **데몬 재시작+리빌드+재기동** 필요(사용자는 데몬 재시작 승인 이력 있음 — 공유 인프라라 그때그때 확인).

## 정지 조건 / do-not
- **팝업 WIP 커밋 금지** — 4결함 수정 + 재리뷰 PASS + GUI 실측 전엔 known-bad.
- **데몬 강제 종료 = 사용자 승인 후에만**(공유 인프라). **앱 재기동 시 포트 1420 스테일 Vite** 먼저 kill(`Get-NetTCPConnection -LocalPort 1420`→Stop-Process).
- **전체 `cargo test` 금지**(src-tauri lib 0xC0000139) → 멤버별 `-p protocol -p core -p discovery -p daemon`.
- cdp 실측: 우클릭 메뉴는 **fire(contextmenu)와 read를 분리**(React commit ~500ms 갭)해야 관측됨.

## 참조 (읽을 것만)
- 이 핸드오프 → 팝업 WIP 4결함이 핵심.
- step-log 2026-07-08 "B 착수 Brick 1" 절 + 코더가 추가한 팝업 절(정정 필요).
- 코드 포인터: `src-tauri/src/commands/popout.rs`(pop_out_slot·bind_window·cleanup_popup_window·PopupCounter) · `src-tauri/src/lib.rs`(Destroyed arm·invoke_handler) · `src-tauri/src/layout/manager.rs`(bind_window/unbind_window/slot_agent, window_bindings) · `src-tauri/src/output_router.rs`(rebuild, MAIN_WINDOW_LABEL) · `src/pages/PopoutPage.tsx` · `src/store/eventBus.ts`(__engramLayout.popOutSlot) · `src/components/slot/SlotContextMenu.tsx`(팝업으로 분리 항목+enabled 가드).
- 아키텍처 조감도: `docs/reference/architecture-overview.md`. 라우팅 = ADR-0046, 레이아웃 권위 = ADR-0035, 락순서 = ADR-0006.
