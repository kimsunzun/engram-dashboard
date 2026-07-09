# 핸드오프: Phase 1(커맨드 레지스트리) 완성·커밋 — Phase 2(탭) TRD-먼저→구현, Phase 3(스폰) 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** WezTerm 창>탭>슬롯 트랙 **Phase 1 = 커맨드 시스템(command registry) 완성·검증·커밋**(`8e7138b`, ADR-0055). `/implement standard`(코더 Opus → /review code full 2R → /qa full GUI실측) 전 게이트 PASS. **Phase 2(탭)·3(스폰 커맨드)이 이 세션의 승계 몫** — 컨텍스트 한도로 fresh 승계(직전 세션이 "1~3 쭉 진행" 요청, Phase 1까지 완주 후 여기서 끊음).
- **다음 첫 액션:** **Phase 2 = 탭. ★TRD 먼저★**(PRD `docs/process/B-wezterm-tabs/PRD.md` §10-3 스펙갭 해소 — 아래 목록). 굵은 결정 D-1~D-8은 직전 세션서 사용자 확정 완료 → "선택 전 구현 금지" 충족, TRD는 그 결정 구체화. TRD 확정 후 `/implement critical`(ViewManager 마이그레이션 = 동시성·lifetime·마이그레이션 = critical-tier).

## ★ 사용자 목표 (한 사이클) — 변동 없음 ★
WezTerm처럼 여러 창(팝업), 각 창에 탭 줄, 각 탭 안 분할 슬롯. 명령으로 에이전트 스폰·창 생성·오케스트라(A→C 메시지)까지 = "켜서 쓸만한" 한 사이클. **순서(사용자 확정):**
1. ~~커맨드 시스템(레지스트리 골격) — ADR-0055~~ ✅ **완료(`8e7138b`)**
2. **탭**(WezTerm 창>탭>슬롯) — PRD B-tabs ← **지금**
3. **데모 커맨드 + 클로드 터미널/JSON 모드 스폰 커맨드**
4. 에이전트 트리 고도화 · 5. 뷰쪽 스폰 정리 · 6. 미니 오케스트레이션(메시지)

각 단계 = `/implement`(코더→리뷰→qa) 하나씩 커밋.

## Phase 1 완료 내역 (참고 — 재작업 금지)
- **커밋 `8e7138b`** `feat(commands): Phase 1 커맨드 레지스트리 골격`. 10파일 +719.
- **구조:** `src/commands/registry.ts`(Map 레지스트리 `register/run/list`, `Command={id,title,category?,keybinding?,when?,run(args)}`, 인자=객체 하나·가변인자 X) · `dispatch.ts`(공유 `fireAndForget` = 클릭·키바인딩 안전경로, try/catch + `Promise.resolve().catch`; **`registry.run()`은 raw 반환 유지 = cdp await용**) · `keybindings.ts`(전역 **document** keydown→id; ★포커스 가드 load-bearing = `isContentEditable`+`.xterm`+INPUT/TEXTAREA/SELECT★; **`when`은 키바인딩 층만 게이트 = VS Code 시맨틱, `run`은 무조건**) · `themeCommands.ts`(첫 어댑터 `theme.set`/`theme.toggle`→`useThemeStore.setTheme`). `store/eventBus.ts`에 `window.__engramCmd={list,run}`(§5). `App.tsx` 어댑터 side-effect import + `installKeybindings()` useEffect. 7파일 전부 `// ADR-0055` 앵커.
- **Phase 2 연결점:** 탭 command(`tab.switch`/`tab.create`/`tab.close`·`create_window`)를 **이 레지스트리에 등록**(ADR-0055 §영향 — 탭이 첫 실전 어댑터). `register({id:'tab.switch',run:(a)=>invoke('switch_view',a)})` 식으로 기존/신규 invoke에 라우팅.
- **후속(ADR-0055, 지금 아님):** 팔레트 UI · 백엔드 미러 레지스트리 · 커스텀 키맵 저장 · `when` 풀 컨텍스트 모델.

