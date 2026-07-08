# PRD — WezTerm식 창>탭>슬롯 (레이아웃 B, Brick 2+)

> 상태: **초안 + 적대 리뷰 완료(사용자 결정 대기)** · 2026-07-09 · 자율 세션. `/review prd`: 방향 OK · 명세 공백으로 BLOCK → §10에서 강화·결정거리 확장(사용자는 §10까지 보고 픽).
> 이 문서는 옵션·트레이드오프를 깔아 **사용자가 고르게** 하는 PRD다(임의 채택 금지). 굵은 결정은 선택 후 ADR로 박는다.
> 발견체인: `docs/README.md` → 레이아웃 B 트랙(Brick 1 = 커밋 297796e, 팝업 = bebbf66) → 이 문서.

## 0. 한 줄

여러 에이전트를 **WezTerm처럼** 다룬다: **창(팝업) 여러 개 → 각 창에 탭 줄 → 각 탭 안에 분할 슬롯.** 지금은 "창"과 "슬롯"은 되는데 그 사이의 **"창별 탭"** 층이 없다. 이걸 채운다.

## 1. 개념 모델 (사용자 확정 방향)

```
창(Window/팝업)  ── WezTerm 창          [됨: 런타임 팝업 bebbf66]
  └ 탭(Tab = 코드의 "View")             [갭: 탭 줄 UI + 창별 소유]
      └ 슬롯(Slot = pane, 분할)          [됨: split_slot]
```

- **창** = OS 창(메인 + 런타임 팝업). "코드 에이전트 창", "개인작품 창"처럼 용도별로 여러 개.
- **탭** = 슬롯 배치 한 벌(= 코드의 `View`). 한 창 안에서 여러 탭을 두고 전환.
- **슬롯** = 탭 안의 한 칸(에이전트 하나, 또는 다시 분할).

**용어 고정(혼동 방지):** 사용자의 "탭" = 코드의 **View**. 사용자의 "창(A/B)" = **OS 창(팝업)**. 이 문서는 앞으로 **탭(=View)** 로 병기한다.

## 2. 현재 상태 — 됨 vs 갭 (조사 실측)

| 층 | 상태 | 근거 |
|---|---|---|
| 창 여러 개(런타임 팝업) | ✅ 됨 | `pop_out_slot`(bebbf66) |
| 슬롯 분할 | ✅ 됨 | `split_slot`/`close_slot` (manager.rs) |
| 탭(View) 만들기/전환 **동작** | ✅ 백엔드 됨 | `create_view`/`switch_view` (manager.rs:168,234) |
| **탭 줄 UI** | ❌ 없음 | 프론트에 tab bar 컴포넌트 0 |
| **창별 탭 소유** | ❌ 없음 | ViewManager = 전역 `active_view_id` 1개 + `window_bindings`(팝업 1뷰 고정) |

**핵심 갭:** 지금은 **전역 활성 뷰 1개**(메인) + **팝업=뷰 1개 고정**이라, "한 창이 탭 목록을 갖고 그 안에서 전환"하는 개념이 아예 없다. 이걸 만드는 게 이번 작업의 본체.

## 3. OSS 근거 (설계 서베이 — /research 위임)

| 앱 | 탭 소재 | 창↔탭 | 창 간 이동 |
|---|---|---|---|
| **WezTerm** | MuxWindow(창)가 자기 탭 목록 소유 (mux는 전역 레지스트리) | 1:1 | 가능(pane 단위) |
| **VS Code** | EditorGroup(창별) 소유 | strict per-window | 가능(drag) |
| tmux | 서버가 전부 소유, 세션이 window를 winlink 참조 | M:N 공유 | 가능 |
| zellij | 세션의 Screen이 탭 BTreeMap 소유 · **active 탭은 클라이언트별** | 1:1 | 미확인 |

**권고:** **창별 탭 소유 + 전역 UUID 탭 ID**. WezTerm·VS Code가 검증한 모델이고, "같은 탭을 두 창에 동시 노출" 같은 고급 요구가 현재 없으므로 global-pool의 복잡도(공유·GC·2곳 동기화)는 불필요. 탭 ID를 전역 UUID로 두면 나중에 창 간 이동/global-pool로 점진 확장 가능.

**출처:** wezterm.mux 공식 API·DeepWiki 소스, VS Code custom-layout 문서, tmux/zellij DeepWiki. (전체 URL = 이 세션 리서치 로그.)

