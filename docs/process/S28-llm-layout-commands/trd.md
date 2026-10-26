# S28 TRD — LLM 레이아웃 명령 (T-65) — 1판 (2026-10-26)

- **상태:** 1판 · 메인 작성(사용자 외출 중 자율 진행 「쭉 진행해」 · 명령 세부 = 사용자 위임 2026-10-26 「잘 지시되는쪽으로 알아서」). 사용자가 돌아오면 볼 결정 = §9.
- **규칙:** ADR-0307 · `docs/reference/llm-command-conventions.md`(이하 「규칙」).
- **옛 설계:** `docs/process/S21-slot-docking/prd.md` R11b(LLM 명령은 창 활성 · 보이는 탭 · 포커스를 안 바꾼다) · R16 · Q10 · D22(탭 순서 바꾸기도 T-65) · `docs/process/S21-slot-docking/trd.md` §3-6(`FocusPolicy::Keep` · 기본 share · 새 창 크기를 마우스와 같은 함수로 · 예열 창 그대로) · §9-5(끄는 중 LLM 이 옮긴 탭의 놓기) · ADR-0297(끄는 중에도 LLM 명령은 막지 않는다).

## 1. 범위

| 묶음 | 내용 |
|---|---|
| A | 데몬 명령 `agent.resolve` — 이름/ id → `{id, name}`(데몬 해석기 `resolve_in` 그대로 · ADR-0132) · `resolve_in` 의 NOT_FOUND / CONFLICT 문구에 후보를 싣는다 |
| B | 셸 대상 풀기 — 에이전트 이름 → (데몬 왕복) id → 그 에이전트가 든 칸 · 탭 · 창 · 공간 낱말 → 칸 · 배치 조회 몸통 짓기 · `DockError` → 버스 오류 코드 |
| C | 새 명령 — `tab.move` · `tab.dock` · `tab.detach`(셋 다 `apply::dock_tab` · `FocusPolicy::Keep`) · `layout.get`(배치 조회 + 거르기) |
| D | 기존 명령 넓히기 — 칸 · 탭을 받는 명령이 `agent` · `at` 인자도 받게(id 인자는 선택으로) |
| E | `prompts/engram-help.md` 「## window」 를 이름 기준으로 간결히 다시 쓰기 · CLAUDE.md 「LLM-우선 제어」 남은 갭 줄 고치기 |

**범위 밖:** 프론트(웹 레지스트리 · 렌더) — 셸 표에 선언한 이름은 웹 등록에서 자동으로 빠지고(`reserved_names()`), 배치 알림은 `dock_tab` 이 이미 낸다. 「크면 파일로」 안전장치(자리 미정 — ADR-0307 결정 6). 칸 번호 지목(ADR-0307 결정 8). 탭 이름으로 지목(이름이 유일하지 않고 자동 이름은 포커스로 바뀐다 — `layout/tab_name.rs`).

## 2. 옮기는 단위 = 탭(마우스와 같다)

- `apply::dock_tab` 은 **탭(View) 하나를 통째로** 옮긴다(`layout/dock.rs` `docked_tree` · `ViewManager::dock` 이 원본 탭을 지운다). 한 칸만 빼내 다른 칸 옆에 붙이는 길은 없다(`slot.popout` 은 칸 하나를 **새 창의 새 탭**으로만 뺀다).
- 그래서 `--agent X` 로 고른 원본 = **X 가 든 칸이 속한 탭**. 그 탭에 칸이 여럿이면 **탭째 옮긴다**(마우스로 탭을 끄는 것과 같다). 응답에 함께 옮겨진 칸들을 싣는다 — LLM 이 결과를 알게.
- X 가 둘 이상의 탭에 떠 있으면(같은 에이전트 다중 표시 — `manager.rs` 「다중 참조 허용」) CONFLICT + 그 탭들(id · 창)을 후보로. 어느 칸에도 없으면 NOT_FOUND + 칸에 떠 있는 에이전트 이름들을 후보로.

## 3. 인자 모양(공통)

- **원본(옮길 탭):** `agent`(이름 또는 id — 데몬이 푼다) | `tab`(탭 id). 정확히 하나 — 둘 다 / 둘 다 없음 = `INVALID_ARGUMENT`.
- **말의 종류마다 인자를 따로 둔다**(규칙 §1-2) — 이름과 공간 낱말을 한 인자에 섞지 않는다.
- **창 인자**는 ADR-0302 그대로(영속 id · 같은 실행 label · 번호 거부 — `layout/window_arg.rs`).
- **탭 id · 칸 id** 는 버스 매크로 규칙대로 `String`(처리기가 Uuid 로 푼다).

## 4. 새 명령

