# S25 step 6 — context material (codex launch-material translation + settings-check port codex impl)

Repo `I:\Engram\apps\engram-dashboard-wt1` · branch `v0.5.0/feat/preset-library` · HEAD `829f920a` (clean apart from
`src-tauri/bindings/*.ts` EOL noise). Step 5 = `baf769ff`. Installed: `codex-cli 0.163.0` (QA lane record
`lanes.tsv`: codex PASS on `codex-cli 0.163.0` at `b631515e +uncommitted`).
TRD = `docs/process/S25-preset-library/trd.md` (1162 lines) · PRD = `.../prd.md`. Paths below under
`crates/engram-dashboard-agent/src/` unless absolute.

---

## 0. Where docs differ from code (read first)

1. **The port as landed lacks four codex pieces the TRD says it "holds"** (§1-5 L272-280). `backend/settings_check.rs`
   has `backend · schema_source · version_probe · parse_version · raw_to_json · merge · unknown_keys · loads ·
   accepts_skill_name · folder_entry_blocks · folder_load_set` — **no codex skill-format method, no command-line guard /
   8191 budget method, no codex front-matter type, and `FrontMatterParser` has only `paths` + `subagent`** (claude
   types). The step-5 hand-over (L880) lists only four additive items (skill-root kind · malformed mark · per-version
   `SchemaRefetch` · `omitted` list) — it **omits** the skill-format method, the agent-defined codex front-matter serde
   type (M72) + its parser-seam method, and the guard/budget method. All are additive but the hand-over list is
   incomplete. Only one `FrontMatterParser` impl exists today (`backend/claude/settings.rs:896`, test fake).
2. **`additionalContext` is an experimental field and our handshake opts out of experimental APIs.** Measured now
   (`codex app-server generate-json-schema`, 0.163.0): `TurnStartParams.additionalContext`
   (`{<key>: {kind: "untrusted"|"application", value}}`) appears **only** with `--experimental`;
   `InitializeCapabilities.experimentalApi` ("Opt into receiving experimental API methods and fields", default false)
   exists. Our `InitializeParams` (`backend/codex/protocol.rs:338`) deliberately sends **no `capabilities`**, pinned by a
   test (`protocol.rs:1168` `assert!(v.get("capabilities").is_none())`). TRD/ADR-0337 never mention the opt-in.
   `skills/extraRoots/set` IS in the stable schema (`SkillsExtraRootsSetParams {extraRoots: [AbsolutePathBuf]}`,
   required). Neither method nor field exists in `protocol.rs` / `transport.rs` today (method list `protocol.rs:214-226`;
   `TurnStartParams` `protocol.rs:559` has `thread_id · input · client_user_message_id` only).
3. **The codex backend cannot see the stored instructions hash.** T6 needs "hash differs from the last-applied value"
   but `materialize(command, kit, cwd, fs)`, `build_spec(...)`, `open_spawn(...)` get no profile; `LaunchKit` carries
   only the new `instructions_hash` (`launch.rs:195`); profile field `last_applied_instructions_hash`
   (`profile.rs:181`) is never written (`manager.rs:1780` `TODO(TRD S25 §5-2 4 (b))`). No TRD step owns the
   manager-side recording + the stored-value hand-down. TRD §5-2 4 (b) only says "taken from the kit".
4. **No fallible spot after argv assembly.** TRD L279: launch-step 7 re-measures the real argv and on overflow "cuts it
   off before spawn with the same `Broken`". In code `build_spec` returns `CommandSpec` (infallible); only
   `materialize` → `Err(String)` → `PtyError::SpawnFailed` (`manager.rs:1719-1722`) and `precheck_control_endpoint`
   (runs before materialize) can fail. There is no path that turns a build-time overflow into `LaunchBlocked`/`Broken`.
5. **"Current path as is" vs "not dropped silently" for the terminal priming.** §1-5 table L309 says the 「기본」 priming
   goes by "the current path as is (`-c developer_instructions=`)"; today that path is fail-open
   (`developer_instructions_override` `codex/mod.rs:379` warns and drops on a bad char or budget overflow). L279 says the
   count must include it so a preset `-c` line cannot silently push it out (M8). The two statements conflict for the
   real path.
6. **`extraRoots` "junction folder" rests on an unmeasured premise.** §1-5 (나) 6 L299, table L310, ADR-0337 d1 and the
   code TODO (`codex/mod.rs:658`) say JSON skills = a junction folder in the volatile dir passed to
   `skills/extraRoots/set`. §6-2 L1004 says "whether codex's `extraRoots` walk follows a link (junction · symlink) under a
   root" is **not established**. Research 2026-10-20 row 6 measured junction-following only for user/project scopes
   (a working-folder `.agents/skills` junction), not for an extra root. If the walk does not follow junctions the
   designed layout loads nothing.
7. **Notice producer.** `launch::notice_text` doc (`launch.rs` ~L345) says producers "arrive with the asset library and
   the codex translation"; §2-4 8 (L722) says step 6 owns codex-terminal omission codes + wording. But notices live only
   in `LaunchPlan.notices` (filled by `check` = asset-library, step 7); `materialize`/`LaunchArtifacts` have no notice
   channel. So step 6 can define codes/wording + `loads()` knowledge + the folder `omitted` list, not emit notices.
