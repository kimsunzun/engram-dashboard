# TRD — 평평한 분할 렌더러(allotment 대체) + 분할 비율 되쓰기 + 셸의 창 캔버스 기하

- 상태: **확정 — /review trd full 3 라운드(r1·r2 두 리뷰어 FIX 반영, r3 doc-aware 단독 확인 PASS) · 사용자 결정 D1~D6·R11 반영** · 날짜: 2026-09-24 · ADR = ADR-0227
- 구현: **§6 1~8단계 커밋** — `e401966` 부터 `5e99268` 까지(2026-09-24~25). 단계별 게이트는 각 커밋 본문, 라운드 기록은 `docs/process/step-log.md`. 9단계(문서)에서 이 문서를 구현에 맞췄다 — 구현이 설계와 달라졌거나 설계가 안 정한 것을 정한 자리는 **「구현:」** 으로 표시한다.
- ★**줄 포인터는 설계 시점 코드(`473da52`) 기준이다 — 「구현:」(「구현 —」·「구현(…)」 포함) 자리 안에 있거나 「지금(9단계)」 를 단 포인터만 9단계 시점 트리 기준이다**★. 그 자리 밖의 맨 「지금」·「구현 메모」 는 설계 시점의 말이라 그 포인터도 `473da52` 다. 그 뒤 구현과 master 흡수가 파일을 바꿔 줄이 어긋나므로 `git show 473da52:<경로>` 로 연다.
- 근거 조사: `docs/research/split-layout-library-survey-2026-09-24.md`(렌더러 구조 — 선택지 D) · `docs/research/split-resize-policy-survey-2026-09-24.md`(D1~D6 정책)
- 용어: **셸** = 클라이언트 셸 백엔드(`src-tauri/src/layout/` 의 `ViewManager`)다. 데몬이 아니다. **칸** = 슬롯(잎), **구분선** = 분할 경계의 드래그 손잡이.
- 범위: **뷰 하나 안**. 탭은 이미 keep-alive 라(`src/components/layout/WindowLayout.tsx:182-197`, ADR-0056) 손대지 않는다.

## 1. 현 렌더러가 하는 일 — 살아남아야 할 책임

`src/components/layout/ViewLayoutRenderer.tsx`(325줄)가 트리를 **재귀 컴포넌트**로 그린다. 슬롯 노드 하나가 렌더러 인스턴스 하나다.

| # | 책임 | 위치 | 새 자리 |
|---|---|---|---|
| R1 | props `node`·`focusedSlotId`·`viewIdOverride` | `:26-37` | 루트 렌더러(§2e) |
| R2 | 메뉴·포커스의 View 좌표 = `viewIdOverride ?? useCurrentViewId()` | `:68-70` | 잎 |
| R3 | 렌더 모드 = `renderModeOverride[slotId] ?? defaultRenderMode(agent)`(스토어 키는 이미 slot id — `src/store/viewStore.ts:78`) | `:38`, `:75-80` | 잎 |
| R4 | caps 게이트 — AgentInfo 도착 전엔 구체 렌더러를 띄우지 않는다(ADR-0041/0044) | `:101-109` | 잎 |
| R5 | ADR-0148 마지막 마운트 기억 `lastMountRef` + 커밋 후 기록·어긋나면 폐기하는 effect | `:47-62`, `:82-97`, `:110-116` | 잎(slot id key 로 살아남는다 — §2g) |
| R6 | 부재 세 갈래(연결 중 / 뷰 유지 / 없습니다 — ADR-0148·0149) | `:117-127`, `:193-205` | 잎 |
| R7 | 콘텐츠 분기: agent→dom/rich/terminal · preset_palette · agent_list · empty(`+` 아이콘, `pointer-events:none`) | `:192-241` | 잎 |
| R8 | 슬롯 틀(1px 테두리 고정·`position:relative`·hasContent 에 따른 overflow/중앙정렬) | `:128-159` | 잎(위치는 절대 배치로) |
| R9 | `data-slot-id` — cdp QA 가 이것으로 좁힌다(`.claude/skill-bindings/qa.md:262`) | `:161` | 잎 래퍼에 유지 |
| R10 | click-to-focus, 콘텐츠 슬롯만(`isContentSlot`, ADR-0066), 버블 허용 | `:162-180` | 잎 |
| R11 | 우클릭 메뉴 상태(슬롯별 `useState`) + `SlotContextMenu`(ADR-0064/0144) | `:67`, `:183-186`, `:243-254` | 잎 |
| R12 | 포커스 링 오버레이(accent 40%, `zIndex:10`) | `:255-270` | 잎 |
| R13 | 슬롯별 오류 격리 `SlotErrorBoundary`(resetKey `${id}:${type}:${agent}:${mode}`) | `:188-191` | 잎 |
| R14 | ADR-0140 방향 매핑(`top_bottom` = 위/아래) — 「유일한 진실 경계」 | `:274-275`, `:313` | **셸 기하 함수**(§2a) |
| R15 | 비율 초기 사이징 `defaultSizes`(ADR-0063) | `:276-291`, `:314` | 셸 사각형이 곧 크기 |
| R16 | 더블클릭 리셋(pane-a `preferredSize` 정수 %) | `:287-290`, `:316` | 구분선 → 50:50 명령(D5) |
| R17 | 분할 인스턴스 key `${id}:${dir}`(ADR-0223) · pane key 위치 고정 | `:292-308`, `:312`, `:316-319` | **소멸** — split id 는 구분선 key·미리보기 key 로 쓴다 |
| R18 | allotment 가 암묵으로 하던 것: 칸 최소 30px(`node_modules/allotment/dist/modern.mjs:963`) · 창 리사이즈 때 비례 배분 + 최소 클램프 + 정수 px 반올림(`:659`) · 구분선 히트 8px/호버선 4px(`dist/style.css:73-74`) · 호버 지연 300ms(`modern.mjs:436`) · 더블클릭(`:466`). 키보드 조절 없음 | — | 구분선·셸 |

allotment 에 기대는 그 밖의 것: `src/main.tsx:5-8`(CSS import) · `src/index.css:33-40`(allotment 변수를 테마 토큰으로 덮는 규칙) · `package.json:28`(+lock) · `src/components/layout/ViewLayoutRenderer.allotment.test.tsx`(437줄 — 분할 교체 4 `:179` · 같은 분할 유지 3 `:243` · 첫 프레임 2 `:291` · 오류 격리 5 `:337`) · `ViewLayoutRenderer.test.tsx` 의 stub(`:66-101`)과 거기 기대는 분할 테스트(`:563-658`) · `src/components/slot/slotDaemonRestart.test.tsx:72-74`(mock) · `src/components/slot/TerminalSlot.tsx:301`(주석) · `CLAUDE.md` 「기술 스택」. allotment 클래스(`.split-view`·`.sash`)를 고르는 셀렉터·cdp 레시피는 없다(`rg "split-view|sash" src/ scripts/ .claude/skill-bindings/` → 주석·`index.css` 뿐).

## 2. 아키텍처

### 2a. 셸 기하 함수 — 화면과 셸이 같은 사각형을 쓴다

**지금 셸에 있는 것으로는 모자란다.** 셸은 이미 잎마다 정규화 사각형을 계산하지만(`src-tauri/src/layout/spatial.rs:57-110` `assign_rects`), 그대로 쓸 수 없는 이유가 넷이다.

1. 내부값이다 — `NormRect` 는 `pub(crate)`(`spatial.rs:33`)이고 스냅샷엔 이웃·순서만 싣는다(`src-tauri/src/layout/types.rs:113-116`, ADR-0068 결정 3).
2. `{x,y,w,h}` 꼴이라 중첩에서 이웃 경계가 비트 단위로 같다는 보장이 없다(`b2.x + b2.w` 가 부모 오른쪽 경계와 부동소수 끝자리에서 갈릴 수 있다 → 화면에 머리카락 틈).
3. 인접 판정용 `[EPS,1-EPS]` 클램프가 들어 있다(`spatial.rs:78`).
4. 분할(구분선) 상자가 없다.

**더하는 것:** 새 순수 모듈 `src-tauri/src/layout/geometry.rs`(Tauri·락 무의존, ADR-0012 단독 테스트).

- 입력 `&LayoutNode`, 출력 `Vec<SlotRect>` + `Vec<SplitRect>`. 좌표는 **경계 꼴** `x0,y0,x1,y1`(뷰 기준 [0,1], `f64`).
- **자식은 부모 경계를 복사하고 새 값은 분할 경계 하나뿐이다** — `at = x0 + (x1 - x0) * ratio`. a 의 `x1` 과 b 의 `x0` 이 같은 변수라 이웃 경계가 비트 단위로 같다. 깊이와 무관하다.
- ADR-0140 매핑은 여기 한 곳이다: `left_right` → x 축 분할(a 왼쪽), `top_bottom` → y 축 분할(a 위).
- `SplitRect = { split_id, dir, x0,y0,x1,y1, at }`(이 분할 자신의 상자 + 경계 좌표).
- 비율은 여기서도 `[RATIO_MIN, RATIO_MAX]`(§2c)로 클램프한다. 쓰기 경로가 이미 보장하지만, 테스트가 트리에 직접 넣는 `0.0` 같은 값이 면적 0 인 잎을 못 만들게 하기 위해서다. *구현:* NaN 은 클램프를 그대로 지나 자손 좌표 전체로 번지므로 반분(0.5)으로 읽는다(`geometry.rs` 의 `boundary`).
- `spatial.rs` 는 자기 사각형 계산을 버리고 이 함수의 경계를 입력으로 받는다(§2h). 셸 안에서도 기하 출처가 하나가 된다.
- **픽셀 변환도 여기 있다:** `px_edge(canvas: u32, e: f64) = (canvas as f64 * e).round()`. 폭은 **반올림한 두 경계의 차**다(`round(W·x1) − round(W·x0)`) — 폭을 따로 반올림하지 않는다. 그래서 이웃 칸의 px 경계가 언제나 같은 정수다. 음수가 없으므로 Rust `round`(0 에서 먼 쪽)와 CSS `round(nearest)`(+∞ 쪽)의 동률 처리가 같다.

### 2b. 스냅샷 확장

- `ViewSnapshot` 에 `slot_rects: Vec<SlotRect>` · `split_rects: Vec<SplitRect>` · `ratio_min: f64` · `ratio_max: f64` 를 더한다(`manager.rs:140-152` 의 `snapshot()` 이 채운다 · 구조체는 지금(9단계) `types.rs:112-141`). ts-rs 로 `SlotRect.ts`·`SplitRect.ts` 가 새로 생기고 `ViewSnapshot.ts` 가 바뀐다.
  - 비율 한계 둘을 스냅샷에 싣는 이유: 화면 드래그가 셸 상수 0.1/0.9 를 따로 베껴 두지 않게 하기 위해서다. 교차 언어 골든 테스트보다 단순하다.
- 프론트 캐시 `CachedView`(`src/store/viewStore.ts:58-63`, 채우는 곳 `:242-250`)가 둘을 함께 담는다. 지금은 `slot_spatial` 조차 캐시에 안 담는다.
- 이것은 ADR-0068 결정 3(「좌표 노출 보류」)의 개정이다 — 정규화 좌표가 셸→화면 스냅샷에 실린다. **LLM 명령 표면으로 좌표·px 를 내보내는 것은 이번 범위가 아니다**(§7 R10).

### 2c. 비율 명령 — 사람과 LLM 이 같은 적용 서비스로

- **비율 저장을 `f32` → `f64` 로 올린다**(`types.rs:78`).
  - 이유: 버스로 받은 `0.3` 이 `f32` 를 거치면 `0.30000001192092896` 으로 돌아가 클램프된 것처럼 보인다. `f64` 면 받은 값이 그대로 돌아간다.
  - ts-rs 는 둘 다 `number` 로 내보내므로 `LayoutNode.ts` 본문은 그대로다(생성물 sync 게이트가 확인한다).
  - `tree.rs:269` 등 `f32` 리터럴 테스트는 형만 바꾼다.
