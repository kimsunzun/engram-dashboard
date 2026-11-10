# Codex launch-material measurements before S25 step 6 (2026-11-09)

- Status: measurement record · no decision. Inputs to the step-6 TRD (`docs/process/S25-preset-library/trd-codex-translation.md`), which records the decisions taken on them.
- CLI: `codex-cli 0.163.0` (the installed version; the QA lane last passed on it).
- Method: one measurement worker drove `codex app-server --stdio` over its JSON-RPC line protocol with a trimmed copy of the existing harness `.claude/handoff/attachments/20260925-midturn-phase0/codex_harness.js` (same `initialize {clientInfo: engram-dashboard}` → `initialized` handshake, server requests refused with `-32601`). `codex debug prompt-input` was used for the offline AGENTS check (M7). Nothing in the product was changed or launched.
- Confidence labels: **certain** = observed directly in this run · **likely** = inferred from what was observed · **unknown** = not settled by this run · **reported** = an outcome stated in a record whose method and version were not recorded (M11).

## Run environment

- **Spawn shape.** The process was started as `cmd.exe <verbatim command line>` (`windowsVerbatimArguments`), so the command line could be built with either quoting rule our backend uses:
  - **std** = Rust `std::process::Command` (JSON mode — `transport/spawn.rs` `spawn_piped`): quote only when the argument has a space or tab or is empty; `"` → `\"` (backslashes before it doubled). Emulated from the documented std rule; the std source was not installed, so this emulation is **likely**, not read.
  - **pty** = portable-pty 0.8.1 `cmdbuilder.rs` `append_quoted` (terminal mode; read from the registry source): also quote when the argument has `"`, LF or VT.
  - Shape = `cmd.exe /c codex app-server --stdio …` → the npm shim `codex.cmd` (`"%_prog%" "…codex.js" %*`).
- **No-token runs** used a temporary, empty `CODEX_HOME` (no auth). Turns there end with `401 Unauthorized` before any model work; they were used for the M3 accept/reject check and the handshake diffs. 4 such turns.
- **Real turns** (M2, M4) used the user's real `~/.codex` (auth needed) with `-c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]`, `approvalPolicy: never`, `sandbox: read-only`, and an empty temporary folder as cwd. Nothing was written to `~/.codex/config.toml` or to any skills folder. **Real model turns used: 3** (M2 ×1, M4 ×2).
- M7 used `codex debug prompt-input` (an offline render of the model input) with the temporary `CODEX_HOME`.
- **Leftovers.** The temporary work folder (fixtures, temporary `CODEX_HOME`, logs, generated schemas) was sent to the recycle bin after the run. The real `~/.codex` keeps **one test thread** (created by M2, resumed by M4) with codex's own state rows for it; it was not deleted — removing it is the user's call.

---

## M1 — Does the `skills/extraRoots/set` walk follow a directory junction? — **Yes (certain)**

**Fixtures.** Under one root `rootJ\engram\`:
- `p0-plain\SKILL.md` — a plain folder;
- `p0-junc` — a junction to `real\jtarget` (which holds `SKILL.md`, name `p0-junc`);
- `nestlink` — a junction to `real\nest` (which holds `p0-nested\SKILL.md`).

Separately, `rootLink` — a junction to `real\rootTarget` (which holds `p0-viaroot\SKILL.md`).

**Calls per case.** `skills/extraRoots/set {extraRoots: [<root>]}` → `{}`, then `skills/list {forceReload: true}`.

**Evidence** (skills with our names; `errors: []` in every case):

```
roots=[rootJ]          p0-junc    @ ...\real\jtarget\SKILL.md              [user]
                       p0-nested  @ ...\real\nest\p0-nested\SKILL.md       [user]
                       p0-plain   @ ...\rootJ\engram\p0-plain\SKILL.md     [user]
