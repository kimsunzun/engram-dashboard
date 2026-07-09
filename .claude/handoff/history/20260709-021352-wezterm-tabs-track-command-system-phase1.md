# 핸드오프: 팝업 완성·커밋 후 — WezTerm 창>탭>슬롯 트랙 설계 완료, Phase 1(커맨드 시스템)→2(탭)→3(커맨드) 구현 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** 팝업 유령 창 근본원인 수정 **완성·커밋·푸시**(WebView2 args parity, ADR-0054). 이어서 **WezTerm식 창>탭>슬롯 트랙 설계 완료** — 커맨드 시스템 확정(ADR-0055), 탭 PRD(`docs/process/B-wezterm-tabs/PRD.md`, /review prd 반영). **Phase 1~3 구현이 이 세션의 승계 몫**(컨텍스트 한도로 fresh 승계 — 사용자 지시).
- **다음 첫 액션:** **Phase 1 = 커맨드 시스템 구현** (ADR-0055 §결정대로). `/implement standard` → 코더 스폰. 순수 프론트 seam, 저위험. 첫 어댑터로 기존 command 하나 얹어 GUI실측(cdp `__engramCmd.run(...)`) → 커밋. 이어서 Phase 2(탭), Phase 3(커맨드).

## ★ 사용자 목표 (한 사이클) ★
WezTerm처럼 여러 창(팝업), 각 창에 탭 줄, 각 탭 안 분할 슬롯. 명령으로 에이전트 스폰·창 생성·오케스트라(A→C 메시지)까지 되면 "켜서 쓸만한" 한 사이클. **순서(사용자 확정):**
1. **커맨드 시스템**(레지스트리 골격) — ADR-0055
2. **탭**(WezTerm 창>탭>슬롯) — PRD B-tabs
3. **데모 커맨드 + 클로드 터미널/JSON 모드 스폰 커맨드**
4. 에이전트 트리 고도화
5. 뷰쪽 스폰 정리(에이전트-스폰-슬롯화)
6. 미니 오케스트레이션(메시지) 얹어 실행

각 단계 = `/implement`(코더→리뷰→qa) 하나씩 커밋. **1~3을 이 승계 세션서 쭉 진행하는 게 사용자 요청.**

## Phase 1 — 커맨드 시스템 (정본 = ADR-0055, 지금 구현)
- **설계 확정(재논의 금지):** 프론트 Map 레지스트리 `{id,title,category?,keybinding?,when?,run(args)}` + **handler가 기존 invoke/store로 라우팅**(신규 싱글톤 0). 소비자 = 클릭·전역 keydown(★포커스 가드: input/textarea/.xterm 타이핑 중 가로채기 금지★)·`window.__engramCmd.{list,run}`(LLM/cdp §5). 인자=객체 하나(가변인자 X). 레지스트리 프론트 단독. 팔레트·백엔드미러·커스텀키맵·when 풀모델 = **후속**.
- **코드 스케치**(이 세션 대화에서 합의 — 그대로): `src/commands/registry.ts`(register/run/list) · `src/commands/*Commands.ts`(register('tab.switch',(a)=>invoke('switch_tab',a)) 식) · `src/commands/keybindings.ts`(전역 keydown→id, 포커스 가드) · `eventBus.ts`에 `__engramCmd={list,run}` 한 줄 · UI 버튼 onClick=run(id,args).
- **첫 어댑터:** 기존 레이아웃 command 하나(또는 theme) 얹어 **GUI실측**(cdp로 `__engramCmd.run` → 동작 확인). vitest로 register/run 단위.

## Phase 2 — 탭 (정본 = PRD `docs/process/B-wezterm-tabs/PRD.md`)
- **사용자 결정 확정:** D-1=**C안**(owner-index 하이브리드: `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId,WindowLabel>` + `windows[label].tabs: Vec<ViewId>` + active, **유니크 소유**) · D-2=**예**(메인도 탭 가진 일반 창, `active_view_id` 특별취급 제거) · D-3=**메인 최소 1탭 / 팝업 마지막 탭 닫으면 창도 닫힘, 에이전트는 생존** · D-6=**빈 `create_window` command 추가** · D-8 키보드=**최소(Ctrl+Tab 전환)만** · D-4 드래그·D-5 저장복원=**후속**.
- **★TRD 먼저★ (PRD §10-3 스펙갭 — 구현 전 필수 해소):** ① `window_bindings`+`active_view_id` → 새 `windows` 모델 **마이그레이션**(기존 팝업·PopoutPage `?view=` URL) ② **탭 전환 시 라우팅/replay**(ADR-0046 — 숨은 탭 수신중단? 활성탭 remount 전량 replay? 두 창 같은 에이전트 진도 독립?) ③ **동시성 contract**(create/switch/close/pop_out 동시 → 직렬화·no-op/err, ADR-0006) ④ **pop-out mid-flight 롤백** ⑤ **한 View 두 창 금지 불변식**(현 router는 허용 output_router.rs:156 — 새 모델서 금지).
- 백엔드 = ViewManager 모델 변경(manager.rs). 탭 command(`tab.switch`/`create`/`close`, `create_window`)는 **Phase 1 레지스트리에 등록**(첫 실전 어댑터, §5).