- **순수:** `tree::set_ratio_in_tree(node, split_id, ratio) -> bool`. 상수 `RATIO_MIN = 0.1`·`RATIO_MAX = 0.9`(herdr `src/layout.rs:275-277` 선례). 옛 `clamp_ratio`(`src-tauri/src/layout/tree.rs:13`, 0~1, 테스트만 부른다)는 이 상수로 바꾼다.
  - **유한값 검사는 관리자에 둔다**(버스에만 두지 않는다). 비유한이면 `LayoutError::InvalidRatio` 이고, Tauri 경로도 같은 검사를 지난다.
- **관리자:** `ViewManager::set_split_ratio(view, split, ratio) -> Result<SplitRatioResult { ratio: f64, outcome }, LayoutError>`. `LayoutError::SplitNotFound`·`InvalidRatio` 를 더한다(`manager.rs:46-56`).
  - `outcome` 은 셋 중 하나다.
    - `Applied` — 값이 바뀌어 `bump_version`(`:187`)했다.
    - `Unchanged` — 클램프 뒤 값이 지금과 같다. 무변경·무통지.
    - `TooSmall` — px 범위가 비어 손대지 않았다. 무변경·무통지.
  - **돌려주는 version 은 `Applied` 일 때만 의미가 있다** — 그 값이 곧 이 뷰에 통지된 스냅샷의 version 이다. 관리자 version 은 전역이고(`:150`) 화면 캐시는 뷰별이라(`src/store/viewStore.ts:240-241`), 무변경일 때 전역 version 을 기다리게 하면 미리보기가 당김까지 남는다(§2f 규칙 2).
  - **그 version 은 적용 서비스가 같은 임계 구역에서 뜬 스냅샷에서 꺼낸다** — 락 안에서 쓰기 → `snapshot(view)` → 그 스냅샷의 `version` 을 반환값에 싣는다(`apply.rs:311-321` 의 `split_slot` 모양). 락을 놓은 뒤 전역 카운터를 다시 읽지 않는다. 그 사이 다른 쓰기가 끼면 통지된 스냅샷과 어긋난다.
  - **퇴화 방지(표현 가능성 규칙):** 적용 결과 트리의 어떤 잎이라도 f64 기하에서 면적 0(`x0 >= x1` 또는 `y0 >= y1`)이 되면 쓰지 않고 `outcome = TooSmall` 로 돌려준다. `geometry.rs` 로 뷰 전체를 다시 계산해 확인하며 O(잎 수)다. 조상 비율 변경이 깊은 자손을 무너뜨리는 경로까지 막는다.
    - *구현 메모(방어 경로 전용):* **새로** 면적 0 이 되는 잎만 거른다. 쓰기 전에 이미 면적 0 인 잎(쓰기 경로로는 안 생긴다 — 테스트가 심은 값 등)은 비교에서 뺀다. 그러지 않으면 그런 잎이 하나 있는 뷰에서는 모든 `set_split_ratio` 가 `TooSmall` 로 잠긴다.
    - *구현 메모:* 이로써 `TooSmall` 의 원인은 둘이다. ① px 범위가 비었다 ② 잎 하나가 무너진다. 화면 드래그 범위는 ②를 미리 알 수 없다. 16 단 이상의 사슬에서만 생기므로 받아들인다 — 그때는 확정 응답이 `TooSmall` 이고 미리보기가 뗄 때 셸 값으로 돌아간다(§2f 규칙 2).
- **셸의 px 최소 강제(이웃·순서를 지키는 1차 방어):** 소유 창의 `canvas` 와 `metrics.min_pane_px`(§2d)를 둘 다 알면, 비율 클램프 뒤에 한 번 더 좁힌다.
  - `L` = 캔버스 축 길이 × 그 분할 상자의 축 폭(`geometry.rs`, f64). `m = min_pane_px`. 허용 범위 = `[max(0.1, m/L), min(0.9, 1 − m/L)]` — 분할의 두 쪽(서브트리 통째)이 각각 `m` px 이상.
  - 요청값을 **가장 가까운 허용값으로 클램프**하고 적용값을 돌려준다(거절하지 않는다 — 드래그 경로와 같다). 범위가 비면(`L < 2m`) 현재 비율을 그대로 두고 **`outcome = TooSmall`** 로 돌려준다 — 「적용됨」과 구별된다(드래그 경로의 「잠김」과 같은 조건).
  - 둘 중 하나라도 모르면(보고 전·팝아웃 직후) 비율 클램프만 한다.
  - 셸은 px 상수를 박지 않는다 — `m` 은 화면이 보낸 값이다.
  - 화면 드래그도 **같은 식·같은 입력**(자기가 보고한 정수 캔버스 × 스냅샷 상자)으로 범위를 구한다. 그래서 확정값이 셸에서 다시 잘리지 않고, 뗄 때 튀지 않는다(§2f).
- **적용 서비스:** `apply::set_split_ratio` — `focus_slot`(`src-tauri/src/layout/apply.rs:348-366`)과 같은 모양. 출력 라우팅이 안 바뀌므로 `SubscriptionSync` 는 안 받는다. 레이아웃 스냅샷만 통지하고 탭은 통지하지 않는다(`notify` `:157-168`에 `tabs=None`).
- **사람 경로:** `#[tauri::command] set_split_ratio(view_id, split_id, ratio) -> SplitRatioApplied { ratio: f64, outcome, version }`(`src-tauri/src/commands/layout.rs` 의 `split_slot` `:321-342` 모양, 등록 `src-tauri/src/lib.rs:211` `generate_handler!`). 반환 타입은 ts-rs 로 내보내고(*구현:* `SplitRatioApplied.ts` + 결말 enum `SplitRatioOutcome.ts`), `version: u64` 에는 `#[ts(type = "number")]` 를 박는다(지금(9단계) `types.rs:136-140` 선례).
- **LLM 경로(명령 버스):** `src-tauri/src/layout/commands.rs` 선언 블록(`:71`)에 추가.
  - `"split.setRatio" => args { view_id: String, split_id: String, ratio: f64 } -> ok { ratio: f64, outcome: RatioOutcome }`(`Applied`·`Unchanged`·`TooSmall` — 필드 없는 enum 이라 매크로가 싣는다).
    - *구현:* 선언 매크로 쪽 enum 이름은 `RatioOutcome` 이다. 셸 안쪽 쌍둥이 `SplitRatioOutcome`(Tauri 답)과 타입 이름만 다르고 variant 철자는 같다(§4 「구현 중 메인이 정한 것」).
    - 유한값이 아니면 `INVALID_ARGUMENT`. 범위 밖이면 클램프한 적용값을 돌려준다(오류 아님).
    - 매크로가 `f64` 를 `{"type":"number"}` 로 싣는다(`crates/engram-dashboard-command/src/macros.rs:149`).
    - **요약문에 「ratio = a 쪽(왼쪽/위) 칸의 몫」을 적는다**(ADR-0140).
  - `"split.list" => args { view_id: String } -> ok { splits: Vec<SplitRow { split_id, dir: SplitDirection, ratio: f64, a_slots: Vec<String>, b_slots: Vec<String> }> }` — 요약문에 같은 ratio 정의와 「a = 왼쪽/위」를 적는다.
    - **LLM 이 split id 를 찾을 길이 지금 없다.** `get_view` 는 재귀 타입이라 버스에 없고(`commands.rs` 헤더), `slot.resolveSpatial`(`:266-272`)은 slot id 만 준다.
    - 양쪽 slot 목록을 주면 「x 와 y 사이 구분선」을 고를 수 있다.
  - `catalog_version` 7 → 8(`:72` — 선언이 바뀌면 올린다, `:62-70`).
- **동시 쓰기:** 셸 도착 순서의 마지막 쓰기가 이긴다. 셸은 락 하나로 직렬화하므로 추가 장치는 없다. 화면 쪽 화해는 §2f.
- **분할 경로에도 같은 표현 가능성 규칙을 건다.** 비율 0.1~0.9 만으로는 면적이 양수라는 보장이 없다 — 늘 같은 쪽 자식을 나누면 f64 경계가 부모 끝과 같아진다(0.5 로 약 54 단, 0.9 쪽으로 약 17 단). 지금 `split_in_tree`(`tree.rs:57`)엔 깊이 가드가 없다.
  - `ViewManager::split_slot` 은 새 경계 `at` 가 대상 칸 상자 안에 **엄격히** 들지 않으면(`at <= x0 || at >= x1`, 위아래 분할이면 y) 트리를 바꾸지 않는다. 새 `LayoutError::SplitTooDeep` 를 돌려주고, 버스에선 `CONFLICT`·Tauri 에선 `Err` 문자열이 된다. 패닉하지 않는다.
    - *구현:* 이미 면적 0 인 칸(쓰기 경로로는 안 생긴다 — 심은 트리)도 축과 무관하게 `SplitTooDeep` 이다. 다른 축으로 나누면 새 경계는 칸 안에 들어도 두 새 칸이 다 면적 0 이다.
  - **`min_pane_px` 미만의 칸을 만드는 분할은 허용한다**(R11 — 사용자 결정 2026-09-24). 막는 것은 위 표현 가능성 규칙(`SplitTooDeep`)뿐이다. 캔버스를 알아도 px 로 분할을 거절하지 않는다(§7 R11).

### 2d. 창 캔버스 크기와 기본 지표 — 칸별 보고 없이, 상수 없이

- **저장:** `WindowTabs`(`src-tauri/src/layout/manager.rs:58-62`)에 `canvas: Option<CanvasPx { w: u32, h: u32 }>` · `metrics: Option<UiMetrics>` 를 더한다. 생성자 3곳(`manager.rs:102,239,504`)은 `None` 으로 시작한다. 창이 닫히면 항목째 사라진다. 팝아웃 창은 새 항목이라 그 웹뷰가 보고할 때까지 `None`.
- **두 보고 명령 모두 창 label 을 인자로 받지 않는다.** `tauri::Window` 를 받아 `window.label()` 을 쓴다 — `report_view_commands` 선례(`src-tauri/src/commands/view_bus.rs:24-26`: 「잘못 적힌 label 하나가 남의 창으로 간다 — Tauri 가 넣어 주는 Window 가 유일한 권위」).
- **캔버스 보고(바뀔 때만):** `WindowLayout` 의 탭 내용 영역(`WindowLayout.tsx:182` — 모든 탭이 그 안에 `inset:0` 으로 겹친다 `:189-193`)을 `ResizeObserver` 로 잰다. 이 영역은 숨지 않는다(숨는 것은 탭 캔버스다).
  - 그 div 는 `win` 이 온 뒤에야 렌더되므로(`:152-167` 의 조기 반환) `useEffect([])+useRef` 가 아니라 **callback ref** 로 관측을 붙이고 뗀다.
  - `ResizeObserver` 가 없는 환경(jsdom — vitest 에 setupFiles 없음)에서는 관측을 건너뛴다.
  - 100ms 디바운스 → 정수 반올림 → **`w == 0 || h == 0` 이면 보내지 않는다**(0×0 RO 콜백은 실재한다 — `TerminalSlot.tsx:99-101`) → 직전 **성공한** 값과 다를 때만 `report_window_canvas(w, h)`.
  - 실패하면 직전 값을 갱신하지 않는다. RO 는 크기가 바뀔 때만 울리므로 다음 관측을 기다리지 않고 **같은 유계 재시도**(`src/util/retryInvoke.ts`)로 다시 보낸다. 그 사이 새 크기가 오면 옛 재시도는 취소한다.
  - *구현:* 무엇을 언제 보낼지는 `src/components/layout/windowCanvasReport.ts` 가 정한다(관측·디바운스만 `WindowLayout.tsx` 의 훅). 상태는 **웹뷰 싱글톤**(모듈 범위)이다 — 같은 웹뷰에서도 훅 인스턴스는 바뀌므로(오류 경계 다시 그리기·HMR 재마운트), 인스턴스 상태면 새 인스턴스가 날아가는 요청을 모른 채 하나를 더 날린다.
    - **invoke 는 웹뷰당 한 번에 하나.** 겹친 두 요청을 셸이 도착 역순으로 처리하면 낡은 캔버스가 다음 리사이즈까지 남는다(WebView2 가 발행 순서대로 올리는지는 문서에 없다 — 모른다). 날아가는 것이 있으면 그 응답 뒤에 보낸다. 대가: 응답이 끝내 안 오면 그 창의 보고가 거기서 멈춘다.
    - **보낼 값은 최신 하나만** 둔다 — 새 값이 대기 값을 덮는다. 디바운스 전의 새 관측이 보내는 중·대기 중인 값과 다르면 그 재시도·대기 값을 곧바로 거둔다.
    - 성공 응답은 취소된 작업의 것이라도 셸의 현재 값으로 기록한다 — 한 번에 하나라 응답 순서가 곧 셸 적용 순서다.
    - 셸이 받아들인 값이 바뀌면 구독자에게 알린다(`subscribeCanvasReport`) — 구분선 허용 범위가 이것을 읽는다(§2f).
  - 숨은 탭은 창 캔버스를 공유하므로 창당 하나로 충분하다.
  - 부팅·팝아웃 생성·웹뷰 재로드는 모두 `WindowLayout` 새 마운트이므로 첫 측정이 곧 보고다.
