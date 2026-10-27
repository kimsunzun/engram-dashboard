# S28 TRD — LLM 레이아웃 명령 (T-65) — 3판 (2026-10-26)

- **상태:** 3판 · 메인 작성(사용자 외출 중 자율 진행 「쭉 진행해」 · 명령 세부 = 사용자 위임 2026-10-26 「잘 지시되는쪽으로 알아서」). 1판(`76d29cd0`)을 `/review trd full`(codex Designer · Claude Architect-breaker — 둘 다 FIX) 지적으로, 2판(`61c66724`)을 Claude 재확인(FIX) 지적으로 고쳤다 — 고친 것 = §12. 사용자가 돌아오면 볼 결정 = §11.
- ★**ADR-0308(2026-10-27)이 이름 풀기 경로를 바꿨다**★ — 셸이 데몬에 되묻는 §5-1 대신, 셸이 등록 때 「에이전트 참조」로 표시한 인자를 데몬이 중계 전에 id 로 바꿔 넘긴다. §5-1 · §6 의 되묻기 서술과 미커밋 U3a · U3b 의 되묻기 코드는 다음 세션에서 이 결정대로 고쳐 쓴다(조사 = `docs/research/agent-name-resolution-relay-vs-callback-2026-10-27.md`).
- **규칙:** ADR-0307 · `docs/reference/llm-command-conventions.md`(이하 「규칙」).
- **옛 설계:** `docs/process/S21-slot-docking/prd.md` R11b(LLM 명령은 창 활성 · 보이는 탭 · 포커스를 안 바꾼다) · R16 · Q10 · D9 · D22(탭 순서 바꾸기도 T-65) · `docs/process/S21-slot-docking/trd.md` §3-6(`FocusPolicy::Keep` · 기본 share · 새 창 크기를 마우스와 같은 함수로 · 예열 창 그대로) · §9-5 · ADR-0297(끄는 중에도 LLM 명령은 막지 않는다).

## 1. 범위

| 묶음 | 내용 |
|---|---|
| A | 데몬 명령 `agent.resolve` — 이름/id **여럿을 한 번에** → `{id, name, live}`(데몬 해석기 `resolve_in` 그대로 · ADR-0132) · `resolve_in` 의 NOT_FOUND / CONFLICT 문구에 후보를 싣는다 |
| B | 셸 대상 풀기 — 에이전트 이름 → (데몬 왕복 한 번) id · **id · 공간 낱말 → 칸 · 탭 · 창은 적용과 같은 잠금 안에서** · 배치 줄 짓기 · `DockError` → 버스 오류 코드 |
| C | 새 명령 — `tab.move` · `tab.dock` · `tab.detach`(셋 다 `apply::dock_tab` · `FocusPolicy::Keep`) · `layout.get` |
| D | 기존 칸 · 탭 명령 넓히기 — 이름 · 공간 낱말로 지목 · 배치를 바꾸면 응답에 바뀐 배치 |
| E | `prompts/engram-help.md` 「## window」 를 이름 기준으로 간결히 다시 쓰기 · CLAUDE.md 「LLM-우선 제어」 남은 갭 줄 고치기 |

**범위 밖:** 프론트(웹 레지스트리 · 렌더) — 셸 표에 선언한 이름은 웹 등록에서 자동으로 빠지고(`view_commands.rs` `reserved_names()`), 배치 알림은 적용 서비스가 이미 낸다. 「크면 파일로」 안전장치(자리 미정 — ADR-0307 결정 6). 칸 번호 지목(ADR-0307 결정 8). 탭 이름으로 지목(이름이 유일하지 않고 자동 이름은 포커스로 바뀐다 — `layout/tab_name.rs`).

## 2. 옮기는 단위 = 탭(마우스와 같다)

- `apply::dock_tab` 은 **탭(View) 하나를 통째로** 옮긴다(`layout/dock.rs` `docked_tree` · `ViewManager::dock`). 한 칸만 빼내 다른 칸 옆에 붙이는 길은 없다(`slot.popout` 은 칸 하나를 **새 창의 새 탭**으로만 뺀다).
- 그래서 원본을 에이전트로 고르면 = **그 에이전트가 든 칸이 속한 탭**. 그 탭에 칸이 여럿이면 **탭째 옮긴다**(§11-1). 응답 배치에 함께 옮겨진 칸들이 드러난다.
- **`tab.dock`(칸 옆에 붙이기)은 원본 탭을 지운다** — 원본 탭의 칸들이 목적 탭 안으로 들어간다(`manager.rs` 의 Seam 경로 `remove_view`). 응답의 `tab` 은 **살아남은 목적 탭**이다(§4-4).

