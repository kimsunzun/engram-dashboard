# ADR-0419: Step 10b: file and trash commands - names and shapes, shell-only emptyTrash gate, the long wait for file writes

- 상태: 확정 (2026-11-24, 근거: user decision 2026-11-24 (D7 · D8 as recommended — 「ㅇㅇ 리뷰 구현 QA 까지 쭉 진행해.」) · main defaults D1–D6, D9–D16 · TRD review deep (lead, risk axis, Codex) PASS 2026-11-24)
- 관련: Amends ADR-0369 (decision 1 launch requests) · Amends ADR-0322 (decision 1 agent command table and engram help) · ADR-0386 (10a surface rules) · ADR-0408 · ADR-0409 · ADR-0414 – ADR-0418 · ADR-0155 · ADR-0157 · ADR-0307 · ADR-0379 · step-10b TRD `docs/process/S25-preset-library/trd-file-commands.md`

## 맥락
Step 10b declares the commands over the step-9b service (move, rename, copy, delete, the trash view, restore, emptying). ADR-0322 decision 1 made emptying human-only through a shell-only bus command "not in the agent command table". ADR-0369 decision 1 gave the long deadline class to launch requests only. The TRD review found that a refusal the daemon gives never reached an agent through the CLI (the CLI refused unlisted names locally), and that on the agent's path the daemon's own commands run with no delivery seat, so a long file command ends in the CLI's `CONNECT_FAILED` while the command keeps running.

## 결정
1. **Names (D1):** `library.move` · `rename` · `copy` · `delete` · `listTrash` (the view — `library.trash` reads as a verb) · `restore` · `emptyTrash`. Arguments snake_case (`to_folder`, `name`, `on_conflict` with `Overwrite` · `Skip` · `KeepBoth`, `detach_refs`, `plan_digest`, `rows`, `keep_both`). The 9b codes are final as built (D4); per-item rows carry `status` (`done` · `skipped` · `conflict` · `failed`) and skip codes (D3); the `CONFLICT` forms spell the wire's argument names (`DeletePlan::to_json`, D5).
2. **The shell-only gate (D6, amends ADR-0322 decision 1's wording):** `library.emptyTrash` is declared in the agent crate's shared command block (the socket runs only that table) and listed in `SHELL_ONLY`; `call_daemon_command` refuses it when the caller has no connection (every control route), and the control route's command list omits it. Its name is absent from `engram commands`, the `engram help` command lines and the priming (a text guard over every help blob); one help sentence says emptying and deleting for good are done by the user in the library screen (D7).
3. **The refusal (D7, user):** bus code `UNSUPPORTED` with `args.code` `shell-only` and that sentence, no catalog hint. The CLI posts a name its catalog read does not list to `/control/call` so the daemon's refusal reaches the agent (D16), except when the catalog read dropped rows (then the local refusal stays); after a daemon `UNKNOWN_COMMAND` the CLI adds its `engram commands` line.
4. **The long wait (D8, user; amends ADR-0369 decision 1):** the launch deadline class also covers `library.move` · `rename` · `copy` · `delete` · `restore` · `emptyTrash` (not `listTrash`). On the CLI path that is the CLI's 94 s wait (no seat there; past it `CONNECT_FAILED` "read failed" while the command finishes — the help says outcome unknown, do not repeat); on the shell socket the seat's 85 s bound, the shell's own wait widened by the client stage.

## 거부한 대안
- **Hide `emptyTrash` as an unknown name (D7 (a))** — the agent cannot tell the user where emptying is done; the gate guards against mistakes, not an attacker. (user chose (b))
- **Keep the CLI's local refusal for every unlisted name** — the daemon's `shell-only` refusal would never reach an agent through `engram`. (TRD review round 1, all three reviewers; code: `run_invoke`)
- **Today's ordinary waits for file commands (D8 (a))** — a cross-volume move of a large folder ends in "outcome unknown" after 10 s and invites a repeat. (user chose (b))
- **`library.trash` for the view** — reads as "move to the trash" (`gio trash`); trash-cli `trash-list` and `gio trash --list` name the view separately.

## 근거
- TRD review deep, 2 rounds (lead, risk axis, Codex), PASS 2026-11-24; conventions: trash-cli and GNOME `gio` command shapes and blocking behaviour (README / man pages, checked 2026-11-24).

## 영향 / 불변식
- agent `commands.rs` / `library_commands.rs`, asset-library `host.rs`, daemon `control/commands.rs` / `catalog.rs` / `help.rs`, CLI `engram.rs`, `prompts/engram-help.md`, the priming.
- The gate is keyed on `caller == None` at `call_daemon_command`: a future seat for the daemon's own commands on the control route (tracking T-133) gives a synthetic `ConnId` and must keep the gate on purpose.
- Client stage: the shell's `reply_deadline_of` must classify these launch-class commands, or screen calls wait only the shared 30 s.