- **기본 지표 보고(웹뷰당 한 번):** `report_ui_metrics(UiMetrics { frame_insets: Insets { t, r, b, l: f64 }, min_pane_px: u32 })`.
  - `frame_insets` = **실측**. 칸 틀 안쪽 테두리 요소(§2e)의 `getComputedStyle` 테두리 폭 넷이다. 배율 125% 에서 Chromium 이 테두리를 장치 px 로 맞추면 소수가 나올 수 있어 `f64` 로 받는다.
  - `min_pane_px` = 화면의 **정책 상수** `MIN_PANE_PX`(30) — 실측값이 아니라서 별도 필드다. 이 값의 **유일한 정본은 화면**이고 셸은 받아 쓴다(§2c).
  - 첫 잎이 마운트될 때 보낸다. **「보냈음」 표식은 성공 응답 뒤에만** 세우고, 실패하면 유계 재시도한다(`src/util/retryInvoke.ts`). 표식은 모듈 수준이라 웹뷰 재로드와 함께 초기화된다.
  - **진행 중 걸쇠**: 부팅 때 잎 N 개가 한꺼번에 마운트되므로, 모듈 수준 in-flight 프라미스 하나를 두고 두 번째 이후 잎은 그것을 공유한다. 보고는 한 번만 나간다. 실패로 끝나면 걸쇠를 풀어 다음 잎 마운트가 다시 시도할 수 있게 한다.
  - *구현:* `src/components/layout/uiMetricsReport.ts`. 잎의 몸(`SlotBody` — §4)이 마운트될 때 자기 `[data-slot-border]` 를 잰다. 테두리 폭 하나라도 유한한 수로 못 읽으면 보내지 않고 오류 로그만 남긴다(셸이 거절할 값이라 보내 봐야 재시도만 소진한다).
  - 지금 칸마다 붙는 고정 장식은 틀 테두리 1px 뿐이다(`ViewLayoutRenderer.tsx:139`). 머리줄은 없다. 콘텐츠별 안쪽 여백(`DomSlot.tsx:203` 의 `4px 8px` 등)은 지표에 넣지 않는다 — 칸 종류의 사정이다.
- **셸 쪽 방어:**
  - `report_window_canvas` 는 `w == 0 || h == 0` 이면 무시하고 직전 값을 유지한다.
  - `report_ui_metrics` 는 범위를 검사해 벗어나면 거절하고(`Err`) 직전 값을 유지한다 — inset 은 유한·`0 ≤ v ≤ 64`, `min_pane_px` 는 `1 ≤ v ≤ 1000`.
- **셸 계산:** `ViewManager::slot_px(view, slot) -> Result<Option<SlotPx { frame: PxRect, content: RectF64 }>, LayoutError>`. 캔버스를 모르면 `None`. *구현:* 지표를 몰라도 `None` 이다 — 캔버스·지표를 **둘 다** 알 때만 `Some`(§4 「구현 중 메인이 정한 것」). 없는 view·slot 은 그보다 먼저 `Err`.
  - `frame` = 소유 창의 `canvas` × 그 칸의 `SlotRect` 를 §2a 규칙으로 반올림한 정수 사각형.
  - `content` = `frame` 을 `frame_insets` 만큼 줄인 것. **폭·높이는 `max(0, …)` 로 0 아래로 내려가지 않는다** — 창 최소 크기가 없고(D4) 조상 드래그로 칸이 틀 테두리보다 작아질 수 있다(D2).
  - **구분선 두께를 빼지 않는다** — 구분선은 레이아웃 공간을 먹지 않는 오버레이다(§2f).
  - 셸은 화면에 묻지 않는다.
  - 두 보고는 **버스 명령이 아니고 version 을 올리지도 통지하지도 않는다**(측정 보고이지 제어가 아니다 — 버스에 두면 LLM 이 캔버스를 위조할 수 있다). 선례: 창마다 부팅 때 한 번 보고하는 `report_view_commands`(`src-tauri/src/commands/view_bus.rs:33`).
  - 지금 소비자는 `split.setRatio` 의 px 최소 강제(§2c) 하나다. `slot_px` 자체는 앞으로의 셸 기능 몫이다(사용자 사유 — §3 D1).

### 2e. 평평한 렌더러와 반올림

- 루트 `div`(`position:absolute; inset:0; overflow:hidden`) **하나** 아래에 `slot_rects` 를 **slot id 순**으로 정렬해 `<LayoutLeaf key={slotId}>` 목록으로 그리고(D6), 그 뒤에 `split_rects` 를 **split id 순**으로 정렬해 `<Splitter key={splitId}>` 로 그린다.
  - 생존 잎은 부모·key·상대 순서가 안 바뀌어 재마운트도 DOM 이동도 없다. 바뀌는 것은 위치 값뿐이고, 새 잎은 끼워 넣고 닫힌 잎은 뺀다.
  - 구분선도 id 순인 이유: 트리 전위 순이면 LLM 의 재구성으로 순서가 바뀌고, React 가 포인터 캡처를 쥔 노드를 옮겨 `lostpointercapture` 가 난다.
- **칸 틀과 테두리를 가른다 — 틀은 사각형과 정확히 같고 테두리는 그 안에서 잘린다.** 지금은 래퍼 자체에 1px 테두리 + `border-box` 다(`ViewLayoutRenderer.tsx:139,144`). 그대로 두면 0~1px 칸에서 테두리가 할당 폭을 넘어 이웃과 겹친다.
  - 바깥 **틀** = 사각형 위치·크기 + `overflow:hidden` + **`data-slot-id` + `onClick`(click-to-focus) + `onContextMenu`**. 테두리 없음.
    - 이벤트를 틀에 두는 이유: 테스트(`ViewLayoutRenderer.test.tsx:679,706,826` 클릭 · `:781,834` 우클릭)와 cdp QA 레시피가 `[data-slot-id]` 에 이벤트를 직접 쏜다. 안쪽 클릭(아이콘 `:699` 포함)은 버블로 틀에 닿는다.
  - 안쪽 **테두리 요소** = `data-slot-border` · `position:absolute; inset:0; border:1px; box-sizing:border-box; background`. 지금 래퍼가 하던 **hasContent 에 따른 overflow/중앙정렬**(`ViewLayoutRenderer.tsx:147-158`)이 여기로 온다.
    - 콘텐츠·`+` 아이콘·포커스 링·메뉴를 담는다. 지표 `frame_insets` 도 이 요소에서 잰다.
  - 틀이 0~1px 여도 테두리 요소는 틀의 `overflow:hidden` 에 잘리므로 칸끼리 겹치지 않는다. 셸의 `frame` 이 화면의 틀과 정확히 같다.
  - 테두리·중앙정렬을 틀에서 보던 테스트는 **안쪽 요소로 겨냥을 옮긴다**(§5 — 지우지 않는다. 옮기지 않으면 `!== 'center'` 단언이 헛되이 통과한다).
- **위치 = 셸 사각형을 CSS 로 반올림.** 잎마다 인라인 사용자 속성 `--x0 --y0 --x1 --y1` 만 싣고, 규칙은 정적 CSS 에 둔다.
  - 기본: `left: calc(var(--x0)*100%)` · `width: calc((var(--x1) - var(--x0))*100%)`(top/height 동형).
  - `@supports (width: round(nearest, 1.5px, 1px))` 안: `left: round(nearest, calc(var(--x0)*100%), 1px)` · `width: calc(round(nearest, calc(var(--x1)*100%), 1px) - round(nearest, calc(var(--x0)*100%), 1px))`.
  - 이 모양은 셸의 `px_edge` 와 같은 식이다. 이웃 칸은 같은 경계식을 쓰므로 틈·겹침이 0 이다.
  - `@supports` 로 가르는 이유: `var()` 가 섞인 선언은 파싱 때 무효로 걸러지지 않고 계산 시점에 `unset` 이 되어, 같은 속성을 두 번 적는 폴백이 통하지 않는다.
- **정밀도:** 셸 px 와 화면 px 는 경계마다 **0 이 정상이고 최악 1 CSS px** 다. 1px 어긋남은 셋 중 하나에서만 난다. ① 창 캔버스의 CSS 크기가 정수가 아닐 때(배율 ≠ 1 — 셸은 반올림한 정수를 갖는다) ② 경계가 .5 동률 근처일 때(Chromium 레이아웃 단위 1/64px 양자화) ③ `round()` 미지원일 때.
- **transform·`contain` 금지** — `SlotContextMenu` 가 `position:fixed`(`src/components/slot/SlotContextMenu.tsx:121,127`)라 조상에 transform 이 걸리면 좌표계가 깨진다.
- 첫 페인트: 위치는 렌더 결과 스타일 자체다. 측정도 ResizeObserver 도 필요 없고, 창 리사이즈도 CSS 가 그 프레임에 처리한다(D4 = 순수 비례).
- props: `ViewLayoutRenderer` 이름·경로는 물론 **트리 prop 이름 `node` 도 유지한다**. 호출부 51곳(`ViewLayoutRenderer.test.tsx`의 `node=`)과 `slotDaemonRestart.test.tsx` 가 수정 없이 돈다. *구현:* prop 이름은 그대로 돌았다 — 분할 분기 테스트는 §5 대로 다시 써서 호출부 수가 바뀌었고, 뒤 파일의 그 호출은 지금(9단계) `:220` 이다(allotment mock 삭제로 밀렸다).
  - 더하는 선택 props: `slotRects`·`splitRects`·`ratioBounds`·`version`. `TabCanvas`(`WindowLayout.tsx:213-229`)가 캐시에서 넘긴다.
  - **사각형이 없고 루트가 슬롯이면 전체 상자**로 그린다. 사각형 없이 루트가 분할이면 오류 로그를 남기고 아무것도 그리지 않는다(운영 경로엔 없다 — 스냅샷이 늘 싣는다).
    - *구현:* 사각형은 칸·분할 id 집합이 트리와 **정확히** 같을 때만 쓴다(빠짐·남음·중복 없음). 어긋나면 없을 때와 같이 가른다 — 루트가 슬롯이면 전체 상자, 분할이면 아무것도 안 그린다. 둘 다 오류 로그를 남긴다(사각형이 아예 없는 단일 칸만 허용된 입력이라 조용하다). 어긋난 사각형으로 그리면 칸이 빠지고 구분선이 트리에 없는 분할을 끈다.
    - *구현:* 뷰 좌표나 비율 한계(`ratioBounds`)를 모르면 구분선을 잠근다 — 확정할 곳도 범위도 없다.