## 3. 인자 이름 — 종류마다 따로, 뜻이 겹치지 않게

| 무엇을 고르나 | id 로 | 에이전트로(이름 또는 id — 데몬이 푼다) | 공간 낱말로 |
|---|---|---|---|
| 탭 | `tab` | `tab_of` | — |
| 칸 | `slot_id` | `slot_of` | `at` (+ `window`) |
| 붙일 자리(칸) — `tab.dock` | `beside_slot` | `beside` | `beside_at` (+ `window`) |
| 옮길 자리 — `tab.move` | `to_window`(ADR-0302 창 인자) · `after_tab`(그 탭 바로 뒤) | `after_tab_of`(그 에이전트 탭 바로 뒤) | — |
| 칸에 넣을 에이전트 | `assign`(이름 또는 id) | | |

- 한 줄(무엇을 고르나) 안에서 **정확히 하나**를 준다 — 둘 이상 / 하나도 없음 = `INVALID_ARGUMENT`(필수인 줄만).
- `slot_of` · `beside` 의 에이전트가 **여러 칸에 떠 있으면** `CONFLICT` + 후보(그 칸들의 `slot_id` · 탭 · 창) — 다음 호출에 `slot_id` · `beside_slot` 으로 바로 고친다. `tab_of` · `after_tab_of` 는 **탭 단위**로 본다 — 한 탭 안 여러 칸이면 그 탭으로 풀리고(탭을 고르는 데는 모호함이 없다), 여러 탭에 떠 있을 때만 `CONFLICT` + 후보(`tab` 으로 고친다).
- 어느 칸에도 없으면 `NOT_FOUND` + 후보(칸에 떠 있는 에이전트 이름 최대 5).
- `at` · `beside_at` 은 `window` 의 **보이는 탭**에서 푼다(`apply::resolve_spatial` 그대로 · `window` 없으면 main). 낱말이 칸을 못 집으면 `NOT_FOUND`.
- 창 인자는 ADR-0302 그대로(영속 id · 같은 실행 label · 번호 거부 — `layout/window_arg.rs`).
- id 는 버스 매크로 규칙대로 `String`(처리기가 Uuid 로 푼다).
- 옛 인자 `agent_id`(`slot.assignAgent` · `layout.setSlotContent` 의 넣을 에이전트)는 **받던 그대로 받는다**(옛 호출자) — 새 호출은 `assign`. 둘 다 = `INVALID_ARGUMENT`.

## 4. 새 명령

### 4-1. `tab.move` — 탭 줄로 옮기기 · 순서 바꾸기 (`DockDest::TabBar`)

- args: `tab?` · `tab_of?` · `to_window?` · `after_tab_of?` · `index?: u32`
- 목적 = `to_window`(그 창 탭 줄 · `index` 자리 — 0 부터 · 없거나 끝을 넘으면 맨 끝) | `after_tab_of`(그 에이전트가 든 탭 **바로 뒤** · `index` 를 같이 주면 `INVALID_ARGUMENT`) | 둘 다 없으면 **원본의 창**(같은 창 안 순서 바꾸기 · `index` 필수).
- `after_tab_of` 는 **목적도 `Target` 으로**(§5-2 — `AfterTabOf(AgentId)`) 넘겨 **잠금 안에서** 그 에이전트의 탭과 그 탭의 창을 찾고, 원본을 목록에서 뺀 뒤 그 탭 바로 뒤 자리를 계산한다(같은 창에서 원본이 앞에 있을 때 하나 어긋나는 것을 막는다). `DockTo::TabBar`(`check_dock` 이 읽는 쪽)와 `DockDest::TabBar` 에 같은 「뒤에」 칸을 더한다 — 마우스 경로는 비운다.
- 가장자리: 그 에이전트가 **원본 탭 안**에 있으면(제 뒤에 넣기) `INVALID_ARGUMENT` · 여러 탭에 떠 있으면 `CONFLICT` + 후보.

### 4-2. `tab.dock` — 칸 옆에 붙이기(분할) (`DockDest::Seam`)

