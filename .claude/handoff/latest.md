# 핸드오프: Phase 2(탭) — 렌더 전략 확정(ADR-0056)·부팅 resume OFF까지 완료, 탭 구현은 fresh 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** Phase 1(커맨드 레지스트리) 완성·커밋(`8e7138b`). 이번 세션 추가 = **탭 렌더링 전략 확정(ADR-0056)** + **부팅 자동 resume OFF**(`b9cd5ea`). Phase 2(탭) 구현은 아직 0 — **다음 세션 몫**(사용자가 "탭은 핸드오프로 다음세션에" 명시).
- **다음 첫 액션:** **Phase 2 = 탭. TRD 먼저**(PRD `docs/process/B-wezterm-tabs/PRD.md` §10-3 스펙갭 해소) → `/implement critical`(ViewManager 모델 마이그레이션 = 동시성·lifetime·마이그레이션). **렌더 전략은 이미 ADR-0056으로 결정됨**(아래) — TRD는 모델 마이그레이션·라우팅·동시성만 다루면 됨.

## ★ 사용자 확정 작업 순서 (이번 세션 갱신) ★
1. ~~커맨드 시스템~~ ✅ `8e7138b`
2. **탭 레이아웃** (Phase 2) ← 다음
3. **dom/xterm 렌더선택 커맨드화** — 기존 `__engramLayout.setRenderMode`/`toggleDomMode`를 ADR-0055 레지스트리 command로 감싸기(가벼움, 이미 절반 존재). = ADR-0056 "렌더모드 레버".
4. **에이전트 트리 → 슬롯 ★설계 논의 지점(사용자와 먼저 얘기)★** — 슬롯 콘텐츠 종류 모델(slot content = 터미널|트리|diff…) 필요 = 별도 설계. 가벼운 구현 아님.
5. **에이전트 트리 정교화**
6. **View 우클릭 메뉴 정교화** (= 메뉴 항목이 command, 커맨드 시스템과 시너지)
7. **메시지 시스템 간단히** (`send_message` = 기존 `write_input` 재사용, 데모 페이오프)

각 단계 = `/implement` 하나씩 커밋.

## ★ 이번 세션 결정 (재론 금지) ★
### ADR-0056 — 탭 렌더링 전략 (확정, 커밋 `61c2c05`)
- **탭 전환 = keep-alive(A):** 비활성 탭 xterm 인스턴스 살려둠(dispose X), 숨김만. 출력 계속 누적, 전환 즉시·무손실(replay 불필요).
- **WebGL 좌석은 "보이는 슬롯"만:** 탭 숨김 시 WebglAddon dispose(좌석 반납), 데이터 버퍼는 유지. 표시 시 재부착 + `fit()`+`refresh()`.
- **렌더모드(dom/xterm) 교체 = command 레버**(위 순서 3): 기존 `setRenderMode` 감쌈. 트리·diff 등 비-터미널 슬롯은 애초에 DOM이라 좌석 0.
- **사전 풀링/replay-tiering = YAGNI**(실측 문제 전엔 안 만듦). 콜드 탈출구 = ADR-0046 뷰별 replay(이미 존재).
- **실측 근거:** 실 WebView2 WebGL2 좌석 = **정확히 16개**(cdp 40개 생성→16 생존). **사용자 가정 최대 = 창3×탭2×슬롯4** → 살아있는 24 / **보이는 12 ≤ 16 → 항상 안전.** 이 상한 깨는 레이아웃(창/슬롯 최대치 상향)은 ADR-0056 재검토.
- **거부:** B(replay 우선 — 콜드 탈출구로만 남김)·3-tier 사전풀링(같은-모델 앱 아무도 안 함)·WezTerm 직이식(네이티브라 파서/렌더 분리 = xterm.js 불가)·naive-A(숨은 탭도 WebGL → 24>16 초과).
- **같은-모델 근거:** VS Code·Tabby·Hyper·Theia 전부 keep-alive(A) + context-loss 반응형 canvas 폴백. 우리 `TerminalSlot.tsx`도 이미 `onContextLoss→DOM` 폴백 보유.

### 부팅 자동 resume OFF (stopgap, 커밋 `b9cd5ea`)
- `daemon/src/lib.rs:315` `restore_all()` **주석 처리**. 기본 = "부팅 자동 복원 안 함". `auto_restore` 필드·reaper disposition·`restore_all()` 구현은 유지(호출만 끔).
- **후속(미완):** 특정 에이전트만 이벤트성 복원하는 **opt-in command**(RestoreAgents 류) 추가 + **ADR-0016 "부팅 복원" 기본 뒤집음을 정식 ADR로 박기** + `tests/ws_e2e.rs:ignored_daemon_kill_cleans_pty_child`(현 `#[ignore]`, 부팅 복원 의존) opt-in 경로로 갱신.