### 2f. 구분선 — 미리보기·확정·화해

- **모양:** 분할 경계 `at` 에 중심을 둔 오버레이. 레이아웃 공간 0, 히트 8px, 보이는 선 1px `var(--border)`, 호버·드래그 중 `var(--accent)`(현 `index.css:37-40` 매핑과 같은 색), `zIndex:20`(포커스 링 10 위, 메뉴 1000 아래). cdp 용으로 `data-split-id`·`data-dir` 를 단다.
- **드래그:**
  - `pointerdown` 에서 축 길이 `L` = **이 창이 셸에 보고한 정수 캔버스 px** × 스냅샷의 분할 상자 폭을 구한다. 셸 강제(§2c)와 같은 입력이다. 루트 `getBoundingClientRect()` 는 포인터 원점을 잡는 데만 쓴다. 캔버스 보고 전이면 `getBoundingClientRect()` 로 대신한다.
    - *구현:* 범위는 `pointerdown` 이 아니라 **렌더 때** 구분선마다 구한다(`useSplitDrag.ts`). 캔버스 = 이 창이 셸에 보고해 **성공한** 값이고 보고 모듈을 구독하므로(§2d) 보고가 바뀌면 다시 계산한다. 보고 전에는 루트 `getBoundingClientRect()` 를 정수로 반올림해 쓴다(알려진 한계: 렌더러가 다시 그려지지 않는 사이의 루트 크기 변화는 못 보고, 보고가 오면 그쪽을 쓴다). 상자 폭은 재사상 전 스냅샷 값이다 — 셸과 비트 단위로 같은 입력이어야 한다. `Splitter` 는 포인터가 움직일 때마다 그때의 범위를 읽고, 드래그 중 그 분할이 잠기거나 방향·split id 가 바뀌면 취소한다. 루트 rect 는 누를 때 한 번 포인터 원점·크기를 잡는 데 쓴다.
  - 허용 범위는 `[max(ratio_min, 30/L), min(ratio_max, 1 − 30/L)]` — 셸 클램프와 같은 한계(스냅샷의 `ratio_min/max` — 화면이 0.1/0.9 를 베끼지 않는다)에, 분할의 두 쪽(서브트리 통째)이 각각 30px 이상이라는 조건(D2)을 겹친 것이다. 범위가 비면(`L < 60`) 그 구분선은 움직이지 않는다.
  - `pointermove` 는 rAF 로 합친다(orca `pane-divider-drag.ts:251-253`). `setPointerCapture` 와 창 수준 리스너를 함께 건다(Chromium 캡처 순간 소실 — orca `:97`). 드래그 중엔 `body` 커서를 고정하고 `user-select:none` 을 건다. 움직임 없이 뗀 클릭은 명령을 보내지 않는다.
- **미리보기 계산 = 셸 사각형의 선형 재사상**(프론트에 두 번째 트리 기하 구현을 두지 않는다). 끌리는 분할의 상자 `[X0,X1]`, 옛 경계 `B`, 새 경계 `B'` 에서:
  - a 쪽 좌표는 `X0 + (x−X0)·(B'−X0)/(B−X0)`, b 쪽은 `X1 − (X1−x)·(X1−B')/(X1−B)`, `x == B` 는 `B'` 그대로.
  - 자손 비율을 고정한 채 비례로 줄이는 셸 재계산과 같은 결과다(부동소수 끝자리 차이는 확정 스냅샷이 덮는다). 그 서브트리 안의 구분선도 같은 사상을 받는다.
- **확정:** `pointerup` 에 `set_split_ratio` 를 **한 번** 부른다. **더블클릭** = 미리보기 없이 같은 명령에 0.5(D5).
- **화해 규칙**(미리보기 상태 = `{ token, splitId, ratio, phase: 'drag' | 'commit', awaitVersion? }`, 뷰당 하나):
  1. `drag` 중 들어온 스냅샷은 나머지를 전부 반영한다. 끌리는 분할만 미리보기 값을 유지한다. 그 split id 가 스냅샷에서 사라지면 드래그를 취소한다(캡처 해제·미리보기 삭제·명령 없음).
  2. 떼면 `commit` 으로 간다. 응답의 `outcome` 으로 가른다.
     - `Applied` — `version` 을 `awaitVersion` 에 둔다(그 뷰에 통지된 스냅샷의 version 이다). 캐시 version 이 `awaitVersion` 이상이 되는 순간(응답 시점 또는 이후 스냅샷) 미리보기를 버린다 — 그 뒤 화면 = 셸 값이다.
     - `Unchanged`·`TooSmall` — 스냅샷이 오지 않으므로 **즉시** 미리보기를 버린다. 화면은 캐시(= 셸 값)로 돌아간다.
  3. 응답 전이나 `awaitVersion` 미만의 스냅샷에서는 미리보기를 유지한다(옛 값으로 튀지 않게).
  4. 명령이 실패하면 미리보기를 버린다(셸 값으로 돌아간다) + `console.error`.
  5. 새 드래그가 앞선 확정보다 먼저 시작되면 새 `token` 이 덮는다. 늦게 온 옛 응답은 token 이 달라 무시된다.
  6. 응답은 왔는데 500ms 안에 `awaitVersion` 스냅샷이 오지 않으면(이벤트 유실) `get_view` 를 한 번 당긴다(`WindowLayout.tsx:116` 과 같은 당김).
  - *구현 — 규칙 밖 판정 둘(`useSplitDrag.ts`, 규칙 1~6 의 정본은 순수 리듀서 `splitPreview.ts` 의 `reducePreview`):*
    - **떼는 순간 미리보기가 이미 이 드래그의 것이 아니면 명령을 보내지 않는다** — 다른 구분선의 새 드래그가 가져갔거나(두 포인터 — 규칙 5), 스냅샷이 그 split id 를 지워 취소됐을 때(규칙 1). 화면에 보이던 값은 이미 그 드래그의 것(또는 셸 값)이라, 보내면 보이지 않던 값이 확정된다. 판정은 렌더를 기다리지 않고 보낸 사건을 곧바로 접은 상태로 한다.
    - 미리보기·확정 응답·당김은 그것을 시작한 뷰에 묶인다. 같은 렌더러에서 뷰가 바뀌면 미리보기와 진행 중 제스처를 버리고, 옛 뷰에서 시작한 확정의 응답·당김은 새 뷰에 닿지 않는다.
  - 결과적으로 LLM 쓰기가 사용자 확정보다 셸에 늦게 도착하면 LLM 값이 보이고, 먼저 도착하면 사용자 값이 이긴다 — 도착 순서의 마지막 쓰기.
- 호출 배선은 `viewStore` 의 기존 invoke 묶음(`src/store/viewStore.ts:169` `split` 옆)에 `setSplitRatio` 를 더한다.
- 프론트 명령 레지스트리(`window.__engramCmd`)엔 등록하지 않는다 — LLM 경로는 버스 `split.setRatio` 하나다. 같은 id 가 두 표면에서 달라지는 선례(`commands.rs` 의 `slot.popout` 주석)를 새로 만들지 않는다.

### 2g. 슬롯별 상태의 이사

- 렌더러 인스턴스가 쥐던 `lastMountRef`(`ViewLayoutRenderer.tsx:62`) · 메뉴 좌표(`:67`) · 파생값·effect(`:72-97`) · 슬롯 분기(`:99-273`) 전부를 `LayoutLeaf`(key = slot id)로 **로직 그대로** 옮긴다.
- **효과:** 분할·닫기·승격 뒤에도 보존 기억과 터미널 인스턴스가 살아남는다.
  - 분할은 원래 슬롯을 `a` 로 두고 id 를 유지한다(`src-tauri/src/layout/tree.rs:57-77`).
  - 그래서 렌더러 주석 「알려진 한계 ①」(`:57-60`)과 ADR-0223 「영향」의 「승격된 서브트리는 다시 마운트된다」가 해소된다.
  - 팝아웃은 새 slot id·새 웹뷰라 여전히 재마운트된다.

### 2h. 셸 이웃·순서 — 아주 얇은 잎에서도 안 깨지게

§2c 의 px 강제는 **끌린 분할의 두 쪽**만 지킨다. 조상 구분선을 끌면 깊은 잎은 여전히 비례로 줄고(D2), 비율 곱이 1e-4 아래로 가는 잎이 생길 수 있다. 그래서 `spatial.rs` 가 크기와 무관하게 옳아야 한다.

- **지금 처리(코드 확인):**
  - 패닉 경로는 없다. 순서는 `partial_cmp(..).unwrap_or(Equal)` 에 안정 정렬(`spatial.rs:212-222`)이라 결정적이다. 코너 선택도 동률이면 먼저 본 것을 유지한다(`:305-317`). 나눗셈이 없고, `ordinal_of[&i]`(`:241`)는 모든 인덱스가 순서표에 들어 있어 안전하다.
  - 그러나 인접 판정이 **절대 EPS** 다(`EPS = 1e-4` `:28`, `edge_eq` `:55`, `overlap > EPS` `:184`). 폭·높이가 EPS 미만인 잎에서 **패닉 없이 조용히 틀린다.**
    - ① 그 잎을 다시 나누면 하위 겹침이 정확히 EPS 라 탈락한다(이웃 소실).
    - ② 그 너머 칸이 `edge_eq` 로 인접 오판돼 얇은 잎을 건너뛴다(`:66-77` 주석, ADR-0068 「영향」:43).
    - 분할별 `[EPS,1-EPS]` 클램프(`:78`)는 경로 곱이라 이것을 못 막는다고 같은 주석이 스스로 적는다.
- **바꾸는 것:**
  - `spatial.rs` 는 `geometry.rs` 의 경계 꼴 f64 사각형을 입력으로 받는다(`leaf_rects` `:115` 대체).
  - **인접은 정확 비교**(`me.x1 == other.x0`), **겹침은 `> 0.0`** 으로 바꾼다. 근거: 절단(guillotine) 레이아웃에서 두 칸이 맞닿는 경계는 언제나 어느 한 분할의 `at` 하나이고, `geometry.rs` 가 그 한 값을 양쪽 자손에 복사한다(§2a). 그래서 **참 인접은 크기와 무관하게 비트 단위로 같아 절대 놓치지 않는다.**
  - 거짓 후보는 이론상 남는다. 모서리만 닿는 칸은 수학적으로 직교축 겹침이 0 이지만, 그 두 끝점이 서로 다른 분할 경로에서 따로 계산되면 반올림 오차만큼 겹침이 양수가 될 수 있다. ★*구현:* 그 오차는 1 ulp 로 묶이지 않는다★ — 경로가 깊을수록 몇 ulp 로 커질 수 있다(지금(9단계) 코드 주석 `spatial.rs:86-89`).
    - 이 후보는 **직교축 겹침이 가장 큰 후보를 고르는 규칙**(지금(9단계) `spatial.rs:108-113`)에서 밀린다. 내 변의 반대편은 잎들이 빈틈없이 덮으므로 내 변과 실제로 맞닿은 참 이웃이 있고, 보통 그 겹침은 오차보다 훨씬 크다.
    - ★*구현:* 보장은 아니다★ — 내 칸 자체가 몇 ulp 두께로 무너지기 직전이면 그 여유가 사라져 동률·역전이 날 수 있다(같은 주석). 테스트가 박은 것은 1 ulp 어긋남 한 경우다(`spatial.rs:625` 의 `one_ulp_corner_candidate_loses_to_true_neighbor` — 전제인 1 ulp 차이를 `to_bits` 로 먼저 실측한다).
  - 모서리만 닿는 경우(겹침 0)는 인접이 아니다 — 지금과 같다.
  - 쓰기 경로는 면적 0 인 잎을 만들지 않는다 — 비율 쓰기와 분할이 f64 표현 가능성 규칙으로 거른다(§2c).
  - **그래도 면적 0 인 잎이 들어오면 결정적으로 다룬다**(방어 — 테스트가 트리에 직접 심은 값 등): 그 잎은 이웃을 갖지 않고 누구의 이웃도 되지 않는다(인접 판정 전에 건너뛴다). 순서는 아래 규칙대로 매겨 명단에서 빠지지 않는다. 패닉 없음.
    - *구현 메모:* 모서리 토큰(`resolve_spatial` 의 `corner_slot` — `spatial.rs:305-317`)도 면적 0 잎을 건너뛴다. 이웃 제외와 같은 규칙이다. 면적 0 잎만 남는 일은 없다(뷰엔 늘 면적 양수 잎이 있다).
