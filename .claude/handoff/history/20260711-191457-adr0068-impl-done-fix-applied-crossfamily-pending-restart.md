# 핸드오프: ADR-0068 구현 + review FIX 반영 완료 · 미커밋 · cross-family(Codex) 리뷰만 남음 (MCP 재등록 위해 세션 재시작 승계)

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0068(슬롯 공간 타깃 = 논리 트리 방향/이웃/순서 핸들) **구현 + 코드리뷰 FIX-2/FIX-4 반영 완료, 전부 워킹트리 미커밋.** 게이트 = doc-aware 리뷰(Opus) FIX 2건 반영됨 · **cross-family(Codex blind) 리뷰 미실행**(세션 중 Codex MCP 끊김 → 이 세션에선 ToolSearch로 안 잡힘). 사용자가 Codex MCP 재설치 + CC 재시작 예정.
- **다음 첫 액션:** ① `ToolSearch select:mcp__codex__codex`로 Codex MCP 가용 확인. ② 되면 **cross-family blind 코드리뷰**를 워킹트리 diff에 실행(프롬프트 = 아래 "cross-family blind 프롬프트" 그대로 — 결정 근거·ADR 주지 말 것, blind). ③ 판정 취합(doc-aware는 이미 FIX 반영 → PASS 상당, cross-family 결과와 합쳐 full 판정). ④ PASS/FIX면 **`/qa standard`**(build/test) → ⑤ **커밋**(아래 메시지 초안). BLOCK·불일치는 사용자에게.

## 무엇이 됨 — 구현 (미커밋, `/implement standard`: 코더 Opus → review)
ADR-0068 실현: `ViewManager`(src-tauri Rust)가 논리 레이아웃 트리에서 각 leaf 슬롯의 이웃(up/down/left/right slot id)·ordinal·방향/코너 토큰 해소를 산출(픽셀·getBoundingClientRect 안 씀). 스냅샷에 실어 내려보내고(기존 layout:updated/get_view 경로) + §5 registry command로 노출.

**미커밋 파일 (git status):**
- 신규: `src-tauri/src/layout/spatial.rs`(순수 로직 + 단위테스트 10 — 정규 rect 내부계산→neighbors/ordinal/corner·SpatialToken·resolve_spatial) · `src-tauri/bindings/SlotSpatial.ts` · `Neighbors.ts`(ts-rs 바인딩 **수기 작성** — 생성 테스트가 WebView2로 안 돌아 손으로 씀, CI 재생성 필요)
- 수정: `layout/manager.rs`(ViewSnapshot에 `slot_spatial: Vec<SlotSpatial>`, snapshot()에서 compute_spatial) · `layout/types.rs`(ViewSnapshot 필드 + doc) · `layout/mod.rs`(spatial 모듈 등록) · `commands/layout.rs`(`resolve_spatial` read-only cmd) · `lib.rs`(cmd 등록) · `bindings/ViewSnapshot.ts` · `src/commands/slotCommands.ts`(`slot.resolveSpatial` — 순수 backend 위임) · `src/api/layoutTypes.ts`(re-export) · 테스트 픽스처 2(`WindowLayout.test.tsx`·`viewStore.test.ts`에 `slot_spatial: []`)
- **ADR-0068 본문 수정(미커밋):** ordinal 명세를 center 전역 정렬로 확정(FIX-2). ADR-0068 파일 자체는 이전에 커밋됨(`266bfd3`)이나 이 wording 수정은 미커밋 — 커밋 시 함께.

## 게이트 상태 (핵심 — 이어서 완결할 것)
- **doc-aware 리뷰(Opus/reviewer-deep) = FIX 2건 → 둘 다 반영 완료:**
  - **FIX-2(중, 갈림길):** ordinal이 center 전역 정렬인데 명세는 "reading order"라 불일치. **사용자 결정 = ADR 명세를 center 정렬로 재정의**(코드 로직 불변). → ADR-0068 문구 + spatial.rs 주석 5곳 + types.rs doc 교정 완료.
  - **FIX-4(현재 낮음/장기 중):** `assign_rects` ratio `clamp(0,1)`이 0/1 허용 → 폭0 leaf가 neighbor/corner 깨뜨림(현재 트리거 없음, 예정 리사이즈 cmd에서 발현). → `clamp(EPS,1-EPS)` EPS=1e-4 + load-bearing 주석 + 테스트 `degenerate_ratio_produces_no_zero_area_leaf`(옛 clamp에서 fail 확인) 추가 완료.
  - doc-aware가 스스로 FIX-1·FIX-3은 반례검토 후 철회. PASS 항목: ts-rs 바인딩 정확(최고위험 지목 — 기존 snake_case 생성물 대조 통과)·L-shape bottom-right 수용기준·§5/ADR-0035 권위(매 snapshot 재계산·프론트 순수 위임)·코어 미접촉.
- **cross-family(Codex blind) = 미실행.** ← **이게 유일 잔여 게이트.** Codex MCP 세션 끊김으로 못 돌림. 재시작 후 아래 프롬프트로 실행.
- **루프 카운터:** 코더 재작업 1회차 사용(FIX-2/FIX-4). `/implement` 상한 2회 — 1회 남음.

