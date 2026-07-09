# 핸드오프: 팝업 기능 — 런타임 생성 창이 화면에 안 뜸(유령 창) 근본원인 발견, GUI 실측 블로킹

## 한 줄 상태 · 다음 첫 액션
- **상태:** 팝업 WIP 4결함 + 리뷰 rework + 권한/cascade 수정까지 **코드·리뷰·코드게이트(build/멤버테스트/fmt/코어격리/tsc/vitest 282)는 전부 PASS.** 그러나 **GUI 실측에서 치명 발견: 런타임 `WebviewWindowBuilder::build()`로 만든 창(실제 slot-popup + 테스트용 hello 둘 다)이 `build()` Ok·`getAllWindows()` 등록에도 *실제 OS 창으로 안 뜬다*(유령 창).** → 팝업 기능이 end-to-end 미작동. **미커밋 known-bad.**
- **다음 첫 액션:** **`/research`로 "Tauri v2 런타임 `WebviewWindowBuilder` 창이 등록되나 OS 창 미생성/미표시 (Windows / WebView2)" 근본원인 조사** (ADR-0038 — 비자명 결함 솔로 추측·매직넘버 금지, OSS 사례 먼저). 후보 가설: 메인스레드 요건(`app.run_on_main_thread`)·`.visible(true)`/`.focused` 명시·`WebviewWindowBuilder` vs `WindowBuilder`+webview 조합·WebView2 런타임 버전·dev vs build 차이. 조사→수정→`spawn_hello_windows` 스파이크로 재현/검증(창이 실제로 뜨는지 Win32 EnumWindows로 확인)→실제 팝업 GUI 실측→기존 게이트 재통과→**스파이크 제거**→커밋.

## ★ 치명 발견 (이번 세션 핵심) ★
- **Win32 `EnumWindows`(숨김 포함 열거):** 실제 OS 창 = **"Engram Dashboard"(메인) 하나뿐**. `hello-1/2/3`·`slot-popup-*` 전무(visible=false로도 없음).
- **Tauri `getAllWindows()`:** `[main, agent-tree, hello-1, hello-2, hello-3]` — 레지스트리엔 등록됨.
- → **런타임 창 생성이 "유령"**: Tauri 레지스트리엔 있지만 OS 창은 없음. 시작 시 `tauri.conf.json` config 창(`agent-tree`)은 정상 = **런타임 생성(WebviewWindowBuilder at command time)만 안 뜬다.**
- **왜 이제야 발견:** 팝업 기능이 코드/로직/리뷰로만 검증되고 **GUI 실측을 한 번도 안 했음**(직전 핸드오프도 "CDP 동적 창 열거 라이브 미검"으로 명시). 실측 게이트가 정확히 이걸 잡음. → 사용자가 계속 "1개만 뜬다"고 한 것 = 메인 창 하나뿐이었던 것(팝업은 애초에 화면에 안 떴음).
- **최소 재현 하네스(THROWAWAY):** `spawn_hello_windows(count)` 커맨드 — 팝업/View/에이전트/라우팅 전부 배제하고 plain `about:blank` 창을 대각 위치에 count개 생성. `invoke('spawn_hello_windows',{count:3})` → `getAllWindows`엔 hello-1/2/3, EnumWindows엔 0개 = **버그의 순수 repro.** 조사에 이걸 써라(커밋 전 제거).

## 이번 세션 한 것 (전부 미커밋)
파이프라인 = `/implement critical`:
1. 코더1(Opus): 팝업 4결함 수정(①비원자 MOVE 경쟁 재검증 가드 ②View 누수: cleanup close_view+emit / view_metas 바인딩 제외 ③팝업 우클릭 viewId override ④bind_window 커맨드 삭제).
2. `/review code deep`(doc-aware Opus + cross-family Codex) → **FIX**: self-close가 필터된 리스트와 비교해 팝업 즉시·연쇄 자멸(니어-BLOCK, cross-family가 doc-aware 오판을 잡음) + close_view가 팝업 View를 active 승격(ADR-0035) + JS 서피스 §5 갭.
3. 코더2(Opus) rework: self-close를 전용 `view:closed{id}` 이벤트(정확 id 일치)로 교체 + close_view active 재선택이 바인딩 View 제외 + 공유 `resolveDefaultViewId()`(팝업이면 hash `?view=`, 아니면 activeViewId).
4. `/review code deep` 재리뷰 → **PASS**: #1(다른 JS 메서드 §5 갭)은 메인이 코드로 grounding 기각(액션들이 viewId 명시 요구), #2(malformed hash)는 코더3이 `readViewIdFromHash`를 `#/popup` 라우트로 스코프 제한.
5. `/qa full`: 코드 게이트 전부 PASS. GUI 실측에서 위 치명 발견.
- 실측 중 추가 수정(전부 미커밋): `popup.json`에 `core:window:allow-close`(self-close 권한 — **단 창 자체가 안 떠서 무의미해짐**) + `pop_out_slot` 빌더 위치 cascade + `.inner_size` 축소 + **throwaway `spawn_hello_windows`**.