- args: `tab?` · `tab_of?` · `beside_slot?` · `beside?` · `beside_at?` · `window?` · `side: DockSide` · `whole?: bool` · `share?: f64`
- `side` = 새로 붙는 탭이 놓일 쪽(`Left` · `Right` · `Top` · `Bottom` — `layout/dock.rs` `DockSide` 와 같은 철자 · 버스 블록에 같은 이름 enum 을 하나 더 둔다 — 매크로는 블록 밖 enum 을 못 쓴다).
- `whole: true` = 목적 칸이 속한 **탭 전체의 바깥 변**(`DockAnchor::Outer`). 기본 = 그 칸(`DockAnchor::Slot`).
- `window` 는 `beside_at` 과만 쓴다(다른 것과 함께면 `INVALID_ARGUMENT`).
- `share` = 붙는 쪽 몫(0.01~0.5 — `InvalidShare` 범위 그대로). 없으면 **마우스와 같은 공식** `bands::attach_share(original, side, 목적 px 틀)` — `original` = **원본 탭 창의 캔버스 크기**(셸 모델 `windows[label].canvas`) · 목적 px 틀 = 목적 칸 px(`slot_px`) | `whole` 이면 목적 창 캔버스. 입력이 없으면(웹뷰가 크기를 아직 안 알림) **0.5**. ★마우스와 다른 점★: 마우스는 `original` 을 웹이 잰 `drag_sizes().original`(원본 창이 최대화면 복원 크기)로 쓴다 — 셸 모델에는 그 입력(모니터 작업 영역 · 창 틀 · 뷰포트)이 없어 캔버스로 갈음한다(§11-6).
- 원본 탭 = 목적 칸이 든 탭이면 `SelfTarget` → `INVALID_ARGUMENT`.

### 4-3. `tab.detach` — 새 창으로 떼기 (`DockDest::NewWindow`)

- args: `tab?` · `tab_of?`
- 크기 = `plan_new_window` 기본(원본이 팝아웃이면 그 보통 크기 · main 이면 팝아웃 기본 크기). 자리 = 커서가 없으므로 label 기본 자리. 예열 창을 그대로 쓴다(활성화 없이 z 만 — `Keep`). ★마우스와 다른 점★: 마우스는 `drag_size::drag_sizes` 로 main 복원 크기를 상한까지 쓰는데, 그 입력(모니터 작업 영역 · 창 틀 · 뷰포트)이 셸 모델에 없다 — 그 입력을 셸로 올리는 포트는 이번 범위 밖(§11-6).
- **두 잠금 사이 지킴**: 새 창 적용은 잠금을 두 번 잡는다(계획 → 잠금 밖에서 창 열기 → 커밋 · `apply.rs` 의 `dock_into_window`). 커밋 잠금에서 대상을 **다시 `locate`** 해 첫 잠금 때와 다른 탭이면 연 창을 거두고 `CONFLICT`(아무것도 안 옮김).
- 원본이 **탭 하나뿐인 팝아웃**이면 `plan_new_window` 가 창째 다시 쓰고(`reused`) 바뀌는 것이 없다 → `ok` + `changed: false`.

### 4-4. 응답(셋 공통 — 규칙 §1-4)

```
TabMoveOk / TabDockOk / TabDetachOk {
  changed: bool,          // 바뀐 것 없음(같은 자리 · 이미 혼자인 팝아웃) = false
  window: String,         // 탭이 놓인 창 label
  window_id: String,
  number: u32,            // 창 번호(사람이 보는 표식 — 인자로는 안 받는다 · ADR-0302)
  tab: String,            // 결과 탭: move · detach = 옮긴 탭(그대로) · dock = 살아남은 목적 탭
  layout: Vec<LayoutRow>  // 바뀐 탭들의 배치(§5 줄) — 목적 탭 + 원본 창에서 새로 보이게 된 탭
}
```

- `DockOutcome` 에 `changed: bool` 을 더한다(지금은 「바뀐 것 없음」이 알림만 빠지는 꼴이라 몸통이 모른다).
- 다시 쓴 창(혼자인 팝아웃 재사용)이 그 사이 닫히면 그 탭도 모델에서 사라진다 — 결말을 읽는 잠금에서 대상을 다시 풀어 다르거나 없으면 `CONFLICT`(옮기려던 탭이 없어졌다 · 구현 = `TargetDockError::Moved`). 마우스 경로(`identity == None` → 성공)는 그대로.
- `emptied` · `main_refilled` 은 `layout` 에 드러나므로 따로 싣지 않는다.

### 4-5. `layout.get` — 배치 조회 (#[effect(Read)])

- args: `window?` · `agent?`(그 에이전트가 든 칸의 줄만 — 핀포인트 · 이름은 데몬이 푼다) · `ids?: bool`(기본 false)
- ok: `{ rows: Vec<LayoutRow> }` — **칸마다 한 줄, 평평하게**(매크로가 재귀 타입을 못 싣는다):

