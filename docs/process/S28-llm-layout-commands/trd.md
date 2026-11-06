# S28 TRD — LLM layout commands (T-65) — edition 4 (2026-10-27)

- **Status:** edition 4 · written by main (autonomous work while the user was away 「쭉 진행해」 · command details = delegated by the user 2026-10-26 「잘 지시되는쪽으로 알아서」). Edition 1 (`76d29cd0`) was fixed from the `/review trd full` findings (codex Designer · Claude Architect-breaker — both FIX), edition 2 (`61c66724`) from the Claude re-check (FIX) findings, and edition 4 reflects ADR-0308 as implemented — what was fixed = §12. Decisions for the user to look at on return = §11.
- **Name resolution = ADR-0308** — when registering, the shell marks the arguments that select an agent as 「에이전트 참조」 (agent reference), and the daemon turns the name in that field into an id before relaying (§5-1 · §6 · research = `docs/research/agent-name-resolution-relay-vs-callback-2026-10-27.md`). The shell does not ask the daemon back.
- **Rules:** ADR-0307 · `docs/reference/llm-command-conventions.md` (hereafter 「규칙」, "rule" below).
- **Old design:** `docs/process/S21-slot-docking/prd.md` R11b (LLM commands do not change the window activation · the visible tab · the focus) · R16 · Q10 · D9 · D22 (reordering tabs is also T-65) · `docs/process/S21-slot-docking/trd.md` §3-6 (`FocusPolicy::Keep` · default share · new-window size by the same function as the mouse · the prewarmed window as is) · §9-5 · ADR-0297 (LLM commands are not blocked even during a drag).

## 1. Scope

| Group | Content |
|---|---|
| A | Daemon — convert agent-reference fields before relaying (ADR-0308 · resolver `resolve_in` as is · ADR-0132) · put candidates in the NOT_FOUND / CONFLICT messages of `resolve_in` · command `agent.resolve` (**several names/ids at once** → `{id, name, live}` — a CLI convenience · the shell does not call it) |
| B | Shell target resolution — agent fields are marked with `#[agent_ref]` (the daemon passes them on as ids) · **id · spatial word → pane · tab · window inside the same lock as the apply** · building layout rows · `DockError` → bus error code |
| C | New commands — `tab.move` · `tab.dock` · `tab.detach` (all three go through `apply::dock_tab` · `FocusPolicy::Keep`) · `layout.get` |
| D | Widen the existing pane · tab commands — address by name · spatial word · if the layout changes, the response carries the changed layout |
| E | Rewrite `prompts/engram-help.md` 「## window」 concisely, name-based · fix the remaining-gap line of CLAUDE.md 「LLM-우선 제어」 |

**Out of scope:** the frontend (web registry · rendering) — names declared in the shell table automatically drop out of web registration (`view_commands.rs` `reserved_names()`), and the layout notification is already emitted by the apply service. The 「크면 파일로」 safeguard (place undecided — ADR-0307 decision 6). Addressing by pane number (ADR-0307 decision 8). Addressing by tab name (names are not unique and automatic names change with focus — `layout/tab_name.rs`).

## 2. The unit that moves = the tab (same as the mouse)

- `apply::dock_tab` moves **one whole tab (View)** (`layout/dock.rs` `docked_tree` · `ViewManager::dock`). There is no path that pulls out a single pane and attaches it beside another pane (`slot.popout` only pulls a single pane out as the **new tab of a new window**).
- So when the source is selected by agent = **the tab that holds the pane containing that agent**. If that tab has several panes, **the whole tab moves** (§11-1). The panes that moved along show up in the response layout.
- **`tab.dock` (attach beside a pane) deletes the source tab** — the source tab's panes go into the destination tab (the Seam path `remove_view` in `manager.rs`). The `tab` in the response is **the surviving destination tab** (§4-4).

## 3. Argument names — separate per kind, with no overlapping meaning

