# Codex launch-material measurements before S25 step 6 (2026-11-09)

- Status: measurement record · no decision. Inputs to the step-6 TRD (`docs/process/S25-preset-library/trd-codex-translation.md`), which records the decisions taken on them.
- CLI: `codex-cli 0.163.0` (the installed version; the QA lane last passed on it).
- Method: one measurement worker drove `codex app-server --stdio` over its JSON-RPC line protocol with a trimmed copy of the existing harness `.claude/handoff/attachments/20260925-midturn-phase0/codex_harness.js` (same `initialize {clientInfo: engram-dashboard}` → `initialized` handshake, server requests refused with `-32601`). `codex debug prompt-input` was used for the offline AGENTS check (M7). Nothing in the product was changed or launched.
- Confidence labels: **certain** = observed directly in this run · **likely** = inferred from what was observed · **unknown** = not settled by this run.

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