```
LayoutRow {
  window: String, window_number: u32,
  tab_index: u32, tab_name: String, tab_active: bool,   // tab_index = 그 창 탭 줄 자리(0 부터 · 탭 묶음 열쇠 — 이름은 겹칠 수 있다) · tab_active = 보이는 탭
  content: SlotKind,                          // Empty | Agent | AgentList | PresetPalette | Usage | Unknown
  agent: Option<String>,                      // content = Agent 의 표시 이름(셸 캐시)
  focused: bool,                              // 그 탭의 포커스 칸
  x: f64, y: f64, w: f64, h: f64,             // 탭 캔버스 안 정규 좌표(0~1 · 소수 둘째 자리) — 좌우 · 위아래 · 몫이 드러난다
  // ids = true 일 때만:
  window_id: Option<String>, tab: Option<String>, slot_id: Option<String>, agent_id: Option<String>
}
```

- 좌표 = `geometry::compute`(셸 안 기하의 단일 출처 — ADR-0227)의 칸 사각형. 픽셀 측정 없이도 나온다.
- 기본은 id 를 싣지 않는다(규칙 §4 — 크기 · LLM 은 이름 · 낱말로 부른다). 에이전트 없는 칸을 id 로 짚어야 하면 `ids: true`.
- 창 순서 = `window.list` 순 · 탭 순서 = 탭 줄 순 · 칸 순서 = 트리 전위 순.
- **표시 이름은 셸 캐시(`agent_names`)** — 풀이가 아니라 보여 주기라 ADR-0132 의 「해석기 하나」에 걸리지 않는다. 캐시에 없으면 `agent: None`.

## 5. 대상 풀기 (묶음 B)

### 5-1. 이름 → id: 데몬 왕복 한 번

- 처리기가 인자 속 에이전트 말(`tab_of` · `slot_of` · `beside` · `after_tab_of` · `assign` · `agent`)을 **모아 한 번에** `agent.resolve{targets: [...]}` 을 데몬에 보낸다(`DaemonClient::send_command(AgentCommand::Command{envelope})` — `agent.spawnInto` 가 데몬을 기다리는 길과 같은 꼴 · 포트 trait 로 끊어 시험은 가짜로). 셸이 자기 캐시로 이름을 풀지 않는다(ADR-0132 · 규칙 §1-2).
- 시한: CLI → 데몬 → 셸 배달 시한이 7 초(`crates/engram-dashboard-daemon/src/command_delivery.rs` `DEFAULT_DEADLINE`). 이름 왕복은 **한 번 · 2 초**로 묶는다 — 넘으면 **적용하지 않고** `CONFLICT` + 재시도 `AfterCondition`(「이름 풀기에 데몬이 답하지 않았다 — 아무것도 바꾸지 않았다」 · 「아무것도 안 했다 · 잠시 뒤 다시」의 기존 꼴 = `layout/commands.rs` 의 설정 읽기 실패). `TIMEOUT`(결과를 모른다) · `INTERNAL`(재시도 없음 · 패닉 · 직렬화 실패)은 쓰지 않는다.
- **옛 데몬**: 앱만 새것이고 데몬은 옛것인 조합이 정상으로 생긴다(ADR-0178) — 그 데몬은 `agent.resolve` 를 모른다(`UNKNOWN_COMMAND`). 그때 ① 에이전트 **이름**을 받은 호출은 `CONFLICT`(「데몬이 옛 판이라 이름을 못 푼다 — 데몬을 다시 띄우거나 id 로」) ② **id 만** 준 옛 호출(`agent_id`)은 확인을 건너뛰고 옛 동작대로 적용한다.
- 에이전트 말이 하나도 없으면 왕복을 건너뛴다.

### 5-2. id · 공간 낱말 → 칸 · 탭 · 창: 적용과 같은 잠금 안에서