8. **Version output shape.** `codex --version` prints `codex-cli 0.163.0` — the claude parser rule "first word of a line
   is a dotted version" would return None; codex needs "first dotted-version word". TRD §6-1 L959 still cites
   `codex 0.156.1 --help`; `codex app-server --help` on 0.163.0 shows the same `-c` semantics ("parsed as TOML. If it
   fails to parse as TOML, the raw string is used as a literal") plus `--enable/--disable/--strict-config`.
9. **`CMD_UNSAFE_CHARS` is private** (`engram-dashboard-platform/src/shell.rs:52`, `const`, not `pub`); only
   `path_is_cli_safe(&Path)` is public. A codex `-c` *text* guard has no public helper to reuse.
10. **`LaunchArtifacts.skill_roots` doc** says "inside the volatile folder" (`launch.rs` ~L826); codex JSON must also
    carry folder-entry roots (outside the volatile folder) — field doc or a new field must change.
11. **No TOML parser in agent.** `raw_to_json` for codex needs TOML value parsing; agent `Cargo.toml` has none; TRD
    §1-12 library table lists none. `toml` 1.1.2 / 0.9.12 / 0.8.2 are in `Cargo.lock` (via tauri-utils etc.). 0a survey
    rule (L562: "re-survey libraries for each implementation part") applies.
12. Registry test `each_settings_check_key_is_the_backend_word_of_its_backend` (`backend/mod.rs:1989`) is pinned to a
    `[_; 1]` array with claude at `[0]`; `settings_check_for_ignores_ascii_case_and_refuses_unknown_words`
    (`backend/mod.rs:2017`) checks only claude spellings. Both grow with codex (as L880 says).

---

## 1. Spec excerpts (verbatim, trd.md unless noted)

### Step 6 entry — L876
> 6\. `S25: feat(agent): codex 실행 재료 번역(JSON · 터미널) + 설정 검사 포트 codex 구현` — JSON = developerInstructions · extraRoots (including skills in nested presets — M13) · raw text `-c` (including the model · effort lines) · resume `additionalContext` (T6) / terminal = the priming of 「기본」 by the current path (`-c developer_instructions=`) + the preset raw-text `-c` · preset instructions · skills are not loaded and go to `notices` · **nothing is written to the working folder** (P4-FINAL — the old 「작업 폴더 파일 · 크기 한도」 was withdrawn). The port's codex implementation = `-c` assembly (dotted key · TOML) · skill format (P7AC — YAML through the parse function handed in · the front-matter serde type is defined by agent — M72 · this step's tests use a fake function · the real parse golden is in 7 — main 2026-10-31 M63) · skill name rule (codex JSON only — M24-3) · command-line guard · 8191-character budget (both modes — M8) · unknown key = ⚠. It may be split in two (JSON / terminal). *Stop (after)*

### ADR-0341 in 6 — L878
> *ADR-0341 in 6* — codex: JSON = the folder's instruction file joins the merged instructions in plan order (so the T6 hash covers it) · the folder's `.agents/skills` · `.codex/skills` that exist are appended to the `skills/extraRoots/set` roots (no junction · nothing written) / terminal = nothing from the folder · the omission notice · the codex implementation of the port's folder load set (`AGENTS.override.md` > `AGENTS.md` · the two skill roots · the skills under them by codex's root-walk rules with the codex skill-format check as a mark — tested with a fake listing). The §6-2 unmeasured item 「codex behaviour when a folder skill and a library skill share a name across extra roots」 is measured in this step (real codex JSON turn) and the result is written back to §6-2 · §6-3, together with the fallback policy if codex drops, shadows or rejects one (decided here — PRD §6 row).

### Hand-over from step 5 — L880
> *Handed over by step 5 (2026-11-09 · `baf769ff`)* — the codex additions to the port are additive (the claude half does not change shape): a skill-root kind in the folder load set · a malformed-skill mark · a per-version `SchemaRefetch` variant · an `omitted` list for the codex-terminal folder notice. Do not reuse `unsure` for a malformed codex skill (`unsure` = "claude's finer rules not predicted", not "the format check failed"). The registry test (`backend/mod.rs` `each_settings_check_key_is_the_backend_word_of_its_backend`) is pinned to claude (`settings_checks()` has one slot) and grows with codex.

### §1-5 translation table, codex columns (L307-314)
- L309 Instructions — JSON: "New conversation = `thread/start.developerInstructions` (measured certain). Resume = only when the instructions hash differs from the last-applied value (profile field), `additionalContext` on the first turn (experimental API — T6)" · terminal: "**Only the priming of 「기본」** — the current path as is (`-c developer_instructions=` — P4-FINAL) · the preset instructions (all common · tagged ones) are not loaded and the notice carries the names (§1-3 6 · G28)"
- L310 Skills — JSON: "the same junction folder with `skills/extraRoots/set` (per process · measured likely) — including the skills of the presets nested inside (M13) · if the format is one codex cannot load, block ② (P7AC)" · terminal: "not loaded — notice (P4-FINAL)"
- L311 Raw settings — JSON: "the preset's codex raw-text lines one by one with `-c` · after the passthrough · before our overrides · if the command-line guard · length budget is hit, block ② (M8)" · terminal: "the same — loaded (P4-FINAL · it is a CLI arg, so it works in the terminal too) · if hit, block ② (M8 — withdraws the old 「빼고 경고」)"
- L312 Model — JSON: "not loaded separately — the `model=` line of the raw codex field goes into `-c` above (withdraws the old `-c model=<m>` translation)" · terminal: "the same"
- L313 Size limit — terminal: "(withdrawn — P4-FINAL …)"
- L314 Folder entries — JSON: "instructions = the folder's `AGENTS.override.md`, else `AGENTS.md` (the pick = backend settings-check port · the text is read by asset-library) joins the merged instructions in plan order → `developerInstructions` · resume `additionalContext` (the T6 hash covers it) · skills = the folder's `.agents/skills` and `.codex/skills` that exist, appended as extra roots to the same `skills/extraRoots/set` call (codex prunes dot-folders under a root, so the subfolders are passed, not the folder · no junction · nothing written) · `.codex/config.toml` not applied" · terminal: "nothing from the folder (no `--add-dir` — codex `--add-dir` is sandbox write only) · omitted instruction file and skills → notice ① (§1-3 6)"
- L299 (나) 6: "`backend::materialize(command, kit, cwd, fs)` … (claude plugin-folder junction · codex JSON's `extraRoots` junction folder · the codex terminal has nothing to lay out — P4-FINAL)"
- L317 Plan and artifacts: "for codex JSON, a list of extra skill roots … the extra skill roots stay apart from `skills` — they are roots, not named skills, so they enter neither the conflict check nor the junction folder … only `backend/claude` and `backend/codex` read these fields (ADR-0004 · ADR-0341 impact)."