## 검증 상태 (쌍)
- **돌린 것:** `cargo build`(workspace) clean · `cargo test -p engram-dashboard-protocol` 12/12 · `npx tsc --noEmit` clean · `npm test`(vitest) 491 · spatial 단위테스트 10/10(**throwaway `rustc --test` 하네스** — byte-identical 복사) · `cargo test -p engram-dashboard --no-run` 컴파일 OK.
- **검증 안 된 것:** **in-crate spatial 테스트 실제 실행**(WebView2 0xc0000139로 src-tauri 테스트 바이너리 실행 불가 — 환경 한계) · **ts-rs export 재생성**(수기 바인딩이 생성물과 byte-identical한지 CI 확인 필요) · **`/qa` GUI/실동작 실측 안 함**(qa는 cross-family 리뷰 후).

## do-not / 주의
- **bare `cargo test`·`-p engram-dashboard`·`--lib` = 0xc0000139(WebView2Loader 사망).** member-scoped만(`-core`/`-protocol`). src-tauri 로직 = `cargo build` + throwaway 하네스/GUI 실측.
- **`docs/reference/architecture-overview.md` = 타 세션 작업중(미커밋 215줄). 건드리거나 커밋하지 말 것.** 이번 커밋 스테이징에서 반드시 제외.
- **MCP 툴은 세션 시작 시 등록 — 세션 중 재설치·재연결로 재주입 안 됨.** Codex 끊기면 CC 재시작이 정답(이번 승계 사유). CLI(`codex exec`) 경로는 사용자가 사양.
- **ordinal은 트리 pre-order 아니라 center 전역 정렬**(FIX-2 확정). 재론 금지.
- geometry 좌표계·실측 픽셀 노출 = 보류(ADR-0068). 프론트 getBoundingClientRect 1차 = 거부.

## cross-family blind 프롬프트 (재시작 후 그대로 사용 — MCP `mcp__codex__codex`, sandbox read-only, effort medium, web_search true)
> 역할: adversarial code reviewer (blind — 결정 근거·ADR 미제공). 워킹트리 미커밋 변경(`git diff` + `src-tauri/src/layout/`·`src-tauri/bindings/` 신규)을 correctness/edge/race/off-by-one/panic/regression/계약·직렬화 불일치로 공격. 칭찬 금지. FIX/BLOCK finding은 file:line + 깨지는 입력/시나리오 근거 필수. 끝에 VERDICT: PASS|FIX|BLOCK + findings.
> 기능(근거 아닌 사실만): 중첩 h/v split 트리(각 ratio, leaf=슬롯)에서 각 leaf의 이웃(edge adjacency up/down/left/right)·ordinal(center (y,x) 사전순)·토큰 해소(top-left/…/bottom-right + focused 기준 상대 left/right/up/down→slot id)를 논리 트리에서만 산출, 스냅샷 필드 + read-only Tauri cmd + 프론트 registry cmd로 노출. ts-rs 바인딩 3개 수기 작성.
> 집중 공격: neighbor edge-adjacency float 경계/epsilon·degenerate/zero-area rect·비대칭 중첩·adjacency 대칭성 · ordinal 결정성 · corner/방향 해소(bottom-right 실제 우하단? focus null 상대방향? 동률?) · ratio clamp `[EPS,1-EPS]` 적정성 · panic(unwrap/index 빈 트리·미지 id) · serde 필드 추가 회귀 · **ts-rs 바인딩 불일치(최고위험 — 수기, 같은 폴더 기존 생성물과 대조)** · Tauri cmd 락규율·read-only·올바른 view/active tab 해소.
> 환경: bare `cargo test` src-tauri 크래시(WebView2) — 로직으로 추론. `cargo build`·`cargo test -p engram-dashboard-protocol` OK.

## 커밋 메시지 초안 (게이트 PASS 후 — architecture-overview.md 제외 스테이징)
```
feat(layout): ADR-0068 슬롯 공간 타깃 핸들 — ViewManager 논리 트리에서 neighbor/ordinal/방향 산출 (§5)

각 leaf 슬롯의 이웃(up/down/left/right)·ordinal(center 정렬)·코너/방향 토큰
해소를 논리 트리에서 산출(픽셀 무관), 스냅샷 필드 + resolve_spatial read-only
command + slot.resolveSpatial registry(§5). 코드리뷰 FIX-2(ordinal 명세=center
정렬 확정)·FIX-4(ratio clamp [EPS,1-EPS] 폭0 leaf 방어) 반영.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```
스테이징: 위 "미커밋 파일" 전부 + ADR-0068 wording 수정. **`docs/reference/architecture-overview.md` 제외.** ts-rs 수기 바인딩은 CI 재생성으로 검증.

## 참조 (읽을 것만)
- **ADR-0068**(정본 — 결정·거부 대안·ordinal center-sort 확정) · 0066(결정3 폐기원) · 0035(레이아웃 권위=클라Rust ViewManager) · 0022/0055(command registry) · CLAUDE.md §5.
- **코드:** `src-tauri/src/layout/spatial.rs`(핵심 로직) · `manager.rs`(snapshot) · `commands/layout.rs`(resolve_spatial) · `src/commands/slotCommands.ts`.
- **step-log 최근:** "LLM 공간 타깃 재설계 — 논리 도면 방향·이웃·순서 핸들 우선".