### 4-1. `tab.move` — 탭 줄로 옮기기 · 순서 바꾸기 (`DockDest::TabBar`)

- args: `agent?` · `tab?` · `to_window?` · `to_agent?` · `index?: u32`
- 목적 창 = `to_window` | `to_agent`(그 에이전트가 든 탭의 창 — 그 탭 바로 뒤에 넣는다, `index` 를 주면 그것이 이긴다) | 둘 다 없으면 **원본의 창**(= 같은 창 안 순서 바꾸기). 둘 다 = `INVALID_ARGUMENT`.
- `index` = 옮긴 뒤 탭 줄에서의 자리(0 부터 · 없거나 끝을 넘으면 맨 끝) — `DockDest::TabBar.index` 그대로.
- 같은 자리 = 바뀐 것 없는 `ok`(`changed: false`).

### 4-2. `tab.dock` — 칸 옆에 붙이기(분할) (`DockDest::Seam`)

- args: `agent?` · `tab?` · `beside_agent?` · `beside_at?` · `window?` · `side: DockSide` · `whole?: bool` · `share?: f64`
- 목적 칸 = `beside_agent`(그 에이전트가 든 칸 — 여러 칸이면 CONFLICT + 후보) | `beside_at`(공간 낱말 — `window` 의 보이는 탭에서 · `window` 없으면 main · `apply::resolve_spatial` 그대로). 정확히 하나.
- `side` = 새로 붙는 탭이 놓일 쪽(`Left` · `Right` · `Top` · `Bottom` — `layout/dock.rs` `DockSide` 와 같은 철자 · 버스 블록에 같은 이름 enum 을 하나 더 둔다 — 매크로는 블록 밖 enum 을 못 쓴다).
- `whole: true` = 목적 칸 하나가 아니라 **그 칸이 속한 탭 전체의 바깥 변**에 붙인다(`DockAnchor::Outer`). 기본 = 그 칸(`DockAnchor::Slot`).
- `share` = 붙는 쪽 몫(0.01~0.5 — `InvalidShare` 범위 그대로). 없으면 **마우스와 같은 값**(`bands::attach_share(원본 탭 캔버스 크기, side, 목적 칸 px 틀 | Outer 면 캔버스)`). 캔버스 · 칸 px 를 모르면(웹뷰가 아직 크기를 안 알림) **0.5**.
- 원본 탭 = 목적 칸이 든 탭이면 `SelfTarget` → `INVALID_ARGUMENT`.

### 4-3. `tab.detach` — 새 창으로 떼기 (`DockDest::NewWindow`)

- args: `agent?` · `tab?`
- 자리 · 크기 = 마우스와 같은 기본(`plan_new_window` — 원본이 팝아웃이면 그 크기 · main 이면 팝아웃 기본 · 자리 = 커서가 없으므로 label 기본 자리). 예열 창을 그대로 쓴다(활성화 없이 z 만 — `Keep`).
- 원본이 **탭 하나뿐인 팝아웃**이면 `plan_new_window` 가 창째 다시 쓰고(`reused`) 바뀌는 것이 없다 → `ok` + `changed: false`.

### 4-4. 응답(셋 공통 — 규칙 §1-4)

```
TabMoveOk / TabDockOk / TabDetachOk {
  changed: bool,          // 바뀐 것이 없으면 false(같은 자리 · 이미 혼자인 팝아웃)
  window: String,         // 탭이 놓인 창 label
  window_id: String,
  number: u32,            // 창 번호(사람이 보는 표식 — 인자로는 안 받는다 · ADR-0302)
  tab: String,            // 옮긴 탭 id(그대로다)
  layout: Vec<LayoutRow>  // 바뀐 창들(원본 창 · 목적 창)의 배치 — §5 의 줄 모양
}
```

- `emptied`(원본 팝아웃이 닫힘) · `main_refilled`(main 이 빈 탭을 새로 받음)는 `layout` 에 그대로 드러나므로 따로 싣지 않는다.

### 4-5. `layout.get` — 배치 조회 (#[effect(Read)])

- args: `window?` · `agent?` — `agent` = 그 에이전트가 든 칸의 줄만(핀포인트 · 이름은 데몬이 푼다) · `window` = 그 창만 · 둘 다 없으면 전부.
- ok: `{ rows: Vec<LayoutRow> }` — **칸마다 한 줄, 평평하게**(매크로가 재귀 타입을 못 싣는다 · 평평한 줄이 LLM 에게도 짧다):