- 이름을 푼 뒤 실제 적용 사이에 다른 명령 · 마우스가 배치를 바꿀 수 있다(TOCTOU). 그래서 **에이전트 id · 공간 낱말 → (창, 탭, 칸) 고르기는 적용 함수 안, 바꾸는 잠금을 쥔 채** 한다.
  - 적용 서비스에 **대상 꼴** 을 더한다 — `enum Target { Tab(ViewId), TabOf(AgentId), AfterTabOf(AgentId), Slot{view: Option<ViewId>, slot: SlotId}, SlotOf(AgentId), At{token, window} }`(이름 · 위치는 구현이 정한다). **`Target` 을 받는 새 적용 함수**를 두고 잠금 안에서 `locate` 로 푼다.
  - ★옛 id 시그니처는 그대로 남긴다★ — 마우스 · 웹 · `tab_drop` · 팝아웃 등 기존 호출자(10 파일 남짓)는 손대지 않는다. 옛 함수는 새 함수를 `Target::Tab` · `Target::Slot` 으로 부르는 얇은 겉이거나 그 반대(구현이 고른다). 그래서 U2 가 혼자 빌드된다.
  - 잠금을 두 번 잡는 적용(새 창 — `dock_into_window`)은 커밋 잠금에서 다시 `locate` 해 같은지 본다(§4-3).
  - `agent.spawnInto` 는 데몬 스폰을 중간에 기다리는 `async` 적용이라(`apply.rs` 의 `spawn_into`) §8 의 `spawn_blocking` 에서 뺀다 — `at` 은 그 함수의 첫 잠금 안에서 푼다.
- 풀린 뒤 사라진 것(`TabNotFound` · `SlotNotFound` · `SlotNotInTab` 이 **에이전트 · 낱말로 고른 대상**에서 난 경우)은 호출자 잘못이 아니라 경합이라 `CONFLICT`.

### 5-3. 오류 코드 대응 · 선언

- `DockError` → 버스: `SelfTarget` · `InvalidShare` → `INVALID_ARGUMENT` · 호출자가 **id 로 준** 탭 · 칸이 없음(`TabNotFound` · `SlotNotFound` · `SlotNotInTab`) → `NOT_FOUND` · **`WindowNotFound` → `CONFLICT`**(ADR-0302 결정 9 · `window_arg_error` 와 같게) · 나머지(`ZeroArea` · `WindowClosed` · `WindowExists` · `SlotIdOverlap` · `WindowOpen`) → `CONFLICT`. 문구는 `DockError` 의 것을 그대로.
- **새 명령 · 넓힌 명령의 선언 `errors [...]` 에 실제로 내는 코드를 모두 적는다**(ADR-0156 발견 계약 · `spawn_not_applied` 가 선언으로 거른다) — 대개 `[NOT_FOUND, CONFLICT]`(이름 풀기 시한 넘김 · 옛 데몬도 `CONFLICT`). `commands.rs` 머리의 「적용 실패는 코드 하나로 나간다(CONFLICT)」 문장을 이 대응으로 고친다.
- **후보 싣기**: `CommandError` 는 코드 · 문구 · 재시도만 싣는다(구조 칸 없음). 후보는 **문구 끝에** `— candidates: a, b, c`(최대 5 · 영어 — 기존 오류 문구가 영어).

## 6. 데몬 쪽 (묶음 A)

- `agent.resolve` — `crates/engram-dashboard-agent/src/commands.rs` 에 선언(생산자 옆 · ADR-0155). args `{targets: Vec<String>}`(1~8 개) → ok `{agents: Vec<{target, id, name, live: bool}>}`(같은 순서). 몸통 = 말마다 `resolve_in(roster, target)` 그대로(id 먼저 · 정확한 이름 · 중복 = CONFLICT · 없음 = NOT_FOUND). 하나라도 실패하면 그 코드로 실패하고 문구에 **어느 말**이 실패했는지 적는다. `#[effect(Read)]`.
- `resolve_in` 의 오류 문구에 후보를 싣는다 — NOT_FOUND: 이름에 그 낱말을 (대소문자 무시) 품은 것 먼저, 없으면 앞에서부터 최대 5 · CONFLICT: 겹친 에이전트들의 id(최대 5). **모든 `target` 동사가 함께 받는다**(규칙 §1-3). 교차 시험은 코드만 비교하고(`control_agent.rs`) CLI 는 문구를 해석하지 않는다(리뷰 확인) — 문구 단언 시험만 고친다.
- agent crate 의 `catalog_version` 을 올린다.
- **응답 모양 고정**: 셸은 agent crate 를 의존하지 않아(ADR-0270) 답을 타입 없는 JSON 으로 읽는다. 그래서 `agent.resolve` 의 답 JSON 골든 파일을 agent crate 시험에 두고(그 시험이 직렬화 결과와 대조) **셸 시험의 가짜 데몬도 같은 파일을 읽어** 답을 만든다 — 필드 이름이 바뀌면 둘 중 하나가 빨개진다.
- TS 바인딩: agent crate 는 명령 타입을 명시 내보낸다(`crates/engram-dashboard-agent/tests/ts_export.rs`) — 새 타입도 더하고 생성물을 커밋한다(CI 동기 게이트).

