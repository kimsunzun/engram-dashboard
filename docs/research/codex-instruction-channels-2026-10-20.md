# Every codex instruction channel and when it is read — measured: does 「띄울 때 만들고 띄운 뒤 지우기」 (create at launch, delete after launch) work? (2026-10-20)

- **Status:** investigation complete · decision pending (input for the option of putting preset instructions and skills in temporary files)
- **Method:** fake model server + sentinels (「방법」 below — continues the method of the earlier report, and the reproduction essentials are kept here) · source cross-check (openai/codex tag `rust-v0.156.1` = `b412ff32`, 2026-09-22) · 13 codex runs (version · `--help` 5 · `codex exec` / `exec resume` 7 · `codex app-server` 1) · 0 real model calls
- **Target version:** codex-cli **0.156.1** (npm · Windows 11)
- **Questions:** (1) Besides `AGENTS.md` / `AGENTS.override.md`, is there an instruction channel that terminal mode follows — in particular one whose file is **outside** the working folder or is per process? (2) When is `AGENTS.override.md` read (at start · every turn · after compaction · on resume)? (3) Is the skill body read when it is used, and does terminal mode have a per-process skill folder?
- **Earlier reports (what is not re-measured here is canonical there):** [`codex-instruction-injection-2026-09-20.md`](codex-instruction-injection-2026-09-20.md) (channels · limits · the cmd command-line wall) · [`external-skill-dir-loading-claude-codex-2026-10-10.md`](external-skill-dir-loading-claude-codex-2026-10-10.md) (where skills are discovered · `developer_instructions` being ignored on resume)
- **Confidence legend:** certain = measured (or measured + cross-checked against source) · likely = source only · uncertain = not tested

## Conclusion

1. **There are three channels that are outside the working folder and per process** (all measured on the terminal path `codex exec`):
   - **Profile v2 file `-p <이름>`** → `$CODEX_HOME/<이름>.config.toml` is layered on as one config layer. It can hold `developer_instructions` (multiple lines · no length limit — it does not hit the cmd command-line wall) · `model_instructions_file` · hooks. ★**The location is fixed at `$CODEX_HOME` (the real `~/.codex`), not the working folder**★ — the name accepts only `[A-Za-z0-9_-]` and cannot take a path.
   - **`model_instructions_file` (any absolute path)** → **replaces** the system prompt (`instructions`) **as a whole**. It reads the content when it reads the config and **copies it aside into the session metadata at thread start** — even if the file is deleted, the content from start survives resume and compaction.
   - **SessionStart hook** → whatever file the hook command prints is carried as a developer message. **It runs again at every start · resume · compaction** (with the file content at that time). Cost = hook trust (`--dangerously-bypass-hook-trust` or stored trust) · beyond 2,500 tokens it is truncated and the original goes out to `%TEMP%\hook_outputs\…` (with `additionalContextLimit = 0` it is carried whole — measured with 58 KB).
2. **`AGENTS.override.md` (working folder) is read once when the thread is opened, and that process remembers it.** It is not re-read every turn (edits and deletions are ignored), and **after compaction the remembered content is injected again** (even if the file has already changed). The moments when the disk is re-read are **resume (new process)** and — per source — a new thread in the same process (`/new` · `/clear` and so on) or when the working folder or trust level changes. ★**If the file is gone on resume, 「앞서 준 AGENTS.md 지시는 더 이상 적용되지 않는다」 (the AGENTS.md instructions given earlier no longer apply) is appended for the model**★ — what was deleted is not silently ignored but **explicitly withdrawn.**
3. **The global `$CODEX_HOME/AGENTS.md`, by contrast, is re-read on every request** — if it changed, a replacement block is attached on the spot, and if it was deleted, a removal block is. It is a file that every codex session of the user sees together, so it is not a per-agent channel.
4. **Skills:** only the list (name · description · path) is carried and **the body is read from disk when used** (0 body sentinels in the start request). The list is **rebuilt on every request** — delete the junction and on the next request that skill drops out of the list, and a `$이름` mention is silently discarded. **Terminal mode has no per-process skill folder in this version either** (0 config keys · arguments · environment variables — the conclusion of the earlier report stands).
5. **Decision on 「띄울 때 만들고 띄운 뒤 지우기」 (create at launch, delete after launch)**
   - **Instructions (`AGENTS.override.md`) = works conditionally.** Two conditions — (1) delete **after thread start has finished** (the TUI opens the thread asynchronously after launching · we found no signal that tells from outside when that moment is) (2) **recreate with the same content before every resume (and new thread)** — if the content is the same, no further block is attached (measured). If (1) is not kept, that agent starts without the preset; if (2) is not kept, a withdrawal text is attached at the moment of resume.
   - **Skills (junction) = does not work.** They must exist for the whole session — from the request after the deletion they drop out of the list.
   - **The route that does not touch the working folder at all = profile v2 + `developer_instructions`.** But ★**the same profile must also be given on resume for it to survive compaction**★ — if it is not given, compaction erases the preset (measured R5).