```
LayoutRow {
  window: String, window_number: u32,
  tab: String, tab_name: String, tab_active: bool,   // tab_active = 그 창에서 보이는 탭
  slot: String, focused: bool,                       // focused = 그 탭의 포커스 칸
  content: SlotKind,                                 // Empty | Agent | AgentList | PresetPalette | Usage | Unknown
  agent: Option<String>, agent_id: Option<String>    // content = Agent 일 때 · 이름은 셸 캐시(표시용)
}
```

- 창 순서 = `window.list` 순 · 탭 순서 = 탭 줄 순 · 칸 순서 = 트리 전위 순.
- **표시용 이름은 셸 캐시(`agent_names`)를 쓴다** — 풀이가 아니라 보여 주기라 ADR-0132 의 「해석기 하나」에 걸리지 않는다. 캐시에 없으면 `agent: None`(id 는 있다).
- 탭 하나 · 칸 하나뿐이어도 줄은 하나 나온다(빈 탭 = `Empty` 칸 한 줄).

## 5. 대상 풀기 (묶음 B)

- **이름 → id = 데몬 왕복**: 셸 처리기가 `agent.resolve{target}` 을 데몬에 보낸다(`DaemonClient::send_command(AgentCommand::Command{envelope})` — `agent.spawnInto` 가 데몬을 기다리는 길과 같은 꼴 · 포트 trait 로 끊어 시험은 가짜로). 셸이 자기 캐시로 이름을 풀지 않는다(ADR-0132 · 규칙 §1-2).
  - 시간: CLI → 데몬 → 셸 배달 시한이 7 초(`command_delivery.rs` `DEFAULT_DEADLINE`)라 안쪽 왕복은 그 안에서 끝나야 한다 — 셸 쪽 왕복에 **3 초** 시한을 두고 넘으면 `TIMEOUT`.
- **id → 칸**: 모든 창 · 탭의 트리를 훑어 `SlotContent::Agent{agent_id}` 가 같은 칸을 모은다(새 도우미 — `ViewManager` 안 · 락 안에서 읽고 바로 놓는다).
- **공간 낱말 → 칸**: `apply::resolve_spatial` 그대로. 낱말이 칸을 못 집으면(`None`) NOT_FOUND.
- **`DockError` → 버스 코드**: `TabNotFound` · `WindowNotFound` · `SlotNotFound` → `NOT_FOUND` · `SelfTarget` · `InvalidShare` · `SlotNotInTab` → `INVALID_ARGUMENT` · 나머지(`ZeroArea` · `WindowClosed` · `WindowExists` · `SlotIdOverlap` · `WindowOpen`) → `CONFLICT`. 문구는 `DockError` 의 것을 그대로.
- **후보 싣기**: `CommandError` 는 코드 · 문구 · 재시도만 싣는다(구조 칸 없음 — `crates/engram-dashboard-command/src/error.rs`). 그래서 **후보는 문구 끝에** `— candidates: a, b, c` 꼴로 붙인다(최대 5 개 · 영어 문구 — 기존 오류 문구가 영어). 구조 칸을 새로 들이는 것은 이 범위 밖.

## 6. 데몬 쪽 (묶음 A)

- `agent.resolve` — `crates/engram-dashboard-agent/src/commands.rs` 에 선언(생산자 옆 · ADR-0155). args `{target: String}` → ok `{id: String, name: String, live: bool}`. 몸통 = `resolve_in(roster, target)` 그대로(id 먼저 · 정확한 이름 · 중복 = CONFLICT · 없음 = NOT_FOUND). `#[effect(Read)]`.
- `resolve_in` 의 오류 문구에 후보를 싣는다 — NOT_FOUND: 이름에 그 낱말을 (대소문자 무시) 품은 것 먼저, 없으면 앞에서부터 최대 5 개 · CONFLICT: 겹친 에이전트들의 id(최대 5 개). **모든 `target` 동사가 함께 받는다**(규칙 §1-3). 기존 시험의 문구 단언은 그에 맞춰 고친다.
- TS 바인딩: agent crate 는 명령 타입을 명시 내보낸다(`crates/engram-dashboard-agent/tests/ts_export.rs`) — 새 타입도 거기에 더하고 생성물을 커밋한다(CI 동기 게이트).

## 7. 기존 명령 넓히기 (묶음 D)

- 칸을 받는 명령(`slot.focus` · `slot.close` · `slot.split` · `slot.popout` · `slot.assignAgent` · `layout.setSlotContent`)에 `agent?`(그 에이전트가 든 칸) · `at?`(공간 낱말 + `window?`) 을 더하고 `view_id` · `slot_id` 를 선택으로 바꾼다. 칸 지목은 `slot_id`(+`view_id`) | `agent` | `at` 중 정확히 하나.
  - `agent` 로 칸을 고르면 탭은 그 칸의 탭으로 정해지므로 `view_id` 가 필요 없다.