roots=[rootJ\engram]   (the same three)
roots=[rootLink]       p0-viaroot @ ...\real\rootTarget\p0-viaroot\SKILL.md [user]
```

**Verdict.**
- A junction **under** a root is followed — both when it is the skill folder itself and when it is a folder one level above the skill. A root that **is** a junction also works.
- The reported `path` is the **target** path, not the link path. The scope is reported as `user`.
- Passing `<root>` or `<root>\engram` both work: the walk starts at the root and descends (this layout adds one level, well within the depth limit).
- A skill without a front-matter `name` takes the **target** folder name (consistent with `docs/research/skill-md-recognition-claude-codex-2026-10-20.md` row 4), so a junction's own name does not name the skill in codex.
- Not measured: a file symlink named `SKILL.md` (the earlier research says it is dropped — source reading only).

## M2 — Same-name skills across two extra roots — **Both listed, no error; `$dup` selects neither (certain, n = 1 turn)**

**Fixtures.**
- `rootA\dup\SKILL.md` — `name: dup`, description "… root A (alpha)", body "reply ALPHA7731";
- `rootB\dup\SKILL.md` — the same name, description "… root B (bravo)", body "reply BRAVO4419";
- `rootM\p0-marker\SKILL.md` — a marker skill.

**Calls.** `skills/extraRoots/set [rootA, rootB, rootM]` → **immediately** `thread/start` → `skills/list` → one `turn/start` with the text: `Use the $dup skill now and follow its instructions exactly. If you cannot tell which dup skill is meant, reply with the single word AMBIGUOUS.`

**Evidence.**
- `skills/list`: `dup @ …\rootA\dup\SKILL.md [user]`, `dup @ …\rootB\dup\SKILL.md [user]`, `p0-marker …`, `errors: []` — no warning, no error.
- The thread's rollout, developer message (line 3): a "Skill roots" table `r6 = …/rootA`, `r7 = …/rootB`, `r8 = …/rootM`, and under "Available skills":
  ```
  - dup: Duplicate-name test skill from root A (alpha). (file: r6/dup/SKILL.md)
  - dup: Duplicate-name test skill from root B (bravo). (file: r7/dup/SKILL.md)
  ```
- The `$dup` user message (rollout line 10) was **not** followed by an injected skill body: neither `ALPHA7731` nor `BRAVO4419` appears in the rollout before the model's own tool call. The model then ran `pwsh -Command "Get-Content '…/rootA/dup/SKILL.md'; Get-Content '…/rootB/dup/SKILL.md'"` and answered `AMBIGUOUS` (turn `completed`).
- This matches the earlier research on 0.156.1 (`docs/research/skill-md-recognition-claude-codex-2026-10-20.md` row 5): every same-name skill is listed, there is no priority, and a mention whose name matches more than one skill selects none.

**Verdict.** Codex does not drop, shadow or reject a duplicate: it loads both and leaves the choice to the model. The skills are not lost; only an explicit `$name` mention stops selecting one. Whether the model would pick one at random without the "AMBIGUOUS" way out was not measured (the prompt deliberately offered it).

## M3 — `turn/start.additionalContext` and the `experimentalApi` capability — **Rejected without the opt-in; accepted and effective with it (certain)**

**Calls** (temporary home, no auth): `initialize` (with and without `capabilities: {experimentalApi: true}`) → `initialized` → `thread/start` → `turn/start {input: [text "hi"], additionalContext: {"engram-instructions": {kind: "application", value: "P0-ADDCTX-MARKER-9137"}}}`.

**Evidence.**
- **Without the opt-in:** `{"code": -32600, "message": "turn/start.additionalContext requires experimentalApi capability"}` — the whole `turn/start` is refused; no turn is created.
- **With the opt-in:** accepted (`turn.status: inProgress`). The rollout carries a **developer-role** message before the user message, `<engram-instructions>P0-ADDCTX-MARKER-9137</engram-instructions>`, with metadata `content_item_kinds: ["additional_content.engram-instructions"]`. The map key becomes the tag name the model sees.
- **Real-turn confirmation** (M4 turn 2, a resumed thread, opt-in): the same wrapper was recorded and the model followed it (it appended `P0-RESUME-5521` as instructed).
- **What else the opt-in changes** — a controlled diff of the same flow without `additionalContext`, temporary home, comparing the field sets of every server message: **only one extra notification, `thread/readState/changed`** (`{threadId, readState: {firstUnread: {type: "turn", turnId}, revision}}`). The `initialize` result, the `thread/start` result keys and every `turn/*` and `item/*` field set were identical.
- In the real opt-in turn (M4), `turn/moderationMetadata` also appeared, and it did not appear in the real turn without the opt-in (M2). The two flows differ (resume vs start), so this is **not attributed**: whether it is gated by the opt-in is **unknown**. Both methods are in the *stable* `ServerNotification` schema.

**Verdict.**
- The opt-in is mandatory for `additionalContext`. Sending it only on spawns that carry `additionalContext` leaves every other handshake unchanged.
- With the opt-in, the reader must tolerate `thread/readState/changed` (certain) and possibly `turn/moderationMetadata` (unknown whether opt-in gated).
- The key is shown to the model as an XML-like tag, so the key should read well as a tag. `kind: "application"` → developer role.
- A rejected `turn/start` creates no turn, so sending the same `turn/start` again until one is accepted does not duplicate anything.

## M4 — Extra roots on resume; ordering — **Process-scoped, not restored by `thread/resume`; setting them before start or resume is enough (certain for the observed n = 1)**

**Flow.**
- Process A = the M2 flow (roots set right before `thread/start`).
- Process B (a new `app-server`, opt-in): `thread/resume` (no roots set) → `skills/list` → turn 2 (with `additionalContext`) → `skills/extraRoots/set [rootL]` → `skills/list` → turn 3. Turns 2 and 3 asked: "Without using any tool, list every skill in your available-skills list whose name starts with p0- or equals dup".

**Evidence.**
- Process A: `p0-marker` (its root set in the same tick as `thread/start`) is in the first turn's skills message — no race.
- Process B after resume: `skills/list` shows no `dup` or `p0-*`. Rollout line 26: a **fresh** skills developer message was injected for turn 2 **without** `dup` or `p0-marker`. The model still answered `p0-marker, dup, P0-RESUME-5521` — it recalled the old list from the conversation record (stale memory, as `docs/research/external-skill-dir-loading-claude-codex-2026-10-10.md` noted).
- After `skills/extraRoots/set [rootL]`: `skills/list` shows `p0-late`; rollout line 38: a new skills message with `p0-late` (alias `r8` — root aliases are renumbered per message; `r8` meant `rootM` in turn 1); turn 3 answered `p0-late`.
- Side observations on resume:
  - a `deprecationNotice`: "Full-history hydration is deprecated for paginated threads; use `excludeTurns: true`, then page with `thread/turns/list` and `thread/items/list`." The harness sent `thread/resume` **without** `excludeTurns`;
  - a `thread/goal/cleared` notification;
  - the `thread/resume` result `path` came back with the verbatim prefix (`\\?\C:\…`), while `thread/start` returned `C:\…`.
- Schema note: the stable `ThreadResumeParams` **does** have `developerInstructions`. Not sending it on resume is a client choice, not a protocol limit; whether it takes effect on resume was not measured.

**Verdict.** Extra roots must be set in every new app-server process, on resume too. Setting them before `thread/start` / `thread/resume` is sufficient — no wait for `skills/changed` is needed for the first turn. A resumed conversation keeps "remembering" skills from its old record even when their roots are gone: removing a skill from a preset does not make a resumed codex forget it.

## M5 — `"` in a `-c` value through `cmd.exe /c codex.cmd` — **The `"` itself survives in both spawn styles, but it flips cmd's quote state for every later argument (certain)**

**Observation channel.** app-server `config/read {includeLayers: true}` → the `sessionFlags` layer (the raw effective `-c` values), temporary home.

**Evidence** (std and pty quoting alike unless noted):
- `-c model="o3"` → `model: "o3"`; `-c developer_instructions="a b"` → `"a b"`; `-c p0.strnum="5"` → the string `"5"`, not the integer 5 (so the quotes reached codex); `-c p0.arrs=["a b",1]` → `["a b",1]`; `-c p0.lit2='x y'` → `"x y"` (the TOML single-quote literal of ADR-0210 still works on 0.163.0); `-c p0.esc="t1\tt2"` → a real TAB.
- A controlled pair:
  ```
  args [model="o3", p0.c=a^b, p0.s="x y"]          std: {model:o3, c:"ab", s:"x y"}   pty: same
  args [p0.q="s, p0.c=a^b, p0.a="m&n", p0.z=1]     std: {q:"s", c:"a^b", a:"m"}  (z lost)
                                                   pty: {q:"s", c:"a^b", a:"m&n", z:1}
  ```
  With balanced quotes, cmd eats the `^`; after an odd `"`, the same `^` survives. In std style the `&` then split the command line — everything after it was lost (cmd printed "The system cannot find the path specified") — while in pty style the same arguments survived. So whether a metacharacter is live depends on every `"` before it **and** on the spawn style.
- `%PATHEXT%` is expanded in both styles.
- A raw **LF inside an argument silently truncates the rest of the command line** (`p0.ml="line1<LF>line2"` → `ml: "line1"`, every later `-c` gone, both styles; the app-server still started).
- A wrong value for a known key is **fatal at startup**: `-c model_reasoning_summary='x y'` → the app-server exits 1 with `unknown variant 'x y', expected one of 'auto', 'concise', 'detailed', 'none' in 'model_reasoning_summary'`.
- Unknown keys are not fatal: `Codex is ignoring 2 unrecognized configuration settings … session-flags: 'p0' is ignored.`

**Verdict.** Blocking `"` is not needed for the value to survive (`model="o3"` works). What must stay out of every argument is `% & ^ | < >` and control characters (LF, CR, others), because a `"` makes metacharacter handling non-local. With those banned in every argument, `"` alone was harmless in every case measured (**likely** safe — not exhaustive). **Not measured:** a later argument that legitimately contains `<`, `>`, `&` or `|` inside its own quotes (for example a quoted multi-word value) after an argument with an odd number of `"`.

## M6 — `-c` parsing — **Split at the first `=`, trim, plain dotted split, TOML value, else the text with outer quotes trimmed; the last one wins (certain)**

**Evidence** (`sessionFlags` layer; the same in std and pty):

| `-c` argument | Effective value |
|---|---|
| `p0.a.b=1` | `{p0: {a: {b: 1}}}` (nested table) |
| `p0.arr=[1,2]` | `[1,2]` |
| `p0.str="s"` | `"s"` |
| `p0.bare=s` | `"s"` (not TOML → literal) |
| `p0.sp = 5` | key `sp`, value `5` (both sides trimmed) |
| `p0.bad=[1,` | `"[1,"` (literal) |
| `p0.q="s` | `"s"` (literal, the leading `"` trimmed) |
| `p0.lit='it''s'` | `"it''s"` (invalid TOML → literal, outer `'` trimmed, inner kept) |
| `p0.eq=a=b` | `"a=b"` (split at the first `=`) |
| `p0.tbl={x=1,y="z"}` | `{x: 1, y: "z"}` |
| `"p0.quoted.key"=1` | `{"\"p0": {quoted: {"key\"": 1}}}` — **no quoted-key support**; a plain split on `.` |
| `p0.dup=1` then `p0.dup=2` | `2` (the last one wins — confirms the 0.155.0 note) |
| `foo` | `Error: Invalid override (missing '='): foo` (exit) |
| `=1`, ` =1` | `Error: Empty key in override` (exit) |
| `a..b=1`, `.a=1` | accepted (an empty path segment is not rejected) |

**Verdict.** To mirror codex:
1. split at the first `=`; a missing `=` or an empty (trimmed) key is an error;
2. trim the key and the value;
3. split the key on `.` with no quoting;
4. parse the value as a TOML value (inline tables, arrays and escapes occur);
5. if that fails, take the trimmed text with leading and trailing `"` / `'` stripped (the exact trim set is inferred from two cases — **likely**);
6. a later override of the same key replaces the earlier one.

Not measured: what happens when a later override goes **through** an earlier scalar (`a=1` then `a.b=2`).

## M7 — Which AGENTS file is picked — **The override wins; an empty or blank override still wins and suppresses `AGENTS.md`; lowercase names match on Windows (certain, offline render)**

**Method.** `codex debug prompt-input x` in each folder (temporary home, no `.git`, so the folder is the project root).

| Folder contents | Loaded |
|---|---|
| `AGENTS.override.md` (MARK-OVERRIDE) + `AGENTS.md` | only MARK-OVERRIDE |
| empty `AGENTS.override.md` + `AGENTS.md` | **nothing** (no AGENTS section at all) |
| whitespace-only `AGENTS.override.md` (`"  \r\n\n"`) + `AGENTS.md` | **nothing** |
| `agents.md` (lowercase) only | MARK-LOWER (rendered as "AGENTS.md instructions for …") |
| `agents.override.md` + `AGENTS.md` | only MARK-LOWEROV |
| none | nothing |

**Verdict.** The pick is `AGENTS.override.md` if it exists (any case, on this case-insensitive NTFS volume), else `AGENTS.md`. An existing empty or blank override is still the pick and contributes no text — codex does **not** fall back to `AGENTS.md`. To mirror codex: pick by existence (by the OS name rule), then drop the text when it is blank. Not tested: folders with per-directory NTFS case sensitivity.

## M8 — `thread/resume.developerInstructions` — **Accepted, but it does not reach the next turn; it is held only in the live process and is used only when a compaction rebuilds the initial context (certain, mock provider)**

**Method.** A second measurement run (same day, same CLI, no token, **no real model turns**). Each step is one `codex app-server --stdio` process (spawn and handshake as above, harness trimmed to a small client), so every resume is a cold resume in a new process — the way our backend resumes. To see the **actual model input** of each turn instead of the rollout alone, the temporary `CODEX_HOME` pointed codex at a local mock of the Responses API on `127.0.0.1` that records every request body and answers with a minimal SSE stream (assistant `ok`), so turns complete and history grows normally. Two provider setups:
- **custom provider** — `config.toml`: `model_provider = "mock"`, `[model_providers.mock] base_url = "http://127.0.0.1:…/v1"`, `wire_api = "responses"`. Manual compaction takes the **local** path (a `CONTEXT CHECKPOINT COMPACTION` summarization request, then a summary user message);
- **built-in `openai` provider** — `config.toml`: `openai_base_url = "http://127.0.0.1:…/v1"`. Manual compaction takes the **remote** path (`request_kind: compaction`, `implementation: responses_compaction_v2`, input ends with `{type: compaction_trigger}`); the mock answered with one `{type: compaction, encrypted_content: "M8ENC-n"}` item. (This provider first tries a WebSocket `GET /v1/responses`, gets 404 and falls back to HTTP POST after 5 retries.)

`thread/start` was sent as our backend sends it (`codex/mod.rs` `ThreadOpen` → `protocol.rs` `ThreadStartParams`): `{cwd, approvalPolicy, sandbox, developerInstructions: "M8-DEV-A-1111 …"}`. `thread/resume` = `{threadId, excludeTurns: true, developerInstructions?}`. Compaction = `thread/compact/start {threadId}`. Nothing was written to the real `~/.codex`; the temporary folder (homes, mock, captured requests) was sent to the recycle bin after the run (M8 and M9). Schema check: `generate-json-schema` **without** `--experimental` lists `developerInstructions` (and `baseInstructions`, `config`, `model`, …) in `ThreadResumeParams` — a stable field; no opt-in was sent for these runs.

**Evidence** (request inputs, codex's own default developer message and the multi-agent/environment items left out; `[A]` = the developer message whose first content item is the `developerInstructions` text, followed in the same message by `<skills_instructions>`, `<permissions instructions>`, `<collaboration_mode>`):

```
custom provider
P1 start(A) + turn             : [A] env  user#1
P2 resume(B) + turn            : [A] env  user#1 ok  user#2                     ← B absent, A still there (history)
P3 resume(no field) + turn     : [A] … user#3
P4 resume(C) + ctx + turn      : [A] … user#3 ok  <engram-instructions>M8-ADDCTX-4444</engram-instructions>  user#4   ← C absent
P5 resume(no field) + compact + turn : user#1 user#2 user#3 [skills…, no instructions] env user#4 <summary> user#5
                                       ← A gone, the additionalContext message gone; no developer instructions at all
V1  P1 start(A)+turn | P2 resume(B)+turn+compact+turn | P3 resume(no field)+compact+turn
    P2 turn 1      : [A] …                                   (B absent)
    P2 after compact: user#1 [B] env user#2 <summary> user#3  (B present, A gone)
    P3 compact req : … [B] …                                  (B came back from the stored compacted history)
    P3 after compact: user#… [skills…, no instructions] …     (B gone too)
V2  one live process: start(A)+turn+compact+turn → after compact: [A] env user#1 <summary> user#2   (A kept)
V3  P1 start(A) | P2 resume(B)+turn | P3 resume(no field)+compact+turn
    P2 turn: [A] …   P3 after compact: no instructions          (B never reached the model)

openai provider, remote compaction (same pattern)
OA1 P2 resume(B)+turn+compact+turn : turn 1 [A] …;  after compact: user#1 [B] env user#2 {compaction M8ENC-17} user#3
    P3 resume(no field)+compact+turn: after compact: user#1 user#2 [skills…, no instructions] env user#3 {compaction} user#4
OA2 P2 resume(no field) + ctx + turn + compact + turn:
    turn 1: [A] … <engram-instructions>M8-ADDCTX-4444</engram-instructions> user#2
    after compact: user#1 [skills…, no instructions] env user#2 {compaction M8ENC-41} user#3   ← A and the context both gone
```

- `thread/resume` with `developerInstructions` returned success in every case; no error, no warning, no notification about it.
- Persistence: in the rollout and the sqlite state files, `B` appears **only** inside a `compacted` rollout item written by a compaction in the process that received `B` (V1: 1 occurrence; V3, no compaction in that process: 0). `turn_context` and `thread_settings_applied` carry no developer-instructions text (`collaboration_mode.settings.developer_instructions: null` there is a different field).
- The no-token run against the real endpoint (401) showed the same rollout shape: after `resume(B)`, the turn wrote only the user message — no developer message with `B`.

**Verdict.**
- **B does not take effect for the next turn** (certain). On a cold resume codex replays the stored history as the model input; it does not inject or diff the developer instructions. A stays (it is part of the stored history); never both.
- `B` is kept **only in the resumed process's session settings**. It reaches the model only when a compaction in **that** process rebuilds the initial context — then `B` replaces `A` (certain, manual compaction, both compaction paths).
- **Not persisted** as a setting: a later resume without the field has no instructions in its session, so its next compaction rebuilds the initial context with **none** — even though `A` (or `B`) was in the history before (certain).
- Corollary for **today's** backend (resume never sends the field): a resumed JSON thread loses its start-time instructions at the first compaction in any resumed process (certain on the mock; a thread that stays in its starting process keeps them — V2).
- `additionalContext` is a separate developer message right before the turn's user message (`<key>value</key>`, as M3). It lives in history only, so **every compaction drops it** (certain on both paths; with a real server the opaque compaction item may still carry its gist — **unknown**).
- Not measured: automatic (token-limit, mid-turn) compaction — likely the same rebuild, not observed; a same-process `thread/resume` of a thread that is already loaded; what the real server puts into the remote compaction item.

**Implication.** `developerInstructions` on resume does **not** replace the `additionalContext` route: the next turn after a resume never sees it. The two are complementary — `additionalContext` on the first turn delivers changed instructions now; `developerInstructions` = the current full text on **every** resume (changed or not) is what keeps the instructions alive across a compaction in that process, and after such a compaction only the new text remains (the `additionalContext` message is dropped, so no duplicate). The `protocol.rs` comment on `ThreadResumeParams` ("no `developerInstructions` counterpart … not measured whether that field is in this request's schema") is now outdated: the field is in the stable schema and accepted.

## M9 — Process scope of `skills/extraRoots/set` — **Not shared between concurrent processes (certain)**

**Method.** One driver started two `codex app-server --stdio` processes A and B at the same time (same temporary `CODEX_HOME`, same cwd, mock provider of M8 so turns complete). Fixtures: `rootA\m9-a\SKILL.md`, `rootB\m9-b\SKILL.md`. Each `skills/list` used `forceReload: true`. Finally each process started a thread and ran one turn; the skills message in the captured request shows what the model sees.

**Evidence** (both processes alive throughout — `exitCode === null` at the end):

```
0  A list → []          B list → []
1  A set [rootA] → {}   A list → [m9-a]   B list → []
2  B set [rootB] → {}   A list → [m9-a]   B list → [m9-b]
3  A turn request skills: m9-a only        B turn request skills: m9-b only
   skills/changed: A got 1 (right after its own set), B got 1 (right after its own set) — none from the other process
```

- Side observation: on the **first** attempt the home was new and both processes initialised it at the same moment; B exited 1 with `failed to initialize sqlite state runtime under …`. The rerun on the now-initialised home had no error. n = 1, so whether this is a first-init race only is **likely**, not settled. Our product runs against an existing `~/.codex`, so this is a note, not a risk for the design.

**Verdict.** Extra roots are per app-server process: a set in one process is invisible to `skills/list`, to `skills/changed` and to the model input of another live process, in both directions, and B's set does not overwrite A's roots. Together with M4: every process must set its own roots, and two agents with different presets can run concurrent codex processes on the same `CODEX_HOME` without leaking roots into each other.

## M10 — Does cmd apply the 8191 limit again to the `codex.cmd` shim line? — **Yes (certain)** · and `a=1` then `a.b=2` (2026-11-10, S25 step 6 P1)

Third run, `codex-cli 0.163.0`, this PC, no model turns (`codex --version` and `config/read` only), an empty temporary `CODEX_HOME` per run (removed afterwards).

### M10a — the shim line

**Why.** `cmd.exe /c codex …` runs the npm shim `C:\Users\<name>\AppData\Roaming\npm\codex.cmd`, whose last line `endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\node_modules\@openai\codex\bin\codex.js" %*` is expanded again. The step-6 TRD (§4-3, R13) reserved `SHIM_MARGIN = 300` units in the command-line count until this is known.

**Method.** A Python script built the command line `cmd.exe /c codex -c p0.pad=<A × n> --version` as one string (passed verbatim to `CreateProcessW`) at total lengths L, and ran it. On this install `%dp0%\node.exe` does not exist, so `_prog` = `node`; the expanded shim line is then E = L − 17 (`cmd.exe /c codex `) + 165 (the expanded prefix), i.e. E = L + 148.

**Raw output** (stderr is cmd's Korean message, abridged; the success rows also print codex's `WARNING: proceeding, even though we could not create PATH aliases …` about the temporary home):

```
L=7900 E~8048 exit=0   stdout='codex-cli 0.163.0'
L=8000 E~8148 exit=0   stdout='codex-cli 0.163.0'
L=8040 E~8188 exit=0   stdout='codex-cli 0.163.0'
L=8041 E~8189 exit=0   stdout='codex-cli 0.163.0'
L=8042 E~8190 exit=0   stdout='codex-cli 0.163.0'
L=8043 E~8191 exit=0   stdout='codex-cli 0.163.0'
L=8044 E~8192 exit=255 stderr='입력 줄이 너무 깁니다. | 명령 구문이 올바르지 않습니다.'   (The input line is too long. | The syntax of the command is incorrect.)
L=8045 E~8193 exit=255 stderr='명령 구문이 올바르지 않습니다.'
L=8050 E~8198 exit=255 stderr='명령 구문이 올바르지 않습니다.'
L=8100 E~8248 exit=255 stderr='명령 구문이 올바르지 않습니다.'
L=8150 E~8298 exit=255 stderr='명령 구문이 올바르지 않습니다.'
L=8191 E~8339 exit=255 stderr='명령 구문이 올바르지 않습니다.'
L=8192 E~8340 exit=1   stderr='명령줄이 너무 깁니다.'   (The command line is too long. — cmd's own line limit)
```

**Verdict.** The limit applies a second time, to the expanded shim line, at exactly 8191 (E = 8191 runs, E = 8192 fails) — so a command line that fits cmd's own limit can still fail in the shim, with no codex process started. On this install the shim adds 148 units net; it grows with the npm folder (the user name) and when `node.exe` sits next to the shim (`_prog` is then its full path). **Decision for the code:** `SHIM_MARGIN` stays at 300 (TRD rule: keep it if the limit applies again). Not measured: the portable-pty spawn (whose program token is the resolved `cmd.exe` path, not `cmd.exe`) — the shim line does not contain the program token, so the second limit does not depend on it; the first limit with a longer program token is L5 (iii)'s.

### M10b — a later `-c` line through an earlier scalar

**Method.** `codex app-server --stdio -c <overrides>` through `cmd.exe /c`, `initialize` → `initialized` → `config/read {includeLayers: true}`; the `sessionFlags` layer is the effective `-c` value.

```
-c p0.a=1   -c p0.a.b=2  →  {"p0":{"a":{"b":2}}}
-c p0.a.b=2 -c p0.a=1    →  {"p0":{"a":1}}
```

**Verdict.** A later line that goes through an earlier scalar replaces the scalar with a table, and a later scalar replaces an earlier table — the same as the source (`codex-rs/config/src/overrides.rs` `apply_toml_override` at `rust-v0.163.0`, read 2026-11-10). The P1 `raw_to_json` mirrors it.

## M11 — A single `%` in the terminal `--cd` folder (2026-11-10, S25 step 6) — **Reaches codex intact; `%USERNAME%` expands (reported — method not recorded)**

**Source.** One line in the S25 step-6 session ledger (`.claude/handoff/attachments/20261110-s25-step6-code/s25-step6-ledger.md`, among the notes of QA rerun 3 — HEAD `9969ac60`, tree `f9cb17db` — the tree with the user's 2026-11-10 working-folder rule in `backend/codex/launch.rs` `cwd_reaches_codex_unchanged`). This section carries only what that line says; "reported" = recorded as an outcome without its method, so weaker than **certain** above.

**What was recorded.** No model tokens were used. A working folder holding a single `%`, passed as `--cd`, reached `codex.exe`'s argv intact through `cmd /c` and the npm `codex.cmd` shim, both with and without a space in the folder; a folder holding `%USERNAME%` was expanded (the reason two or more `%` are refused).

**Not recorded.** The exact folder names, how the argv of `codex.exe` was read, the spawn style (std or portable-pty quoting), the codex version, and who ran it.

**Use.** The basis for letting a single `%` through the real-terminal working-folder guard (ADR-0353 decision 3). A `%` in the pass-through `extra_args` could still pair with it (`docs/tracking.md` T-102) — not measured.

---

## Summary by question

| Question | Answer | Confidence |
|---|---|---|
| `experimentalApi` needed for `additionalContext`? | Yes (`-32600` without it). Side effect with it: `thread/readState/changed`; maybe `turn/moderationMetadata`. | certain / unknown for the second |
| How `additionalContext` reaches the model | The key becomes the tag; `application` → developer message; a rejected `turn/start` creates nothing. | certain |
| Junction under an extra root | Followed; the target path is reported. Per-skill roots would also work. | certain |
| Extra roots on resume | Process-scoped; set them again in every process; set-before-start is race-free. | certain (n = 1) |
| `"` in `-c` | Survives; flips cmd's quote state for later arguments; LF truncates silently. | certain |
| `-c` parse rule | The M6 table. | certain (trim set likely) |
| Same-name skills across roots | Both loaded, no error; `$name` selects neither. | certain (n = 1) |
| AGENTS pick | The M7 table. | certain |
| `thread/resume.developerInstructions` | Accepted (stable field) but not in the next turn's input; held in the process only; used when a compaction in that process rebuilds the initial context (replaces the old text); not persisted. `additionalContext` is dropped by compaction. | certain (mock provider, manual compaction) |
| Extra roots across concurrent processes | Not shared, either direction; no cross-process `skills/changed`. | certain |
| 8191 limit on the expanded `codex.cmd` line | Applies again, at exactly 8191; the shim adds 148 units net on this install. `SHIM_MARGIN` stays 300. | certain |
| `-c` line through an earlier scalar | The scalar becomes a table; a later scalar replaces a table. | certain |
| A single `%` in the terminal `--cd` folder | Reaches `codex.exe` intact through `cmd /c` and the npm shim, with or without a space; `%USERNAME%` expands. | reported (M11 — method and version not recorded) |
