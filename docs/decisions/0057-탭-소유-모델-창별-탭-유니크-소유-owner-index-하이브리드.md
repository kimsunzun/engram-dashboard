# ADR-0057: 탭 소유 모델 — 창별 탭 + 유니크 소유 (owner-index 하이브리드)

- 상태: 확정 (2026-07-09, 근거: PRD/TRD B-tabs + `/research` OSS 서베이 + `/review prd`·`/review trd deep` 적대검증 + 사용자 결정)
- 관련: Amends ADR-0035 (ViewManager 내부 모델: 전역 active_view_id(main-전용)+window_bindings → 창별 active + view_owner/windows 탭 소유) · ADR-0056(탭 렌더 keep-alive — 라우팅 "모든 탭 수신"의 상위 근거) · ADR-0046(뷰 직결 replay·라우팅 label-불가지) · ADR-0006(락 순서) · ADR-0055(커맨드 레지스트리 — 탭 command 등록처) · `docs/process/B-wezterm-tabs/PRD.md`·`TRD.md` · step-log B-tabs

## 맥락
WezTerm식 **창(팝업) → 탭(=코드의 `View`) → 슬롯(분할 pane)** 3층 중, "창"과 "슬롯"은 되는데 그 사이 **"창별 탭"** 층이 없다. 현 `ViewManager` = 전역 `active_view_id` 1개(메인 전용) + `window_bindings`(보조/팝업 창을 고정 View 1개에 바인딩). 즉 **전역 활성 뷰 1개 + 팝업=뷰 고정**이라 "한 창이 탭 목록을 갖고 그 안에서 전환"하는 개념이 아예 없다. 레이아웃 B Phase 1(탭)에 들어가기 전 **탭을 어디에·어떻게 소유시키나**를 확정해야 한다(굵은 설계 → ADR).

## 결정
**D-1 = C안 (owner-index 하이브리드).** `ViewManager` 내부를 다음으로 바꾼다:
- `views: HashMap<ViewId, View>` — 전역 View 풀(id lookup).
- `view_owner: HashMap<ViewId, WindowLabel>` — View → 소유 창(**유니크 소유 강제**, 캐시된 역인덱스).
- `windows: HashMap<WindowLabel, WindowTabs { tabs: Vec<ViewId>, active: ViewId }>` — 창별 탭 목록 + 그 창의 활성 탭.

**메인도 특별취급 없는 일반 창**(D-2 — 전역 `active_view_id` 제거, `windows["main"]`로 통일). **한 View는 정확히 한 창의 한 탭에만 속한다(유니크 소유).** `ViewId`는 전역 UUID로 둬 후속(창 간 이동·저장복원)을 얹을 수 있게 한다. 창 닫기 규칙(D-3): 메인은 항상 최소 1탭(non-closable), 팝업은 마지막 탭 닫으면 창도 닫힘 — 두 경우 모두 **에이전트(데몬 프로세스)는 생존**(§5 손발/두뇌 분리).

## 거부한 대안
- **A — 창별 소유 (ref-list = 소유):** 각 창이 `{ tabs: Vec<ViewId>, active }`만 갖고 소유를 tabs Vec가 겸함. WezTerm(MuxWindow가 탭 목록 소유) 직역이라 유효하지만, **"ref-list이 곧 소유"인 척이 개념을 흐리고 유니크 소유를 타입으로 강제 못 한다**(한 View가 두 창 tabs에 들어가는 걸 런타임 assert로만 막음). C는 `view_owner` 별도 맵으로 **유니크 소유를 타입 수준 불변식**(View당 정확히 1창)으로 박고, view-id-키 command(`assign_agent`/`split_slot`/`close_slot`)의 소속 창 **O(1) 역참조**까지 준다. (Codex `/review prd` 제안으로 A→C 추천 상향.)
- **B — 글로벌 풀 + 매핑 (탭 공유):** `tabs: HashMap<TabId, Tab>` + `windows: { tab_ids, active }`. "같은 탭을 두 창에 동시 노출"·workspace 스위치엔 유리하나 **GC·2곳 동기화가 복잡**하다. 그 공유 요구가 **현재 use case에 없어** over-engineering(저위험 아님 — 동기화 결함 여지). `ViewId` 전역 UUID라 정말 필요해지면 후속으로 B/창간 이동을 점진 확장할 수 있다.

## 근거
- **OSS 서베이(`/research`):** WezTerm(mux 전역 레지스트리 + MuxWindow가 탭 목록 소유, 창↔탭 1:1) · VS Code(EditorGroup 창별 소유, strict per-window) · tmux(서버가 전부 소유, M:N 공유) · zellij(Screen이 탭 BTreeMap, active는 클라별). **창별 탭 소유 + 전역 UUID 탭 ID**가 WezTerm·VS Code가 검증한 모델이고, 공유 고급 요구가 없어 global-pool 복잡도 불필요.
- **적대검증:** `/review prd`(User렌즈 Opus + Tester렌즈 Codex)에서 소유 불변식(한 View=한 창) 확정 필요 지적 + C안(유니크 소유 타입강제) 제안 → 추천 상향. `/review trd deep`(3인: Designer blind=Codex · Architect-breaker + 마이그레이션/동시성 전문 = Opus doc-aware) + 재리뷰로 마이그레이션·라우팅 반전·동시성 롤백·멀티탭 정리 누수 명세 공백을 FIX 반영 후 통과.

## 영향 / 불변식
- **ADR-0035 부분 개정:** `ViewManager` 내부 `(views · active_view_id · window_bindings)` → `(views · view_owner · windows)`. **전역 `active_view_id` 제거**(창별 active로 대체). ADR-0035 **핵심은 불변**(레이아웃 권위 = src-tauri · 데몬 UI 불가지론 · JS 순수 렌더러 · lock-after-emit).
- **라우팅:** `output_router.rs`의 "한 View가 두 창(active-main + 바인딩)에 동시 라우팅" 허용 제거 → 유니크 소유. rebuild는 **각 창의 모든 탭**(활성+숨김)을 walk해 라우팅(ADR-0056 keep-alive 파생 — 숨은 탭도 수신·전환 무손실). 구독 delta는 ADR-0046 유지(wire는 Unsubscribe 1→0만, Subscribe는 `request_replay` 단독).
- **불변식(코드에 `// ADR-0057` 앵커):** ① 양방향 일관성 `view_owner[v]==L ⟺ windows[L].tabs ∋ v` ② 유니크 소유(View당 owner 1개) ③ `windows[L].active ∈ windows[L].tabs` ④ 메인 최소 1탭 + non-closable(`close_window("main")` 금지) ⑤ 에이전트 참조 다중 허용(같은 `agent_id`가 두 View에 배정 가능 → 두 창이 같은 에이전트 봄, 진도 독립·ADR-0046 — "한 View 두 창" 금지(불변식 ②)와 다른 얘기).
- **소유 갱신은 항상 쌍으로** — `windows[L].tabs` ↔ `view_owner[v]`를 따로 갱신 금지(불변식 ① 깨짐). 상세 구현·마이그레이션·command 표면·동시성 contract = TRD B-tabs.
- **후속(범위 밖):** 창 간 탭 이동(D-4)·탭/창 배치 저장복원(D-5) — `ViewId` 전역 UUID로 확장 여지만 확보.
