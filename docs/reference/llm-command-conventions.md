# LLM command rules

Rules to follow when you create or change a command that an LLM calls through the bus or the `engram` CLI. **Goal = the fewest calls · the least amount loaded into context** (the user's standard: 「LLM 시나리오는 여러 번 거치지 않아야 실수가 적다」).

- **Status:** draft (S27 · 2026-10-26). 「정함」 (decided) is what was agreed with the user; 「미정」 (undecided) is not yet decided — do not follow an undecided item as if it were decided.
- **Decision record:** ADR-0307 (`docs/decisions/0307-llm-명령-규칙-명령은-한-호출-조회는-영역별로-바로-출력.md`)
- **Supporting research:** `docs/research/llm-command-design-query-vs-action-2026-10-26.md` · `docs/research/pane-positional-addressing-2026-10-26.md`
- **This document is edited in place** (settled doctrine). The *reason* a rule changed and the alternatives dropped are kept in an ADR.

## 1. Commands (writes) — decided (detailed names · arguments · response wording are chosen by the main session, toward what the LLM uses well — delegated by the user 2026-10-26)

1. **One scenario = one command.** Build commands so a common request ends in one call. The multiple steps (find → change → place) happen inside the command — for example, `agent.spawnInto` does the spawning and the slot placement in one go.
2. **A target is accepted in words the LLM already knows** — do not make it query first to find out an id. The command resolves the id inside (the lookup step does not disappear; it moves inside the shell). The accepted words:
   - Agent name (already known, because the LLM spawned the agent itself or it is a messaging counterpart) — **resolution follows the one resolver of the daemon registry (the `resolve_in` semantics — id first · exact name · duplicates are CONFLICT · NOT_FOUND if none).** Do not resolve separately through the shell's id→name cache (`agent_names` in `layout/manager.rs` — entries are never removed) — with two resolvers the decisions diverge (ADR-0132 · the head comment of `resolve_in` in `crates/engram-dashboard-agent/src/commands.rs`). **Mark an argument that takes an agent with `#[agent_ref]` — the daemon converts it to an id before relaying (ADR-0308).** The shell handler receives that field only as an id.
   - Spatial word (corner · direction relative to the current slot — the words of `slot.resolveSpatial` · ADR-0068)
   - id (accepted as is — for programmatic consumers · when the caller already holds an id)
   **Give each kind of word its own argument** (for example `--agent` · `--at` · `--id`). If one argument takes several kinds mixed together, an agent named 「2」 or 「left」 collides with the number or the word.
3. **If it is ambiguous or missing, stop with an error and give a clue for fixing it.** If several match, do not guess (same as tmux). Put a few matching candidates in the error — so the next single call can fix it.
4. **Put the state after the change briefly in the response.** A command that changed the layout returns the resulting layout (what is inside each slot) — the LLM does not have to infer the result, and there are fewer separate queries to make. A command that creates something returns the new id. The shape of the response (one-line form · text/JSON) is in §4.
5. **Do not build batch execution or scripting** (until it becomes necessary). If the same two or three calls actually repeat, turn that scenario into one command. A batch (a list of commands in order · stop on failure) is brought back when that is still not enough. Scripts that need variables · conditions · loops are not done.

## 2. Queries (reads) — decided (only the slot of item 4 is undecided)

1. **Query only when you do not know the word to call with.** If you can call with the words of §1-2, go to the command without a query.
2. **One query per area + a filter argument.** Split by area, like layout · agent · settings (font · theme …), and provide an argument that picks 「어느 부분만」 (only which part) (precedent = the key prefix of `settings.get`). If you know the word to look for, use a pinpoint query narrowed by that argument.
3. **Output the result straight into the context.** Do not write it to a file and make the LLM find it with bash — the result of one area is small (estimate = ADR-0307 「근거」 — the assumptions and the method are written there), and file + search adds calls. If you know the word to look for, the pinpoint finishes in one call, and if you do not, you have to read all of it anyway.
4. **One shared safeguard that hands a large result off to a file — where to put it is undecided.** When a response crosses a threshold, write it to a file and output only the path + the beginning (precedent = LangChain Deep Agents' 20,000-token threshold · path + first 10 lines). Candidate places: ① the `engram` CLI — every response on the agent path passes through it, but it collides with the user's decision that the CLI only does authentication + delivery (ADR-0285 decision 1) ② the shell handler · the daemon — the side that holds the content, but the places to touch differ per area. Where to write the file (the data location) is also the user's decision. The threshold value is set at implementation. (Not implemented yet — today the CLI prints the body as is.)
5. **Do not build a query that outputs all areas at once** (until it becomes necessary). If it ever gets built, that is the size where item 4's file + search fits.

## 3. Slot numbers — we do not address by number for now (user decision 2026-10-26)

- **Call a slot by the name of the agent in it or by a spatial word.** Agent-side commands already take a name (daemon `resolve_in`), and a name does not change on insert · close · resize. The current defect where names overlap (tracking T-45) only stops with an error + candidates and does not hit a wrong slot, and it goes away once the switch from names to internal unique ids (T-29 · T-45) is done.
- Addressing by number will be looked at again when we later design the on-screen slot-number display. The snags below are settled first then.

- **Coordinates are not used (decided).** Column · row coordinates like 「x1 y2」 — it is a split tree, so the rows differ per column and it is ambiguous, and none of the tools surveyed used them (accepted by the user 2026-10-26).
- **Two snags of addressing by number:**
  - **It hits the same reason we dropped window numbers** — the window argument does not accept a number (ADR-0302 decision 9 · user decision 2026-10-23: after 2 is closed and a new 2 appears, a late-arriving `window.close("2")` closes the new window). Slot numbers change more often than window numbers.
  - **The ordinal also changes on resize** — the ordinal in the code (`ordinal` in `src-tauri/src/layout/spatial.rs`) compares the slot center heights as they are (with no tolerance) and numbers top→bottom, and only when equal, left→right. So dragging a slot boundary even slightly (`split.setRatio`) changes the order, 「같은 높이면 왼→오」 (same height then left→right) almost never happens, and there is no guarantee either that the slots of one column get consecutive numbers (a limit that ADR-0068 decision 1 accepted). Counting starts at 0.
- Things to decide together if numbers are adopted: the range of the number (within one window · tab) · whether it is shown starting at 1 · whether to put a tolerance in the height comparison (grouping into rows) · whether to show it on screen all the time or only when needed (tmux `display-panes` only on a key press) · a notation that avoids confusion with the window number (title-bar marker · ADR-0300).

## 4. Output format — undecided (the main session decides at implementation)

- Among text · JSON · other compressed formats there is no universal winner (it differs by model · data). Like the command details, the main session decides toward what the LLM reads well and reports (delegated by the user 2026-10-26).
- The effect on size is large — in ADR-0307's estimate, JSON carrying ids (UUIDs) is about ten times text without ids. The LLM calls by name, so ids need not always be carried.
- Constraints (facts): the daemon does not interpret the body of a UI command (ADR-0081 · ADR-0156) — to change the shape, do it in the shell handler. There is one exception, only the arguments the shell marked with `#[agent_ref]` — the daemon converts only the name in that field to an id and relays it (ADR-0308 amends ADR-0081 · ADR-0156). The response body stays as is. The CLI today prints the response body as is (including the corrective JSON of a failure body).

## 5. Check when you add a new command

- How many calls does a common request take — if two or more, can §1-1 · §1-2 reduce it?
- Is there an argument left that takes only an id · does each kind of word have its own argument (§1-2)?
- Does it resolve an agent name with the daemon resolver's semantics (§1-2)?
- Mark an argument that takes an agent with `#[agent_ref]` — the daemon converts it to an id before relaying (ADR-0308). For a marked command, list `NOT_FOUND` · `CONFLICT` in `errors` (if the daemon cannot resolve it, it answers with those codes — `lint_spec` catches this).
- Do the candidates go into the ambiguous · missing error (§1-3)?
- Does the response carry what the next call needs (a new id · the changed layout) (§1-4)?
- Did you write the usage in `prompts/engram-help.md` — the usage instructions the LLM reads are there.