## 4. ★핵심 설계 결정 (사용자 결정 필요)★

### D-1. 탭 소유 모델 — 어디에 탭 목록을 두나

- **(A) 창별 소유 [추천]** — 각 창(메인 포함)이 `{ tabs: Vec<ViewId>, active: ViewId }`를 가짐. View 자체(슬롯 트리)는 지금처럼 전역 `views: Vec<View>` 풀에 두되, **어느 탭이 어느 창에 · 그 창의 활성 탭**을 창별로 소유. (= WezTerm: mux 전역 레지스트리 + MuxWindow가 탭 목록 소유.)
  - 장점: 소유 명확, 창 닫으면 소속 탭 정리 자연, 창 단위 자기완결 저장, `{window, tab}` 좌표가 LLM 제어에 직관.
  - 거부 대안 대비: global-pool은 지금 use case에 over-engineering.
- **(B) 글로벌 풀 + 매핑** — `tabs: HashMap<TabId, Tab>` + `windows: {tab_ids, active}`. 탭 공유·workspace 스위치엔 유리하나 GC·동기화 복잡. **지금은 거부, 필요 시 후속.**

> **추천 = A.** 현 `active_view_id: Uuid` + `window_bindings: HashMap<label, view_id>` 를 → `windows: HashMap<WindowLabel, WindowTabs { tabs: Vec<ViewId>, active: ViewId }>` 로 대체. 메인("main")도 특별취급 없이 같은 구조.

### D-2. 메인 창을 "탭 가진 일반 창"으로 통일하나

- **(예) [추천]** — 메인도 `WindowTabs` 하나. `active_view_id` 특별취급 제거 → 메인/팝업 동일 코드경로(ADR-0035 단순화).
- (아니오) — 메인만 전역 active 유지, 팝업만 탭. → 두 경로 분기 유지(복잡, 비추천).

### D-3. 창 닫을 때 그 창의 탭(뷰)들은?

- **(추천)** 창 닫으면 그 창의 탭(뷰) 전부 닫힘. **단 에이전트(데몬 프로세스)는 안 죽음** — §5 손발/두뇌 분리(표시 표면만 사라짐). 현 `cleanup_popup_window`(Destroyed 정리)를 멀티탭으로 확장.
- 대안: 닫히는 창의 탭을 메인으로 reparent(유실 방지). → WezTerm은 "탭 0 → 창 자동 닫힘"으로 회피. **기본 = 닫힘, reparent는 옵션 후속.**

### D-4. 창 간 탭 이동(드래그) — 이번 범위?

- **(추천) 이번 범위 밖** — 사용자가 앞서 "나중에". 단 **D-1을 전역 UUID 탭 ID로** 설계해 나중에 얹을 수 있게만 해둔다. (WezTerm `move_to_new_window` 패턴.)

### D-5. 탭·창 배치 저장/복원 — 이번 범위?

- **(추천) 이번 범위 밖** — 로드맵 D-7(프론트 localStorage, 데몬화 뒤로 보류)와 합류. 지금은 런타임 상태만.

> **순수 내부(메인이 정하고 보고, 결정 아님):** 탭 줄 UI 위치(각 창 상단)·스타일(shadcn 탭)·[+] 버튼·라우팅(OutputRouter를 "창의 활성 탭" 기준으로 rebuild)·이벤트(창별 `view:list-updated`).

## 5. 시나리오 / 엣지 (BDD식 — 수용 확인용)

- **여러 탭 전환:** 메인 창에 탭 2개(코드에이전트·개인작품) → 탭 클릭 시 그 창 캔버스가 해당 뷰로 바뀐다. LLM `switch_tab(window, view)`도 동일.
- **팝업도 탭:** 팝업 창에도 탭 줄이 있고, 그 창 안에서 탭 추가/전환 가능(지금은 1뷰 고정 → 확장).
- **창 닫기:** 팝업 창 닫으면 그 창의 탭들 사라짐, 하지만 그 에이전트들은 데몬에 살아있음(다른 창에서 다시 붙일 수 있음).
- **단일 창 엣지:** 창이 1개일 때 "탭을 새 창으로"는 새 창 생성이 기본(이동 대상 없음).
- **마지막 탭 닫기:** 창의 마지막 탭을 닫으면? → (결정 D-3 연동) 창도 닫힘 or 빈 탭 유지. 추천 = 빈 탭 하나 유지(메인은 항상 최소 1탭).
- **LLM 제어(§5 불변):** 위 전부 command로 노출 — `create_tab(window)`, `switch_tab(window, view)`, `close_tab(window, view)`, `move_slot_to_window`(기존 pop_out 확장). 사람 클릭 = 같은 command.