## Phase 3 — 데모 커맨드 + 클로드/JSON 스폰
- 클로드 **터미널 모드** 스폰 · **JSON(StreamJson) 모드** 스폰 · `create_window` · **배치 지정 스폰**(스폰→창/탭/슬롯 배치→라우팅 한 흐름, PRD D-7)을 command로. 목표 데모("A/B/C/D 스폰·일부 새 창·A→C 메시지")에 필요한 만큼. (spawn 경로 = agentClient.createClaudeProfile/spawnProfile, output_format 'Terminal'|'StreamJson' — Sidebar.tsx:54 참조.)

## Phase 4~6 (스케치 — 각자 착수 시 상세)
- **4 에이전트 트리 고도화.** **5 뷰쪽 스폰 정리:** Sidebar(AgentTree+스폰)는 현재 슬롯 밖 **고정 패널**(AppLayout.tsx:43, 프론트 로컬 상태) → 슬롯 콘텐츠로 편입하려면 **슬롯 콘텐츠 종류 모델**(slot content = AgentTerminal|AgentTree|…) 필요 = 별도 PRD(모드 시스템과 합류). **6 오케스트레이션:** `send_message(to, text)` = 기존 `write_input` 재사용(대상 stdin 주입, 최소). 한 사이클 실행 데모.

## repo / 검증 상태
- 브랜치 **master**, **origin 최신**. 커밋: `bebbf66`(팝업 완성·유령창 수정) · `663bc0b`(ADR-0054 앵커) · `12c9afd`(PRD B-tabs) · `f76fa87`(ADR-0055). 전부 푸시.
- **워킹트리 클린** — `cdp-final-check.png`만 untracked(이전 세션 스크래치, 내가 만든 거 아니라 안 지움 — 정리는 사용자 확인 후).
- 새 ADR: **0054**(WebView2 args parity) · **0055**(command registry). PRD: `docs/process/B-wezterm-tabs/PRD.md`.
- **검증됨:** 팝업 = 코드게이트(build·멤버test·fmt·격리·tsc·vitest 282) + GUI실측(실제 slot-popup 창) PASS. **미착수:** 커맨드/탭 = 설계만, 구현 0.

## 실패한 접근 (do-not — 재론 금지)
- **팝업 유령 = 스레드 문제 아님** — `run_on_main_thread`로 메인스레드 build도 유령 실측 반증. 원인 = **WebView2 additionalBrowserArgs parity**(config↔런타임 불일치, ADR-0054). 스레드/STA 재조사 금지.
- **커맨드 = enum-데이터 중앙 dispatch 재검토 금지**(ADR-0055 거부 — 탈중앙/여파0 위배). **big-bang 전체 이관 금지**(골격+점진).
- **탭 D-1 = global-pool(B안) 재검토 금지** — C안(owner-index) 확정.

## 정지 조건 (stop conditions)
- **데몬(pid 33636류)/앱 강제종료 = 사용자 승인 후만**(공유 인프라). **전체 `cargo build`/`cargo test`는 실행 중 데몬이 daemon.exe 잠금** → 변경 포함 범위로 스코프(`-p engram-dashboard` + `-p core/protocol/discovery` 멤버). **전체 `cargo test` = 0xC0000139 로더 금지** → 멤버별.
- **dev 워처가 src-tauri 편집 시 자동 리빌드·재시작** → 결정적 테스트는 kill→수동 재기동 or 재빌드 완료(새 PID + CDP 9223) 대기. **dev 로그를 프로젝트 폴더 안으로 리다이렉트 금지** — vite가 그 파일을 watch해 무한 reload 루프(이번 세션 당함). bg task output(temp)은 안전.
- **Phase 2(탭)는 TRD 먼저**(위 스펙갭). 굵은 결정(D-1~D-8)은 이미 사용자 확정이라 "선택 전 구현 금지" 충족 — TRD는 그 결정 구체화.
- 비자명 windowing/결함 = **ADR-0038**(OSS/교차조사 우선, 솔로 추측 금지). GUI실측 = Win32 EnumWindows P/Invoke(PowerShell) + `scripts/cdp.mjs eval`.

## 참조 (읽을 것만)
- **Phase 1 정본 = ADR-0055** · **Phase 2 정본 = PRD B-tabs**(§4 결정·§10 스펙갭). 
- ADR-0054(WebView2 args) · ADR-0035(레이아웃 권위) · ADR-0046(라우팅 replay·탭 전환 민감) · ADR-0022(커맨드 방향 원류) · ADR-0006(락 순서) · ADR-0038(디버깅 규약).
- 코드 포인터: `src-tauri/src/layout/manager.rs`(ViewManager — 탭 모델 변경 대상) · `src/store/eventBus.ts:64`(__engramLayout — 커맨드 표면) · `src/components/layout/AppLayout.tsx:43`(Sidebar=고정패널) · `src/components/layout/Sidebar.tsx:54`(스폰 경로) · `src-tauri/src/commands/popout.rs`(pop_out_slot — 탭/창 이동 확장 기반).
