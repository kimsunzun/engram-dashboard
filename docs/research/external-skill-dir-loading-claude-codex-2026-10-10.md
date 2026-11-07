# Can an outside skill folder be loaded as native skills in claude · codex (2026-10-10)

- **Status:** survey done · before the decision (skill preset idea stage — before the PRD)
- **Method:** `/research` medium · 2 tracks (claude · codex — 1 primary-family collector each, official docs + measured on the installed build) · main-session grounding (measurement records · comparison with the generated schema) · 1 cross-family adversarial review (codex, effort high · level 2~3 — verdict FIX with 6 items, applied below)
- **Target versions:** Claude Code 2.1.284 · codex-cli 0.156.1 (source comparison = openai/codex main `d3f9b22`, 2026-10-09) · Windows 11
- **Confidence legend:** certain = measured + cross-checked with docs/source · likely = supported by one source, no counter-evidence · uncertain = not checked

## Conclusion

**claude can attach an outside folder as native skills in both modes, and codex only in JSON mode, per process (or server).** codex terminal mode has no way to add a skill folder through a launch argument · `-c` · environment variable, and what remains is file system placement (global · working folder) or a local plugin install (which changes global settings).

| | Path | Unit | Confidence |
|---|---|---|---|
| claude | `--plugin-dir <dir>` → `<dir>/skills/<이름>/SKILL.md` (the name gets `<폴더>:<스킬>` prefixed · no manifest needed) | process | certain |
| claude | `--add-dir <dir>` → `<dir>/.claude/skills/<이름>/SKILL.md` (no name prefix · also grants read permission on that folder) | process | certain |
| codex JSON | app-server method `skills/extraRoots/set {extraRoots:[절대경로]}` | the whole app-server process (not per thread) | certain (schema · `skills/list` measured) · loading in an actual turn is likely |
| codex terminal | `~/.agents/skills` · `$CODEX_HOME/skills` · `.agents/skills` from the working folder up to the project root (directory junctions · symbolic links are followed) | global or working folder | certain |
| codex terminal | local plugin: `codex plugin marketplace add <로컬경로>` → `codex plugin add <PLUGIN@MARKETPLACE>` | global install (changes the settings file) | likely (only the existence of the commands was confirmed · loading not measured) |

**The SKILL.md format is the same on both sides** (frontmatter `name` · `description` + body · only the list goes into the system prompt and the body is read when used). On this PC `~/.agents/skills` is already junctioned to `I:\Engram\core\claude-global-shared\skills` (2026-07-21), so the shared skills are already loaded in codex globally (certain — `dir` measured).

## Findings

### claude

1. **`--plugin-dir` · `--add-dir` load outside skills natively** — certain. Measured: made three fake skills and ran `claude -p … --output-format stream-json --plugin-dir plug --plugin-dir nomanifest --add-dir adddir` → the init event `skills` carried `zebra-adddir` · `zprobe:zebra-plug` · `nomanifest:zebra-nomani`, and when told to use the skill, it returned the body's secret word through a Skill tool call. Sources: https://code.claude.com/docs/en/skills · https://code.claude.com/docs/en/cli-reference
   - **Conditions (review applied):** `--add-dir` skill discovery needs the settings source `project` to be on (on by default). With `--safe-mode` · `strictPluginOnlyCustomization` they are not loaded. `--bare` turns off automatic discovery, but `--add-dir` skills are still loaded (docs).
2. **There is no settings key · environment variable that adds a skill folder** — likely (docs summary + no `--help` entry · a full grep of the settings reference was not done). `permissions.additionalDirectories` gives file access only. Indirect paths are a symbolic link inside the personal skill folder and `CLAUDE_CONFIG_DIR` (moves the whole settings home — authentication · sessions come along, so it is unsuitable as a preset switch · not measured).
3. **Reading files outside the working folder is denied by default** — certain (measured `permission_denials:[Read …]`). It is lifted by `--add-dir` or `--settings '{"permissions":{"additionalDirectories":[…]}}'` (measured). **A plugin skill body is loaded without read permission** — but if SKILL.md makes it read neighboring files · scripts, permission on that folder is needed separately.
4. **Resume (`--resume`)** — the skill list is rebuilt per process (certain — when a session made with plugin A was resumed with only B given, only B was in the init list). **However, the system prompt (including `--append-system-prompt-file`) uses the copy taken at the first request even after resume** — certain (CLI reference primary text · it changes when compacted or on a new conversation · turned off with `--system-prompt-snapshot off`). **Review applied:** even if the list changes, an already invoked skill body stays in the conversation context and compaction passes it along within budget (skills docs). Whether CLAUDE.md is inside that copy is uncertain.
5. **Alternative injection paths** — `--append-system-prompt-file` (any absolute path · the path in use now) · CLAUDE.md `@경로` import (absolute paths allowed · when project memory points outside, a first-time approval dialog — if not approved, it silently does not load) · `.claude/rules/*.md` (scope can be limited with `paths:`) · `~/.claude/rules` and the user CLAUDE.md import from outside without approval. Source: https://code.claude.com/docs/en/memory
6. **Not checked (review applied):** all measurements were `-p` — the interaction between the interactive mode's working-folder trust dialog and `--add-dir` skills was not measured (per the permissions docs, `-p` skips the trust dialog).