### Settings-check port, codex parts (L273-280, L319)
- L273 "**Schema source** — … codex = `rust-v<ver>/config-schema.json` per edition · when to fetch again (… codex = that edition's file — P10). It does not do the download itself — it only gives the address · edition" (full URL form in research: `https://github.com/openai/codex/releases/download/rust-v0.156.1/config-schema.json` — `docs/research/settings-asset-validation-spike-2026-10-17.md:16`)
- L274 "**Reading the CLI edition** (`--version` — tests use a fake)."
- L275 "**Raw text → JSON** — … codex = assemble `key=value` into an object by the `-c` rules (dotted key = nested table · value = TOML, and if it does not read, the literal text)."
- L277 "**Unknown-key handling** — claude = not reported · codex = ⚠ (§1-4 `validate.rs`)."
- L278 "**codex skill format** — the file name is exactly `SKILL.md` · `---` front matter · YAML · `description` not empty · no BOM · `name` ≤ 64 … **This port does not read YAML** (main 2026-10-31 M63 …): the caller … passes the parsing function of `format.rs` in as a closure (front-matter text → a value deserialized into **a serde type that agent defines** (name · `description` · `metadata.short-description` — the same fields · types codex deserializes in typed form with `SkillFrontmatter`) or an error with a line — a value of the wrong type is also a parse failure, so it calls the ② repair exactly as codex does …), and the port (codex backend) does ① separation · ② repair … · ③ check … basis = codex `codex-rs/skills/src/parser.rs` @ rust-v0.156.1 — the same as 0.166.0 …: ① front-matter separation = codex `extract_frontmatter` copied by hand as is (`lines()` · first line `trim() == "---"` · up to the next `---` line · missing or 0 lines = no front matter · a whitespace-only block goes on to the YAML step · the BOM is not stripped …) ② **the repair is ported in full** — if the parsing function fails, run codex `repair_frontmatter_scalar_fields` once at the text level and call the same parsing function again on the repaired text (M63): among all top-level unquoted scalars, the values that contain a colon + space · the values that start with `[` `{` `@` `` ` `` … are wrapped in single quotes (an inner `'` is doubled · a trailing `#` comment is preserved · block scalars · already-quoted values are left as is) · if it still fails after the repair, the **original** error ③ the name · `description` check comes after deserialization … (name = whitespace collapsed, and if empty, the folder name · an error if `chars().count() > 64` · `description` is an error if empty · no length cap · a whitespace-only `metadata.short-description` counts as absent) · **golden tests** = upstream `codex-rs/skills/src/parser_tests.rs` L56–85 + … (… the backend-side tests are separated with a fake parsing function and measure only the order of separation · repair · retry — M63 …)."
- L279 "**Command-line guard** — is the codex `-c` line safe to pass through `cmd.exe /c` (two modes — review #8) · command-line length budget (8191 characters) — if it is hit, setting 「오류」 (block ② · nothing is dropped silently … — M8). … **Decision time (flow review G17)** — … `check` decides by counting the varying pieces at their **upper-bound length** … · step 7 measures once more with the actual argv and, if it overflows, cuts it off before spawn with the same `Broken` … The count includes the codex terminal's priming `-c developer_instructions=` too — today that value is only warned about and dropped when it exceeds the budget (`backend/codex/mod.rs` `developer_instructions_override`), so a preset `-c` line can silently push the priming out (M8 「말없이 빼지 않는다」)." ("step 7" here = launch step (나) 7 = `build_command_spec`, not TRD step 7.)
- L280 "**What it loads** … (now = the codex terminal loads only the priming of 「기본」 · everything else loads all — P4-FINAL). The resolution's notice ① (§1-3 6) · the conflict count (§1-3 5) · the targets of the codex skill-format check look at this".
- L319 (folder load set, codex part): "codex JSON = the instruction file (`AGENTS.override.md` > `AGENTS.md`) + skill roots (`.agents/skills` · `.codex/skills`) + the skills under those roots, found by **codex's `skills/extraRoots/set` walk rules** (research table 「Findings — codex」: walked from the root itself · ≤ 6 levels · dot-folders pruned below the root · `SKILL.md` only; links — the research does not establish whether codex follows a link under a root, so the walk does not follow links (§1-4) and this is unmeasured — §6-2), each skill run through the existing **codex skill-format check** (P7AC · M63 — the same front-matter closure) and returned with a **mark**, not a block (Engram passes the root, not the skill, so a malformed folder skill is codex's to skip) · codex terminal = nothing (the codex JSON set becomes the omission notice). The codex launch and the preview (§1-7) call the same method: the instruction file the preview names is the one carried …"
- Skill-name rule (§2-4 5, closed M24-3): "the 1–64 character `[a-z0-9-]` name check is an 「오류」 (error) only on the backend that rejects it (codex JSON)".