## 7. 기존 명령 넓히기 (묶음 D)

| 명령 | 더하는 인자 | 선택으로 바뀌는 인자 | 응답 |
|---|---|---|---|
| `slot.focus` · `slot.close` · `slot.split` · `slot.popout` | `slot_of` · `at` (+`window`) | `view_id` · `slot_id` | 배치를 바꾸면 `layout`(바뀐 탭 줄) 을 더한다 |
| `slot.assignAgent` · `layout.setSlotContent` | `slot_of` · `at` (+`window`) · `assign` | `view_id` · `slot_id` · `agent_id` | `layout` 더함 |
| `tab.switch` · `tab.close` · `tab.rename` | `tab_of` | `view_id` · **`window`**(탭에서 창을 알 수 있다) | `tab.close` 는 `layout` 더함 |
| `agent.spawnInto` | `at` (+`window` 그대로) | — | 그대로(+`layout`) |

- `assign` · `agent_id` 가 **풀리지 않으면 거절**한다 — 지금은 없는 id 도 받아 칸이 빈 채 남는다(help 가 적어 둔 함정 · §11-3). `agent_id` 는 id 모양 검사 + 데몬 명부 확인(같은 `agent.resolve` 왕복 · 데몬이 옛 판이면 확인을 건너뛴다 — §5-1).
- 응답에 `layout` 을 더하는 것은 **덧붙이기**(필드 추가)라 옛 호출자는 그대로다. 규칙 §1-4 · ADR-0307 결정 4(사용자 결정)를 이 명령들에도 지킨다.
- 인자를 선택으로 바꾸는 것은 id 를 주는 옛 호출자에 그대로 맞는다. 셸 `catalog_version` 을 올린다.

### 7-1. 기존 호출자에게 보이는 변화(구현 · 리뷰 실측 — 코드 · 결정 없이 기록만)

- `agent_id`(`slot.assignAgent` · `layout.setSlotContent` 의 Agent)가 데몬 왕복으로 확인된다 — 모양이 UUID 가 아니면 `INVALID_ARGUMENT` · 명부에 없으면 `NOT_FOUND` · 데몬이 답하지 않으면 `CONFLICT`(예전엔 성공) · 옛 데몬이면 확인을 건너뛰고 받은 그대로 적는다 · 성공하면 데몬의 정규 철자로 적는다(§11-3).
- 예전엔 무시되던 엉뚱한 인자가 이제 효력을 낸다 — 칸 동사에 `window` 만 주면 `INVALID_ARGUMENT`(「window 는 at 과만」) · `at` · `slot_of` · `assign` · `tab_of` 도 같다(알 수 없는 인자는 무시한다는 버스 규칙 — ADR-0157 — 에서 그 이름이 이제 「아는 인자」가 되었기 때문).
- 이름 풀기가 칸 · 탭 존재 확인보다 먼저 돈다 — 없는 칸 + 모르는 `agent_id` 는 칸의 `CONFLICT` 대신 에이전트의 `NOT_FOUND`.
- 같은 코드 · 다른 문구: 필수 id 빠짐(serde 「missing field」 → 「the slot/tab is missing — give one of …」) · 빈 id 문자열 · 내용이 Agent 가 아닌데 모양이 틀린 `agent_id` · 숫자 창 + 모양이 틀린 `view_id`(UUID 문구가 먼저).
- 탭 동사의 `window` 가 선택이 됐다 — 주면 그 창의 탭이어야 한다(옛 오류 그대로).
- 대기 슬롯 정리가 모든 버려진 데몬 왕복에 걸린다 — 늦은 답은 예전처럼 짝 없이 버려지고, 앞 대기자가 이미 사라진 경우의 「같은 요청 번호」 경고는 더는 안 난다.

## 8. 처리기 · 스레드

- 에이전트 말을 받는 명령(새 넷 + 넓힌 것)은 데몬을 기다려야 하므로 **비동기 처리기**(`SpawnInto` 꼴 — `CommandHandler` 를 직접 구현)로 꽂는다. 기다리는 동안 레이아웃 잠금을 쥐지 않는다.
- 적용(잠금 + 고르기 + 바꾸기)은 **`spawn_blocking`** 으로 옮긴다 — `NewWindow` 는 main 스레드 · 동기 명령에서 부르면 창 짓기가 교착하고(`apply.rs` 의 `dock_tab` 문서), 예열 창 대기(~1 초) 동안 tokio 일꾼을 막지 않게.
- 끄는 중(마우스 탭 끌기)에도 막지 않는다(ADR-0297) — 놓기가 옮겨진 탭을 다시 찾는다.