## Phase 2 — 탭 (정본 = PRD B-tabs) — 렌더 외 남은 결정
- **사용자 결정 확정(재론 금지):** D-1=**C안**(owner-index: `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId,WindowLabel>` + `windows[label].tabs: Vec<ViewId>`+active, 유니크 소유) · D-2=**예**(메인도 일반 창, `active_view_id` 특별취급 제거) · D-3=**메인 최소 1탭 / 팝업 마지막 탭 닫으면 창도 닫힘, 에이전트 생존** · D-6=**빈 `create_window`** · D-8=**Ctrl+Tab 전환만** · D-4/D-5=후속.
- **★TRD 먼저 (§10-3 스펙갭):** ① `window_bindings`+`active_view_id` → 새 `windows` 모델 마이그레이션(팝업·PopoutPage `?view=` URL) ② 탭 전환 라우팅/replay(ADR-0046 — 숨은 탭 수신중단? 활성탭 remount replay? 두 창 같은 에이전트 진도 독립?) ③ 동시성 contract(create/switch/close/pop_out 직렬화, ADR-0006) ④ pop-out mid-flight 롤백 ⑤ 한 View 두 창 금지 불변식(현 `output_router.rs:156` 허용 → 새 모델서 금지).
- 백엔드 = ViewManager 모델 변경(`src-tauri/src/layout/manager.rs`). 탭 command는 Phase 1 레지스트리에 등록.

## repo / 검증 상태
- 브랜치 **master**. 이번 세션 커밋: `61c2c05`(ADR-0056+step-log) · `b9cd5ea`(resume off) · `8e7138b`(Phase 1). **셋 다 origin 미푸시** → **다음 세션 `git push` 필요**(또는 사용자 확인 후).
- **워킹트리:** `cdp-final-check.png`만 untracked(이전 세션 스크래치, 내 것 아님 — 안 지움).
- **핸드오프는 git에 안 들어감** — `.gitignore` line 10 `.claude/handoff/`(의도적, "머신마다 쌓이는 작업 메모"). 로컬 `history/`에만 append-only 누적. **미해결 질문: 사용자가 git 추적 원하는지**(원하면 gitignore 규칙 제거).
- **검증됨:** ADR-0056 = 실측(WebGL 좌석 16) + 같은-모델 조사(/research medium ×3, Codex 적대). resume off = `cargo check -p daemon` clean. **미착수:** Phase 2 탭 구현 0.
- **실행 중 앱:** 이번 세션 GUI실측·WebGL probe용 `npm run tauri dev`(CDP 9223) bg 기동 상태 — 세션 종료 시 소멸 가능. 다음 세션 CDP 9223 확인 후 없으면 재기동.

## 실패한 접근 / do-not (재론 금지)
- **탭 렌더 = B/3-tier/WezTerm직이식/naive-A 재검토 금지** — ADR-0056 거부. keep-alive(A) + 보이는 것만 좌석 확정.
- **WebGL 좌석 대응 = 사전 풀링 만들지 말 것**(실측 문제 전엔 YAGNI). 반응형 canvas 폴백 이미 있음.
- **탭 D-1 = global-pool(B안) 재검토 금지**(C안 확정). **커맨드 = enum 중앙 dispatch 재검토 금지**(ADR-0055).
- **부팅 resume 다시 켜지 말 것** — 사용자 결정(기본 OFF). 복원은 opt-in command로만(후속).

## 정지 조건 (stop conditions)
- **데몬/앱 강제종료 = 사용자 승인 후만**. **전체 `cargo build`/`cargo test` = 실행 중 데몬 daemon.exe 잠금 + 0xC0000139 로더** → 멤버별 스코프(`cargo check -p <member>`는 exe 안 만들어 잠금 회피 — resume 검증에 씀). Phase 2는 src-tauri(manager.rs) 편집 = dev 워처 자동 리빌드/재시작 → 결정적 테스트는 kill→수동 재기동 대기.
- **dev 로그를 프로젝트 폴더로 리다이렉트 금지**(vite watch 무한 reload). bg task output(temp)은 안전.
- **Phase 2는 TRD 먼저.** 비자명 코드 변경 = `/implement`(코더→/review→/qa), 메인 직접 편집 금지.
- 비자명 결함/windowing = ADR-0038(OSS 우선). GUI실측 = `scripts/cdp.mjs eval`.

## 참조 (읽을 것만)
- **Phase 2 정본 = PRD B-tabs**(§4 결정·§10 스펙갭) · **렌더 전략 = ADR-0056**(커밋됨) · **Phase 1 = ADR-0055**.
- ADR-0046(라우팅 replay·탭 전환) · ADR-0006(동시성·락) · ADR-0035(레이아웃 권위) · ADR-0016(부팅 복원 — resume off가 뒤집음, 정식 ADR 후속) · ADR-0008(S9 복원) · ADR-0038(디버깅).
- 코드 포인터: `src-tauri/src/layout/manager.rs`(ViewManager 탭 모델) · `output_router.rs:156`(한 View 두 창 허용 — 새 모델서 금지) · `src/store/eventBus.ts`(`__engramLayout.setRenderMode`/`toggleDomMode` = 렌더모드 레버 감쌀 대상, `__engramCmd` 레지스트리) · `src/components/slot/TerminalSlot.tsx`(WebglAddon onContextLoss→DOM, ADR-0056 앵커 달 곳) · `src/commands/registry.ts`(탭·렌더 command 등록처) · `crates/engram-dashboard-daemon/src/lib.rs:315`(resume off 지점) · `src/components/layout/AppLayout.tsx:43`(Sidebar 고정패널 — 순서4 트리→슬롯 대상).