### §4 test-plan rows touching step 6
- L792 agent backend — translation: "claude · codex two modes × the material table (§1-5) · order invariant (… overrides come after pass-through) … · **codex terminal** (P4-FINAL) = only the priming of 「기본」 by the current path (`-c developer_instructions=`) + the preset's codex raw-text `-c` lines · preset instructions · skills are in neither argv nor the volatile folder · **zero writes to the working folder** (… ★but a test that goes through `backend::materialize` dispatch cannot use that as evidence★: the containment port (`place_in_volatile`) blocks first … either assert the placement is `Ok`, or measure 「밖에 기록 없음」 by calling that backend's `materialize` directly …) · codex JSON skills = including those in nested presets (M13) · … the model goes only through the raw-text field (… codex raw text `-c model=` — no separate `--model`)"
- L793 settings-check port: "**raw text → JSON** — codex `key=value`'s dotted key = nested table · value = TOML · if unreadable, literal text · … **schema source · update trigger** (… codex = a file per version …) · **codex skill format** (P7AC — a file name that is not exactly `SKILL.md` · no `---` front matter · broken YAML · empty `description` · BOM · `name` over 64 characters are one line each · codex front-matter split · repair golden — colon values revived by repair · an unknown field that looks like a flow (`tags: [next,@supabase/ssr]`) is not an error · if still broken after repair, the original error · the BOM is not stripped so it is an error · expected value = codex behavior … · **here a fake parse function** …: after the split, the first call fails → once more with the repaired text · if the second also fails, the original error · if it succeeds, it is not called again …) · **skill name rule** — outside 1–64 characters of `[a-z0-9-]` = 「오류」 only in codex JSON · for claude agents the same name is not an error (M24-3) · **command-line guard** — a codex `-c` line dangerous in `cmd.exe /c` · exceeding the 8191-character budget = 「오류」 (both modes — M8)"
- L799 Folder entries (ADR-0341): "… codex JSON instruction pick (`AGENTS.override.md` > `AGENTS.md`) and extra roots · codex folder load set over a fake listing (≤ 6 levels · dot-folders below a root pruned · `SKILL.md` only · malformed skill marked, not blocked) · … codex terminal notice · nothing written into the folder …"
- L800 real CLI (local `#[ignore]` or manual): "… a real codex `extraRoots` turn · the codex terminal comes up with the priming of `builtin\` and no file is created in the working folder (P4-FINAL) · survival of codex JSON skills (`extraRoots`) after compaction (T9 measured item) …"

### §6 rows
- §6-2 L972: "Actual model-turn loading of codex `skills/extraRoots/set` (research gap) · survival of `additionalContext` after compaction."
- §6-2 L973: "Does the codex JSON skill (`extraRoots`) survive compaction …"
- §6-2 L1002: "**Is our codex skill check the same as codex's** (step 7 — … step 6 uses a fake parsing function and checks only the order …)"
- §6-2 L1004: "**Folder entries (ADR-0341) — unmeasured** (step 8 real CLI lane unless noted): codex `skills/extraRoots/set` with `<folder>/.agents/skills` passed as a root — … a real turn with such a root is not measured · codex behaviour when a folder skill and a library skill share a name across extra roots (measured in step 6 — §5-2) · … · whether codex's `extraRoots` walk follows a link (junction · symlink) under a root (the research does not say — the preview does not follow links) · …"
- §6-3 L1012: "… for codex, the CLI behavior when several `-c` lines have the same key — not measured" (research `codex/mod.rs` comment: measured 0.155.0 "last `-c` wins").
- §6-3 L1017: codex terminal does not receive preset instructions · skills — accepted, notice only, wording facts-only, "final wording is shown to the user at implementation (§2-4 8)".
- §6-3 L1026: codex integer upper bounds (uint16/uint32 without `maximum`) not caught — accepted.
- §6-3 L1041: "codex prunes dot-folders under an extra root and we pass only `.agents/skills` · `.codex/skills` — a skill placed elsewhere … is not loaded by codex JSON | intended".
- §6-3 L1046: known overlaps — "codex JSON gets its `AGENTS.md` from its own project scan and again from our composition — likely … a `folder:` source and a library path never dedup | accepted — loaded twice".
- §2-4 8 L722: "The notice codes and their per-code wording are owned by the steps that produce the notices (6 · 7 — codex terminal omissions · dim lines · the folder line) and are shown to the user there."
- §2-2 T6 L621: "only when the instructions hash changed, on the first turn use the experimental API `additionalContext` to 「바뀌었다」 + the new text (inside backend/codex). … The place of the last-applied hash = the profile field".
- §7 item 5 L1085 = ADR-0337 summary (above).
- PRD §6 row L281: "codex JSON: a skill in a folder entry and a library skill (or skills in two folder entries) with the same name reach codex through separate skill roots — Engram's conflict check does not see folder skills (R57), and whether codex loads both, drops one or fails is unmeasured; it is measured in TRD step 6, which also decides the fallback policy | R57 · ADR-0341 · TRD §6-2"

### PRD R-numbers
R5 (settings asset = per-backend raw field; codex = `-c` list, appended) · R35 (model/effort only via raw field) · R36 (changes take effect next launch incl. resume — T6 reason) · R37 + R37 addendum (codex terminal: 「기본」 priming + `-c` only; instructions/skills omitted → notice; draft wording 「알림: codex 터미널 모드는 지시서·스킬을 싣지 않습니다(의도된 동작). 빠진 것 — 지시서 <이름> · 스킬 <이름>」; final wording to user) · R54 (backend tags) · R57 (folder entries per backend; codex JSON preview marks malformed skills, not blocks) · R58 (notice once per agent) · E4 ② (malformed codex skill blocks codex JSON — acceptance L187) · acceptance L190 (terminal: `-c` line + priming in args, nothing written, notice), L204 (folder override > AGENTS.md), L205 (terminal folder notice once), L209 (preview).

### ADRs named
ADR-0334 (port; decisions 5-6 = no YAML in agent, agent-owned serde type) · ADR-0337 (codex materials: d1 JSON, d2 terminal, d3 notice (amended by 0341), d4 nothing written, d5 `-c` after passthrough · before our overrides · blocked on guard/budget) · ADR-0341 (folder entries; d3 codex JSON, d4 terminal, d5 notice once) · ADR-0215 (codex instructions = `developer_instructions`, backend reads the file) · ADR-0216 (overrides after passthrough, warn on override, never fail the spawn — the fail-open rule the real path must revisit) · ADR-0210 (TOML single-quote literal survives `cmd.exe /c` + `.cmd` shim) · ADR-0128/0209 (MCP `-c` attach) · ADR-0226/0231 (first-turn latch — where the hash would be recorded) · ADR-0331 (notice is not a warning category) · ADR-0004 (backend knowledge isolation) · ADR-0316/0328 (claude counterpart, pattern only).

---

## 2. Code state