## Phase 2 — 탭 (정본 = PRD `docs/process/B-wezterm-tabs/PRD.md`)
- **사용자 결정 확정(재논의 금지):** D-1=**C안**(owner-index 하이브리드: `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId,WindowLabel>` + `windows[label].tabs: Vec<ViewId>` + active, **유니크 소유**) · D-2=**예**(메인도 탭 가진 일반 창, `active_view_id` 특별취급 제거) · D-3=**메인 최소 1탭 / 팝업 마지막 탭 닫으면 창도 닫힘, 에이전트는 생존** · D-6=**빈 `create_window` command** · D-8 키보드=**최소(Ctrl+Tab 전환)만** · D-4 드래그·D-5 저장복원=**후속**.
- **★TRD 먼저 (PRD §10-3 스펙갭 — 구현 전 필수 해소):** ① `window_bindings`+`active_view_id` → 새 `windows` 모델 **마이그레이션**(기존 팝업·PopoutPage `?view=` URL) ② **탭 전환 시 라우팅/replay**(ADR-0046 — 숨은 탭 수신중단? 활성탭 remount 전량 replay? 두 창 같은 에이전트 진도 독립?) ③ **동시성 contract**(create/switch/close/pop_out 동시 → 직렬화·no-op/err, ADR-0006) ④ **pop-out mid-flight 롤백** ⑤ **한 View 두 창 금지 불변식**(현 router는 허용 `output_router.rs:156` — 새 모델서 금지).
- 백엔드 = ViewManager 모델 변경(`src-tauri/src/layout/manager.rs`). **= critical-tier(`/implement critical` → review deep → qa full).**

## Phase 3 — 데모 커맨드 + 클로드/JSON 스폰
- 클로드 **터미널 모드** 스폰 · **JSON(StreamJson) 모드** 스폰 · `create_window` · **배치 지정 스폰**(스폰→창/탭/슬롯 배치→라우팅 한 흐름, PRD D-7)을 command로. 데모 목표("A/B/C/D 스폰·일부 새 창·A→C 메시지")에 필요한 만큼. spawn 경로 = `agentClient.createClaudeProfile/spawnProfile`, output_format `'Terminal'|'StreamJson'` — `src/components/layout/Sidebar.tsx:54` 참조.

## Phase 4~6 (스케치 — 착수 시 상세)
- **4 에이전트 트리 고도화.** **5 뷰쪽 스폰 정리:** Sidebar(AgentTree+스폰)는 슬롯 밖 **고정 패널**(`AppLayout.tsx:43`) → 슬롯 콘텐츠 편입하려면 **슬롯 콘텐츠 종류 모델**(slot content = AgentTerminal|AgentTree|…) 필요 = 별도 PRD(모드 시스템과 합류). **6 오케스트레이션:** `send_message(to,text)` = 기존 `write_input` 재사용(대상 stdin 주입). 한 사이클 데모.

## repo / 검증 상태
- 브랜치 **master**. 커밋 `8e7138b`(Phase 1) — **아직 origin 미푸시**(직전 세션 커밋들 bebbf66/663bc0b/12c9afd/f76fa87은 푸시됨). **다음 세션: `git push` 필요**(또는 사용자 확인 후).
- **워킹트리:** `cdp-final-check.png`만 untracked(이전 세션 스크래치, 내 것 아님 — 안 지움).
- **검증됨(Phase 1):** `/review code full` 2R(doc-aware Opus PASS + cross-family Codex PASS, 재수정 2회로 FIX 전부 반영) · `/qa full` PASS = tsc 0 · **vitest 320** · **GUI실측 cdp 실WebView2**(`__engramCmd.list()`=[theme.set,theme.toggle]·테마 dark→light→e-ink 순환·포커스가드 매트릭스 body발화/input·contenteditable억제/네이티브 isContentEditable=true). **미착수:** Phase 2·3.
- **재실행 명령:** 프론트 게이트 `npm test`(vitest run) + `npx tsc --noEmit`. GUI실측 = `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` → `node scripts/cdp.mjs eval "..."`.
- **실행 중 앱:** 이 세션이 GUI실측용 `npm run tauri dev`(CDP 9223)를 bg로 띄웠음 — **세션 종료 시 소멸 가능**. 다음 세션은 CDP 9223 확인 후 없으면 재기동.