- **순서:** 중심점을 `f64::total_cmp` 로 비교해 NaN 에도 전순서를 만든다. 동률은 트리 전위 순(안정 정렬)으로 두어 결정적으로 유지한다.
- **걷는 것:** `EPS` 상수 · `edge_eq` · `[EPS,1-EPS]` 클램프. 기존 spatial 테스트가 회귀망이다. `:515` 의 퇴화 비율 테스트는 「geometry 클램프가 면적 0 잎을 막는다」로 대상만 바꿔 남긴다.

## 3. 사용자 결정 (2026-09-24 확정)

| # | 결정 | 버린 선택지 · 사유 |
|---|---|---|
| D1 | 드래그 크기는 **셸에 되쓴다.** 드래그 중엔 화면이 split id 로 미리보기를 하고, 뗄 때 `split.setRatio` 를 한 번 부른다. 셸은 0.1~0.9 로 클램프해 트리에 쓰고 version 을 올려 평소처럼 통지한다. LLM 도 같은 명령을 쓴다. 동시 쓰기는 셸 도착 순서의 마지막 쓰기. | 화면 전용 덮어쓰기 — **사용자:** 「어차피 레이아웃을 셸로 관리하니 사이즈가 있으면 좋을테니 진행하던지」(`spatial.rs` 이웃 계산이 이미 비율을 쓴다). 드래그 중 스로틀 전송 — **사용자 판정 기준:** 「동기화 비용 vs 사용 횟수? 정도로 판정해야될듯」(두 비용 모두 실측 없음). |
| D1+ | 셸이 **px 기하**를 가진다. UI 고정 지표는 실측해 초기화 때 한 번 보내고, 창 캔버스 px 는 바뀔 때만 보내 창 항목(`WindowTabs`)에 둔다. 칸 px = 캔버스 × 정규화 사각형. 셸은 UI 에 묻지 않는다. | (a) 바뀔 때마다 칸별 실측 크기 보고 — **사용자:** 「아니아니 내말은 기준선이나 그런건 아주 최초에 한번 보내면 된다는 얘기였는데 레이아웃 계산 완전 기본정보」(기본 정보만 한 번, 계산은 셸). (b) 캔버스 × 사각형에 셸 쪽 하드코딩 여백·오프셋 상수 — **사용자:** 「그 픽셀들은 View에서 가져와서 적립해야된다는거알지? 하드코딩하지말고.」. (c) 뷰별·칸별 저장 — **사용자:** 「어쨋든 window 클래스에 정보가 잇으면 되겠네」(한 창의 탭은 캔버스를 공유한다 — `WindowLayout.tsx:182-197`). |
| D2 | 최소 = **끌린 분할의 두 쪽(서브트리 통째)** 이 각각 30px. 드래그 중엔 화면이 강제한다. 셸은 비율을 0.1~0.9 로 자르고, 캔버스·지표를 알면 같은 30px 조건도 강제한다(§2c — 값은 화면이 보낸 것 · ★이 셸 쪽 강제는 리뷰가 더한 내부 결정이지 사용자 D2 가 아니다★). 바깥 구분선을 끌어 깊은 칸이 30px 아래로 가는 것은 **사용자가 감수했다**. 그래도 셸 이웃·순서가 깨지지 않게 설계로 막는다(§2h). | 하위 최소를 트리 전체로 전파(tmux·VS Code·Unity). **사용자:** 메인 권고의 이유를 받아들였다 — 피어 대부분이 분할의 두 자식만 보고, 전파는 트리 전체 알고리즘이 필요하다. |
| D3 | 구분선 키보드 조절 없음. 조절 = 마우스(사람) + `split.setRatio`(AI). 상대 증감 명령도 지금은 두지 않는다. | APG 구분선 포커스 · 상대 증감 명령. **사용자:** 「사실 AI에게도 크게 필요한 기능은아닌데 어쨋든 키보드로는 필요없고 마우스 AI 가 되도록해. A안이 되는건가?」(A = 추가 명령 없음). |
| D4 | 창 최소 크기 없음. 칸은 끝까지 비례로 준다(화면 = 셸 비율). | 창 최소 크기. **사용자:** 「상관없긴한데 창 크기를 구지 막을 이유는 없다고 생각」. |
| D5 | 구분선 더블클릭 = 같은 명령으로 **50:50**. | 동작 없음. **사용자:** 「필요하면 그냥 넣고. 넣는다고 손해는 아니라서」. |
| D6 | slot id 로 정렬한 평평한 목록. 기존 칸 컨테이너는 DOM 에서 움직이지 않는다(위치 값만 바뀌고, 새 칸은 끼워 넣고, 닫힌 칸은 뺀다). Tab·스크린리더 순서 ≠ 화면 순서를 **감수**한다. | 화면 순 — **사용자:** 「기존 컨테이너가 변경이 되지 않는게 좋지」(DOM 이동의 부작용 — orca `pane-split-scroll.ts:83-93` 은 `scrollTop` 초기화와 WebGL 선제 해제를, Chrome `moveBefore()` 블로그는 포커스 소실을 적는다 — 정책 조사 F6). |
| R11 | `min_pane_px` 미만의 칸을 만드는 분할도 허용한다. 막는 것은 폭이 표현 불가(0)가 되는 극단뿐(`SplitTooDeep`). | 캔버스를 알 때 px 미만 분할 거절 — **보류**. **사용자:** 「0이되는 극단적인 상황만 막으면 될것같아 일단은」 · 우려 인정: 「너무 작아서 유저가 해당칸을 조작 못하는것도 문제긴 하겠네. 적당히 막던지.」(그 뒤 극단만 막기로). 재론 조건·피어 증거 = §7 R11. |

## 4. 내가 정한 내부 선택 (보고)

- **파일:** 셸 `layout/geometry.rs`(신규) · `tree.rs`·`manager.rs`·`apply.rs`·`types.rs`·`commands.rs`·`commands/layout.rs`·`lib.rs`(수정). 화면 `src/components/layout/LayoutLeaf.tsx`·`Splitter.tsx`·`splitPreview.ts`(순수 미리보기 상태·재사상·범위)·`ViewLayoutRenderer.tsx`(평평한 루트) · 캔버스 측정은 `WindowLayout.tsx` 안 훅 · 위치 규칙 CSS 는 `src/index.css`.
  - *구현:* 화면 파일이 셋 더 섰다 — `useSplitDrag.ts`(미리보기 리듀서에 token 발급·확정 호출·당김 타이머·허용 범위를 붙이는 훅) · `windowCanvasReport.ts`(캔버스 보고 상태 — §2d) · `uiMetricsReport.ts`(지표 보고 — §2d).
- **이름:** 버스 `split.setRatio`·`split.list`(목적어 먼저 — `slot.split` 과 같은 어순) · Tauri `set_split_ratio`·`report_window_canvas`·`report_ui_metrics`(snake_case — `split_slot`·`report_view_commands` 선례).
- **셸 px 최소 = 클램프(거절 아님)** — 적용값을 돌려준다. 범위가 비면 `TooSmall` 로 무변경을 알린다(§2c). 드래그 경로와 같은 모양이라 LLM 도 사람도 같은 결과를 본다.
- **결과 신호 = `outcome` 셋(`Applied`·`Unchanged`·`TooSmall`)** — 무변경을 「적용됨」과 구별하고, 화면은 무변경이면 미리보기를 즉시 버린다. 뷰별 version 을 따로 두는 대안은 관리자 구조를 바꿔야 해 택하지 않았다.
- **비율 저장 `f64`** — 받은 값이 그대로 돌아간다. 바인딩 본문 무변경.
- **비율 한계는 스냅샷이 싣는다**(`ratio_min/max`) — 화면이 상수를 베끼지 않는다.
- **칸 틀/테두리 분리**(§2e) + 셸 `content` 폭·높이 `max(0, …)` — 0~1px 칸에서도 틀 = 사각형, 겹침 없음.
- **보고 명령 방어:**
  - label 인자 없음(`tauri::Window`).
  - 0 크기 캔버스는 화면이 안 보내고 셸도 무시한다. 지표는 범위를 검사한다.
  - inset 은 `f64`. `min_pane_px` 는 실측 inset 과 가른 정책 필드다.
  - 「보냈음」은 성공 뒤에만 세운다.
- **캔버스 측정 = callback ref + RO 부재 가드.** 실패 시 RO 를 기다리지 않고 유계 재시도한다. 지표 보고는 in-flight 걸쇠로 1 회.
  - *구현:* 캔버스 보고 상태는 `windowCanvasReport.ts` 의 **웹뷰 싱글톤**이다 — 보낼 값은 최신 하나 · invoke 는 한 번에 하나 · 성공한 값만 기록 · 바뀌면 구독자에게 알림(§2d).
- **틀 = 이벤트·`data-slot-id`, 안쪽 `data-slot-border` = 테두리·중앙정렬·내용.** 스타일 단언은 안쪽으로 겨냥을 옮기고 양성 짝을 더한다.
- **f64 표현 가능성 규칙** — 분할은 `SplitTooDeep` 로 거절하고, 비율 쓰기는 **새로** 면적 0 잎을 만들면 `TooSmall` 이다. px 기반 분할 거절은 하지 않는다(R11 — 사용자 결정).
- **반환 version = 락 안에서 뜬 스냅샷의 version.**
- **정렬:** 잎은 slot id 순, 구분선은 split id 순.
- **렌더러 prop 이름 `node` 유지.**
- **상수:** `RATIO_MIN/MAX = 0.1/0.9`(셸 — 스냅샷으로 화면에 간다) · `MIN_PANE_PX = 30`(화면만 — 셸은 지표로 받는다) · 지표 허용 범위 inset `0~64`·`min_pane_px` `1~1000` · 구분선 히트 8px · 선 1px · 호버 지연 300ms · 구분선 `zIndex 20` · 캔버스 디바운스 100ms · 확정 후 당김 대기 500ms.
- **좌표:** `f64` 경계 꼴. 셸 `round` = CSS `round(nearest)`(양수에서 같다).
- **`split.list`** 는 LLM 이 split id 를 찾을 유일한 길이라 넣었다(범위 추가).
- 잎 `React.memo` — 미리보기 중 상자가 안 바뀐 잎은 재렌더하지 않는다. 잎 안 구체 렌더러의 `key={node.id}`(`:219-224`)는 그대로 둔다.
  - *구현:* 잎을 둘로 갈랐다(`src/components/layout/LayoutLeaf.tsx`). 바깥 **위치 틀**(`LayoutLeaf`, memo — `--x0..--y1`·`data-slot-id`·클릭 포커스·우클릭 메뉴 상태)과 **몸**(`SlotBody`, memo — `:112` · ★사각형을 받지 않는다★ · `[data-slot-border]`·내용·포커스 링·메뉴·오류 경계·ADR-0148 기억)이다. 재사상이 상자 밖 사각형을 같은 객체로 돌려주므로 그 잎은 틀도 다시 그리지 않고, 사각형이 바뀐 잎도 드래그 프레임마다 다시 그리는 것은 틀뿐이다(몸이 사각형을 받으면 터미널·마크다운까지 매 프레임 다시 그린다). 구체 렌더러의 `key={node.id}` 는 그대로다.
