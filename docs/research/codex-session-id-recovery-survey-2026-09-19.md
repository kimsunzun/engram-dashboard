# Recovering the codex session id — technique choice and mechanical contract for terminal-mode resume (measurement survey)

| Item | Value |
|---|---|
| **Status** | **Two-pass document** — §1~§9 = after-the-fact transcription (two adversarial reviewers' input folded in) · **§10 = second-pass direct measurement** (two adversarial reviewers' input folded in) |
| **Date** | 2026-09-19. **§1~§9 transcribe the previous session's measurements**, and **§10 is what this document measured directly** |
| **Method** | Three survey tracks (technique analysis · peer survey · standards) + **three direct measurements on codex-cli 0.155.0** (first pass) + ★**11 launches (second pass — §10)**★ |
| **★The sections differ in nature★** | ★**§1~§9 did not take a single new measurement**★ — they are a transcription and structuring of **two sources** (the previous session's handoff + the body of commit `75bdfda`). ★**§10 is different — it launched the TUI on a real PTY and measured directly**★ (home copy only · real `~/.codex` md5 confirmed unchanged). **Do not read §10 down to a transcription on the strength of the preamble alone.** |
| **Confidence legend** | ★**Measured**★ = confirmed by **running** codex-cli 0.155.0 **directly** (§1~§9 = by the previous session, §10 = by this document) · **precedent reading** = reading peer source and comments · **unmeasured** = not done (§7 · §10-5) · **unverified transcription** = a transcription that cannot cite its source |
| **Why this survey** | Launch codex in terminal mode and the conversation does not continue after a restart — **codex issues the identifier to resume from by itself**, and the terminal path had **no endpoint to receive it** |

> ★**Read this first — this document's value lies in the 「실측」 label**★
>
> ★**The codex vendor docs describe only the hook concept, and do not write a single line about the mechanical contract below (`trusted_hash` · `hooks.state` · storage location · silent skip · non-interactive trust grant).**★ So the 「실측」 marks in §2~§4 are not decoration but **a mark of reproduction cost** — re-measuring costs **10~16 minutes + 150k worker tokens** per measurement (three measurements = 145k·149k·170k, three survey tracks = 177k·159k·136k).
>
> That session **could not leave this survey as a report, and the handoff was the only record.** This file is the canonical source of that record.

---

## 0. Conclusion and scope

**Question:** how does the supervisor (our daemon) recover the codex session id to resume a **terminal-mode** session.

**Answer = codex's `SessionStart` hook calls our CLI, and that CLI sends the `session_id` from the stdin JSON back to us.** The supervisor **plants its own correlation token in env**, and the hook process carries it back — this holds because codex injects no env at all into the hook and **purely inherits** (§2-2).

★**This section once said 「세 경로 중 둘에서만 재 봤다」 — §10 closed that gap.**★ The first pass measured only `codex exec` and `codex app-server`, leaving the TUI path empty, and **the second pass (§10) drove that path directly on a real PTY.**

**Three of the four conditions for confirmation are closed**:

1. ~~Does the `SessionStart` hook **actually fire** on the TUI path~~ → ★**closed**★ (§10-1.4 — fires on the first turn after approval)
2. ~~Is that stdin payload **exactly the 7 fields**~~ → ★**closed**★ (§10-1.4 has the payload verbatim)
3. ~~Does approval **persist through the trust-review screen**~~ → ★**closed**★ (§10-1.2 — the key is stable across launches and on restart it is `trusted` without a prompt)
4. **So does our daemon actually receive the id — end-to-end including our wiring** → ★**Not done yet. This one alone remains.**★ Every measurement used a separate harness, and we have not seen our code ride that contract.

★**The axis that picked that answer is not the technique's success rate but 「못 찾았을 때 무슨 일이 일어나나」**★ — §1 is that axis, and it is the **section to read first** in this report.

★**But what we actually built is not the full form of that technique category — it prevents misattribution only halfway**★. The cause, the extent of the empty-slot window, and the one line of defense that exists today are canonical in §1 "it closes the axis only halfway" · the state as a remaining risk = §7.

### What was measured and what was not

| Measured path | Status |
|---|---|
| `codex exec` | ★Measured★ — hook firing · timing · payload · trust |
| `codex app-server` | ★Measured★ — hook firing · timing · id identity with `thread/start` · trust-grant round trip |
| **TUI (terminal) path** | ★**Measured (second pass · §10)**★ — review screen · approval · firing · `session_id` · resume · even the precedence of duplicate `-c`. **What remains is the one item 4 above (our wiring end-to-end)** |

- **Our side's wiring has never been run against a real codex** — that is, §2 is a measurement of **codex's contract**, and we have not yet seen **our code actually ride that contract**. Why there was nothing to wire to (= the commit boundary) = §9.
- The canonical source for the landed code is the body of commit `75bdfda`, and it is not repeated here.

---

## 1. ★The deciding axis — what is dangerous is not the technique but 「못 찾았을 때 하나 고르는 tie-break」★

★**The moment a rule like `가장 최근 mtime` or `첫 매치` attaches, that technique converts not-found into wrong-found. And that conversion is entirely silent.**★ The worst scenario — **a new conversation reported as 「resumed」** — arises exactly and only by this path. The deciding axis is not a technique's average accuracy but **the loudness of its failure mode**.

**Only two techniques make misattribution structurally impossible, and both are propagation, not inference:**

1. **The supervisor pre-issues the id and injects it** (the claude way) — ★**codex has no such flag (confirmed twice, independently)**★.
2. ★**The supervisor plants a correlation token in env, and the child's hook sends it back**★ — **W3C Trace Context gives the same answer.** This is what we chose.

### ★★But what we actually built is not the full form of ② — it closes the axis only halfway★★

**Category ② being immune to misattribution is a statement about 「토큰이 그 세션 하나만 가리킬 때」.** The token we plant does not satisfy that condition — ★**it is a control-plane token that is one per agent** (`ENGRAM_TOKEN`, ADR-0086 · planted at `inject_cli_entrance` in `crates/engram-dashboard-agent/src/backend/mod.rs:102`) **and env is inherited by children automatically.**★ So **when an agent launches a sub-agent, that sub-session's hook also reports with literally the same token.**

★**There is no means to tell parent from child**★ — the token and env are the same, and **the measured 7-field payload has no field that speaks of lineage** (§2-2). ★**`CODEX_THREAD_ID`, which the mature peer (herdr) reads for exactly this purpose, does not exist in 0.155.0**★ (§2-2) — so we could not copy that means.

**The only defense we use on this axis today is the one line 「이미 찬 칸을 덮지 않는다」 and what it blocks is a report that arrives *after* the slot is filled.** ★**While the slot is empty, whoever arrives first wins**★ — if the child's hook arrives before the parent's, **the child's session id sits in the parent's slot**, and the parent is then rejected.

- ★**That empty-slot window opens once per incarnation**★ — because a Fresh spawn returns that slot. (Without the return, the window would open only once, but **from the second incarnation on every normal restart prints a rejection log, which hides a real nesting accident** — we chose the side that keeps the signal alive.)
- ★**In app-server mode that window is practically absent**★ — the hook fires **16ms after the first `turn/start`**, so **the channel sink always fills the slot first**, and the value the hook carries is the **same value** as that thread id (§2-2). That is, in that mode the hook's report always ends as 「이미 같은 값」. ★**The window is real on the terminal path.**★

★**The ranking below is still correct — it is a ranking of technique *categories*.**★ What changes is **one step down from the category to our implementation**: ② is generally immune to misattribution, but **the concrete token we planted does not inherit that property.** Canonical grounds = the header of `crates/engram-dashboard-daemon/src/control/hook.rs` (the threat model, the rejected means and the extent of the window are there).

### Quiet/loud ranking (safest first — the louder side is the safer)

```
미지원 선언  >  id 선발급  >  ★훅 + 상관 토큰★  >  구조화 서버 모드
           >  조회 서브커맨드  >  프로세스 introspection  >  출력 스크래핑
           >  ★파일 감시(최신 고르기) = 가장 위험 · 가장 조용★
```

---

## 2. ★Measured — codex-cli 0.155.0 (the two paths `codex exec` and `codex app-server`)★

### 2-1. Hook firing and trust

- **If you pass the hook only as an execution argument (`-c hooks.SessionStart=[…]`), the parser accepts it but it does not run.** ★**It is skipped in complete silence**★ — no error, prompt or log at all, and **exit code 0**. Even with `RUST_LOG=codex_hooks=trace` on, **not a single line is left**. **What blocks it is trust.**
- It runs if you add `--dangerously-bypass-hook-trust` (**reproduced twice**). ★**But that argument exists only on root/`exec`, and `codex app-server` does not accept it**★ — adding it kills it with `error: unexpected argument`.
- **Where trust persists = `$CODEX_HOME/config.toml`:**

  ```toml
  [hooks.state.'<sourcePath>:<event_snake>:<group_index>:<handler_index>']
  trusted_hash = "sha256:<hex>"
  ```

  ★**The key is per hook slot**★ — changing one hook makes only that one `modified`, but **reordering the array or deleting an earlier entry shifts the keys of later slots and releases the trust.**
- ★**Granting trust can be done programmatically (round trip confirmed by measurement)**★: via app-server, `hooks/list` → receive each hook's `key`·`currentHash`·`trustStatus` → `config/batchWrite` upserts `hooks.state` (`mergeStrategy:"upsert"`, `reloadUserConfig:true`) → `untrusted` → `trusted`. ★**It persists in a new process too**★. A hook injected with `-c` gets trust the same way.
- ★**Do not compute the hash yourself**★ — it can be reproduced (sha256 of normalized JSON), but **the precedent (orca) broke on every codex release, dropped its own computation and moved to the RPC path** (precedent reading). Skip that trial and error.
- ★**Injecting the trust state with `-c` failed**★ — because the key mixes colons and backslashes. **It has to go in as a file.**
- ★**`exec` and `app-server` do not ask for review and silently skip**★ — **this half is measured** (we ran both modes directly).
- **The claim that the review prompt appears only in the TUI is precedent reading, and this document did not even open the source file behind it.** The cited `tui/src/startup_hooks_review.rs` belongs to **codex's own source tree, not a path in this repository** — the repository is `openai/codex` (this repository already cites that upstream by that name and has a precedent of writing the same tree as `codex-rs/<crate>/src/…` — `docs/reference/backend-capabilities.md`), ★**no revision is pinned**★ (the only anchor we have is the `codex-cli 0.155.0` used for measurement, and its npm distribution has no Rust source). So this one line was once an **unverified transcription**. ★**§10 replaced it with measurement**★ — we actually saw the review screen in the TUI (§10-3 has the wording verbatim), and the silent skip of `exec` and `app-server` was already measured by the line just above, so **the two close as a pair.** ★**A re-check that opens the source revision is no longer needed — do not go spend that cost (10~16 minutes + 150k tokens per measurement).**★
- `trustStatus` vocabulary = `trusted` / `untrusted` / `modified` / `managed`.

### 2-2. When the hook fires and what it gives

- **`codex exec`**: fires right after session creation, ★**0.13 seconds before the user prompt is put into the conversation**★. **The interval is identical in two samples** → the structure is **wait for the hook synchronously, then insert the prompt.**
- ★**`codex app-server` fires too**★ — but not at `thread/start`, ★**16ms after the first `turn/start`**★. **If you only open a thread and run no turn, it does not fire.**
- ★★**In app-server, the hook's `session_id` == the `thread.id` in the `thread/start` response (same value, measured)**★★ — the two paths write **the same value into the same slot**. Only **the channel sink always arrives first.**
- The **first turn after `thread/resume` fires too** and `source` is `resume` (a new one is `startup`).
- **app-server emits `hook/started` and `hook/completed` in the protocol** → firing and timing can be measured without an external recorder (§8).
- ★**stdin payload = exactly 7 fields**★ (verbatim):

  ```
  session_id · transcript_path · cwd · hook_event_name · model · permission_mode · source
  ```

  ★**467 bytes, no newline**★. ★**argv is empty — delivery is stdin alone**★. It fires when `.cmd` is specified directly (**no shell wrapping needed**). **Hook process cwd = session cwd.**
- ★**Before you use that 467, read this — the repository has three numbers speaking of payload size, and all three differ**★:
  - ① **467 bytes** — this line (the real payload that session saw).
  - ② **「300 바이트 미만」** — the comment on the stdin cap constant (`HOOK_STDIN_CAP` in `crates/engram-dashboard-daemon/src/bin/engram.rs`).
  - ③ **211 bytes** — the test constant `MEASURED_HOOK_PAYLOAD` in the same file (counted directly).

  ③ is **a variant that abbreviates** the two path fields to `C:/x/y.jsonl` and `C:/x` — the 「실측된 페이로드 그대로」 that constant speaks of means the **field composition**, not the length. The real payload carries the rollout absolute path and the session cwd, and JSON doubles the backslashes, so those two fields eat several hundred bytes. ★**② matches neither ① nor ③**★ (larger than ③ and smaller than ①) — **which sample that line was written from has not been established** (this pass did zero re-measurement · so it did not fix that comment either — fixing without measuring merely turns one unverified number into another).
  ★**Do not pick one of the three at the spot where you size the buffer**★ — the fact the decision needs is just that **all three are under 1% of the 64 KiB cap**, and the ground of that cap is not size but 「이 stdin 은 상대가 채운다」 (that constant's doc).
- ★**codex injects none of its own env into the hook process — it is pure inheritance**★. **So the `ENGRAM_TOKEN` we planted arrives as is, and the bearer the hook presents to `/control/hook` is exactly that token** (ADR-0086 · planted at `backend/mod.rs:102`). ★**This one line is both the ground that makes §1 technique ② hold and the hole in that technique**★ — the same inheritance gives the same token **to a sub-agent's hook too** (§1 "it closes the axis only halfway").
- ★**`CODEX_THREAD_ID` does not exist in 0.155.0**★ (**zero hits in the binary's full strings**). **herdr's codex hook uses it to tell nesting apart, and if you copy that guard it always reads as unset.**
- **codex hook event vocabulary** (from herdr's install code — precedent reading): `SessionStart` · `UserPromptSubmit` · `PreToolUse` · `Stop` · `PermissionRequest`. **herdr too now uses only `SessionStart`** (for the rest only code that deletes old entries remains).

---

## 3. What passes through execution arguments / what does not

**Discrimination method = `codex -c '<키>=<값>' --strict-config app-server --stdio < /dev/null`** — ★**a fake key dies with `unknown configuration field`, so a control group holds**★. This is what makes this table a **test** rather than an observation.

| What we try to pass | Result |
|---|---|
| `model_instructions_file=<경로>` | ✅ **works** (given a nonexistent path, it **actually went to read it and failed**) |
| `project_doc_max_bytes=<수>` | ✅ works |
| `features.hooks=true` | ✅ works (but `features list` shows `hooks stable true` — **on by default, so unnecessary**) |
| `hooks.<이벤트>=[{hooks=[{type="command",command="…"}]}]` | ✅ ★**passes inline**★ (the hook table is open by event name) |
| `skills.include_instructions` · `skills.max_context_tokens` · `skills.bundled` | ✅ works |
| **`skills_root` / `skills.root` / `skills.dir` …** | ❌ ★**no such key**★. The `SkillsConfig` fields are `bundled`·`include_instructions`·`max_context_tokens`·`config` — **there is no location knob** |
| `--config <파일>` | ❌ ★**no such flag**★ |

### Two warnings about discriminating power

- ★**Do not try to validate `-c` key validity with `codex doctor`**★ — **a fake key passes too (discriminating power zero)**. Only the `--strict-config` + `app-server --stdio < /dev/null` combination above is a discrimination path.
- ★**`--strict-config` acceptance is weak**★ — **it validates types** (giving a string where an array goes kills it) but ★**an unknown key inside a matcher group passes**★. That is, little weight should rest on "**the parser accepted it**" — of the ✅ rows above, only `model_instructions_file` (it went to read) and the inline hook (it fired) were seen to actually behave.

### `CODEX_HOME`

- ★**The `CODEX_HOME` environment variable can move the whole home (measured)**★ — **it applies to that process only, and the user's own codex keeps using the original home.**
- ★**But auth moves with it**★ — in an empty home, 「자격증명 없음」. (This property creates §8's free measurement trick.)
- **The skill location is fixed at `.codex/skills` under the home**, so **if we supply skills ourselves, this is the only path.**

---

## 4. Storage structure (terminal mode)

- **`codex resume [SESSION_ID]` exists.** `--last` (skip the picker) · `--all` (drop the cwd filter) · with no argument the **TUI picker**. **Non-TUI counterpart = `codex exec resume`.**
- ★**`codex continue` does not exist in 0.155.0**★ — **it disagrees with web sources.**
- ★**There is no command that LISTs sessions**★ — the `resume` picker is **pure TUI** and has no `--json`. **Everything a machine can read is direct reading of undocumented files/DB.**
- **Storage is a pair of two places:**
  - rollout JSONL — `~/.codex/sessions/<Y>/<M>/<D>/rollout-<로컬시각>-<uuid>.jsonl`, **id and cwd in the first-line `session_meta`**
  - the **`threads` table** of `state_5.sqlite` — `id`·`rollout_path`·`created_at`·`cwd`·`title`·`name`·`originator`
  - **`codex doctor` checks the pairing of the two stores by itself.**
- ★**The format is mid-transition**★ — `threads.history_mode` is a **mix of legacy/paginated**, and `codex migrate-rollouts` exists (both measured). ★**Going the direct-file-reading route gets you bitten by this transition**★ — this is the second reason the lowest technique in §1 is technically unstable too. **That decision stands on these two facts alone.**
  - ★**「관련 버그 다수 열림」 — this document cannot cite a source for it**★ — it is issue-tracker knowledge, so it is neither measured nor precedent reading, nor an unmeasured item of §7. The legend's **unverified transcription** slot is its place — do not put weight on the conclusion, and to confirm, look at the `openai/codex` issues directly.
- **`originator`/`source` tell the executing party apart:** `codex-tui`/`cli` · `codex_exec`/`exec` · `Claude Code`·`engram-dashboard`/`vscode`.

### ★One internal contradiction — left unresolved★

**On `codex exec` the rollout file appears 0.7 seconds after launch** (measured). ★**The observation once written as 「첫 턴에 생긴다(5s·23s 지연)」 did not reproduce on `exec`**★. **It may have been the TUI path — not established.** We could not determine which of the two observations belongs to which condition, so **we keep both.**

---

## 5. Peer survey

| Peer | What it does | Precedent value for this problem |
|---|---|---|
| **herdr** | **Runs every agent on a PTY**, installs **only one `SessionStart` hook** for codex, receives `session_id` from the stdin payload and sends it back through a socket (POSIX) or CLI re-entry (Windows). **Zero lines** of file scanning or reading the id off the screen | ★**A positive precedent — the statement below that it 「못 받는다」 was refuted by a close reading of the clone on 2026-09-19**★. It is true that it does not automate trust (`trusted_hash` zero hits), but **it receives the id normally through the hook** — it merely shows the review screen in a 「막힘」 state and **hands it to a human to approve once**. Every failure is silent (on restore it does not revive the agent at all and just lays down the old screen — ★do not copy that side★) |
| **orca** ★closest★ | **Grants trust through app-server RPC (main path)** + its own hash fallback + **recognition** of the review screen (scraping). A comment reads `Codex 0.129+ gates each hook on a trusted_hash` | ★**A positive precedent — when provenance verification fails, it strips the resume argument, launches a clean new session and notifies the user**★. **It added this after suffering a cross-account resume accident.** ★**This is the exact behavior shape we want**★. **But 「심사 화면 스크래핑 = 안전망」 was overstated** (close reading 2026-09-19) — the screen match is **only for classifying blocked** and does not send key input back |
| **octoally** | **The only one that scrapes the screen.** ★**Adopts the first match unconditionally · zero verification**★. It even attached a file-search fallback | ★**A negative precedent** — the existence of the fallback is a confession that 「긁기만으로 부족했다」★. The real instance of the lowest technique in §1 |
| **Multiplexers** (wezterm·zellij·tmux) | Session survival + client re-attach | ★**They do not solve this problem**★ — **they restore the terminal, not the agent session** |

**orca's lesson about computing the hash yourself is in §2-1** — moving the main path to RPC is the heart of that precedent, and we will not build even a fallback.

---

## 6. Standards ground

★**RFC 9413** (IAB, *Maintaining Robust Protocols*)★ **flatly rejects** the reading 「**모호한 입력을 추론된 의도로 처리하라**」. It is the standards document that fits our situation most exactly — meaning the tie-break ban in §1 is not our taste but **a position a standards document has already taken**.

(That technique ② in §1 is the same answer as **W3C Trace Context** is ground of the same nature — the form where the parent plants the correlation token and the child returns it is **an already-standardized shape**.)

---

## 7. ★Unmeasured · remaining risks★

- ~~★**Hook firing on the TUI (terminal) path is unmeasured**★~~ → ★**§10 closed it**★ (review screen, approval, firing and `session_id` on a real PTY). **Do not reopen 「가장 큰 공백」 on the strength of this line in this section.** ★**But a remainder inherited that spot — the line 「우리 쪽 훅 배선」 below is it, and that is now the biggest gap.**★
- **Unconfirmed whether a codex hook fires once per session** — we could not run a clean two-turn experiment.
- **Whether `thread/fork`, compaction and sub-agent threads fire, and their `source` values, are unmeasured** — ★but the two values `startup`/`resume` were measured by §10-8★ (fork is still unmeasured).
- ★**The rollout-creation-time contradiction in §4 is not established**★ — **kept as an open item.** The values and conditions of the two observations, and the fact that we could not decide which belongs to which path, are carried by §4 "One internal contradiction".
- **Our side's hook wiring has never been run against a real codex** (§0 · the reason for that boundary = §9).
- ★★**A nested session can occupy the parent's slot — a hole left by design (unresolved)**★★ — the cause, the extent of the empty-slot window and the one line of defense that exists today are canonical in §1 "it closes the axis only halfway" and are not repeated here. **What this section carries is 「그래서 언제 닫히나」: not before a means to tell parent from child exists** — because the absence of that means today is exactly the cause. Canonical code-side description = the header of `crates/engram-dashboard-daemon/src/control/hook.rs`.

### Do-nots that came out of this survey (only what was actually experienced)

1. ★**Do not pass `--dangerously-bypass-hook-trust` to app-server**★ — **it dies with an argument error.** And **we do not recommend permanent use either** — it runs even the user's own hooks without verification + ★**mixes two `type:"error"` items into the `--json` stream and causes parser false positives**★.
2. ★**Do not compute the hook trust hash yourself**★ — the precedent broke on every release and moved to RPC.
3. ★**Do not copy herdr's `CODEX_THREAD_ID` nesting guard**★ — 0.155.0 has no such variable, so **it always reads as unset.**
4. ★**Do not pick a codex session id from files as 「가장 최근 것」**★ — **it turns not-found into wrong-found, and it is silent** (§1).
5. **Do not try to validate `-c` key validity with `codex doctor`** — discriminating power zero (§3).

---

## 8. ★How to measure cheaply when measuring again★

The order for when the same class needs measuring again. It is left here because that session **learned it expensively**.

1. ★**Call `hooks/list` first**★ — `trustStatus`·`currentHash`·`key` come out at once and ★**silent skip and non-firing are told apart immediately**★. **This comes before setting up a recorder control.**
2. **app-server emits `hook/started` and `hook/completed` in the protocol** → ★**firing and timing are measured without an external recorder.**★
3. ★**How to measure without using quota**★ — point `CODEX_HOME` at an **empty folder**: with no auth the turn **dies with 401, but the hook fires before that.** With this trick we measured even through `turn/start` **for free**.
4. ★**For 「바이너리가 뭘 출력하나」 launching a tty once is overwhelmingly cheaper than digging strings**★ — **we spent 6 round trips on 307MB string archaeology and still ended unverified.**

---

## 9. Links

- **Canonical source for the landed code = the body of commit `75bdfda`** (`S21: feat(agent,daemon): codex 에 제어 평면 입구를 열고 훅 세션 id 수신을 세운다`). This report carries **why a hook + codex's contract**, and that commit carries **what was built**.
- ★**That commit has no hook registration — it only built the endpoint to receive.**★ This is not unfinished but **an intended boundary**.
- **The plan, order and unreceived review items of the follow-up stage are carried by that session's handoff** — not repeated here.
- ★**That session left 「1단계에 대한 ADR 이 없다」 as a debt**★ — because the `supports_control_channel` flip · the new control route · the 「어떤 갈래로도 exit 0」 policy · the 「빈 칸에만 쓴다」 rule are all **non-obvious decisions with rejected alternatives**. **Whether that debt was closed afterwards, this document did not confirm** (outside the two sources) — look at the decision index.

---

## 10. ★Second-pass measurement — the TUI trust path (2026-09-19, codex-cli 0.155.0)★

> Closes the **「TUI 경로 미측정」** that §7 left. Unlike the first pass (§2~§4), which measured only the two paths `codex exec` and `app-server`, this section **launched the TUI on a real PTY** and drove the review prompt directly. **11 launches · home copy throughout**, and the real `~/.codex/config.toml` was **confirmed unchanged** by md5 comparison (both before and after launch).

### 10-1. Six conclusions (all measured)

1. ★**A hook passed with `-c` also goes through the TUI review, receives trust, and then fires normally**★ — in a home whose config.toml has **no hook entry at all**, `-c hooks.SessionStart=[…]` alone brings up the prompt, and on approval the hook runs on the first turn with `session_id` in the payload. **§2-1's 「`-c` 훅은 완전 침묵으로 스킵된다」 is still true, and that section's decision that trust was what blocked it is also true** — give it trust and it runs as is.
   - ★**But there is no way to get trust that avoids writing the user's `config.toml`**★. Even if we do not touch registration, **at the moment of approval codex itself** writes `[hooks.state.'…:session_start:0:0'] trusted_hash` there. It is codex that writes, not us, but **the file changes.**
2. ★**The trust key is stable across launches**★ — the `<session-flags>` in a `-c` hook's key is a **literal placeholder**, and is neither a path nor a per-run id. Launch again with the same `-c` and it is **`trusted` with no prompt**. That is, the `-c` path is not 「평생 재승인」.
3. **The hash covers the hook *definition* and does not cover the *source path*** — whether the same definition was registered as a file or passed with `-c`, `currentHash` was identical. If the command string changes it drops to `modified` and re-prompts (↔ **this is why a fixed wrapper path is needed**).
4. **Approval takes effect from that conversation on** — the hook does not run at approval time (only trust is recorded immediately and `sessions/` does not exist yet). **It fires once on the first turn**, and that `session_id` equals the `codex resume` id the TUI prints. ★**So the notice 「첫 대화는 이어받기 불가」 is not needed**★ — the constraint we worried about at design time does not actually exist.
5. ★**The failure mode 「끄기가 영구 저장된다」 does not exist**★ — option 3, `Continue without trusting (hooks won't run)`, is **session-scoped** and writes nothing to config. ESC is the same. Restart and it asks again. There is no permanent-disable toggle anywhere in the TUI, and the `HookTrustStatus` vocabulary (`managed|untrusted|trusted|modified`) has **no reject-only state**. The branch §7 called 「가장 무섭다」 is closed.
6. ★**`--dangerously-bypass-hook-trust` is out**★ — it works (it skips the prompt, the hook runs, and it writes no trust. There are also a `BYPASS_HOOK_TRUST` env and a `bypass_hook_trust` key). But **it is global, so the user's own unreviewed hooks run unreviewed along with it** (measured with two decoy hooks — both ran). **It cannot be used unless the home is one we own to the end.**

### 10-2. Approval key sequence (measured)

**Five separate writes**, in order:

| # | Bytes | Meaning |
|---|---|---|
| 1 | `0x0D` | Confirm 1, `Review hooks` (**already selected — do not send an arrow first**) |
| 2 | `0x0D` | Enter the SessionStart detail list (Hook 1 expands) |
| 3 | `0x1B 0x5B 0x42` (`ESC [ B`) | Move the expansion to the row below — repeat for the number of rows |
| 4 | `0x74` (`t`) | ★Trust **only the one expanded hook**★ |
| 5 | `0x1B` | Return to the overview |

- ★**The spacing is mandatory**★ — write them all at once and **nothing registers** (measured: both ended `untrusted`). **A 300ms interval was measured to succeed** and 0ms fails. We did not bisect in between. It is the same nature as what this repository's `SUBMIT_PACING` targets.
- ★**On the overview screen `t` means 「전부 신뢰」 — only `t` on the detail screen is singular**★. Pressed on the overview, it trusts even the user's own hooks.
- ★**Do not count Down presses**★ — in measurement the user's config.toml hook came first and ours later, but **that order is not guaranteed**. We **identify** our row by the `Source    Session flags - …` line on screen (that line exists only on ours). Row movement is **Down/Up only** — Tab and Right do nothing.
- Success signals: the detail row becomes `Trust     Trusted` · the header count decreases · the footer goes from `Press t to trust; esc to go back` to `Press space or enter to toggle; esc to go back`. ★**Even so, the decision is made structurally with `hooks/list`**★.

### 10-3. Screen wording verbatim (ANSI stripped · no box characters · two leading spaces except on the cursor row)

```
  Hooks need review
  1 hook is new or changed.
  Hooks can run outside the sandbox after you trust them.

› 1. Review hooks
  2. Trust all and continue
  3. Continue without trusting (hooks won't run)

  Press enter to confirm or esc to go back
```

- The plural was measured too — `2 hooks are new or changed.` · `2 hooks need review before they can run.`
- Detail rows: `Trust     New hook - review required` · `Trust     Modified since last trusted - review required` · `Trusted`.
- **Detection anchor** = `^\s*Hooks need review\s*$` followed within 2 lines by `^\s*\d+ hooks? (?:is|are) new or changed\.` — **the second line is the same for new and changed** (confirmed with the changed case).

### 10-4. The config shape the parser accepted

```toml
[[hooks.SessionStart]]
[[hooks.SessionStart.hooks]]
type = "command"
command = 'C:\path\to\probe.cmd'
```

`command` is a single string (the schema also has `commandWindows`). A `.cmd` file spawns normally as argv[0].

### 10-6. ★A hook command takes arguments — the premise that a wrapper file is needed was wrong★ (measured)

- ★**The `command` string is split into program + arguments**★ — codex **takes the first whitespace-separated token as the program and spawns it directly** (it does not go through a shell) and splits the rest into arguments **while recognizing quotes**. So a single string `"<경로>\engram.exe hook session-start"` holds as is. ★**That is, there is no reason to install a wrapper script on the user's machine**★ — the schema has no `args` array (`ConfiguredHookHandler.oneOf[0]`), and `commandWindows` is not a carrier of arguments but a per-OS substitute string.
- ★**Instead one constraint — it fails if the program path has a space**★. Wrapping it in quotes does not work, and not wrapping it does not work. **Workaround = the 8.3 short path** (`C:\PROGRA~1\…`) was measured to work. **On the argument side**, if you wrap it in quotes it arrives as one token even with spaces in it.
  - So our implementation **swaps in `GetShortPathNameW` when there is a space, and if a space still remains (a volume with 8.3 off returns the original as 「성공」) it skips registration and only leaves a warning.**
- **A wrong command does not die silently** — in the TUI `• Hook failed` / `└ hook exited with code 1` appears, one line per hook. ★**Only the 「신뢰 안 됨」 side is silent**★ — do not confuse the two. On the app-server side `HookErrorInfo` and `HookCompletedNotification` exist too, so non-TUI clients also see the failure.
- **Env is pure inheritance** — even for a hook registered with `-c`, codex puts in none of its own variables (only what we planted into the codex process comes down). All session data arrives by **stdin**.

### 10-7. ★With a duplicate `-c` the later one wins — the order is itself the defect★ (measured)

- Give the same key with `-c` twice and there is **neither merge nor error, and the second wins** (one entry in `hooks/list`, and only the second one runs).
- ★**And an empty array is the worst**★ — give `-c hooks.SessionStart=[]` second and `hooks/list` returns an **empty array**, **not even the review prompt appears**, and the turn runs normally. That is, **the hooks vanish wholesale with no signal at all.**
- **Consequence:** our override has to be placed **after the user passthrough** to win. ★It was once built as 「override 를 passthrough 앞에 둔다」 and that was the defect — do not revert it.★ Instead, if the user gives the same key, **we leave a warning that we overrode it** (no silent overwriting).

### 10-8. ★`codex resume <id>` keeps the id★ (measured — it was a design premise)

- After a new session reports S (`source:"startup"`), launching again with `codex resume S` reports **the same S** (`source:"resume"`). **It does not waver across repeated resumes.** The rollout file also keeps appending to **the one original of S** (after three sessions there is still only one `.jsonl`).
- ★**So both tails we worried about are absent**★ — ① a 「찬 칸 충돌」 warning printed on every resume ② the profile holding on to an old id so that conversations from the second onward are silently discarded. The hook report always ends as **「이미 같은 값」**.
- ★**`source` is the discriminator**★ — it tells `startup` and `resume` apart. It is in the same stdin JSON as `session_id`, so one parse yields both. **We did not look at what fork reports.**

### 10-9. Does the generated override survive the real argv path (measured)

The operational spawn does not exec codex directly but **goes through `cmd.exe /c`, and `codex` on PATH is a `.cmd` shim**. Even launched through that round trip, `hooks/list` returns **the command string as is** (`errors: []`). Backslashes, brackets and both kinds of quotes all survive, and TOML takes the single quotes as literal. ★**Do not move on having confirmed only by direct invocation without looking at this round trip**★ — the same file already records `%VAR%` being mangled on that path.

### 10-5. What this section did not measure

- ★**§0 condition of confirmation 4 — end-to-end through to our daemon. This section did not measure it either**★ (every measurement used a separate harness and we did not see our code ride the contract). Even though §10 closed three, **this one remains** — it points at the same thing as §7's 「우리 쪽 훅 배선」 line.
- The `managed` trust state (needs an MDM or plugin source) · whether a plugin-sourced hook is asked about differently · **the `source` value fork reports**
- The real lower bound between 0 and 300ms
- **The 45 evidence files landed in the session temp folder** — that folder disappears with the session. The preserved copy = `I:\Engram_Workspace\codex-hook-trust-evidence-2026-09-19\` (outside the repo · untracked, so **it is not on other PCs**). If reproduction is needed, measure again using §8's cheap measurement + the sequence in 10-2.