## 9. 시험

- 데몬(agent crate): `agent.resolve` 의 id · 이름 · 중복 · 없음 · 여럿 · 후보 문구 · 답 골든 JSON.
- 셸 `src-tauri/tests/layout_commands.rs`: 새 명령 넷 + 넓힌 인자 — 가짜 데몬이 골든 JSON 으로 `agent.resolve` 를 답한다 · 표 이름 골든 목록 갱신 · 인자 배타(둘 다 / 없음) · 후보 문구 · 오류 코드 대응(WindowNotFound = CONFLICT 포함) · 선언 `errors` 와 실제 코드 일치 · `Keep`(보이는 탭 · 포커스 불변 — 예외 셋 §11-4) · `tab.detach` 의 `reused` → `changed: false` · `tab.dock` 의 응답 `tab` = 목적 탭 · 이름 왕복 시한 넘김 → 적용 없음 · NewWindow 는 가짜 `WindowHost` 로 headless.
- 셸 `src-tauri/tests/layout_apply.rs`: `Target` 풀기(잠금 안) · **끼어들기 시험**(데몬이 이름을 푼 뒤 · 적용 전에 그 에이전트가 옮겨지면 적용 잠금 안에서 **지금 자리**로 풀린다 — 옮겨 간 탭이 움직이고 떠난 탭은 안 건드린다 · 어느 칸에도 없으면 `NOT_FOUND` · 새 창 두 잠금 사이에 옮겨지면 연 창을 거두고 `CONFLICT`) · `after_tab_of` 자리(같은 창에서 원본이 앞 · 뒤) · `LayoutRow` 짓기(창 · 탭 · 칸 순서 · 좌표 · 다중 표시 · 빈 탭).
- 명령: `cargo test -p engram-dashboard --test layout_commands` · `--test layout_apply` · `--test lib_unit` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · CI 생성물 동기.

## 10. 구현 단위(파일 겹침으로 가른다 · 단위마다 빌드가 선다)

| 단위 | 파일 | 의존 |
|---|---|---|
| U1 데몬 `agent.resolve` + 후보 문구 + 골든 | agent crate `commands.rs` · 그 시험 · `tests/ts_export.rs` · 바인딩 | 없음 |
| U2 셸 적용 층 — `Target` · `locate` · `DockDest::TabBar.after` · `DockOutcome.changed` · `LayoutRow` 짓기 · 크기 계산 연결 | `src-tauri/src/layout/{apply.rs, manager.rs, dock.rs}` + 새 모듈 · `tests/layout_apply.rs` | 없음(U1 과 병렬) |
| U3 셸 버스 — 이름 왕복 포트 · 새 명령 넷 · 오류 대응 · 넓힌 명령 | `src-tauri/src/layout/commands.rs` · `src-tauri/src/commands/layout.rs` · `tests/layout_commands.rs` | U1(골든 파일) · U2 |
| U4 help · CLAUDE.md · step-log · tracking | `prompts/engram-help.md` · `CLAUDE.md` · `docs/` | U3 |

## 11. 사용자가 돌아오면 볼 결정(메인이 기본값으로 진행)

1. **원본 탭에 칸이 여럿이면 탭째 옮긴다**(§2 — 마우스와 같다). 대안 = 거절 · 칸 하나만 빼내는 새 길(지금 코드에 없음).
2. **`tab.dock` 의 `share` 기본** = 마우스와 같은 값, 크기를 모르면 0.5(§4-2).
3. **`slot.assignAgent` · `layout.setSlotContent` 가 풀리지 않는 에이전트를 거절**(§7 — 지금은 받아서 빈 칸).
4. **`Keep` 이어도 보이는 탭이 바뀌는 세 경우**(슬롯 도킹 PRD R11b 의 예외 — 원본 탭이 떠나면 무언가를 보여야 한다): ⓐ 같은 창 안에서 보이던 탭을 옮기면 목적 탭이 보이게 된다 ⓑ 원본 창에서 보이던 탭이 떠나면 이웃 탭이 보인다 ⓒ main 의 마지막 탭이 떠나면 빈 탭이 새로 생겨 보인다(`manager.rs` 의 `dock` 승계).
5. **끄는 중에 LLM 이 옮긴 탭을 우리 웹뷰 밖에 놓으면 새 창이 안 생긴다**(슬롯 도킹 PRD D9 · TRD §9-5 의 「T-65 가 다시 정한다」) — 이 TRD 는 그 동작을 **그대로 둔다**(드문 겹침 · ADR-0297 「막지 않는다」 유지).
7. **칸 하나만 다른 칸 옆에 붙이기가 없다** — `tab.dock` 은 탭째 옮긴다. 두 에이전트가 이미 한 탭에 있으면 `slot.popout` 으로 빼낸 뒤 `tab.dock`(두 호출). 한 번에 하려면 칸 단위 도킹 경로가 새로 필요하다(지금 코드에 없음 · U3a 리뷰가 짚음).
6. **LLM 의 새 창 크기 · 붙임 몫 기본이 마우스와 조금 다르다**(§4-2 · §4-3 — 원본 창이 최대화일 때 · main 에서 뗄 때). 같게 하려면 모니터 작업 영역 · 창 틀 입력을 셸로 올리는 포트가 필요하다(이번 범위 밖).