- **구현 중 메인이 정한 것(사용자 결정 아님 — 보고, 2026-09-25):**
  - **`slot_px` 는 캔버스와 지표를 둘 다 알 때만 `Some`** — 캔버스만으로 내면 `content`(틀 − 인셋)가 틀린 값이 된다. 없는 view·slot 은 그보다 먼저 `Err`(§2d).
  - **비율 쓰기 결말의 철자는 두 표면이 같다** — 버스 `split.setRatio` 의 `RatioOutcome`(선언 매크로 enum)과 Tauri `set_split_ratio` 의 `SplitRatioOutcome` 이 variant 이름 `Applied`·`Unchanged`·`TooSmall` 을 그대로 싣는다. 타입 이름만 다르고(`ThemeOrigin`↔`ThemeSource` 와 같은 관계) 변환은 버스 핸들러가 한다. 철자는 지금(9단계) `src-tauri/tests/layout_commands.rs:1061` 의 `both_surfaces_spell_the_split_ratio_outcome_the_same_way` 가 맞댄다.
  - **버스에서 없는 view·split 은 `CONFLICT`** + 적용 서비스의 사유 문구다 — 이 표의 모든 적용 실패가 쓰는 한 코드다(`src-tauri/src/layout/commands.rs` 헤더 「적용 실패는 코드 하나로 나간다」). id 형식 불량은 `INVALID_ARGUMENT` 이고, `split_id` 형식 불량의 반려 문구는 `split.list` 를 가리킨다.

## 5. 테스트 계획

**셸(Rust)**

- `geometry.rs`(lib_unit):
  - 단일 슬롯 = (0,0,1,1). ADR-0140 두 방향.
  - 이웃 경계 `==` 동일(3단 중첩 포함). 분할 상자·`at`.
  - `spatial.rs` 사각형과 1e-6 교차 일치.
    - *구현:* ★1단계(`e401966`)에만 있었다★ — 1b(`3f8c4ef`)가 `spatial.rs` 자신의 사각형 계산을 걷어 대조 기준이 사라졌고, 테스트도 함께 걷었다. 지금은 `spatial.rs` 가 이 모듈의 경계를 입력으로 받으므로 두 셸 기하가 갈릴 자리가 없다. 셸 식과 화면 식의 일치를 재는 것은 이 테스트가 아니다 — 허용 범위는 `splitPreview.test.ts` 가 셸 px 클램프 식을 옮긴 오라클과 비트 단위로 맞대고, CSS 반올림 경계와 셸 `px_edge` 의 일치는 jsdom 에 레이아웃이 없어 GUI 실측(이음매 0 — §7 R1)만 봤다.
  - `px_edge` 반올림 · 이웃 px 경계 동일 · `content = frame − insets` · 캔버스 없음 → `None`(*구현:* 이 마지막 것은 관리자 `slot_px` 테스트 몫이고, 캔버스·지표 중 하나라도 없으면 `None` 이다).
  - **0px·1px 틀**: 캔버스를 줄여 칸 틀이 0px·1px 가 되는 트리에서 `content` 폭·높이가 0(음수 아님)이고 이웃 틀 px 경계가 겹치지 않는다.
  - 비율 한계 밖의 트리 값(`0.0`)이 클램프된다.
- `spatial.rs`(lib_unit): 기존 테스트 전부 유지. 추가:
  - 0.1 을 같은 축으로 네 단 겹친 뒤 조상을 0.1 로 두어 잎 폭이 1e-4 아래로 간 트리에서, 얇은 잎의 좌우 이웃이 정확하다(건너뜀·소실 없음).
  - 순서가 결정적이다(같은 트리 두 번 = 같은 결과). 패닉이 없다.
  - 모서리만 닿는 칸은 이웃이 아니다. 모서리 끝점이 1 ulp 어긋나 겹침이 양수가 된 거짓 후보가 있어도 참 이웃이 선택된다.
  - 트리에 직접 심은 면적 0 잎: 이웃 없음·누구의 이웃도 아님·순서 명단에 포함·패닉 없음·두 번 계산해 같은 결과.
- `tree.rs`·`manager.rs`(lib_unit):
  - 비율 쓰기: id 로 찾기 · 없는 id = `SplitNotFound`·무변경 · 비유한 = `InvalidRatio`(관리자에서) · 0.1/0.9 클램프 · `0.3` 이 정확히 `0.3` 으로 돌아온다(`f64`).
  - `outcome`: 변경 = `Applied`·version +1 · 같은 값 = `Unchanged`·version 불변 · `L < 2m` = `TooSmall`·무변경.
  - **작은 분할 허용(R11):** 캔버스·지표를 심은 세계에서 결과 두 칸이 `min_pane_px`(30) 미만이 되는 분할도 성공한다. 트리가 바뀌고 오류가 없다.
  - **퇴화 사슬(유지):**
    - 같은 쪽 자식을 0.5 로 계속 나누면 경계가 부모 끝과 같아지기 직전 단에서 `SplitTooDeep` 가 나고, 트리는 불변이며 패닉이 없다.
    - 0.9 쪽 사슬도 같다.
    - 조상 비율을 바꿔 깊은 자손 하나가 면적 0 이 될 요청은 `TooSmall`·무변경이다.
    - 두 경우 모두 결과 트리의 모든 잎이 면적 양수다.
  - px 최소: 캔버스·지표가 있으면 두 쪽 `m` px 경계로 클램프하고 적용값을 반환한다. 캔버스나 지표가 없으면 비율 클램프만.
  - 캔버스 저장: 창별 · 숨은 탭도 같은 창 캔버스로 `slot_px` · 팝아웃 새 창은 `None` · 창 닫힘과 함께 소멸 · 보고가 version 불변 · **0 크기 보고 무시(직전 값 유지)** · **범위 밖 지표 거절(직전 값 유지)**.
- `tests/layout_apply.rs`:
  - `Applied` 는 레이아웃 통지 정확히 1 회(새 비율·사각형 포함), `Unchanged`·`TooSmall` 은 0 회, 탭 통지 0. 캔버스·지표 보고는 통지 0.
  - **반환 version == 통지된 스냅샷의 `version`**(같은 임계 구역 캡처 — §2c).
- `tests/layout_commands.rs`:
  - 명단 골든(`:440-466`)에 두 이름 추가 · 세대 8.
  - `split.setRatio` 인자 검증(uuid 불량·비유한 = `INVALID_ARGUMENT`, 범위 밖 = 클램프 값 반환). 캔버스·지표를 심은 세계에서 LLM 비율이 px 최소로 클램프되고 `outcome` 이 실린다.
  - `split.list` 행의 a/b slot 목록.
  - 두 명령 요약문에 a 쪽 정의가 있다.

**화면(vitest)**

- `splitPreview.test.ts`:
  - 화해 규칙 1~6 각각. `Unchanged`·`TooSmall` 응답에서 미리보기를 즉시 버린다(당김 대기 없음).
  - 재사상: 경계 `B → B'` 정확 · 공유 경계 유지 · 서브트리 구분선.
  - 범위: 두 쪽 30px · 스냅샷 `ratio_min/max` · `L<60` 잠금 · 같은 입력(보고한 캔버스 × 상자)에서 셸 식과 같은 값.
- `Splitter.test.tsx`(루트 `getBoundingClientRect` 목):
  - down/move/up → 적용값으로 invoke 정확히 1 회.
  - 움직임 없는 클릭 → 0 회. 더블클릭 → 0.5 1 회.
  - cancel/blur → 복원·0 회.
  - jsdom 의 PointerEvent·`setPointerCapture` 지원은 **모른다** → 선택적으로 부르고 `fireEvent` 좌표로 민다.
- `ViewLayoutRenderer.flat.test.tsx`(`.allotment.test.tsx` 대체):
  - **재마운트 없음** — slot id 별 마운트 카운터 목 + 전후 DOM 노드 동일성 + `MutationObserver` 로 생존 잎이 떼어지지 않음을 본다.
    - 시나리오: 닫기(`LR{x,TB{y,z}}` → y) · 승격(`LR{TB{x,y},z}` → z) · 분할(x → `LR{x,new}`) · 같은 split id 방향 전환 · 비율 변경 스냅샷 · **ADR-0148 보존 뷰가 승격·분할 뒤 같은 인스턴스**.
  - 잎·구분선 DOM 순서 = 각각 slot id·split id 순(LLM 재구성 뒤에도 구분선 노드가 안 움직인다). 첫 렌더에 `--x0..--y1` 이 박혀 있다(ResizeObserver 전역 없이). transform 없음. `data-slot-id`·`data-split-id`.
  - 칸 틀에 테두리가 없고 안쪽 테두리 요소(`[data-slot-border]`)가 틀 안에 있다(0px·1px 틀 사각형에서도 틀 스타일이 사각형 그대로). 클릭·우클릭 핸들러와 `data-slot-id` 는 틀에 있다.
- `WindowLayout.test.tsx`(같은 커밋에 RO 스텁 — vitest 에 setupFiles 가 없고 jsdom 에 `ResizeObserver` 가 없다. 기존 목 `:37-49` 는 렌더러만 갈아 끼운다):
  - `win` 이 늦게 와도 callback ref 로 관측이 붙는다.
  - 디바운스 → 정수 → 바뀔 때만 `report_window_canvas` · 같은 크기 재전송 없음 · 0×0 은 안 보냄.
  - 실패하면 **RO 변화 없이도** 유계 재시도로 재전송한다. 재시도 중 새 크기가 오면 옛 재시도를 취소한다.
  - 지표는 성공할 때까지 재시도, 성공 뒤 웹뷰당 1 회. 잎 N 개가 동시에 마운트돼도 in-flight 걸쇠로 보고 1 회. 실패로 끝나면 걸쇠가 풀려 다음 마운트가 재시도한다.
    - *구현:* 이 줄의 단언은 `src/components/layout/uiMetricsReport.test.ts` 에 있다. 잎 마운트 쪽 한 건(잎이 늘어도 다시 안 보냄)은 `ViewLayoutRenderer.flat.test.tsx`.
  - RO 가 없는 환경에서 던지지 않는다.
- *구현:* `src/components/layout/windowCanvasReport.test.ts` — 캔버스 보고 싱글톤(§2d)의 구독 알림(성공한 값만 · 해제 · 테스트 초기화 · 초기화 전 늦은 성공 무시). 관측·디바운스·재시도는 위 `WindowLayout.test.tsx` 그대로다.
- *구현:* `src/components/layout/useSplitDrag.test.tsx` — 확정·화해 · 구분선별 제스처 token · 확정 뒤 당김(규칙 6) · 뷰 전환 · 허용 범위(보고한 캔버스 × 스냅샷 상자) · 그릴 사각형의 참조 유지.
- **유지:** `ViewLayoutRenderer.test.tsx` 의 슬롯 분기(`:231-560`)·클릭 포커스(`:660-728`)·메뉴(`:730-`) — prop 이름 `node` 그대로, 루트 슬롯 전체 상자 규칙(§2e)으로 돈다. 이벤트는 틀(`[data-slot-id]`)에 쏘는 그대로 닿는다(`:679,699,706,826` 클릭 · `:781,834` 우클릭).
- **겨냥 옮김(지우지 않는다)** — 틀에서 보던 스타일 단언을 안쪽 `[data-slot-border]` 로 옮긴다. 안 옮기면 셀렉터가 빈 배열을 돌려 던지거나 `!== 'center'` 가 헛되이 통과한다.
  - `emptyIcons()`(`:191-193`, 7 곳에서 씀) 셀렉터 `'[data-slot-id] > svg'` → `'[data-slot-border] > svg'`.
  - 테두리 단언 `:263-264`·`:270-271`(`wrapper.style.border`) → 안쪽 요소. 포커스 링 오버레이 탐색(`:272`)도 안쪽 요소 기준.
  - 중앙정렬 부재 단언 `:282`·`:291`·`:304-305`(`justifyContent`·`alignItems`) → 안쪽 요소.
  - 옮긴 뒤 **양성 짝**을 하나 더한다 — 빈 슬롯에서는 안쪽 요소가 실제로 `center` 다. 그래야 겨냥이 비어도 초록이 되는 일이 막힌다.