### codex

7. **SKILL.md skills are an official feature (no feature flag needed)** — certain. Discovery locations: `$CODEX_HOME/skills` (for compatibility · deprecated) · `<OS 홈>/.agents/skills` · the system folder · project `.codex/skills` · `.agents/skills` at each level from the project root (`.git`) to the working folder · plugin skills · `extra_skill_roots`. Source `codex-rs/ext/skills/src/host_roots.rs:58-185`.
8. **There is no CLI argument · `-c` key · environment variable that adds a skill folder** — certain (source). The `[skills]` settings (`bundled.enabled` · `include_instructions` · `max_context_tokens` · `config=[{path|name, enabled}]`) only turn already discovered skills on and off. **Those on/off rules are read only from the user layer and the session flag layer** (`config/src/skills_config.rs:158-180` — `[skills.config]` in project settings has no effect · `-c` does). In terminal mode the value needs TOML quotes, which trips our command-line character guard (inference).
9. **app-server `skills/extraRoots/set`** — certain (the 0.156.1 generated schema `v2/SkillsExtraRootsSetParams.json` exists · measured: after setting it, `skills/list` from a bare working folder returned the outside skill and a `skills/changed` notification arrived). It is global to the running process and is reflected in the next turn's developer message list (upstream test `app-server/tests/suite/v2/host_skills.rs` · not measured with an actual model turn).
10. **app-server `thread/resume` also accepts `developerInstructions` · `config`** — certain (the fields exist in the 0.156.1 generated schema `v2/ThreadResumeParams.json`). ★Our code (the `ThreadResumeParams` comment in `backend/codex/protocol.rs`) says 「그 칸의 존재를 재 보지 않아 비워 둔다 — 실측으로 확인되면 더한다」 — **that measurement is this one.** But for a thread that is already loaded and running, resume's overwrite is ignored and only the mismatch is reported (`thread_processor.rs:217-229`). ★**Correction (measured the same day): even when resuming in a new process, the new value is discarded without an error**★ — a field being in the schema is different from it taking effect (see "Additional measurement — codex preset injection, actual turns" below).
11. **`CODEX_HOME`** moves settings · skills · the global AGENTS.md · sessions · authentication all together — changing it per agent breaks resume lookup and authentication (source certain · side effects likely). `~/.agents/skills` follows the OS home and does not move with it.
12. **AGENTS.md alternative path** — per layer `AGENTS.override.md` > `AGENTS.md` > `project_doc_fallback_filenames` (names containing path separators · dots are rejected — absolute paths impossible) · project root to working folder · cap 32 KiB · no import syntax. There is no way to point at an outside file by absolute path (certain — `core/src/agents_md.rs`).
13. **The sandbox `workspace-write` grants read of the whole disk** (only writes are restricted) — likely (source `protocol/src/permissions.rs:819-870` · Windows execution not measured).
14. **Resume** — the skill list is rebuilt every turn from the current disk · settings (likely · source). developer_instructions is not persisted in the resume metadata and comes from the resume arguments · settings.
15. **Local plugin path (an omission the review caught)** — `codex plugin marketplace add` accepts a local path and `codex plugin add` installs it (confirmed in the installed build's `--help`). A plugin skill folder falls under the discovery locations (source `host_roots.rs:58-62`). Installing changes global settings, so it is not a per-agent switch. Whether `-c plugins."<id>".enabled=true` can be used per run is not checked, and in terminal mode the double quotes trip the guard (inference).

### Additional measurement — the system prompt copy on claude resume (2026-10-10)

The docs basis of finding 4 was confirmed by measurement (haiku · `-p` · resume with the same session id — `--resume` in `-p` does not fork a new session). **Certain.**

| Run | Arguments | Answer (secret word) |
|---|---|---|
| 1 | new conversation + a.md (ALPHA-7) | — |
| 2 | resume + b.md (BRAVO-9) | **ALPHA-7** — new file ignored, first-request copy used |
| 3 | resume + b.md + `--system-prompt-snapshot off` | **BRAVO-9** — new file reflected |
| 4 | new conversation + b.md (control) | BRAVO-9 |
| 5 | resume, no file argument | ALPHA-7 — the off run did not overwrite the copy (the conversation record holds both answers, so on its own this is weak evidence) |

`claude --help` (2.1.284) primary text: `--system-prompt-snapshot <on|off>` — by default on, it records the first request's prompt (including `--append-system-prompt`) and sends it unchanged on every later request · resume (even if given different text) and keeps it until compaction; off rebuilds it on every request (it says 「프롬프트 글을 다듬는 용도」). Not checked: the effect of off on prompt cache hits.

### Additional measurement — codex preset injection, actual turns (2026-10-10)

codex-cli 0.156.1 · model gpt-6-sol · effort low · about 15 actual turns. Terminal mode was measured instead with `codex exec`, which accepts the same `-c` (interactive TUI not measured). **Certain (this build · confirmed with control runs).**

| Mode | Part | New conversation | Resume |
|---|---|---|---|
| JSON | instructions (`developerInstructions`) | **works** | **does not work** — the new value is discarded without an error · warning (control: when a thread started without instructions was resumed with a new value, 「NONE」 · the new value appears 0 times in the record). When resumed without instructions, the first instructions stay alive in the record unchanged |
| JSON | native skills (`skills/extraRoots/set` → `thread/start`) | **works** — the model read SKILL.md and said ZULU-42 | **works — must be set again in the new process**. The skill list is rebuilt every turn against the current process and a new block is appended (if not set, it drops out of the list; if set, it comes back) |
| terminal (`codex exec`) | instructions (`-c developer_instructions=…`) | **works** | **does not work** — `codex exec resume` silently discards `-c developer_instructions` (control 「NONE」) |
| terminal | native skills | **no path** — `--help` · the settings schema · binary strings all have no key · argument · environment variable that adds a skill folder. `-c skills.config=[{path=…}]` only turns already discovered skills on and off (measured NOSKILL). Only a working-folder `.agents/skills` junction works (ZULU-42 · it touches the working folder) | not measured |

- Branches not measured: whether `config` (a free-form object) and `baseInstructions` of `thread/resume` take effect on resume. Interactive TUI `codex resume`.

### Additional measurement · web — ways to change instructions on codex resume (2026-10-10)

Control design: a thread started without instructions → resumed in a new process passing 「preset word X」 as the candidate → 「없으면 NONE」. Once each · the basis of the verdict is the answer + whether X was written in the rollout (`~/.codex/sessions`).

| Candidate | Mode | Result |
|---|---|---|
| `thread/resume.developerInstructions` · `config.developer_instructions` | JSON | **ignored** (no error · warning) |
| `codex exec resume -c developer_instructions=…` | terminal | **ignored** |
| `collaborationMode.settings.developer_instructions` (default mode · both `turn/start` and `thread/settings/update` · experimental API) | JSON | **ignored** — it is written in the settings but does not enter the model input |
| the same, plan mode | JSON | takes effect — but the agent switches to plan mode and cannot be used for ordinary work |
| `thread/resume.baseInstructions` | JSON | takes effect — but it **replaces the whole** codex default instructions and is not persisted (must be resent on every resume) · unsuitable as a preset channel |
| **`turn/start.additionalContext` `{키:{value, kind:"application"}}` (experimental API)** | JSON | **takes effect** — written once in the record as a developer-role message and remains after resume. `kind:"untrusted"` enters as a user-role quotation |
| **`thread/inject_items` developer message (experimental API)** | JSON | **takes effect** — same as above and not tied to a turn |
| **Changing · creating the working folder's AGENTS.md** | JSON · terminal | **takes effect** — it is read again on resume (when the thread is loaded) and a message 「이전 AGENTS.md 지시를 모두 대체한다」 is appended. It is not read again between turns within the same process |
| `project_doc_fallback_filenames=["ENGRAM.md"]` (resume `config` · terminal `-c`) | JSON · terminal | takes effect — **but only when the working folder has no AGENTS.md** (if it does, only AGENTS.md is loaded · `AGENTS.override.md` hides AGENTS.md) |

- **Only appending works** — `additionalContext` · `inject_items` cannot erase an old preset. What changed must be appended with an explicit 「바뀌었다」 (when done that way, the new one was followed · n=1), and that the new one wins on conflict is not guaranteed.
- **Web (primary text · issue state checked with `gh api`):** openai/codex #19045 (open) — ignoring `developerInstructions` on resume · fork is a **known bug** (ignoring on an already loaded thread is intended — the mismatch string in `thread_processor.rs`). #43295 · #43309 (open) — when AGENTS.md changes, the whole bundle is appended and the context can overflow · a request for a 「얼리기」 policy. #16607 (open) — a report that a resumed thread keeps the old skill list, but **in our 0.156.1 measurement (resetting `extraRoots`) a new skill block was appended.** The latest release is rust-v0.162.1 (installed 0.156.1).
- **Web (t3code source comments · not checked):** codex discards client-inserted developer messages on compaction — t3code puts them back with `thread/inject_items` after compaction. paseo · t3code carry the `collaborationMode` instructions on every `turn/start` (per the measurement above, they did not take effect in default mode — a source comment says a model catalog override may hide them).
- Not measured: interactive TUI · whether `additionalContext` survives compaction · conflicts without a 「바뀌었다」 phrase.

### Additional measurement — the full lifetime of a codex JSON preset (2026-10-10)

codex-cli 0.156.1 · gpt-6-sol · effort low · a new app-server process at each step · once each. Conclusion = **the preset (instructions + native skills) can be changed while resuming — but three more handlings are needed.** Likely (n=1).

| Step | Result |
|---|---|
| Start: `extraRoots`=A + `developerInstructions`=A | instructions A · skill alpha worked |
| Resume: `extraRoots`=B + `additionalContext` B | instructions B applied · the skill list was appended anew, alpha dropped out and bravo came in. **But what was read earlier through alpha stays in the conversation record and the model remembers it** |
| Resume, nothing sent | B kept (record) |
| Resend the same key · same value | written only once within a process (key+value deduplication, kept even after compaction). The first turn of a new process writes it again |
| **Compaction (`thread/compact/start`)** | **all developer messages drop out — preset B and even the `developerInstructions` A from the start.** B survived in the answer only through the encrypted summary and A disappeared. Resending the same value is not written because of deduplication, and changing the value (`[rev 2]`) or putting it back with `thread/inject_items` does get written |
| Conflict without a 「바뀌었다」 phrase (start CHERRY → resume DATE) | followed DATE. When asked, it answers that it sees both |

- **Cache:** a resume with no change gets about 99% cache hits. A preset swap reads about 6.6k tokens uncached at once — most of it is the share of resending the whole skill list (about 21k characters) and the preset text itself is small. The skill list is reloaded merely when the remote plugin cache changes, so the same cost arises without a swap. Each reload grows the context by that much (until the next compaction).
- ★**A side finding about existing behavior:**★ it means our current codex JSON priming (ADR-0215 · `developerInstructions`) also drops out after compaction — measured in a resumed process, and compaction inside the process that opened the thread was not measured.
- Not measured: whether `developerInstructions` is restored after compaction within the same process · re-injection that conflicts with the compaction summary.

### Peers — how orca · herdr load instructions · skills (source read 2026-10-10)

orca = upstream main `6cbbe456` (2026-10-10 · the local clone `df7460af` was stale so a fresh one was fetched) · herdr = local `v0.9.1` (`065ef9d`) + partial comparison with master `2563803d`. **Neither puts a system prompt · developer instructions into a terminal agent — they are not precedents for this problem.** Certain (full grep · key lines cross-checked by main).

- **orca instructions:** it **types the task instructions (the 「You are a dispatched worker」 preamble) into the terminal as a user turn** (`src/main/runtime/orchestration/preamble.ts:87` · sending `coordinator-task-dispatch.ts:140-146`). The reason the wording is deliberately mild = claude suspects pasted text of being a prompt injection (STA-8200 comment). `--append-system-prompt` kinds appear only in the user-argument parsing table. The structured (SDK · app-server) path is separate and it does not append instructions there either.
- **orca skills:** `orca skills install` (→ `npx skills add`) installs into the native global · project skill folders — not attached on every run. SKILL.md is a thin shell and the body is produced by the CLI (`orca skills get …`).
- **orca resume · compaction:** resume is just the original command + resume arguments (`src/shared/tui-agent-resume-startup.ts:78` `followupPrompt: null`) — nothing is re-inserted. Compaction is used only for the screen status display (PreCompact is deliberately not registered — `src/shared/claude-compact-completion.ts:3-6`).
- **herdr:** zero instruction injection · only status-report hooks installed · skills are installed globally once by the user (`npx skills add … -g`) or through `herdr --skill` output · resume is bare arguments · compaction is only a status display.
- **Implication (inference · not measured):** codex compaction kept user messages and discarded developer messages (`replacement_history` in the lifetime measurement above). Inserting instructions as a **user turn** like orca may let them survive codex compaction — instead they show on screen as a user bubble, and claude may suspect pasted instructions (orca comment).

### Additional measurement — resume and the prompt cache (2026-10-10)

haiku · `-p` · appended md ≈ 16.7k tokens · first request total ≈ 31.7k · once per case (no repetition). Figures = per-request `usage` (`cache_creation_input_tokens` / `cache_read_input_tokens`). **Certain (this build · n=1).**

| Case | Resume result |
|---|---|
| default (on) | hit — 31,695 read · new write is the new turn's share ≈ 80 |
| off · md unchanged | hit — same figures as on (the first request bytes are the same as on) |
| off · md swapped midway | only that one swap rewrites from the md onward (read 15,046 · write 16,839) → the next resume hits fully again |
| off · git commit in between | hit — the git state in the environment info stayed at the first conversation's value even with off (the new commit was not visible) |
| no md (control) | hit |

- Conclusion: always attaching `--system-prompt-snapshot off` costs no cache if the md is unchanged, and only a resume with a changed md rewrites what follows once.
- Limit: the cache carries across runs, so the first-request figures are affected by run order. The case where the md changes while on was not measured. The position of the cache breakpoint was estimated from the usage figures.

### Additional measurement — CLAUDE.md is read again on resume (2026-10-10)

Same method (haiku · `-p` · an empty working folder with git init). **Certain (this build).**

| Run | CLAUDE.md at the time | Arguments | Answer |
|---|---|---|---|
| 1 | LIMA-3 | new conversation | — |
| 2 | edited to MIKE-4 | resume | **MIKE-4** |
| 3 | MIKE-4 | new conversation (control) | MIKE-4 |

- **It is loaded in a different place.** `--append-system-prompt-file` is stored in the session record (`~/.claude/projects/<폴더>/<세션>.jsonl`) as `attachment.type = "prompt_snapshot"` with the full system prompt, and resume sends that again. CLAUDE.md is not the system prompt but is carried in the conversation thread as an `attachment.type = "instructions"` attachment, and on resume the file is read again and **only the changed file** is appended as a new attachment (`changed: true` · `reason: "session_start"`).
- **The old copy is not erased** — the LIMA-3 attachment stays in the conversation record so both are in context, and the model picked the later one. When instructions conflict, newest-first is not guaranteed (n=1).
- Not checked: whether CLAUDE.md in an `--add-dir` folder (`CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`) rides the same attachment · the same resume refresh.

## Adversarial review result (codex · FIX → applied)

| Target | Defect | Handling |
|---|---|---|
| codex terminal 「길은 파일 배치뿐」 (the only path is file placement) | local plugin path omitted (high) | added to finding 15 · the conclusion table |
| claude `--add-dir` | conditions omitted (`project` source · safe-mode) | finding 1 conditions |
| claude resume | an invoked skill body stays in context | finding 4 |
| claude no settings key | indirect paths (symbolic link · `CLAUDE_CONFIG_DIR`) omitted | finding 2 |
| codex `skills.config` | the restriction on which layers are read omitted | finding 8 |
| claude trust dialog | `-p` measurements alone cannot speak for interactive mode | finding 6 |

No load-bearing claim was overturned by a rebuttal (with evidence).

## Gaps

- codex `skills/extraRoots/set` was not measured with an actual model turn (`skills/list` + upstream tests).
- codex `-c skills.config` off filter · enabling plugins per run · file symbolic link SKILL.md not measured.
- codex Windows sandbox execution of reading outside not measured.
- claude interactive mode (trust dialog) · `CLAUDE_CONFIG_DIR` not measured. Whether CLAUDE.md is inside the copy on claude resume is uncertain.
- The official codex skills doc (developers.openai.com/codex/skills) was not read — source-based.

Measurement records (outside this repository · session scratch, so they do not persist): 7 claude fake-skill measurements (haiku) · the codex app-server JSON-RPC measurement script · the output of `codex app-server generate-json-schema`.