### codex backend today
- Files: `backend/codex/mod.rs` (4108) · `protocol.rs` (1419) · `transport.rs` (11945) · `decoder.rs` · `thread_lock.rs` · `usage*.rs`. No `launch.rs`, no `settings.rs`.
- Mode split: `is_app_server(command)` (`mod.rs:90`, StreamJson = app-server).
- `materialize` NOT overridden — trait default (`backend/mod.rs:311`: `LaunchArtifacts { real: kit.plan.real, ..default }`). TODO at `codex/mod.rs:658` ("JSON = `extraRoots` 정션 폴더 · 터미널 = 배치 없음"; build paths only by joining words onto `kit.volatile_dir`, link path = exactly the value passed to `link_dir`).
- `build_spec` (`mod.rs:791`, ignores `_artifacts`): terminal = [`resume <id>`] `--cd <cwd> -s workspace-write -a on-request`; JSON = `app-server --stdio`; then `extra_args`; then MCP `-c mcp_servers.engram={url=…,bearer_token_env_var=…,default_tools_approval_mode='approve'}` (`mcp_attachment` `mod.rs:453`, both modes); then terminal-only priming `-c developer_instructions='…'` from `control.priming_file` (`priming_text` `mod.rs:338` reads the file; `developer_instructions_override` `mod.rs:379`: rejects `' " % \` + control chars except CR/LF, LF→`\n`, budget check, all fail-open warn+drop). Overrides after passthrough with a warn when passthrough set the same key (`passthrough_overrides_key` `mod.rs:494`). Then env injection + `ORIGINATOR_ENV`; `console_command("codex", args)` → `cmd.exe /c codex …`.
- Budget: `CMD_LINE_LIMIT_UTF16 = 8191` (`mod.rs:301`), `CMD_WRAPPER_COST`, `command_line_cost` (UTF-16, +3 per arg, conservative). Used only for the priming override today (MCP attach is not budget-checked).
- JSON: `open_spawn` (`mod.rs:1076`, ignores `_artifacts`) → `CodexAppServerTransport::open(spec, true, decoder, app_server_thread_open(spec, resume, control), sid_sink, link_sink)` → handshake (`transport.rs` ~L1613 `handshake`): `initialize` (no capabilities) → `initialized` → `thread/start` (cwd, approvalPolicy, sandbox, `developerInstructions` = `priming_text(control)`) or `thread/resume` (no developerInstructions — `ThreadResumeParams` doc `protocol.rs` ~L420). Budget `HANDSHAKE_BUDGET = 10 s` (`transport.rs:229`) shared with history hydration. First `turn/start` built in `take_turn_locked` (`transport.rs` ~L1995).
- Terminal: `PtyTransport::open`, thread-id capture via `thread_lock`.
- Goldens/tests: `tests/launch_argv_golden.rs` + `tests/fixtures/launch_argv_golden.json` (Noop argv, all backends — codex included); `codex/mod.rs` tests: `interactive_args_are_the_measured_ones` (L1353), `app_server_args_are_the_measured_ones`, `extra_args_follow_our_base_args`, `a_resuming_spawn_still_carries_our_override_after_the_passthrough`, `app_server_extra_args_come_last`, `the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit` (~L3706 — Noop handshake pinned per start/resume × priming/no priming/no control), `protocol.rs:1158` initialize-without-capabilities pin. `manager/prelaunch.rs:771` also replays the golden. Live: `tests/backend_contract.rs` q1–q12 (terminal PTY probes only — **no app-server turn, no extraRoots**), `tests/usage_probe_smoke.rs` codex_*. A reusable app-server driver exists: `.claude/handoff/attachments/20260925-midturn-phase0/codex_harness.js`.

### LaunchPlan / LaunchKit / LaunchArtifacts (`launch.rs`)
- `LaunchPlan` (L154): `backend · instructions: Vec<SourceRef> · skills: Vec<SkillRef> · raw_settings: Vec<RawSetting{preset, value}> · include_priming · notices · real · folders · extra_skill_roots` (L180, `TODO(TRD S25 §5-2 6)` at L179). Codex raw `value` = JSON array of `"key=value"` strings (TRD §1-2 L119 YAML example `- model=gpt-5.5`).
- `LaunchKit` (L185): `plan · volatile_dir · instructions_file · skills · instructions_hash: Option<String>`.
- `LaunchArtifacts` (L820): `real · args · instructions_file · skill_roots ("inside the volatile folder") · settings_file · add_dirs · env`.
- `Notice = Reason {code, args, line}`; `notice_text` generic wording; `notice_fingerprint`.
- Noop: `NoopLaunchKits` (L506) → `real: false`, empty lists, `instructions_hash: None`; trait-default materialize → default artifacts → byte-identical argv.

### Port as landed (exact signatures — `backend/settings_check.rs`)
```rust
pub trait SettingsCheck: Send + Sync {
    fn backend(&self) -> &'static str;
    fn schema_source(&self, cli_version: &str) -> SchemaSource;
    fn version_probe(&self) -> VersionProbe;
    fn parse_version(&self, stdout: &str) -> Option<String>;
    fn raw_to_json(&self, raw: &Value) -> Result<Value, Reason>;
    fn merge(&self, layers: &[&Value]) -> Option<Value>;
    fn unknown_keys(&self) -> UnknownKeys;               // Ignore | Warn
    fn loads(&self, mode: AgentOutputFormat) -> ModeLoads; // {instructions: All|BuiltinPrimingOnly, skills: bool}
    fn accepts_skill_name(&self, name: &str, mode: AgentOutputFormat) -> bool;
    fn folder_entry_blocks(&self, folder: &Path) -> bool;
    fn folder_load_set(&self, entries: &[FolderEntry], read: &dyn Fn(&Path) -> std::io::Result<String>,
                       parser: &dyn FrontMatterParser, mode: AgentOutputFormat) -> FolderLoadSet;
}
SchemaSource { url, refetch: SchemaRefetch::LatestOnCliVersionChange }   // only variant
VersionProbe { program, args, env }
FolderEntry::{File(PathBuf), UnenteredLink(PathBuf)}
trait FrontMatterParser { fn paths(&self,&str)->Result<PathsFrontMatter,FrontMatterError>;
                          fn subagent(&self,&str)->Result<SubagentFrontMatter,FrontMatterError>; }
FolderLoadSet { loads: Vec<FolderLoad{path, kind}>, unsure: Vec<FolderUnsure{path, kind: Option<_>, why: UnsureWhy}> }
FolderLoadKind::{Instructions, Rule, Skill, Agent, Command}
```
Registry (`backend/mod.rs:762`): `settings_checks() -> [&'static dyn SettingsCheck; 1] { [&claude::CLAUDE_SETTINGS_CHECK] }`, `settings_check_for(word)` ASCII-case-insensitive. No production caller yet (asset-library = step 7).

### claude implementation (pattern)
- `backend/claude/settings.rs`: `pub(crate) static CLAUDE_SETTINGS_CHECK: ClaudeSettingsCheck`; `version_probe` = `console_command("claude", ["--version"])` + `NO_AUTOUPDATE_ENV`; `parse_version` = first line whose first word is dotted; `merge` = `Some(merge(layers))` (`pub(super) fn merge` reused by `launch.rs`); `folder_entry_blocks` = `!path_is_cli_safe`; `folder_load_set` with helper fns + `split_front_matter`; one test `FakeParser` (L896).
- `backend/claude/launch.rs` `pub(super) fn materialize(kit, fs)`: Noop → default; real → validates every argv path with `path_is_cli_safe` **before any write** (D6) → writes settings → `link_dir` per skill (`<vol>\engram\skills\<sanitized name>`, collision via `same_path` → Err) → artifacts. `claude/mod.rs:170` overrides `materialize` → `launch::materialize`. `build_spec` real-only branch places args.

### What step 6 adds (per TRD + code)
- `backend/codex/settings.rs` (new, mirrors claude) + `pub(crate) use` in `codex/mod.rs` + registry slot 2.
- `backend/codex/launch.rs` (new — TRD L305 names it): `materialize` (JSON: junction folder for `kit.skills` + extra roots list; terminal: nothing), `-c` line assembly for preset raw settings, real-path budget/guard.
- `codex/mod.rs`: `materialize` override; `build_spec` real branch (preset `-c` after passthrough before MCP/priming; terminal priming from kit not control); `open_spawn` feeds `thread_open` from artifacts (merged instructions text, extraRoots, resume additionalContext).
- `protocol.rs`: `skills/extraRoots/set` method + params; `TurnStartParams.additional_context`; `InitializeParams.capabilities.experimentalApi` (conditional).
- `transport.rs`: handshake step `skills/extraRoots/set` (before `thread/start`/`thread/resume`; also on resume — research: must be set again per process); first-turn `additionalContext` attach state.
- `settings_check.rs` additive types (§0 item 1). `launch.rs` doc/TODO cleanup (L179, `skill_roots` doc), possibly new kit field for the stored hash.

---

## 3. Decisions binding step 6

### Closed (carry over)
- Preview policy (main 2026-11-09): predict only clear cases, `unsure` otherwise — but **a malformed codex skill is a mark, not `unsure`** (L880).
- Front-matter types live in the neutral port file (user 2026-11-09) → the codex front-matter serde type goes into `backend/settings_check.rs` too (M72 says "type agent defines").
- `folder_entry_blocks` = `false` for codex (it puts no folder path on its command line; extraRoots go in JSON).
- `merge` returns `Option` — codex = `None` is the shape step 5 built for ("a backend without a merged check answers None").
- No YAML in agent (M63 / ADR-0334 d5); the port gets the parser as a closure; repair lives in codex backend; step-6 tests use a fake parser and test only split → parse → repair → reparse order; real golden = step 7.
- Skill-name rule `[a-z0-9-]` 1–64 = error only for codex JSON (M24-3, §2-4 5 closed); conflict key keeps `_`.
- Unknown keys codex = `Warn` (⚠, never blocks).
- Schema = `rust-v<ver>/config-schema.json` per CLI version; port gives address only (M17).
- `-c` order: base → passthrough → preset `-c` lines → our overrides (MCP, priming) (ADR-0337 d5; last `-c` wins measured 0.155.0).
- Guard/budget hit = block ② `Broken` at check, nothing dropped silently (M8); both modes.
- Terminal loads only 「기본」 priming + `-c`; nothing written to working folder or `~/.codex` (P4-FINAL, ADR-0337 d2/d4).
- Model/effort only via raw `-c` lines (R35); never `-m`.
- Folder: codex JSON passes `<folder>/.agents/skills` · `<folder>/.codex/skills` that exist as roots, no junction; `.codex/config.toml` ignored; codex terminal nothing + notice (ADR-0341 d3/d4).
- Notices once per agent (ADR-0341 d5) — plumbing done in step 16.
- Step-5 D6/D7 analogues on claude: cmd-unsafe argv path → Err before writes; failed write → spawn fails (no warn-and-continue). Codex should mirror unless main decides otherwise.
- T6 (2026-10-18): resume → only when hash differs, `additionalContext` on first turn with 「바뀌었다」 + new text, inside backend/codex.
- Hash recorded at the first-turn event (latch spot) via a separate incarnation-guarded registry mutate, not at spawn (TRD L245, L551; ADR-0337 impact).

### OPEN (main must raise or settle before coding)
1. **Stored-hash hand-down** — how codex learns `profile.last_applied_instructions_hash` (new `LaunchKit` field filled by manager · or manager-computed `instructions_changed` flag · or plan field from `check`), and whether the manager-side recording at the first-turn event (`manager.rs:1780` TODO 4(b)) is step 6 or step 8.
2. **experimentalApi opt-in** — send `capabilities.experimentalApi: true` only on spawns that carry `additionalContext` (keeps the Noop handshake pin) vs always for real kits; plus whether codex rejects the field without opt-in (unverified — §4).
3. **additionalContext payload** — key name (opaque source id, e.g. `engram-instructions`), `kind: "application"`, the 「바뀌었다」 preface wording (language), and retry rule if the first `turn/start` carrying it is rejected (carry until a `turn/start` with it is accepted?).
4. **JSON skill layout** — keep the ADR-0337 junction folder (`<vol>\<dir>\<name>` → one extra root) vs pass each library skill folder directly as an extra root (no junction, no link-follow dependency). Depends on the §4 junction measurement; the junction path also needs a dir name and a name-sanitize rule (codex prunes dot-folders; a name starting with `.` would vanish).
5. **Where the merged instructions text is read** — in `materialize` (fallible → SpawnFailed; needs a new `LaunchArtifacts` text field; tests need a temp file since `LaunchFs` has no read) vs in `build_spec`/`open_spawn` (fail-open like today's priming). Real-path failure policy (fail the spawn like D7, or warn).
6. **Terminal priming on the real path** — fail-open drop (today) vs block/fail when the priming `-c` hits a bad char or the budget (M8 vs "current path as is", §0 item 5). Also: is a bad char in builtin priming text a check-time ② (asset-library) or a spawn failure.
7. **Build-time recheck** — real argv overflow at assembly: return `Err` from `materialize` (it has `command` + `cwd`; control/MCP length is the only unknown → upper bound) → `SpawnFailed`, vs a new fallible hook producing `LaunchBlocked`/`Broken` as TRD L279 says. (Today there is no Broken-at-build path.)
8. **`-c` line guard char set** — proposal: `% & ^ | < >` (CMD_UNSAFE) + `"` (portable-pty `\"` vs cmd quote counting — unmeasured) + control chars (LF truncates the cmd line silently) + non-UTF-8; `'` allowed (TOML literal — ADR-0210 measured). Needs a public text-level helper in `platform::shell` or a codex-local set. Blocking `"` rules out `model="o3"` (bare `model=o3` and `'…'` still work) — user-visible consequence.
9. **Port method shapes** — names/signatures for: codex skill-format check (input = file name + text + parser; output = parsed name/description or `Reason`), the codex front-matter type (`name: Option<String>`, `description: Option<String>`, `metadata: Option<{short-description: Option<String>}>` — typed so wrong types fail), the `FrontMatterParser` extension (new required method breaks the one claude fake — add it there, or a default method, or a separate trait), the guard/budget method (inputs for check-time upper bounds: raw lines, mode, extra_args, priming length, cwd upper bound?), and how claude answers the new methods (skill format = not applicable / Ok).
10. **TOML dependency** — add `toml` (which major; 0a survey) to agent vs hand-rolled scalar parsing; and the exact codex `-c` semantics to mirror (split on first `=`, trim, dotted-key split, fallback-literal quote stripping — unverified §4).
11. **Notice codes + wording** — codes for codex-terminal omissions (library instructions · skills · folder instruction file · folder skills) and the per-code Korean wording (facts only, G28) — must be shown to the user (§2-4 8); whether step 6 defines them (in `launch.rs` `notice_text` arms) or step 7 does with the producer.
12. **Same-name fallback policy** (PRD §6 row / ADR-0341 in 6) — after the measurement: accept (both listed; `$name` mention picks none) and document · extend the Engram conflict check to folder skills · emit a notice · or block. Prior evidence (research 2026-10-20 row 5, codex 0.156.1): codex lists every same-name skill, no error, no priority; `$name` with count ≠ 1 does not pick. Likely outcome = accept + document; still a user/main call.
13. **`folder_load_set` result shape for codex** — the `SkillRoot` kind (a root is a folder, not a file — `FolderLoad.path` doc says "as listed"), the malformed mark (field on `FolderLoad` vs separate `malformed: Vec<_>` with a `Reason`), the `omitted` list for terminal (same set as JSON, marked omitted), and whether terminal returns empty `loads` + populated `omitted`.
14. **codex AGENTS pick details** — `AGENTS.override.md` chosen even when empty/blank? case rule on Windows (OS rule like claude fixed names?) — mirror codex's `project_doc` reader (unverified §4).
15. **Split** — P1/P2/P3 as in §5, or JSON/terminal as the TRD suggests; one commit or several.

---

## 4. Unverified premises (need the real codex CLI — installed `codex-cli 0.163.0`; nothing was started here except `--version`, `app-server --help` and `generate-json-schema` into scratchpad)

1. **Same-name skills across extra roots (TRD-assigned to step 6)** — a real JSON turn: `skills/extraRoots/set [rootA, rootB]` with `dup` in both (different marker words), ask the model to use `dup` / `$dup`; also library-junction root vs folder `.agents/skills` root. Prior: 0.156.1 lists both, no error; `$dup` picks none. Write result + policy back to TRD §6-2 L1004 · §6-3 and PRD §6 L281.
2. **Does the extraRoots walk follow a junction under a root** (§6-2 L1004) — decides open item 4. Cheap: `skills/list` after `skills/extraRoots/set` (no model turn).
3. **Does `turn/start.additionalContext` require `experimentalApi`** and does setting `experimentalApi: true` change anything else in our flow (extra notifications/requests we do not answer — note `attestation/generate` is gated by `requestAttestation`, not this). Research 2026-10-10 measured the field "takes effect" on 0.156.1 but does not say whether opt-in was sent.
4. **extraRoots on resume** — research 2026-10-10: must be set again in each new process (works); confirm on 0.163.0 with `thread/resume`.
5. **Is `skills/extraRoots/set` → immediate `thread/start` race-free** (skills scan async; `skills/changed` notification) — does the first turn see the skills without waiting.
6. **`codex app-server --stdio` is a private in-process server on 0.163.0** (research side note: "the 0.163.0 TUI may run on a shared app-server daemon — a process-wide `extraRoots` must never reach a shared daemon"; `proxy`/`daemon` are separate subcommands, so likely private).
7. **codex `-c` parsing** (`raw_to_json` must match): first-`=` split, key/value trim, dotted path split (quoted keys?), TOML parse of value, fallback literal (quote stripping?) — read codex source at `rust-v0.163.0` (config override parser) or probe with `codex debug`/`--strict-config`.
8. **`"` inside a `-c` argument through `cmd.exe /c codex.cmd`** — survives or breaks (guard char set, open item 8).
9. **AGENTS pick** — codex's rule for an empty `AGENTS.override.md`, case on Windows.
10. **`codex --version` stdout** = `codex-cli 0.163.0` (measured now) — parser target; whether it ever prints an update banner on stdout.
11. **Codex skill-format rules on 0.163.0** vs the 0.156.1/0.166.0 parser basis (§1-5 L278) — real golden is step 7.

**QA lane coverage**: the codex lane (`.claude/skill-bindings/qa.md` L139-142: `usage_probe_smoke codex_` + `backend_contract --ignored`) runs on any change under `backend/codex/**` or `backend/mod.rs`, but has **no app-server turn and no extraRoots probe** — items 1–5 are not covered by any existing test. Options: a manual measurement with `codex_harness.js` (record in research + TRD), and/or a new `#[ignore]` codex-lane test (e.g. in `tests/backend_contract.rs` or a new file added to the lane path list) that does `skills/extraRoots/set` + `skills/list` (no tokens) — the model-turn part costs tokens. The §4 L800 real-CLI row otherwise belongs to step 8.

---

## 5. Proposed split (by file overlap)

**P0 — measurement first (no code commit, or a probe test)**: items 1–3 of §4 with `codex_harness.js` on 0.163.0 (junction follow under an extra root via `skills/list`; same-name across roots with one real turn; `additionalContext` with/without `experimentalApi`). Its answers settle open 2/4/12 before P3. Write-back to TRD §6-2/§6-3 + PRD §6 row lands in step 6's doc pass.

**P1 — port codex implementation** (files: `backend/settings_check.rs` additive · `backend/codex/settings.rs` new · `backend/codex/mod.rs` (`pub(crate) use`, make `CODEX_PROGRAM`/budget consts reachable) · `backend/mod.rs` registry `[_; 2]` + tests · `backend/claude/settings.rs` (answers for new methods, fake parser) · maybe `engram-dashboard-platform/src/shell.rs` text guard · maybe agent `Cargo.toml` (toml)).
- Interface: `CODEX_SETTINGS_CHECK`; `SchemaRefetch::<PerCliVersion>`; `raw_to_json` (array of `k=v` → nested object, TOML values, literal fallback, `Err(Reason)` for non-array / non-string / no `=` / empty key); `merge → None`; `unknown_keys → Warn`; `loads` (JSON all/skills · terminal priming-only/no skills); `accepts_skill_name` (JSON `[a-z0-9-]{1,64}`, terminal true); `folder_entry_blocks → false`; skill-format check (split → parse → repair → reparse → name/description check); `folder_load_set` codex (pick, two roots, walk ≤6 · prune dot-dirs below root · `SKILL.md` exact · links unentered · malformed mark · terminal `omitted`); guard + budget fn (shared with P2).
- Acceptance/tests (L793, L799): table tests per method; fake parser order test (fail → repair → once more; success not recalled; still failing → original error); fake listing walk boundaries (depth 6 vs 7, dot-folder under root pruned, root itself dotted allowed, `skill.md` ignored, link not followed); override > AGENTS.md; registry tests with 2 slots; claude tests unchanged.

**P2 — codex translation, shared + terminal** (files: `backend/codex/launch.rs` new · `backend/codex/mod.rs` `materialize` override + `build_spec` real branch · `launch.rs` doc/TODO).
- Interface: `codex::launch::materialize(command/mode, kit, cwd, fs) -> Result<LaunchArtifacts, String>` (Noop → default; terminal → no writes; JSON layout per open 4); preset `-c` lines into `artifacts.args` (or a typed field); `build_spec` real: `-c` lines after passthrough, before MCP; terminal priming from `artifacts.instructions_file` (control priming dropped on real, like claude); real budget check per open 7.
- Acceptance (L792): Noop golden byte-identical (`launch_argv_golden`, handshake pin); terminal argv = base + passthrough + preset `-c` + MCP + priming; no instructions/skills in argv or volatile folder; zero writes outside the volatile folder measured by calling `codex::launch::materialize` directly with the memory `LaunchFs`; `-c model=` only via raw lines; guard/budget → Err.

**P3 — codex JSON channel** (files: `backend/codex/protocol.rs` · `backend/codex/transport.rs` · `backend/codex/mod.rs` `open_spawn`/`thread_open` · `launch.rs` (stored-hash field if chosen) · maybe `manager.rs` hash recording if main assigns it here).
- Interface: `thread_open` gets real instructions text (new conversation `developerInstructions`), extra roots (`kit.skills` layout + `plan.extra_skill_roots`) → handshake `skills/extraRoots/set` before start/resume (only when non-empty, so Noop handshake stays byte-identical); resume + hash differs → first `turn/start` carries `additionalContext` (+ `experimentalApi` per open 2).
- Acceptance: handshake JSON goldens (start/resume × roots/no roots × changed/unchanged hash); Noop handshake pin unchanged; initialize without capabilities when no additionalContext; first-turn-only attach (second turn has none); folder `AGENTS.md` source appears in merged text order (that is asset-library's merge — here only "text from kit is carried untouched").

Order: P0 → P1 → P2 → P3 (P2/P3 both edit `codex/mod.rs` — sequential; P1 independent, P2 calls P1's guard/budget and `-c` line helpers). The TRD's "JSON / terminal" split maps to P2 (terminal + shared) and P3 (JSON). One commit per TRD step (step 5 landed P1·P2 in one commit) or P1 separately — main's call.

## 6. Size estimate
- P0: ~0.5 day of harness work, a few real turns (tokens), no product code.
- P1: ~900–1,300 lines incl. tests (`codex/settings.rs` comparable to `claude/settings.rs` 1,574 lines; the walk is simpler, skill-format split+repair port adds ~250).
- P2: ~400–600 lines incl. tests (`claude/launch.rs` is 684).
- P3: ~500–800 lines incl. tests (protocol types small; transport handshake + first-turn state in an 11.9k-line file is the risky part; hash plumbing +100–200 if the manager recording lands here).
- Doc pass (TRD step-6 landed block, §6-2/§6-3/PRD §6 write-back, tracking, ADR-0337 amendment if experimentalApi/junction choice changes): ~100–200 lines.