- **삭제·재작성:**
  - stub `:66-101` 삭제.
  - 분할 분기 `:563-586`(재귀 두 자식·자식 caps) → 셸 사각형 픽스처로 재작성.
  - `:587-599` 삭제. `:601-627` → 재마운트 없음. `:629-658` → 셸 사각형 기반.
  - `.allotment.test.tsx` 삭제. 오류 격리 5 건(`:337-436`)은 분할 트리를 그리므로 가짜 RO 를 떼고 **사각형 픽스처를 붙여** 이사한다. 그 안의 `'[data-slot-id="bad"] > svg'`(`:381`)도 `[data-slot-border]` 로 겨냥을 옮긴다.
    - 픽스처는 테스트 전용 도우미 `src/components/layout/testing/rects.ts` 가 만든다. 겹치지 않는 사각형만 보장하면 되고, 값이 셸과 같다고 단언하지 않는다 — 기하 정확성은 Rust `geometry.rs` 테스트의 몫이다.
  - `slotDaemonRestart.test.tsx:72-74` 삭제.
  - 스냅샷 리터럴을 만드는 `viewStore.test.ts`·`WindowLayout.test.tsx` 에 새 칸을 추가한다.
- **GUI 실측(`/qa full`) 필수** — jsdom 은 레이아웃이 없다. 볼 것:
  - 닫기·승격 첫 프레임 · 홀수 폭·배율 125% 에서 이음매·글자 선명도·열 수.
  - 드래그 감·확정 후 튐 없음 · LLM `split.setRatio` 반영 · 더블클릭.
  - WebGL 좌석 · 팝아웃 창 캔버스 보고 · 숨은 탭 전환.
  - `CSS.supports('width','round(nearest, 1.5px, 1px)')` 실측.

## 6. 이행 순서 (커밋마다 초록)

| # | 층 | 내용 | 걸리는 게이트 |
|---|---|---|---|
| 0 | 문서 | ADR(§8) 박기 + step-log 에 이 TRD 링크(고아 금지 — `docs/README.md:47`) | `/review doc` |
| 1 | Rust | `geometry.rs` + 테스트(소비자 없음). 비율은 `f64::from(ratio)` 로 읽는다 — 저장형이 `f32` 든 `f64` 든 컴파일된다 | 워크스페이스 회귀 · `--test lib_unit` · fmt |
| 1b | Rust | **비율 저장 `f32` → `f64`**(`types.rs:78`·`tree.rs` 리터럴) + `spatial.rs` 가 `geometry.rs` 경계를 입력으로 받고 정확 인접·`total_cmp` 순서·면적 0 잎 결정적 처리로 바뀐다 · EPS 걷기 · 얇은 잎 테스트(§2h) — 한 커밋. `spatial.rs:78` 의 `f32` EPS 클램프가 `f64` 비율과 함께 컴파일되지 않으므로 둘을 가르지 않는다 | lib_unit(기존 spatial 테스트 전부) · `--test layout_apply` · 바인딩 sync(`LayoutNode.ts` 본문 무변경 확인) |
| 2 | Rust+TS | 스냅샷에 `slot_rects`·`split_rects`·`ratio_min/max` · 새 바인딩 `SlotRect.ts`·`SplitRect.ts` + `ViewSnapshot.ts` 재생성 · `layoutTypes.ts` 재수출 · `CachedView` 저장 · 스냅샷 리터럴 테스트 갱신(같은 커밋 — 필수 필드라 tsc 가 깨진다) | lib_unit(ts-rs 내보내기) · **CI 바인딩 sync 게이트**(`src-tauri/bindings/`, 새 파일은 `git add -N` 경로로 잡힌다 — 생성물을 커밋할 것) · `npm test`·`tsc` |
| 3 | Rust+TS | `WindowTabs.canvas/metrics`(생성자 3곳) · `report_window_canvas`·`report_ui_metrics`(`tauri::Window`, 0 크기 무시, 범위 검사) · `slot_px`(content `max(0,…)`) · `WindowLayout` 캔버스 측정·보고(callback ref, RO 부재 가드) + **같은 커밋에 `WindowLayout.test.tsx` RO 스텁** — 비율 명령의 px 강제가 이 저장을 읽으므로 **명령보다 먼저** | lib_unit · layout_apply · `npm test`·`tsc` |
| 4 | Rust | `set_ratio_in_tree` · `SplitNotFound`·`InvalidRatio`·`SplitTooDeep` · `set_split_ratio`(관리자 — 유한 검사 + 비율 클램프 + px 최소 강제 + 면적 0 거름 + `outcome` · 적용 서비스는 락 안 스냅샷의 version 반환 · Tauri·`generate_handler`) · `split_slot` 표현 가능성 가드 · 버스 `split.setRatio`·`split.list`(요약문 a 쪽 정의) · 세대 8 · `SplitRatioApplied.ts`(version `number` 고정) | lib_unit · `--test layout_apply` · `--test layout_commands`(명단 골든) · 바인딩 sync |
| 5 | TS | 슬롯 분기를 `LayoutLeaf.tsx` 로 **이사만**(재귀 allotment 가 잎 자리에서 부른다) — 동작 무변 | `npm test`·`tsc` |
| 6 | TS | `splitPreview.ts` + `Splitter.tsx` + `viewStore.setSplitRatio` + 테스트(미연결) | `npm test`·`tsc` |
| 7 | TS | `ViewLayoutRenderer` 몸통을 평평한 루트로 교체(prop `node` 유지) · 칸 틀/테두리 분리 · `TabCanvas` 선택 props · 위치 CSS(`index.css` — 원문을 읽는 `src/index.css.test.ts` 게이트가 함께 돈다) · 지표 보고(첫 잎) · 분할 테스트 재작성(`:563-658`) · 틀 스타일 단언 겨냥 옮김(`emptyIcons`·`:263-305`) + 양성 짝 · 테스트 전용 사각형 도우미 · `.flat.test.tsx` 신설 · `.allotment.test.tsx` 삭제(오류 격리 5 건 이사) | `npm test`·`tsc` · **`/qa full`** |
| 8 | TS | allotment 걷기: `main.tsx:5-8` · `index.css:33-40` · `package.json:28`+lock · `slotDaemonRestart.test.tsx:72-74` · `TerminalSlot.tsx:301` 주석 | `npm test`·`tsc` · `/qa full` |
| 9 | 문서 | ADR-0223·0068·0140·0168 헤더 도장은 이미 박혔다(ADR-0227 선점 `fad7a7f` + 범위 확대). **ADR-0223 「영향」 본문은 고치지 않는다** — ADR 은 덮어쓰지 않고 누적한다(`docs/decisions/README.md:13`). 넓힌 도장(「결정 2 Allotment key와 영향의 승격 재마운트 조항」)이 그 조항을 덮는다 · 렌더러 「알려진 한계 ①」 주석과 `ViewLayoutRenderer.tsx:291` 의 잘못된 ADR-0063 인용은 렌더러 교체(7)와 함께 사라진다 · `CLAUDE.md` 기술 스택 · `qa.md:262` 줄 포인터 · `types.rs:118-120` 주석 · **`types.rs:73-75` `Split.id` 문서 주석**(「프론트는 렌더러 인스턴스를 이 값으로 가른다(`ViewLayoutRenderer`)」 → 구분선·미리보기 key) + `:71` 의 「0.0~1.0 클램프」 → 0.1~0.9(이 셋의 줄은 지금(9단계)). id 주석은 `LayoutNode.ts` 로 내보내지므로 **바인딩을 다시 구워 같은 커밋에 싣는다**(lib_unit + CI 바인딩 sync 게이트) | `/review doc` · lib_unit · 바인딩 sync · 도장이 이미 이 브랜치에 있으므로 머지 전 조건은 충족돼 있다 — 되돌리지 말 것 |

- 1~4 는 화면 동작을 바꾸지 않는다(새 필드·명령은 아직 안 쓰인다). 1b 는 셸 이웃·순서 답이 얇은 잎에서만 달라진다(옳아진다). 7 이 사용자 체감이 바뀌는 유일한 커밋이다.
- 7 이 가장 크다 — 렌더러 교체와 테스트 두 파일 재작성이 한 커밋이어야 초록이다. 2 는 Rust·바인딩·TS 테스트 리터럴이 한 커밋에 묶인다.
- *구현:* 순서는 표 그대로다(5·6 은 리뷰·QA 를 함께 돌았다). 단계별 커밋은 `docs/process/step-log.md` 의 이 구현 라운드 항목이 기록한다.

## 7. 위험·미결

- **R1 소수 px 와 xterm** — 반올림(§2e)으로 칸 경계는 정수 CSS px 가 된다. 남는 것:
  - 배율 ≠ 1 에서 정수 CSS px 도 장치 px 로는 소수라 글자가 흐려질 수 있다 — **흐려지는지 모른다**(정책 조사 F6).
  - 설치된 WebView2 런타임의 `round()` 지원은 **미확인**이다(Edge 125+ 지원 — caniuse 경유). 미지원이면 반올림 없는 % 로 떨어진다.
  - *구현 — 7단계 GUI 실측의 답(125% 배율 PC, 2026-09-25 · 출처 = `d014c0c` 본문, 인셋 값·관찰은 그 라운드 인계 메모 `.claude/handoff/history/20260925-085850-평평한-렌더러-1-7단계-완료-다음은-8단계.md:26-28`):*
    - 이 WebView2 는 `round()` 를 지원한다 — 반올림 경로가 실제로 돈다.
    - 칸 테두리 인셋 실측 = **0.8px**(125%) — `frame_insets` 를 `f64` 로 받은 까닭이 실제로 나왔다.
    - 칸 사이 이음매 **0** — 홀수 폭 · DSF 1.0/1.25/1.5 · 헬퍼로 돌린 네이티브 창 리사이즈(손으로 창 테두리를 끄는 것은 사용자 GUI 확인 몫).
    - 관찰(결함 아님): 루트 CSS 크기가 소수일 때 바깥 테두리가 0.2~0.4 CSS px 넘치거나 모자란다(§2e 정밀도 ①) · 구분선의 선은 반올림 없이 `at × 100%` 에 놓여 최대 약 0.4px 어긋난다(같은 색이라 안 보인다).
    - ★남은 것 — 글자가 흐려지는지·열 수가 안정한지는 **미측정**★이다. 실 에이전트 터미널 칸이 필요해 사용자 GUI 확인 몫으로 넘겼다.