- `slot.assignAgent` 에 붙일 에이전트도 이름으로 — `assign?`(이름/id — 데몬이 푼다) 을 더하고 `agent_id` 를 선택으로. 둘 중 하나. **이름이 풀리지 않으면 거절**한다(지금은 없는 id 도 받아 빈 칸이 된다 — help 가 그 함정을 적어 둠).
- 탭을 받는 명령(`tab.switch` · `tab.close` · `tab.rename`)에 `agent?`(그 에이전트가 든 탭)를 더하고 `view_id` 를 선택으로.
- `agent.spawnInto` 는 이미 탭 · 칸이 선택이다 — `at?` 만 더한다(그 낱말의 칸에 띄운다).
- **응답은 그대로 둔다**(이번 범위 — 규칙 §1-4 의 「바뀐 배치」는 새 명령 셋에만 싣는다. 기존 명령의 응답 넓히기는 따로 — tracking 에 올린다).
- 인자를 선택으로 바꾸는 것은 기존 호출자(id 를 주는 쪽)에 그대로 맞는다. `catalog_version` 을 올린다(그 파일 머리 규칙).

## 8. 처리기 · 스레드

- 이름을 푸는 명령은 데몬을 기다려야 하므로 **비동기 처리기**(`SpawnInto` 꼴 — `CommandHandler` 를 직접 구현)로 꽂는다. 기다리는 동안 레이아웃 락을 쥐지 않는다.
- 이름을 안 쓰는 호출(id · 공간 낱말만)도 같은 처리기를 탄다 — 왕복만 건너뛴다.
- `dock_tab` 의 `NewWindow` 는 main 스레드 · 동기 명령에서 부르면 창 짓기가 교착한다(`apply.rs` 의 `dock_tab` 문서) — 처리기는 클라이언트 tokio 런타임에서 돌고(main 아님) **적용 호출은 `spawn_blocking` 으로 옮겨** 예열 창 대기(최대 ~1 초) 동안 tokio 일꾼을 막지 않는다.
- 끄는 중(마우스 탭 끌기)에도 막지 않는다(ADR-0297) — 놓기가 옮겨진 탭을 다시 찾는다.

## 9. 사용자가 돌아오면 볼 결정(메인이 기본값으로 진행)

1. **원본 탭에 칸이 여럿이면 탭째 옮긴다**(§2 — 마우스와 같다). 대안 = 거절 · 칸 하나만 빼내는 새 길(지금 코드에 없음).
2. **`tab.dock` 의 `share` 기본** = 마우스와 같은 값, 크기를 모르면 0.5(§4-2).
3. **`slot.assignAgent` 가 풀리지 않는 이름을 거절**(§7 — 지금은 받아서 빈 칸).

## 10. 시험

- 데몬(agent crate): `agent.resolve` 의 id · 이름 · 중복 · 없음 · 후보 문구 — `resolve_in` 단위 시험 옆.
- 셸 `src-tauri/tests/layout_commands.rs`: 새 명령 넷 + 넓힌 인자 — `World::build()` 의 가짜 데몬에 `agent.resolve` 응답을 더한다 · 표 이름 골든 목록 갱신 · 인자 배타(둘 다 / 둘 다 없음) · 후보 문구 · 오류 코드 대응 · `Keep`(보이는 탭 · 포커스 불변) · `tab.detach` 의 `reused` → `changed: false` · NewWindow 는 가짜 `WindowHost` 로 headless.
- 셸 `src-tauri/tests/layout_apply.rs`: id → 칸 도우미 · `LayoutRow` 짓기(창 · 탭 · 칸 순서 · 다중 표시 · 빈 탭).
- 명령: `cargo test -p engram-dashboard --test layout_commands` · `--test layout_apply` · `--test lib_unit` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · CI 생성물 동기.

## 11. 구현 단위(파일 겹침으로 가른다)

| 단위 | 파일 | 의존 |
|---|---|---|
| U1 데몬 `agent.resolve` + 후보 문구 | agent crate `commands.rs` · 그 시험 · `tests/ts_export.rs` · 바인딩 | 없음 |
| U2 셸 대상 풀기 + `layout.get` + 새 명령 셋 | `src-tauri/src/layout/{commands.rs, apply.rs, manager.rs}` + 새 모듈 · 셸 시험 | 이름으로 `agent.resolve` 를 부른다(문자열 — crate 의존 없음) · U1 과 병렬 가능 |
| U3 기존 명령 넓히기 | `src-tauri/src/layout/commands.rs` · 시험 | U2 위(같은 파일 — 차례로) |
| U4 help · CLAUDE.md · step-log | `prompts/engram-help.md` · `CLAUDE.md` · `docs/` | U2 · U3 뒤 |