## 6. 수용 기준 (Phase 1 = 탭)

1. 메인·팝업 각 창에 **탭 줄**이 뜨고, 그 창의 탭 목록·활성 탭을 보여준다.
2. 탭 클릭/`switch_tab` → 그 **창만** 해당 뷰로 바뀐다(다른 창 불변 — 창별 독립).
3. `create_tab`/[+] → 그 창에 새 탭(빈 슬롯 뷰) 추가·활성화.
4. `close_tab` → 그 탭 닫힘, 에이전트 생존.
5. 라우팅: 각 창은 **자기 활성 탭**의 에이전트 출력을 받는다(창별 독립 진도, ADR-0046 유지).
6. GUI 실측(cdp+EnumWindows): 메인 탭 2개 생성·전환 + 팝업 탭 전환 엔드투엔드.
7. LLM 제어 경로 command 동반(§5). 굵은 결정 ADR.

## 7. 후속 Phase 스케치 (이번 아님 — 순서상 다음)

### Phase 2 — 메인창 왼쪽 에이전트 스폰을 "슬롯"으로

- **현재:** Sidebar(AgentTree+스폰)는 슬롯 밖 **고정 패널**(프론트 로컬 상태). 슬롯 시스템은 백엔드 권위.
- **목표:** 그 스폰/트리 영역을 **슬롯 콘텐츠의 한 종류**로 → 레이아웃에 편입(이동·팝업·LLM 제어 가능).
- **깔린 결정거리(로드맵 기존 갭):** **슬롯 콘텐츠 종류 모델** — 슬롯이 담는 것 = 에이전트 터미널 / 에이전트 트리 / (나중에 diff·api뷰). 이건 "모드 시스템"(터미널/클로드/코덱스/api)과 합류. 슬롯에 `content: AgentTerminal(id) | AgentTree | …` 타입 추가. **PRD 별도 필요**(Phase 2 착수 시).

### Phase 3 — 미니 오케스트레이션 (메시지만)

- **목표(사용자: "메시지만 딱"):** 한 에이전트가 다른 에이전트에게 텍스트 한 줄 보내기.
- **최소안:** `send_message(to_agent, text)` command = 대상 에이전트 stdin에 text 주입(기존 `write_input` 재사용). 주소 = agent id/name.
- **결정거리:** 주소 체계(id vs 사람이름) · stdin 주입 vs 구조화 인박스 · LLM이 부르는 표면. **최소=stdin 주입**으로 시작, 구조화는 후속.
- **한 사이클 데모(사용자 목표):** 명령으로 에이전트 A/B/C/D 스폰 → 일부는 새 창(탭) → A가 C에 메시지 → 도착 확인.

## 8. 열린 결정 목록 (사용자 픽 대기)

- **D-1** 탭 소유 = A(창별) / B(글로벌풀) — 추천 **A**
- **D-2** 메인=일반창 통일 = 예 / 아니오 — 추천 **예**
- **D-3** 창 닫을 때 탭 = 닫힘(에이전트 생존) / reparent — 추천 **닫힘**
- **D-4** 창 간 탭 이동 = 이번 / 후속 — 추천 **후속(ID만 대비)**
- **D-5** 저장/복원 = 이번 / 후속 — 추천 **후속(D-7 합류)**
- **범위:** Phase 1(탭)만 이번 구현 / 2·3 함께 — 추천 **Phase 1 먼저**(2·3은 각자 PRD)

## 9. 다음 단계 (프로세스)

1. 사용자가 위 D-1~D-5 + 범위 픽 → 굵은 것 ADR.
2. TRD(ViewManager 모델 변경·OutputRouter·이벤트·프론트 탭바 인터페이스 확정).
3. 구현 = `/implement`(코더→리뷰→qa, GUI 실측 포함).

---

## 10. 적대 리뷰 반영 (/review prd — 2026-07-09)

판정: **User렌즈(Opus)=FIX(5) · Tester렌즈(Codex)=BLOCK(8).** 방향(창별 탭, D-1 A계열)은 양쪽 수용 — BLOCK 사유는 "방향 오류"가 아니라 **TRD 전 확정해야 할 명세 공백**(모델 불변식·마이그레이션·엣지 상태기계·라우팅/replay·동시성). 자율 모드 보수 취합 = **BLOCK 유지**(사용자 결정까지 구현 진입 보류). 아래로 PRD 강화.