| What is selected | by id (id 로) | by agent (에이전트로) (name or id — the daemon resolves) | by spatial word (공간 낱말로) |
|---|---|---|---|
| tab | `tab` | `tab_of` | — |
| pane | `slot_id` | `slot_of` | `at` (+ `window`) |
| place to attach (pane) — `tab.dock` | `beside_slot` | `beside` | `beside_at` (+ `window`) |
| place to move to — `tab.move` | `to_window` (ADR-0302 window argument) · `after_tab` (right after that tab) | `after_tab_of` (right after that agent's tab) | — |
| agent to put in a pane | `assign` (name or id) | | |

- Give **exactly one** within one row (what is selected) — two or more / none = `INVALID_ARGUMENT` (only for rows that are required).
- If the agent in `slot_of` · `beside` **appears in several panes**, `CONFLICT` + candidates (the `slot_id`, tab and window of those panes) — the next call fixes it directly with `slot_id` · `beside_slot`. `tab_of` · `after_tab_of` are looked at **per tab** — if the agent is in several panes within one tab it resolves to that tab (there is no ambiguity in selecting a tab), and only when it appears in several tabs is it `CONFLICT` + candidates (fix with `tab`).
- If it is in no pane, `NOT_FOUND` + candidates (up to 5 names of agents that appear in panes).
- `at` · `beside_at` are resolved in the **visible tab** of `window` (`apply::resolve_spatial` as is · main when there is no `window`). If the word does not pick a pane, `NOT_FOUND`.
- The window argument is as in ADR-0302 (persistent id · same-run label · numbers rejected — `layout/window_arg.rs`).
- The id is a `String` per the bus macro rule (the handler resolves it to a Uuid).
- The arguments in the 「에이전트로」 column, `assign`, the old `agent_id` and the `agent` of `layout.get` are marked `#[agent_ref]` in the declaration (§5-1) — they reach the handler as the id the daemon swapped in. The 「id 로」 and 「공간 낱말로」 columns and the window argument are not marked.
- The old argument `agent_id` (the agent to put in, for `slot.assignAgent` · `layout.setSlotContent`) is also still accepted (old callers) — new calls use `assign`. Both given = `INVALID_ARGUMENT`. This is also a marked field, so the daemon resolves it against the registry (§7-1).

## 4. New commands

### 4-1. `tab.move` — move to a tab strip · reorder (`DockDest::TabBar`)

- args: `tab?` · `tab_of?` · `to_window?` · `after_tab_of?` · `index?: u32`
- Destination = `to_window` (the tab strip of that window · position `index` — from 0 · if absent or past the end, the very end) | `after_tab_of` (**right after** the tab holding that agent · giving `index` together is `INVALID_ARGUMENT`) | with neither, **the source's window** (reordering within the same window · `index` required).
- `after_tab_of` is passed **as a `Target` for the destination too** (§5-2 — `AfterTabOf(AgentId)`), so that **inside the lock** it finds that agent's tab and that tab's window, removes the source from the list, and then computes the position right after that tab (this prevents an off-by-one when the source is in front in the same window). The same 「뒤에」 ("after") field is added to `DockTo::TabBar` (the side `check_dock` reads) and `DockDest::TabBar` — the mouse path leaves it empty.
- Edge: if that agent is **inside the source tab** (inserting after itself) `INVALID_ARGUMENT` · if it appears in several tabs `CONFLICT` + candidates.

### 4-2. `tab.dock` — attach beside a pane (split) (`DockDest::Seam`)

- args: `tab?` · `tab_of?` · `beside_slot?` · `beside?` · `beside_at?` · `window?` · `side: DockSide` · `whole?: bool` · `share?: f64`
- `side` = the side where the newly attached tab is placed (`Left` · `Right` · `Top` · `Bottom` — same spelling as `DockSide` in `layout/dock.rs` · one more enum with the same name is put in the bus block — the macro cannot use an enum outside the block).
- `whole: true` = the **outer edge of the whole tab** that the destination pane belongs to (`DockAnchor::Outer`). Default = that pane (`DockAnchor::Slot`).
- `window` is used only with `beside_at` (together with anything else is `INVALID_ARGUMENT`).
- `share` = the share of the attaching side (0.01~0.5 — the `InvalidShare` range as is). If absent, the **same formula as the mouse** `bands::attach_share(original, side, 목적 px 틀)` — `original` = **the canvas size of the source tab's window** (shell model `windows[label].canvas`) · destination px frame = the destination pane px (`slot_px`) | the destination window canvas if `whole`. With no input (the webview has not reported sizes yet) **0.5**. ★Difference from the mouse★: the mouse uses as `original` the `drag_sizes().original` that the web measured (the restored size when the source window is maximized) — the shell model does not have that input (monitor work area · window frame · viewport), so the canvas stands in (§11-6).
- If the source tab is the tab holding the destination pane, `SelfTarget` → `INVALID_ARGUMENT`.

### 4-3. `tab.detach` — detach to a new window (`DockDest::NewWindow`)

- args: `tab?` · `tab_of?`
- Size = the `plan_new_window` default (if the source is a popout, its normal size · if main, the popout default size). Position = there is no cursor, so the label's default position. The prewarmed window is used as is (z only, without activation — `Keep`). ★Difference from the mouse★: the mouse uses the main restore size up to the cap through `drag_size::drag_sizes`, but the shell model does not have that input (monitor work area · window frame · viewport) — a port that lifts that input into the shell is out of scope this time (§11-6).
- **Guard between the two locks**: applying a new window takes the lock twice (plan → open the window outside the lock → commit · `dock_into_window` in `apply.rs`). In the commit lock the target is **`locate`d again**, and if it is a different tab from the first lock, the opened window is withdrawn and `CONFLICT` (nothing is moved).
- If the source is a **popout with only one tab**, `plan_new_window` reuses the whole window (`reused`) and nothing changes → `ok` + `changed: false`.

### 4-4. Response (common to the three — rule §1-4)

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

- Add `changed: bool` to `DockOutcome` (today 「바뀐 것 없음」 is only a missing notification, so the body does not know).
- If the reused window (the reuse of a lone popout) is closed in the meantime, that tab also disappears from the model — in the lock that reads the result, the target is resolved again, and if it differs or is gone, `CONFLICT` (the tab that was about to move is gone · implemented as `TargetDockError::Moved`). The mouse path (`identity == None` → success) is unchanged.
- `emptied` · `main_refilled` show up in `layout`, so they are not carried separately.

### 4-5. `layout.get` — layout query (#[effect(Read)])

- args: `window?` · `agent?` (only the rows of the panes holding that agent — pinpoint · the daemon resolves the name) · `ids?: bool` (default false)
- ok: `{ rows: Vec<LayoutRow> }` — **one row per pane, flat** (the macro cannot carry a recursive type):

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

- Coordinates = the pane rectangles of `geometry::compute` (the single source of geometry in the shell — ADR-0227). They come out without pixel measurement.
- By default ids are not carried (rule §4 — size · the LLM calls things by name · word). Use `ids: true` when a pane without an agent has to be pointed at by id.
- Window order = `window.list` order · tab order = tab strip order · pane order = tree pre-order.
- **The display name is the shell cache (`agent_names`)** — it is for showing, not resolving, so it does not fall under 「해석기 하나」 of ADR-0132. If it is not in the cache, `agent: None`.

## 5. Target resolution (group B)

### 5-1. Name → id: the daemon swaps it before relaying (ADR-0308)

- The shell marks the arguments that select an agent (`tab_of` · `slot_of` · `beside` · `after_tab_of` · `assign` · the old `agent_id` · the `agent` of `layout.get`) with `#[agent_ref]` in the declaration — it is carried as `CommandDecl.agent_args` in the registration packet, and `help` stays opaque as before. All the daemon knows is 「이 칸은 내 에이전트를 가리킨다」.
- Before relaying a shell command, the daemon swaps **only the string values** of the marked fields into the id (canonical spelling) with its own resolver (`resolve_in` — ADR-0132) and rewrites them into the envelope (§6). If one does not resolve, it **does not relay** and answers with the resolver's code as is (not found = `NOT_FOUND` · name overlap = `CONFLICT`) as `"<칸>: <문구 + 후보>"` — the shell received nothing, so nothing changed.
- The shell handler looks at **only the id** in a marked field — it resolves it as a `Uuid`, and if it is not a UUID (a call that did not pass through the daemon) `INVALID_ARGUMENT` (「`<칸>` must be an agent id (a UUID) …」). The shell does not resolve names (ADR-0132 · rule §1-2 — the shell's name cache `agent_names` is for showing).
- Name → id is outside the shell's apply lock (in the daemon) and id → pane is inside — §5-2 guards the movement in between.

### 5-2. id · spatial word → pane · tab · window: inside the same lock as the apply

- After the daemon swaps the name for an id and before the shell applies it, another command or the mouse can change the layout (TOCTOU). So **selecting the (window, tab, pane) from the agent id · spatial word is done inside the apply function, holding the mutating lock**.
  - Add a **target form** to the apply service — `enum Target { Tab(ViewId), TabOf(AgentId), AfterTabOf(AgentId), Slot{view: Option<ViewId>, slot: SlotId}, SlotOf(AgentId), At{token, window} }` (names · locations are up to the implementation). Provide a **new apply function that takes a `Target`** and resolve it with `locate` inside the lock.
  - ★The old id signatures are left as they are★ — existing callers such as the mouse · web · `tab_drop` · popout (about 10 files) are not touched. The old function is either a thin wrapper that calls the new function with `Target::Tab` · `Target::Slot`, or the reverse (the implementation chooses). That is why U2 builds on its own.
  - An apply that takes the lock twice (new window — `dock_into_window`) `locate`s again in the commit lock and checks that it is the same (§4-3).
  - `agent.spawnInto` is an `async` apply that waits for a daemon spawn midway (`spawn_into` in `apply.rs`), so it is excluded from the `spawn_blocking` of §8 — `at` is resolved inside the first lock of that function.
- Things that disappeared after resolving (`TabNotFound` · `SlotNotFound` · `SlotNotInTab` that arise from a **target selected by agent · word**) are a race, not the caller's fault, so `CONFLICT`.

### 5-3. Error code mapping · declaration

- `DockError` → bus: `SelfTarget` · `InvalidShare` → `INVALID_ARGUMENT` · a tab · pane that the caller **gave by id** is missing (`TabNotFound` · `SlotNotFound` · `SlotNotInTab`) → `NOT_FOUND` · **`WindowNotFound` → `CONFLICT`** (ADR-0302 decision 9 · same as `window_arg_error`) · the rest (`ZeroArea` · `WindowClosed` · `WindowExists` · `SlotIdOverlap` · `WindowOpen`) → `CONFLICT`. The message is that of `DockError` as is.
- **List every code actually emitted in the `errors [...]` of the declaration of the new and widened commands** (ADR-0156 discovery contract · `spawn_not_applied` filters by the declaration) — usually `[NOT_FOUND, CONFLICT]`. The answer the daemon gave in place of this command because it could not resolve an agent field (not found `NOT_FOUND` · overlap `CONFLICT` — §5-1) is also this command's answer, so it is in the declaration. Fix the sentence 「적용 실패는 코드 하나로 나간다(CONFLICT)」 at the head of `commands.rs` according to this mapping.
- **Carrying candidates**: `CommandError` carries only the code · message · retry (no structured field). Candidates go **at the end of the message** as `— candidates: a, b, c` (up to 5 · in English — the existing error messages are English).

## 6. Daemon side (group A)

- **Conversion before relaying (ADR-0308)** — location = in `deliver` of `crates/engram-dashboard-daemon/src/command_delivery.rs`, right before another party's name is passed to `route` (the one place both entrances, socket and control route, pass through · `resolve_agent_args`).
  - The marks are read from the registry (`CommandRoster::agent_args_of` — the `agent_args` the owner registered). Only string values of the marked fields are swapped; absent · `null` · non-string values are not touched (the owner bounces them).
  - The resolver registry (`LocalCommands::agent_roster` — the same registry the `agent.*` verbs use to resolve a target) is read in **`spawn_blocking`** — path normalization of a sleeping profile can take long (`AgentManager::roster`). A read cannot be cancelled, so a concurrency cap is set (`MAX_RESOLVING_IN_FLIGHT` = 16) and the count is released when the read ends · when it overflows, `CONFLICT` without relaying · if the caller has already left, no read is done.
  - Failure = the code of `resolve_in` as is · `"<칸>: "` before the message · no relay. If the slot of that round trip is settled first during resolution (deadline · shutdown · caller leaving), the code of that decision is the answer (the message is 「주인에게 보내지 않았다」) and nothing is relayed · if the resolving task dies, `INTERNAL`.
  - The daemon does not know whether the field is a pane target or a tab target — it does not even open `help` (ADR-0308 decisions 2 · 3).
- Marked fields: the declaration macro of the command tool crate (`command`) carries `#[agent_ref]` (only on a top-level argument of `String` · `Option<String>` — anything else is a compile error) as `CommandSpec.agent_args` → `CommandDecl.agent_args`. An old packet without this field reads as an empty list.
- `agent.resolve` — declared in `crates/engram-dashboard-agent/src/commands.rs` (next to the producer · ADR-0155). args `{targets: Vec<String>}` (1~8) → ok `{agents: Vec<{target, id, name, live: bool}>}` (same order). Body = `resolve_in(roster, target)` as is for each word (id first · exact name · duplicate = CONFLICT · none = NOT_FOUND). If even one fails, it fails with that code and the message says **which word** failed. `#[effect(Read)]`. ★This is a CLI convenience command — the shell does not call it★ (name resolution is the pre-relay conversion above).
- Candidates go into the error messages of `resolve_in` — NOT_FOUND: names containing that word (case-insensitive) first, otherwise up to 5 from the front · CONFLICT: the ids of the overlapping agents (up to 5). **All `target` verbs and the pre-relay conversion receive it together** (rule §1-3). The cross test compares only codes (`control_agent.rs`) and the CLI does not interpret the message (review confirmed) — only the tests that assert the message are fixed.
- Raise the `catalog_version` of the agent crate.
- The answer JSON golden of `agent.resolve` (`crates/engram-dashboard-agent/tests/fixtures/agent_resolve_ok.json`) is compared with the serialized result by the agent crate test — the shell tests do not read it.
- TS bindings: the agent crate exports command types explicitly (`crates/engram-dashboard-agent/tests/ts_export.rs`) — add the new types too and commit the generated output (CI sync gate).

## 7. Widening existing commands (group D)

| Command | Added arguments | Arguments that become optional | Response |
|---|---|---|---|
| `slot.focus` · `slot.close` · `slot.split` · `slot.popout` | `slot_of` · `at` (+`window`) | `view_id` · `slot_id` | if the layout changes, add `layout` (changed tab rows) |
| `slot.assignAgent` · `layout.setSlotContent` | `slot_of` · `at` (+`window`) · `assign` | `view_id` · `slot_id` · `agent_id` | add `layout` |
| `tab.switch` · `tab.close` · `tab.rename` | `tab_of` | `view_id` · **`window`** (the window can be known from the tab) | `tab.close` adds `layout` |
| `agent.spawnInto` | `at` (+`window` as is) | — | as is (+`layout`) |

- If `assign` · `agent_id` **does not resolve, it is rejected** — today an id that does not exist is also accepted and the pane is left empty (a trap that help documents · §11-3). Both are agent fields, so the daemon resolves them against the registry before relaying, and if absent the daemon answers `NOT_FOUND` and it does not reach the shell (§5-1).
- Adding `layout` to the response is an **append** (adding a field), so old callers are unaffected. Rule §1-4 · ADR-0307 decision 4 (user decision) are kept for these commands too.
- Making arguments optional still fits old callers that give an id. Raise the shell `catalog_version`.

### 7-1. Changes visible to existing callers (measured in implementation · review — recorded only, no code · no decision)

- `agent_id` (the Agent of `slot.assignAgent` · `layout.setSlotContent`) is also an agent field, so the daemon resolves it against the registry before relaying — if it is an id · name in the registry, it is passed on swapped to that id (canonical spelling), and if absent the daemon answers `NOT_FOUND` · if the name overlaps, `CONFLICT` (before, it was written as received · §11-3). The resolver matches an id against every spelling `Uuid` can read (uppercase · braces · no hyphens · `urn:uuid:`) before names, so an old caller that sent an uppercase UUID still resolves as before (the mail entrance accepts only the canonical spelling — a known leftover mismatch · the cross-comparison test asserts both sides). An empty or nonsense value is also the daemon's `NOT_FOUND` before it reaches the shell's checks (`INVALID_ARGUMENT` — an empty value · an `agent_id` given although the content is not an Agent). A call that did not pass through the daemon is bounced by the shell with `INVALID_ARGUMENT` if it is not a UUID.
- Nonsense arguments that used to be ignored now take effect — giving only `window` to a pane verb is `INVALID_ARGUMENT` (「window 는 at 과만」) · the same for `at` · `slot_of` · `assign` · `tab_of` (because under the bus rule that unknown arguments are ignored — ADR-0157 — those names are now 「아는 인자」).
- The daemon's agent-field resolution runs before all of the shell's checks — a non-existent pane + an unknown `agent_id` gets the agent's `NOT_FOUND` instead of the pane's `CONFLICT` · even for an argument combination the shell would bounce, an unknown agent gets `NOT_FOUND` first.
- Same code · different message: required id missing (serde 「missing field」 → 「the slot/tab is missing — give one of …」) · an empty tab · pane id string · a numeric window + a malformed `view_id` (the UUID message comes first).
- `window` of the tab verbs became optional — if given, the tab must be in that window (the old error as is).

## 8. Handlers · threads

- Commands that take a target (the new four + the widened pane · tab verbs) are plugged in through the handler `targeted_handler` (`layout/commands.rs`) — the whole body (argument checks · window argument resolution · apply = lock + select + mutate) runs in **`spawn_blocking`**, and the answer is JSON with the `null` fields dropped. There is no daemon round trip on the shell side, so there is nothing to wait for (the daemon swapped the name before relaying — §5-1).
- Why the blocking pool — `NewWindow` deadlocks window creation when called from the main thread · a synchronous command (the `dock_tab` doc in `apply.rs`), and so that tokio workers are not blocked during the prewarmed-window wait (~1 s).
- `agent.spawnInto` stays an asynchronous handler (`SpawnInto`) — it waits for the daemon spawn inside the apply (§5-2).
- Not blocked even during a drag (mouse tab drag) (ADR-0297) — the drop looks up the moved tab again.

## 9. Tests

- Daemon (agent crate): `agent.resolve` id · name · duplicate · none · several · candidate messages · answer golden JSON.
- Daemon (daemon crate · `command_delivery.rs`): pre-relay conversion — name → id in marked fields · an id stays as is · unmarked fields · non-string values not touched · marked order · none `NOT_FOUND` · overlap `CONFLICT` (message head `<칸>: `) means no relay · the registry is read in the blocking pool · a deadline during resolution is `TIMEOUT` and no relay.
- Command tool (command crate): `#[agent_ref]` → `CommandSpec.agent_args` (declaration order · schema unchanged — agent crate `tests/command_alphabet.rs`) · the registration packet carries it in a field outside `help` (`table.rs`).
- Shell `src-tauri/tests/layout_commands.rs`: the new four + the widened arguments — agent fields are given an id directly (name → id is for the daemon tests) · the `agent_args` of the registration packet is exactly the marked fields per command · a non-UUID value in a marked field = `INVALID_ARGUMENT` · update the table-name golden list · argument exclusivity (both / none) · candidate messages · error code mapping (including WindowNotFound = CONFLICT) · declared `errors` match the actual codes · `Keep` (visible tab · focus unchanged — three exceptions §11-4) · `reused` of `tab.detach` → `changed: false` · response `tab` of `tab.dock` = destination tab · NewWindow is headless with a fake `WindowHost`.
- Shell `src-tauri/tests/layout_apply.rs`: `Target` resolution (inside the lock) · **interleaving test** (if that agent is moved after the daemon swaps the name for an id and before the shell applies it, it is resolved to its **current place** inside the apply lock — the tab it moved to moves and the tab it left is not touched · in no pane = `NOT_FOUND` · if moved between the two locks of a new window, the opened window is withdrawn and `CONFLICT`) · the `after_tab_of` position (source in front · behind in the same window) · `LayoutRow` building (window · tab · pane order · coordinates · multiple marks · empty tab).
- Commands: `cargo test -p engram-dashboard --test layout_commands` · `--test layout_apply` · `--test lib_unit` · `cargo test -p engram-dashboard-command` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · CI generated-output sync.

## 10. Implementation units (split by file overlap · the build stands per unit)

| Unit | Files | Depends on |
|---|---|---|
| U1 daemon `agent.resolve` + candidate messages + golden | agent crate `commands.rs` · its tests · `tests/ts_export.rs` · bindings | none |
| U2 shell apply layer — `Target` · `locate` · `DockDest::TabBar.after` · `DockOutcome.changed` · `LayoutRow` building · size calculation hookup | `src-tauri/src/layout/{apply.rs, manager.rs, dock.rs}` + new module · `tests/layout_apply.rs` | none (parallel with U1) |
| U3 daemon part — marked fields (`#[agent_ref]` · `agent_args`) · pre-relay conversion (ADR-0308) | `crates/engram-dashboard-command/src/{macros.rs, spec.rs, table.rs, roster.rs}` · `crates/engram-dashboard-daemon/src/{command_delivery.rs, command_roster.rs, control/commands.rs}` · protocol registration packet · its tests | U1 |
| U3 shell part — agent field marks · the new four commands · error mapping · widened commands | `src-tauri/src/layout/{commands.rs, apply.rs, manager.rs, rows.rs, target.rs, window_arg.rs}` · `src-tauri/src/view_commands.rs` · `src-tauri/tests/{layout_commands.rs, layout_apply.rs}` | U2 · U3 daemon part |
| U4 help · CLAUDE.md · step-log · tracking | `prompts/engram-help.md` · `CLAUDE.md` · `docs/` | U3 |

## 11. Decisions for the user to look at on return (main proceeds with the defaults)

1. **If the source tab has several panes, the whole tab moves** (§2 — same as the mouse). Alternatives = reject · a new path that pulls out a single pane (not in the code today).
2. **The `share` default of `tab.dock`** = the same value as the mouse, 0.5 if the size is unknown (§4-2).
3. **`slot.assignAgent` · `layout.setSlotContent` reject an agent that does not resolve** (§7 — today they accept it and leave an empty pane).
4. **Three cases where the visible tab changes even with `Keep`** (exceptions to slot-docking PRD R11b — when the source tab leaves, something must be shown): ⓐ moving the visible tab within the same window makes the destination tab visible ⓑ when the visible tab of the source window leaves, a neighboring tab becomes visible ⓒ when the last tab of main leaves, an empty tab is newly created and shown (the `dock` succession in `manager.rs`).
5. **If the tab an LLM moved during a drag is dropped outside our webview, no new window is created** (slot-docking PRD D9 · TRD §9-5 「T-65 가 다시 정한다」) — this TRD **leaves that behavior as is** (a rare overlap · ADR-0297 「막지 않는다」 kept).
7. **There is no way to attach a single pane beside another pane** — `tab.dock` moves the whole tab. If two agents are already in one tab, pull one out with `slot.popout` and then `tab.dock` (two calls). Doing it in one call needs a new pane-level docking path (not in the code today · pointed out by the U3a review).
6. **The LLM's default new-window size · attach share differ slightly from the mouse** (§4-2 · §4-3 — when the source window is maximized · when detaching from main). To make them the same, a port that lifts the monitor work area · window frame inputs into the shell is needed (out of scope this time).

## 12. Edition 1 → edition 2 (review applied)

- The response `tab` was the source id that dock deleted → the surviving destination tab (codex).
- Selecting a pane from name · word was under a different lock than the apply (TOCTOU) → resolve `Target` inside the apply lock + interleaving test (codex · Claude).
- When the destination pane was an agent that appears in several panes, there was no argument to fix it with → `beside_slot` (both).
- The flat rows lost left/right · up/down → normalized coordinates `x · y · w · h` (codex).
- The name round trip took 3 s per word and could exceed the 7 s delivery deadline → batched into one, 2 s · if exceeded, no apply + `INTERNAL` (Claude).
- 「마우스와 같다」 was false (new-window size · share original size) → `drag_size::drag_sizes` (Claude).
- `agent` overlapped as 「어느 칸」 and 「넣을 에이전트」 → `slot_of` · `tab_of` · `beside` · `assign` (Claude).
- The tab verbs kept requiring `window` → optional (Claude).
- Declared error list · the `WindowNotFound` code · head sentence → §5-3 (Claude).
- The `to_agent` position calculation was outside the lock · off by one → `DockDest::TabBar.after` (Claude).
- Source of `changed` · no identity → `DockOutcome.changed` · `CONFLICT` (Claude).
- Agent crate edition · fixing the answer shape → `catalog_version` · shared golden (Claude).
- `TIMEOUT` had two meanings → `INTERNAL` (Claude).
- Response size → only changed tabs · ids only when `ids: true` (Claude).
- Deferring the response of existing commands violated the user decision (ADR-0307 decision 4) → `layout` for the widened commands too (Claude) · the R11b exception and D9 raised in §11 (Claude).

### Edition 2 → edition 3 (Claude re-check applied)

- Edition 2, which said it would compute the new-window size · share original with `drag_sizes`, was also effectively false because the inputs (work area · window frame · viewport) are not in the shell → the `plan_new_window` · canvas stand in, and the difference is stated in §11-6.
- The code for exceeding the name-resolution deadline, `INTERNAL` (no retry · means panic) → `CONFLICT` + `AfterCondition`.
- Between the two locks of a new-window apply → `locate` again in the commit lock · if different, withdraw the window and `CONFLICT`.
- `after_tab_of` was converted to a `ViewId` outside the lock → `Target::AfterTabOf` · `DockTo::TabBar` also gets the field · edge (inside the source tab · several tabs).
- The `layout.get` rows had no tab-grouping key → `tab_index`.
- A new shell connected to an old daemon (which does not know `agent.resolve`) even blocked old calls that gave only an id → only names get `CONFLICT` · ids skip the check.
- U2 did not build on its own (the apply signature change breaks about 10 caller files) → the old id signatures are kept.
- `agent.spawnInto` cannot go into `spawn_blocking` → exception · `at` inside the first lock.
- `identity == None` as `CONFLICT` → `ok`, since it was already moved.

### Edition 3 → edition 4 (ADR-0308 applied · as implemented)

- The name resolution where the shell asked the daemon back with `agent.resolve` while handling (port · 2 s deadline · its failure code · cleanup of abandoned wait slots) → the shell marks agent fields with `#[agent_ref]` and the daemon swaps them for ids before relaying · if it cannot resolve, the daemon answers without relaying (§5-1 · §6). Reason for rejection = ADR-0308.
- Shell handler: marked fields accept only an id · `INVALID_ARGUMENT` if it is not a UUID · asynchronous handler → the whole body in `spawn_blocking` (`targeted_handler` · §8).
- The basis of the declared `errors` fixed to the daemon's rejection of agent fields (§5-3) · the `agent_id` confirmation · the changes for existing callers rewritten on the daemon-conversion basis (§7 · §7-1).
- The shell tests do not read a fake daemon · the `agent.resolve` golden — they give an id to agent fields · they measure the marked list and the bounce of non-UUID values (§9) · the daemon part added to the implementation units (§10).

### Edition 4 code review (FIX) applied

- Daemon: concurrency cap on registry reads (`MAX_RESOLVING_IN_FLIGHT` 16 — the mark is released inside the read · when it overflows, `CONFLICT` without relaying) · caller-left check before the read · `TIMEOUT` · `OUTCOME_UNKNOWN` settled during resolution keep the code and the message is changed to 「주인에게 보내지 않았다」 + warn · removed the `LocalCommands::agent_roster` default body · `resolve_in` takes every UUID spelling as an id (§7-1) · `lint_spec` requires the `NOT_FOUND` · `CONFLICT` declarations on marked commands.
- Shell: the error head of an agent field carries the display name (`slot_of 'beta' (<id>)` — the id if not in the ledger) · the `agent_arg` bounce says 「이름이나 id 를 주라」 · the registration test measures the field count · field-name length · advertised codes · a missing mark on an agent-name-shaped argument.