- **R2 드래그 중 PTY 리사이즈** — 미리보기가 칸 크기를 매 프레임 바꾸고, `TerminalSlot.tsx:93-119` 가 RO 마다 `fit()` 하고 PTY 전송은 50ms 디바운스다. 지금과 같은 부류다. orca 의 「드래그 중 PTY 리사이즈 보류」(`pane-pty-resize-hold.ts`)는 넣지 않았다. *구현:* 드래그 중 PTY 리사이즈 체감은 **미측정**(실 에이전트 필요 — 사용자 GUI 확인 몫).
- **R3 아주 얇은 잎** — 두 겹으로 막는다. ① `split.setRatio` 가 캔버스·지표를 알면 끌린 분할의 두 쪽에 px 최소를 강제한다(§2c). ② 그래도 조상 드래그로 생기는 얇은 잎은 `spatial.rs` 가 정확 인접·`total_cmp` 순서로 크기와 무관하게 옳게 다룬다(§2h). 남는 것: 캔버스·지표 보고 **전**(부팅 직후·팝아웃 직후)에 들어온 LLM 비율은 0.1~0.9 로만 잘린다 — 화면 최소 30px 은 깨질 수 있지만 이웃·순서는 ②로 옳다.
- **R4 이벤트 유실 시 미리보기 고착** — 화해 규칙 6(당김)이 막는다. 당김도 실패하면 다음 스냅샷까지 미리보기가 남는다. 무변경 응답(`Unchanged`·`TooSmall`)은 스냅샷을 기다리지 않으므로 이 경로에 들지 않는다.
- **R4b 틀보다 작은 칸** — 창 최소 크기가 없고(D4) 조상 드래그로 깊은 칸이 줄어(D2) 칸이 테두리 두께보다 작아질 수 있다. 틀/테두리 분리(§2e)와 셸 `content` 의 `max(0, …)`(§2d)로 겹침·음수 크기는 없다. 그 크기의 칸은 내용이 잘려 보일 뿐이다.
- **R11 작은 칸의 분할 — 결정됨(사용자 2026-09-24)**
  - **결정:** `min_pane_px` 미만의 칸을 만드는 분할을 **허용한다.** 막는 것은 한쪽이 표현 가능한 폭 0 이 되는 극단뿐이다 — 기존 f64 표현 가능성 규칙(`SplitTooDeep`, §2c). 사용자: 「0이되는 극단적인 상황만 막으면 될것같아 일단은」.
  - **보류(기각 아님):** 「캔버스를 알 때 `min_pane_px` 미만이면 분할 거절」. 사용자도 우려를 인정했다 — 「너무 작아서 유저가 해당칸을 조작 못하는것도 문제긴 하겠네. 적당히 막던지.」 — 그 뒤 극단만 막는 쪽을 골랐다.
  - 기각 근거의 성격: **사용자 선호(일단은)**.
  - **재론 조건:** 작은 칸을 조작하지 못하는 것이 실제 문제로 보고되거나, LLM 이 칸을 되풀이해 쪼개는 것이 관측될 때.
  - **재론 때 볼 피어 증거(메인 코드 확인):**
    - tmux 는 공간이 없으면 「no space for a new pane」으로 분할을 거절한다(`tmux/layout.c:1410`, `cmd-split-window.c:153`).
    - zellij 는 명시 분할에서 칸이 최소 크기(`MIN_TERMINAL_WIDTH`·`MIN_TERMINAL_HEIGHT` = 5 — `zellij-server/src/tab/mod.rs:154-155`)의 2 배에 못 미치면 거절한다(`zellij-server/src/panes/tiled_panes/mod.rs:697-747` 의 `can_split_pane_*` — 로컬 클론 `bf8d23a` 확인). 자동 배치(`tiled_pane_grid.rs:1374-1377`, `find_room_for_new_pane`)도 같은 부류의 조건이지만 명시 분할의 근거는 아니다.
- **R4c 캔버스는 버전 없는 최신값** — 디바운스 100ms 만큼 늦을 수 있다. 지금 소비자(px 최소 클램프)는 그 오차를 견딘다(ADR-0068 과의 차이는 §8).
- **R5 팝아웃** — 새 slot id·새 웹뷰라 재마운트는 못 피한다. 새 창 캔버스는 첫 보고 전까지 `None`.
- **R6 숨은 탭** — 캔버스는 숨지 않는 창 내용 영역에서 잰다(§2d). 숨은 탭의 슬롯 숨김 감지(`TerminalSlot.tsx:94-108`)와 WebGL 좌석(가시성 관측 `:224`)은 그대로다.
  - *구현(선재 관찰 — 이 변경이 만든 것 아님):* 구독 직후의 초기 `fit()`+`resizePty`(지금(9단계) `TerminalSlot.tsx:304-305`)엔 숨김 가드(`offsetParent`)가 없다 — 같은 전송을 하는 RO 콜백(`:99-101`)·비우기 콜백(`:285`)·재연결(`:355`) 세 경로는 건다. WebGL 부착 경로(`:211-213`)도 같은 전송을 하지만 이 가드 대신 IntersectionObserver 가시성(`:224-227`)으로 거른다. 재현하지 않았고 손대지 않았다.
- **R7 DOM 순서 ≠ 화면 순서**(D6 감수).
- **R8 성능** — 스냅샷에 사각형 O(N)이 늘고, 확정마다 스냅샷 1 개가 온다(모든 잎 재렌더, 재마운트 0). 미리보기는 끌리는 서브트리 잎만 바꾼다. 실측 없음.
  - *구현(후속 후보):* 몸(`SlotBody`)의 memo 는 `node` 를 참조로 비교하는데 캐시는 스냅샷의 트리를 그대로 담으므로, 스냅샷마다 모든 칸의 몸이 다시 그린다(드래그 프레임에서는 안 그린다 — §4). `node.id` + `content` 값 비교로 줄일 수 있다. 코드 판독이고 실측 없음.
- **R9 표면 철자 둘** — 버스 `split.setRatio` ↔ Tauri `set_split_ratio`. 기존 `slot.split` ↔ `split_slot` 과 같은 관례라 새 분열은 아니다.
- **R10 px 의 LLM 노출** — 셸이 px 를 알지만 버스로 내보내는 명령은 두지 않았다(외부 소비자 없음). 필요해지면 `split.list`·`slot.resolveSpatial` 답에 싣는 것이 후속이다.
- **모르는 것** — jsdom PointerEvent 지원 · 배율 125% 에서 `getComputedStyle` 테두리 폭 값 · Chromium 이 떼었다 붙인 xterm 의 WebGL 을 잃는지(D6 로 피해 가지만 검증 안 함).
  - *구현:* 테두리 폭은 답했다(0.8px@125% — R1). jsdom 포인터 캡처는 여전히 모른다 — 테스트는 계획대로 선택적으로 부르고 좌표로 민다. WebGL 좌석은 미측정(실 에이전트 필요).

## 8. ADR 제안

- **박았다: ADR-0227** 「분할 렌더러를 평평한 칸 목록으로 직접 구현하고 구분선 드래그를 셸 비율 명령으로 되쓴다」. 이 절은 그 ADR 의 초안이었고, 이제 정본은 ADR 본문이다. 아래는 대응 관계만 남긴다.
- **결정:**
  1. 자체 평평한 렌더러(잎 key = slot id · 한 부모 · slot id 순 · 셸 사각형 + CSS 반올림 · 구분선 오버레이).
  2. `split.setRatio`(0.1~0.9 클램프 + 캔버스·지표를 알면 두 쪽 px 최소로 클램프, 드래그 끝 1 회, 도착 순 마지막 쓰기, 더블클릭 50:50) + `split.list`.
  3. 창 캔버스 px 를 창 항목에, 실측 지표(틀 여백·최소 칸 px)를 초기화 때 1 회 두고, 셸이 칸 px 를 계산한다.
  4. `spatial.rs` 는 셸 기하의 경계를 받아 정확 인접으로 판정한다 — 얇은 잎에서도 이웃·순서가 옳다.
  5. D2~D6.
- **거부한 대안:**
  - 렌더러(라이브러리 조사 선택지 표):
    - **A** allotment + 비율 `defaultSizes` — 닫기·승격 때 형제 서브트리 재마운트가 남는다. 방향이 마운트 때만 읽혀 key 재마운트가 필요하다. 상류가 저활동이다(F2).
    - **B** react-resizable-panels v4 중첩 — 백엔드 트리를 그대로 비추면 재마운트가 남는다. 통과 노드로 막으면 React 트리가 백엔드와 어긋나고 `setLayout` 동기화를 따로 짜야 한다(F3).
    - **C** react-mosaic — `react-dnd` 계열 다섯(16.0.1, 2022-06 이후 배포 없음)과 `lodash-es` 등이 딸려 오고 트리 변환이 필요하다(F5).
    - dockview·golden-layout·Lumino·FlexLayout(자기 모델 소유·명령형) · Split.js·react-split(정체)(F6).
  - 크기·기하: D1·D1+ 표의 버린 선택지(§3).
- **개정되는 옛 결정**(도장 문구는 세 곳 — 옛 ADR 상태줄·옛 ADR 관련줄·ADR-0227 관련줄 — 이 같다):
  - **ADR-0223** 「결정 2 Allotment key와 영향의 승격 재마운트 조항」 → 폐기. 결정 1(split id)은 유지하고 구분선·미리보기 key 로 쓴다. 「영향」 본문은 고치지 않는다(누적 원칙 — `docs/decisions/README.md:13`).
  - **ADR-0068** 「결정 3과 거부 대안 백엔드 투영 px와 영향의 실측 픽셀 및 최소 칸 조항」:
    - 결정 3(좌표 노출 보류 — `docs/decisions/0068-*.md:21`) → 정규화 좌표를 스냅샷에 싣고 셸이 px 를 계산하는 쪽으로 개정한다.
    - 거부 대안 「백엔드 투영 물리 px」(`:26`)는 **되살린다.** 당시 거부 사유(gutter·sash·min-size·collapse 모델링 → 렌더 엔진 중복·drift)는 이 설계에서 성립하지 않는다. 렌더러가 셸 사각형을 그대로 그리고, 구분선은 공간 0 오버레이이며, 테두리는 가정하지 않고 실측한다.
    - `:28`(정규화 ≠ 실제 렌더 — sash·min-size·border 때문)은 같은 이유로 구조상 해소된다.
  - ADR-0068 「영향」의 「실측 픽셀 추가 시 프론트 실측 rect 는 **versioned 관측값** — 창크기/render epoch 가 바뀌면 stale」(`:41`, 결정 3 `:21` 의 같은 모델) → **의도적으로 벗어난다.** 이 설계가 받는 실측은 칸별 rect 가 아니라 창당 캔버스 크기 한 쌍(+ 한 번 보내는 지표)이고, 버전 없이 최신값만 둔다.
    - 이유: 레이아웃 권위(사각형)는 여전히 셸 트리이고, 실측은 곱하는 배율 하나라 이중 권위가 생기지 않는다.
    - 늦을 수 있는 폭은 디바운스 100ms 이고, 지금 소비자(px 최소 클램프)가 그만큼의 오차를 견딘다(R4c).
    - 버전을 붙이는 것은 px 를 LLM 에게 내보낼 때(R10) 다시 본다.
  - ADR-0068 「영향」:43 의 「리사이즈 명령은 UX 최소 칸 크기 강제」 → **대체(REPLACED)한다 — 충족이 아니다.**
    - 최소 크기 강제는 끌린 분할의 두 쪽에만 걸린다. 깊은 칸 30px 미만은 감수했고(D2), 30px 미만을 만드는 분할도 허용한다(R11).
    - 결함 ①②는 정확 인접으로 고친다(`EPS`·`edge_eq` 걷기 — §2h).
  - **ADR-0140** 「영향의 allotment 매핑 진실 경계」 → 진실 경계가 셸 `geometry.rs` 의 방향 매핑으로 옮겨 간다. 그 자리에 `// ADR-0140` 앵커를 단다.
  - **ADR-0168** 「결정 7 allotment sash 토큰」 → `index.css:33-40` 이 allotment 와 함께 사라진다. 새 구분선은 테마 토큰 `var(--border)`/`var(--accent)` 를 직접 쓴다.
  - **드래그 되쓰기 범위 제한** → D1 로 해소한다. 출처는 ADR-0063 이 **아니라** `step-log.md:776`(그 슬라이스 미해결 ①)·`:2506`·ADR-0223 결정 2 의 괄호다. 렌더러 주석 `ViewLayoutRenderer.tsx:291` 의 ADR-0063 인용이 잘못이었다(ADR-0063 도장은 되돌렸다).
- **거부 사유의 출처:**
  - D1~D6·R11 의 거부 사유는 사용자 발언 원문이다(§3 표 인용 — 2026-09-24).
  - 렌더러 A 는 「일단 쭈그림만 막고 머지하고」, B 는 「하긴 개념적으로 하위개념이 아닌데 트리로 되는게 이상한거지」 + 조사 F3 이다.
  - C 는 메인이 D 를 권하며 C 의 비용을 정체된 `react-dnd` 계열로 적은 질문에 사용자가 「ㅇㅇ 이것 상관없이 쭉 진행하고, 직접 구현하는걸로 하자」로 답한 것이다 — 사유 서술은 조사 F5.
  - 각 사유의 강도 표시는 ADR-0227 「거부한 대안」이 정본이다.