## 검증 상태 (쌍으로)
- **돌린 것(PASS):** `cargo build`(src-tauri lib 컴파일 OK) · 멤버 테스트 `-p protocol -p core -p discovery -p daemon` · `cargo fmt --check` · 코어격리 `rg "use tauri" crates/engram-dashboard-core/src/`=0 · `npx tsc --noEmit` · `npm test`(vitest 18파일 282). 재실행 = 위 명령.
- **라이브 검증됨(백엔드/프론트 로직만):** MOVE 백엔드(뷰이동·원슬롯 제거) · `view_metas` 유령탭 필터(닫힌 뷰 목록 미노출) · `view:closed` self-close **캐스케이드 없음**(틀린 id → 6개 팝업 유지 = 선택성 작동) · `bind_window` 제거 거부("Command not found") · 메인 우클릭 메뉴 타깃.
- **★검증 실패/블로킹★:** **팝업 창이 실제로 안 뜸(유령 창)** → 팝업 표시·팝업내 렌더·팝업 self-close 실동작·Destroyed cleanup 실동작 **전부 미확인**(코드리뷰로만 커버, 화면 위 통과 0).
- **src-tauri lib 유닛테스트 실행불가**(선재 0xC0000139 로더) 여전 → 멤버별로만.

## 실패한 접근 (do-not — 다시 파지 말 것)
- **"팝업이 큰 메인 창 뒤/겹침으로 가려졌다" 가설 = 오답.** Win32 EnumWindows로 창이 *아예 OS에 없음* 확정(가려진 게 아니라 미생성). 위치 cascade·always-on-top로 해결 시도 무의미.
- **CDP는 런타임 생성 팝업 창을 열거 못 함**(`/json/list`에 안 나옴) → 팝업 내부는 CDP eval 불가. OS 창 존재 확인 = **Win32 `EnumWindows` P/Invoke**(이번에 씀, 결정적).
- 메인 창에서 팝업 창 상태(`isVisible`/`outerPosition`) 조회 = 권한 에러("e") — main capability 스코프가 `["main","agent-tree"]`라 cross-window 조회 불가.
- data: URL 창 = tauri `webview-data-url` 피처 필요 → 스파이크는 `about:blank`로 회피(그래도 유령).

## 정지 조건 (stop conditions)
- **미커밋 커밋 금지** — windowing 미해결 + throwaway 스파이크 존재. **커밋 전 스파이크 제거 필수**(아래 목록).
- **데몬/앱 강제종료 = 사용자 승인 후에만**(공유 인프라). 재기동 시 포트 1420 스테일 Vite 먼저 kill. **`tauri dev` 워처가 `src-tauri/` 편집 시 자동 리빌드·재시작**하니, 결정적 테스트는 kill→수동 재기동 권장(자동 리빌드 타이밍 모호성 있었음).
- **전체 `cargo test` 금지**(0xC0000139) → 멤버별.
- **비자명 windowing 결함 = ADR-0038**(솔로 추측·매직넘버 금지 → OSS 조사 우선).

## repo / 앱 상태
- 브랜치 master, top=`297796e`(Brick 1). **미푸시 다수 + 워킹트리 미커밋 known-bad.**
- 미커밋 파일: `src-tauri/src/commands/popout.rs`(신규+cascade+스파이크) · `src-tauri/src/layout/manager.rs` · `src-tauri/src/lib.rs`(invoke_handler에 spawn_hello_windows 등록) · `src-tauri/src/commands/layout.rs`(view:closed emit) · `src-tauri/capabilities/popup.json`(allow-close) · `src/pages/PopoutPage.tsx`(신규) · `src/components/layout/ViewLayoutRenderer.tsx`·`ViewLayoutRenderer.test.tsx` · `src/components/slot/SlotContextMenu.tsx` · `src/store/viewStore.ts`·`viewStore.test.ts` · `src/store/eventBus.ts` · `src/pages/PopoutPage.test.tsx`(신규) · docs/process/step-log.md 등.
- 앱 실행 중: bg `bn5971a7g`(`RUST_LOG=debug` + CDP 9223). 종료해도 무방.
- 스크래치 스샷(`_shot_*.png`)은 제거 완료.

## ★ 커밋 전 스파이크 정리 목록 ★
- `spawn_hello_windows`: `popout.rs` 함수 제거 + `lib.rs` `generate_handler![]`의 `commands::spawn_hello_windows,` 줄 제거.
- `pop_out_slot` 빌더 cascade/`inner_size` 축소: windowing 수정 후 **정식 위치 정책 재검토**(창이 뜨게 된 뒤 유지 여부 결정 — 멀티모니터 좌표·메인 모니터 기준 배치 고려).

## 참조 (읽을 것만)
- 이 핸드오프 → **windowing 유령 창**이 핵심 블로커.
- 코드 포인터: `src-tauri/src/commands/popout.rs`(`pop_out_slot` 빌더 ~line 138 + `spawn_hello_windows` 스파이크 repro) · `src-tauri/src/lib.rs`(invoke_handler 등록) · `src-tauri/capabilities/popup.json`(allow-close).
- ADR: 결함 조사 규약 = ADR-0038 · 라우팅=ADR-0046 · 레이아웃 권위/active=ADR-0035 · 락순서=ADR-0006.
- Win32 창 열거 스니펫: 이 세션 대화에 PowerShell `EnumWindows` P/Invoke 있음(OS 창 실측용 — 재활용).