## 12. 1판 → 2판 (리뷰 반영)

- 응답 `tab` 이 dock 에서 지워진 원본 id 였다 → 살아남은 목적 탭(codex).
- 이름 · 낱말 → 칸 고르기가 적용과 다른 잠금이었다(TOCTOU) → 적용 잠금 안 `Target` 풀기 + 끼어들기 시험(codex · Claude).
- 목적 칸이 여러 칸에 뜬 에이전트면 고칠 인자가 없었다 → `beside_slot`(둘 다).
- 평평한 줄이 좌우 · 위아래를 잃었다 → 정규 좌표 `x · y · w · h`(codex).
- 이름 왕복이 말마다 3 초라 7 초 배달 시한을 넘을 수 있었다 → 한 번에 묶고 2 초 · 넘으면 적용 없음 + `INTERNAL`(Claude).
- 「마우스와 같다」가 거짓이었다(새 창 크기 · share 원본 크기) → `drag_size::drag_sizes`(Claude).
- `agent` 가 「어느 칸」과 「넣을 에이전트」로 겹쳤다 → `slot_of` · `tab_of` · `beside` · `assign`(Claude).
- 탭 동사가 `window` 를 계속 요구했다 → 선택(Claude).
- 선언 오류 목록 · `WindowNotFound` 코드 · 머리 문장 → §5-3(Claude).
- `to_agent` 자리 계산이 잠금 밖 · 하나 어긋남 → `DockDest::TabBar.after`(Claude).
- `changed` 출처 · identity 없음 → `DockOutcome.changed` · `CONFLICT`(Claude).
- agent crate 판 · 답 모양 고정 → `catalog_version` · 골든 공유(Claude).
- `TIMEOUT` 이 두 뜻 → `INTERNAL`(Claude).
- 응답 크기 → 바뀐 탭만 · id 는 `ids: true` 일 때만(Claude).
- 기존 명령 응답 미룸이 사용자 결정(ADR-0307 결정 4)을 어겼다 → 넓힌 명령에도 `layout`(Claude) · §11 에 R11b 예외 · D9 를 올림(Claude).

### 2판 → 3판 (Claude 재확인 반영)

- 새 창 크기 · share 원본을 `drag_sizes` 로 계산한다던 2판도 입력(작업 영역 · 창 틀 · 뷰포트)이 셸에 없어 사실상 거짓 → `plan_new_window` · 캔버스로 갈음하고 차이를 §11-6 에 밝힘.
- 이름 풀기 시한 넘김 코드 `INTERNAL`(재시도 없음 · 패닉 뜻) → `CONFLICT` + `AfterCondition`.
- 새 창 적용의 두 잠금 사이 → 커밋 잠금에서 다시 `locate` · 다르면 창을 거두고 `CONFLICT`.
- `after_tab_of` 를 잠금 밖에서 `ViewId` 로 바꾸던 것 → `Target::AfterTabOf` · `DockTo::TabBar` 에도 칸 · 가장자리(원본 탭 안 · 여러 탭).
- `layout.get` 줄에 탭 묶음 열쇠가 없었다 → `tab_index`.
- 옛 데몬(`agent.resolve` 모름)과 붙은 새 셸이 id 만 주는 옛 호출까지 막던 것 → 이름만 `CONFLICT` · id 는 확인 건너뜀.
- U2 가 혼자 빌드 안 되던 것(적용 시그니처 변경이 호출자 10 파일 남짓을 깸) → 옛 id 시그니처를 남김.
- `agent.spawnInto` 를 `spawn_blocking` 에 넣을 수 없음 → 예외 · `at` 은 첫 잠금 안.
- `identity == None` 을 `CONFLICT` 로 → 이미 옮겼으므로 `ok`.