## Channel table — 0.156.1 terminal mode

`작업 폴더 밖` = can the **file** of that channel be placed outside the working folder. 「이어받기」 = `resume` with a new process. 「압축 뒤」 = the next request after compacting within the same process (local compaction measured · remote compaction per source — 「한계」).

| Channel | File location (outside the working folder?) | Append / replace | At start | Every turn | Resume (이어받기) | After compaction (압축 뒤) | Basis | Confidence |
|---|---|---|---|---|---|---|---|---|
| `AGENTS.override.md` > `AGENTS.md` (each layer from the project root to the working folder) | **X** — working folder to project root | Append (one block with user · global AGENTS.md) | Read when the thread is opened, then remembered | **Not read** | Read again — changed = replacement block · gone = **withdrawal block** · same = no block | Re-injects the **remembered content** (regardless of disk) | Measured R1~R3 · R6~R7 · A | certain |
| `project_doc_fallback_filenames` | X — name only (paths rejected) · only when there is no AGENTS.md | Append | Same as above | Same as above | Same as above | Same as above | Earlier report measured · source | certain |
| Global `$CODEX_HOME/AGENTS.override.md` > `AGENTS.md` | **O** — but `$CODEX_HOME` = shared by the whole user | Append (the front part of the block above) | Read | **Re-read on every request** — changed = replacement · deleted = removal block | Re-read | Current disk content | Measured R1~R3 · A t2·t3 + upstream test | certain |
| `developer_instructions` (`-c`) | No file (a value) — hits the cmd wall (8,191 characters · cut at newline) | Append (developer · before codex's own block) | Carried | Fixed (once) | ★A new value is not carried in that place (the old value stays in the record)★ | ★Re-injects **the current process's config value**★ — if it was not given on resume it disappears | Measured R2 · R5 · A t3 + earlier report | certain |
| **Profile v2 `-p <이름>`** field `developer_instructions` | **O** — `$CODEX_HOME/<이름>.config.toml` (path cannot be specified) | Same as above | Carried (multi-line measured) | Same as above | Same as above — a profile value is also discarded in that place, just like `-c` | Same as above (the **new** value given on resume appeared after compaction) | Measured R1 · R2 | certain |
| `model_instructions_file` | **O** — any absolute path | **Replace** (field `instructions`) | Content read when the config is read · copy in the session metadata | Fixed | If the key is given, that file (current content) · ★**if not given, the copy from start** (regardless of file deletion or change)★ | Different field, so independent of compaction | Measured R1 · R2 · R3 · R5 · rollout `session_meta` | certain |
| `instructions` (config key) | Value | Replace | — | — | (source: same priority as above) | — | Earlier report · source | likely |
| **SessionStart hook** (`[[hooks.SessionStart]]` — profile · `-c` · `config.toml`) | **O** — the file the command reads can be anywhere | Append (developer · `hooks.additional_context`) | Runs right before the first turn | Does not run | Runs again with `source=resume` (file content at that time) | Runs again with `source=compact` | Measured R1 · R2 · R4 | certain |
| UserPromptSubmit hook | O (same as above) | Append (developer) | — | Runs **on every user turn** → piles up again each time | — | — | Source (context injection path of `hook_runtime.rs`) | likely · not measured |
| Legacy `profile = "x"` + `[profiles.x]` | — | — | ★**There is no `developer_instructions` field**★ — only `model_instructions_file` | | | | Source `config/src/profile_toml.rs` | certain (source) |
| `~/.codex/rules/*.rules` | — | **Not instructions** — execpolicy command approval rules (`codex exec --ignore-rules` help) | | | | | `--help` · `docs/execpolicy.md` | certain |
| `compact_prompt` · `experimental_compact_prompt_file` | — | Not instructions — prompts that produce the compaction summary | | | | | Source | likely |
| managed developer instructions | — | requirements (administrator) layer only · the user cannot set it | | | | | Source `world_state/managed_developer_instructions.rs` | likely |
| 「thread instructions」 provider | — | Internal field of the extension API · not exposed in this version's CLI · config · app-server arguments | | | | | Source `agents_md_manager.rs` | likely |

- **The route of changing `CODEX_HOME` per agent** was already ruled out by the earlier report (authentication and resume lookup move along with it).
- **Trust:** if the working folder is **explicitly marked `untrusted`**, the project AGENTS.md is not read at all (`agents_md.rs:62` · if trust is unspecified it is read — `config_toml.rs:578`).
- ★**The content of a deleted temporary file also remains on disk**★ — the rollout (`$CODEX_HOME/sessions/…jsonl`) writes a copy of the system prompt into `session_meta` and the full AGENTS.md text into the `world_state` entry (measured in the R1~R3 rollouts). What 「지우기」 relieves is **working folder pollution** only.

## `AGENTS.override.md` When it is read — measured

**R = `codex exec` / `exec resume` (terminal path) · A = `codex app-server` one process (multiple turns · compaction in the same process).** A is the same path as the TUI — if the TUI has any one of `-c` · `-p` · `--dangerously-bypass-hook-trust` · `--strict-config`, it does not use the shared daemon but **starts an app-server inside its own process** and opens the thread (`tui/src/daemon_startup.rs:17-44` · the daemon auto-start feature is still experimental · off by default).

| Run | Disk at that time | What was carried in the request |
|---|---|---|
| R1 new conversation | override = A · global = A | One block: global A + override A |
| R2 resume (new process) | override → **B** · global → B | Old block (A) as is + **「이 AGENTS.md 지시가 앞서 준 것을 모두 대체한다」 + B block** |
| R2 after compaction within the same turn | — | Both old blocks disappear and **one B block** is injected again (what this process read) |
| R3 resume | override · global **deleted** | **「앞서 준 AGENTS.md 지시는 더 이상 적용되지 않는다」** block |
| R5 resume + compaction | deletion unchanged | AGENTS blocks **0** after compaction (the withdrawal text also disappears) |
| R6 resume | override = C newly created | C block (no replacement wording, since there were no instructions before) |
| R7 resume | **deleted, then recreated with the same content** | ★**0 new blocks**★ — if the text equals the recorded copy, nothing is attached (regardless of file time) |
| A right after thread/start **delete override** → t1 | gone | ★**override A2 was carried in t1**★ — it was read when the thread was opened |
| A t2 | override = recreated as B2 · global → B2 | Replacement block = **global B2 + override A2** (only global is newly read, project is remembered) |
| A compaction (`thread/compact/start`) → t3 | override B2 · global B2 | Re-injected block = **global B2 + override A2** — remembered A2 even though the disk is B2 |

- **Source:** project documents are read at thread start (`core/src/session/session.rs:1436-1443`), and the refresh at each request boundary (`core/src/session/mod.rs:3754`) **uses the remembered ones if the environment selection (working folder etc.) and trust level are the same** (`refresh_repository` of `core/src/agents_md_manager.rs`). The global file provider reads the disk every time it is called (`codex-rs/codex-home/src/instructions/mod.rs`). The replacement and withdrawal wording and 「같으면 무블록」 = `render_diff` of `core/src/context/world_state/agents_md.rs`. The upstream test `core/tests/suite/agents_md_refresh.rs` (`live_global_removal_preserves_repository_instructions`) pins the same behavior.
- **Origin of re-injection after compaction:** compaction erases the reference point (the previous turn's context), and the next request rebuilds the first context in full **from the current process's state** (`core/src/session/mod.rs:4490-4530` · remote compaction uses the same function — `core/src/compact_remote_v2.rs:323-333`). So the developer instructions and AGENTS blocks become **「지금 프로세스가 아는 것」** (what the current process knows) rather than 「기록에 있던 것」 (what was in the record) — in R2 the resume-time value (B) that had been discarded in that place appeared after compaction, and in R5 a resume that was given nothing ended up with 0 instructions after compaction. The earlier report's 「압축이 developer 메시지를 전부 지운다」 (JSON · resumed process) is explained by this rule — that process was not given `developerInstructions`.
- **The pitfall in when to delete (source):** the TUI opens the thread **asynchronously** after launching (`tui/src/app/startup.rs:16` `spawn_startup_thread_start`). And when creating the session, **reserving the rollout location comes before reading AGENTS.md** (`session.rs:1156` → `1436`) — 「롤아웃 파일이 생겼다」 is not evidence of 「읽었다」. If the first turn has started, the thread is already open (a turn comes only after a thread).
- **New thread (source):** a new thread in the same process, such as `/new` · `/clear`, creates a new session and re-reads the disk (start source `Startup`/`Clear` of `session.rs:1860-1868`) — if the file is gone then, it starts without the preset. Not measured.

## Channels outside the working folder — measured

R1 turned on all channels at once (profile `eng1` = two lines of `developer_instructions` + `model_instructions_file` (outside absolute path) + SessionStart hook (`type <바깥>\hook.md`) · global AGENTS.md · working folder override · junction skill). Giving `--strict-config` together confirmed that the profile's three keys are the official keys of this version.

| Run | `instructions` | developer (bundle) | Hook developer | Note |
|---|---|---|---|---|
| R1 new conversation | MIF-A | PROF-DEV-A | HOOK-A | Skill description carried · body 0 |
| R2 resume · all files to B · same profile | **MIF-B** (the key was given) | PROF-DEV-A **only** (B discarded) | **HOOK-B added** (resume) | — |
| R2 after compaction | MIF-B | **PROF-DEV-B** (A gone) | **HOOK-B** (runs again on compact) | Only two user lines + summary remain |
| R3 resume · without profile | ★**MIF-A**★ (copy from start — even though the file is B and the previous resume was B) | B in the record | None | — |
| R5 resume · without profile · compaction | MIF-A | **0** | 0 | Only the system prompt survives |

- **R4 hook size:** a 58,326-character file was emitted by two identical hooks. Default = **truncated to 10,102 characters** (head · tail kept) + 「Full hook output saved to: `C:\Users\…\AppData\Local\Temp\hook_outputs\<세션>\<id>.txt`」 · `additionalContextLimit = 0` = **the whole 58,326 characters** (both head and tail sentinels). The canonical source of the default threshold of 2,500 tokens = `config/src/hook_config.rs:175-184`.
- **Hook properties (source):** feature `hooks` is stable · on by default. On Windows it runs through `cmd.exe /C "<명령>"` (`hooks/src/engine/command_runner.rs:435`) — **the hook command also goes through cmd** (the command is a short `type <경로>`, so the command-line wall does not apply). The SessionStart pending sources are four: start · resume · `/clear` · compaction (`hooks/src/events/session_start.rs:24-38`), and the execution point after compaction is `core/src/session/turn.rs:637`. Trust is assigned by the hash of the hook definition (including the command) (`hooks/src/engine/discovery.rs:767-791`) — if the command differs per agent, stored trust needs a fresh approval every time, and the route that skips it per process is `--dangerously-bypass-hook-trust` (that flag emits two warning lines on every run · it runs the TUI in embedded mode). The 「훅 기계장치」 that ADR-0216 removed was for harvesting session ids and has a different purpose from this channel.

## Skills

| Run | Junction | Request |
|---|---|---|
| R1 · A t1 | Present | `engram-probe` in the list (description sentinel) · **0 body sentinels** |
| R2 | Present · only the body changed to B | List unchanged · body 0 (nobody called it) |
| R3 · A t2 | **Deleted** (the original folder left as is) | **A new skill list block was appended and `engram-probe` dropped out** · the `$engram-probe` in the same request was **silently discarded** (body 0 · no error) |

- **The body is read when used:** the list's guide text says 「스킬을 쓰기로 했으면 그 `SKILL.md` 를 끝까지 연다」 and tells the model to (`ext/skills/src/catalog_prompt.rs:28`) — the model opens the junction path through the shell. A `$이름` mention is read and inserted by codex at that moment (earlier report). Either way **the link must be alive at that time**.
- **The list is rebuilt on every request:** since the skill list is one 「world state」 field, it is rebuilt from the current disk on every request and a block is appended if it changed (`ext/skills/src/world_state.rs`). Dropping out right on the request after the deletion is the result.
- **Per-process skill folder:** the `[skills]` settings are only `bundled` · `include_instructions` · `max_context_tokens` · `config` (turning already-discovered ones on and off), and there is no key · argument · environment variable that adds a root (`SkillsConfig` of `config/src`) — the earlier report's conclusion as is. The only outside root is the app-server `skills/extraRoots/set`.

## Method (방법) — reproduction essentials

Essentials are kept to fill the gap that ADR-0216 described as 「방법의 서술이 어디에도 남지 않았다」 (the scripts themselves are session scratch and do not remain).

- **Fake server:** with node `http` it receives `POST …/responses` at `127.0.0.1:<포트>` and drops the body to a file. The reply is three SSE events — `response.created {response:{id}}` → `response.output_item.done {item:{type:"message",role:"assistant",content:[{type:"output_text",text}]}}` → `response.completed {response:{id,usage:{input_tokens,output_tokens,total_tokens,…}}}`. `GET …/models` is an empty list.
- **Config (scratch `CODEX_HOME`):** `model_provider = "fake"` · `[model_providers.fake] base_url = "http://127.0.0.1:<포트>/v1"` · `wire_api = "responses"` · 0 retries · `[projects.'<작업 폴더>'] trust_level = "trusted"`. The working folder is `git init` (project root marker). If the model name is not in the catalog, it runs with default metadata after one warning line (harmless).
- **Triggering compaction within the same turn:** put `model_auto_compact_token_limit = 200000`, and if the prompt has the marker, the server answers with a **tool call that does not exist** (`function_call` · name `probe_noop`) + `total_tokens = 900000` → codex returns a tool error and does **in-turn compaction** with 「후속 필요 + 한도 초과」 (local — the summary request arrives as `You are performing a CONTEXT CHECKPOINT COMPACTION…`) → the next request is the post-compaction context. If the last input is `function_call_output`, it answers with plain text to prevent an infinite loop.
- ★**Outbound trap**★ — point `HTTP(S)_PROXY` · `ALL_PROXY` at a logging proxy at `127.0.0.1:<포트+1>` and set `NO_PROXY=127.0.0.1,localhost`. The proxy logs the `CONNECT` and cuts the connection with 403. **codex tries to go out on its own besides model requests** — in 8 processes, `chatgpt.com` 15 · `github.com` 8 · `api.github.com` 7 · `ab.chatgpt.com` 7 (all rejected before TLS → payload 0). Saying 「외부 전송 0」 without a trap means you only saw the model requests.
- **Real `~/.codex` unchanged check:** session jsonl 125 · `session_index.jsonl` 509 lines · `history.jsonl` 150 lines — same before and after. The scratch `CODEX_HOME` names are `h1` · `h2` (following the earlier report's `codexhome` collision warning).

## Limits (한계) · not measured

- **Interactive TUI not measured** — there is no means on this PC to drive a PTY (node-pty etc.), so `codex exec` and `app-server` were measured instead. The basis is a source reading that the code path is the same (the embedded-mode condition above).
- **Remote compaction (OpenAI provider · our actual usage form) not measured** — the fake provider only takes local compaction. That remote compaction uses the same re-injection function is confirmed from source only (likely).
- **`/new` · `/clear` · working folder change · re-reading by sub-agents** — source only.
- **A signal that tells from outside the moment the TUI opened the thread** — not looked for. To do 「지우기」, this signal is needed first.
- **UserPromptSubmit hook injection · the route of storing and using hook trust** — not measured.
- **How differently the model follows instructions per channel** — needs real model calls, so not run (same gap as the earlier report).