### 10-1. D-1 재정리 — 소유 불변식 + 놓친 3안(C)
- ★불변식 확정 필요★: **한 View(탭)는 한 창에만 속하나?** 현 라우터는 한 View가 main-active + popup-bound 양쪽에 동시 라우팅되는 걸 허용(`output_router.rs:156`) — 새 모델이 금지/허용을 명시해야. **추천: 유니크 소유**(한 View = 한 창의 한 탭), 공유는 후속.
- **(C) 소유자 인덱스 하이브리드 [Codex 제안 — 유력]:** `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId, WindowLabel>` + `windows[label].tabs: Vec<ViewId>`. 전역 lookup 유지 + 유니크 소유를 타입으로 강제 — "ref-list=소유"인 척(A의 개념 흐림)을 없앰. **→ A vs C 재검토: C가 더 깨끗(추천 상향).**

### 10-2. 추가 결정 (리뷰 적출 — 사용자 픽)
- **D-6. 빈 새 창 생성 command** — 지금 창 생성 경로는 `pop_out_slot`(에이전트 든 슬롯 분리)뿐, "빈 새 창 열기"가 없음. 데모("C를 새 창에 스폰")에 필수 → `create_window`(빈 탭 1개). **추천: 추가.**
- **D-7. 배치 지정 스폰** — 스폰과 배치(`assign_agent`)가 분리라 "D를 창B의 새 탭에 스폰"이 다단계. 데모의 실제 흐름 = 배치 지정 → 스폰→창/탭/슬롯 배치→라우팅을 한 흐름으로 묶는 수용항목 필요. **추천: Phase 1 수용기준에 편입.**
- **D-8. 키보드 내비(Ctrl+Tab 등)** — WezTerm 핵심 UX인데 PRD 침묵. §5 LLM-우선이라 뒤로 미룰 순 있으나 **사용자-대면 tradeoff라 명시 결정.** 추천: Phase 1 최소(Ctrl+Tab 전환)만 or 후속 — 사용자 픽.
- **D-3 확정(상태기계)** — "창 닫기"와 "마지막 탭 닫기"가 3곳서 엇갈림(D-3 vs §5 vs `close_view` 코드 `manager.rs:186` = 빈 뷰 강제생성=탭 유지 쪽). 창 종류별 규칙 명시: **메인 = 항상 최소 1탭**(마지막 닫아도 빈 탭 유지) · **팝업 = 마지막 탭 닫으면 창도 닫힘**(WezTerm식, 추천). `close_tab(window, last)` 결과를 수용기준에.

### 10-3. TRD에서 반드시 확정할 명세 (BLOCK 근거 공백)
1. **`window_bindings` 마이그레이션** — 현 `active_view_id`+`window_bindings`(팝업 1뷰 고정, main-metas서 숨김 `manager.rs:74`) → 새 `windows` 모델로 이전. 기존 팝업·`PopoutPage ?view=` URL 처리 포함(안 하면 orphan/중복/노출).
2. **탭 전환 시 라우팅/replay 의미(ADR-0046 민감)** — 숨은 탭은 프레임 수신 중단? 활성 탭 remount는 전량 replay 요청? 같은 에이전트를 두 창이 보면 진도 독립? ADR-0046(뷰 직결 replay·slot `lastDeliveredSeq`·wire Subscribe 금지)와 정합. **"순수 내부"에서 격상 — 사용자-대면 + ADR 민감.**
3. **동시성 contract** — `create_tab`/`switch_tab`/`close_tab`/`pop_out` 동시 호출 시 직렬화·no-op/에러 결과. 현 라우터는 ViewManager 직렬 mutation 의존(ADR-0006) 유지.
4. **pop-out mid-flight 롤백** — 창 생성 중 대상 창 닫힘/정리가 새 탭목록 설치와 겹칠 때 orphan/중복 방지.

### 10-4. Phase 순서 재고 (User렌즈 FIX-3)
Phase 1(탭바)만으론 데모("A→C 메시지") 0 도달 — 페이오프는 Phase 3(≈trivial, 기존 `write_input` 재사용). **옵션: 얇은 세로 슬라이스** = 슬롯 2개(이미 됨) + `send_message`(Phase 3 최소)를 먼저 → 오케스트레이션 데모를 탭바 전에. **사용자 픽: Phase 1 먼저 vs send_message 앞당김.**