## 실패한 접근 (do-not — 재론 금지)
- **커맨드 = enum-데이터 중앙 dispatch 재검토 금지**(ADR-0055 거부 — 탈중앙/여파0 위배). **big-bang 전체 이관 금지**(골격+점진). **`window.__engramCmd` 전면노출 allowlist 재검토 금지** = WONTFIX by design(§5 LLM 전면제어).
- **탭 D-1 = global-pool(B안) 재검토 금지** — C안(owner-index) 확정.
- **팝업 유령 = 스레드 문제 아님**(ADR-0054 WebView2 args parity로 종결). 스레드/STA 재조사 금지.
- **포커스 가드 = `closest('[contenteditable]...')` 재도입 금지** — false-섬 경계 넘어 오억제(Codex 지적). `isContentEditable`가 권위(spec대로 정확). jsdom은 미구현이라 단위테스트는 spec shim 사용(실동작은 cdp).

## 정지 조건 (stop conditions)
- **데몬/앱 강제종료 = 사용자 승인 후만**(공유 인프라). **전체 `cargo build`/`cargo test` = 실행 중 데몬이 daemon.exe 잠금 + 0xC0000139 로더** → 변경 포함 멤버로 스코프(`-p engram-dashboard` + `-p core/protocol/discovery`). 멤버별로.
- **dev 워처가 src-tauri 편집 시 자동 리빌드·재시작** → Phase 2는 src-tauri(manager.rs) 편집이라 결정적 테스트는 kill→수동 재기동 or 재빌드 완료(새 PID + CDP 9223) 대기. **dev 로그를 프로젝트 폴더 안으로 리다이렉트 금지**(vite watch 무한 reload). bg task output(temp)은 안전.
- **Phase 2는 TRD 먼저**(위 스펙갭). 굵은 D-1~D-8은 확정이라 "선택 전 구현 금지" 충족 — TRD는 구체화. 새 굵은 결정 나오면 ADR.
- 비자명 windowing/결함 = **ADR-0038**(OSS/교차조사 우선, 솔로 추측 금지). GUI실측 = `scripts/cdp.mjs eval` + (필요시) Win32 EnumWindows P/Invoke(PowerShell).
- **구현 실행 규약 강제:** 비자명 코드 변경은 메인 직접 편집 금지 → `/implement`(코더→/review→/qa). 자율 모드서도 생략 금지.

## 참조 (읽을 것만)
- **Phase 1 정본 = ADR-0055**(커밋됨) · **Phase 2 정본 = PRD B-tabs**(§4 결정·§10 스펙갭).
- ADR-0046(라우팅 replay·탭 전환 민감) · ADR-0006(락 순서·동시성 contract) · ADR-0035(레이아웃 권위) · ADR-0022(커맨드 원류) · ADR-0038(디버깅 규약) · ADR-0054(WebView2 args).
- 코드 포인터: `src-tauri/src/layout/manager.rs`(ViewManager — 탭 모델 변경 대상) · `src-tauri/src/layout/output_router.rs:156`(한 View 두 창 허용 — 새 모델서 금지) · `src/commands/registry.ts`(탭 command 등록처) · `src/store/eventBus.ts`(__engramCmd·__engramLayout 표면) · `src/components/layout/AppLayout.tsx:43`(Sidebar=고정패널) · `Sidebar.tsx:54`(스폰 경로) · `src-tauri/src/commands/popout.rs`(pop_out_slot — 탭/창 이동 확장 기반).
