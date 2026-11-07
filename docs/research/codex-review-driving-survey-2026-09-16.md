# How to drive codex as an automatic code reviewer (research + own measurement)

- 상태: ★**열화(degraded)**★ (2026-09-16) · Strength: research skill **medium — adversarial review omitted (no cross-family verification)**
- Method: 3 lightweight Claude tracks collected in parallel (official primary / real-world precedents / tool-agnostic patterns) → main grounding.
  **We did not run the cross-family adversarial review that the convention requires** — that slot is codex itself, and
  we had already burned 4 measurement runs the same day, so we spared the limit. To remove this label, the adversarial review must be run.
- Also included: ★**own A/B/C measurement**★ (§8) — one and the same review task run four ways and measured.
  As far as we know there is no public material that measures 「같은 도구를 핀 vs 탐색으로 갈라 잰」, so this section is the canon for our conditions.
- Purpose: secure grounds for fixing how the `/review` blind (codex) slot is invoked. Trigger = the per-call cost explosion and
  the 2 limit deaths in the 2026-09-15 session.

## Verification grade legend

Instead of confidence, this document attaches **how it was confirmed** — that axis separated the conclusions better in this survey.

| Mark | Meaning |
|---|---|
| ✅ | Main ran and confirmed it directly on this machine (codex-cli 0.154.0) |
| ◆ | Primary documentation or string extraction from the local binary |
| ○ | Via the collector's WebFetch summary — we did not see the original text directly |
| △ | User report (vendor unconfirmed) |

---

## Key conclusions

1. ✅ **codex already has a review-only path** — `codex exec review`. There is no need to invent a review prompt (§1).
2. ✅ **Own measurement: blocking exploration cuts input 9x · uncached 3x · time 1.8x. In exchange, one defect was missed** (§8).
3. ✅ **Own measurement: continuing a warm thread makes round 2 cheaper by input 6.8x · uncached 13x · time 31x** (§8).
4. ◆ **「diff 만 주는 쪽이 낫다」 holds only for non-agent methods — attaching an agent loop flips it** (§7).
   Our codex is on the agent side, so that conclusion cannot be carried over as is. That is why §8 is the canon.
5. △ **Cached tokens also appear to burn the plan limit** (§5). Do not expect resume to be a limit countermeasure.
6. ✅◆ **Three traps** — the scope flag and a custom instruction cannot be used together · dropping the scope silently reviews something else ·
   resume has a different flag surface (§4).
7. ○ **Do not decide by exit code** — it is 0 even with defects, and a limit death does not show either (§6).

---

## 1. codex already has a review-only path

- ✅ **`codex exec review`** exists. Scope = one of `--base <BRANCH>` · `--uncommitted` · `--commit <SHA>`.
  Retrieval = `--json` · `--output-schema <FILE>` · `-o <FILE>` · `-m <MODEL>`.
  ★**`-s/--sandbox` · `-C/--cd` · `--add-dir` do not exist**★ — they exist only on `codex exec`. On this path the sandbox
  can be specified only with `-c 'sandbox_mode="read-only"'` (if not, it follows the `config.toml` default, which may be
  write in a trusted project).
- ◆ **The review system prompt is compiled into the binary.** Two clauses that bear directly on us:
  - "The bug was introduced in the commit (pre-existing bugs should not be flagged)."
  - "It is not enough to speculate that a change may disrupt another part of the codebase, to be
    considered a bug, one must identify the other parts of the code that are provably affected."
  - Volume policy: "If there is no finding that a person would definitely love to see and fix, prefer
    outputting no findings."
  - ★**The vendor already blocks, via the prompt, the class of false positives our `/review` suffered (flagging pre-existing defects · speculative ripple claims)**★.
- ◆ **The output is structured** — `findings[]` = title · body · confidence_score · priority (0~3) ·
  code_location (absolute_file_path, line_range), plus overall_correctness ("patch is correct" or
  "patch is incorrect") · overall_explanation · overall_confidence_score.
- ◆ **The review runs as a child session** — it does not inherit the parent context (the type description in the binary is
  "Structured review result produced by a child review session.").
- ◆ The built-in prompt states explicitly that **「다른 곳(파일 포함)에 더 구체적인 지침이 있으면 그것이 이 일반 지침을 덮는다」**
  → **injecting review criteria through AGENTS.md is supported by design.** But we found no
  explicit recommendation to 「범위 제한 용도로 쓰라」.

## 2. ★Even within the official sources, 「diff 를 주나 훑게 두나」 splits★

- ○ **The official GitHub Action** = does not pass the diff. It lets the checkout be explored and tells it only the commit range.
  **It does not bundle a default review prompt** — the user supplies it.
- ○ **The OpenAI cookbook** = the opposite. It **appends `git --no-pager diff --unified=5 <BASE> <HEAD>` directly to the prompt text**
  and feeds it on stdin with `codex exec --output-schema ... --sandbox read-only -`.
- ○ The first sentence of the cookbook prompt is **identical, character for character,** to the §1 built-in — the cookbook derives from the built-in.

**→ The primary sources split among themselves. The docs cannot settle it, and the §8 own measurement answers.**

## 3. The field converged on 「diff 인라인 + 탐색 금지」

- ○ **One team's before/after measurement** (the most concrete external figure): when exploration was allowed, it burned the whole 330-second budget and
  died with **403KB of grep output · empty stdout · exit 124**, and the downstream agent misreported it as codex being unavailable.
  After one line of 「탐색 금지」 + an inline diff, **exit 0 · 324s · 54,231 tokens · 0 tool calls**.
  Their conclusion: above a 103KB diff, the right move is to **split the diff or lower the effort** rather than raise the cap.
- ○ **The opposite practice also exists, deliberately** — when the diff is large, they let it collect for itself instead.
  One place's explicit threshold = **inline the full text for 2 or fewer changed files and about 150 lines or fewer; beyond that give only the file list + shortstat and
  「read-only git 명령으로 직접 확인하라」**.
- ○ **A pattern of making a separate slot in the output format for findings outside the diff** (one place calls it `out_of_scope_notes`).
  ★If you do not make it, the model pushes them into the defect list★. The prompt of a real CI example (excerpt):
  - "REVIEW ONLY lines changed in the DIFF; ignore any other code."
  - "You may read some other files outside the DIFF ONLY for CONTEXT."
  - "If you notice issues outside the DIFF, put them in out_of_scope_notes, not in findings."
- ○ **Anti-padding is done with a corrective rule, not with 「아부하지 마라」**:
  "Prefer one strong finding over several weak ones. Do not dilute serious issues with filler.
  If the change looks safe, say so directly and return no findings."
- ○ **There is no completion-marker convention.** Instead, a triple decision of `-o <파일>` + **exit code + non-empty + JSON parse success**.

## 4. ★Traps — two were confirmed directly by main★

- ✅ **The scope flag and a custom instruction cannot be used together.** `codex exec review --base master "focus on security"` →
  `error: the argument '--base <BRANCH>' cannot be used with '[PROMPT]'`
- ○ ★**If you drop the scope flag to keep the instruction, it falls to 「커밋 안 된 작업트리」 with no error**★.
  Whatever scope you write in the prompt, what the CLI feeds the reviewer does not change —
  **it reviews the wrong changes with full confidence.** (observed on 0.144.1 · not rechecked on 0.154.0)
- ✅ **`codex exec resume` accepts neither `-s` nor `-C`** (exit code 2, rejected before the API call).
  The sandbox · working directory are **pinned on the first call and cannot be changed afterwards.**
  → A wrapper that uses the same flag string for fresh and resume dies on every resume.
- ○ **`--ephemeral` and resume cannot be used together** — a new thread is silently created (an open bug).
- ○ **One widely copied secondary document writes these combinations as if they work.** It is false on every version measured.
- ○ Other open issues: an infinite wait before the SSE stream (no timeout · no retry) · a stdin wait hang on non-TTY ·
  **if stdio is detached from the TTY it dies silently with no output** (bears directly on detached launchers) · child processes remaining after the parent exits.

## 5. ★Cache tokens appear to burn the plan limit★

- △ One session **exhausted the 5-hour window from 0% to 98%**, yet **96.8% of its 23.4M input was cache**.
  Another report points the same way (0→16% in 101 seconds, 1.52M of 1.67M input cached).
- ◆ **OpenAI has not answered this** — the issue remains open. In overage credit billing, cached input is
  **computed at a discount** of about 1/10 the rate, but the ChatGPT plan's included limit is described not in tokens but as **a number of messages per 5 hours**,
  so the axis differs.
- ◆ **CLI usage draws from the same pool as cloud/IDE** ("Local messages and cloud chats share your plan's
  usage allowance"). Reset = a 5-hour window + a separate weekly limit possible.

**→ 「가능성 높음」, not confirmed. Even so, do not expect resume to be a limit countermeasure.**
★**But the §8 conclusion stands regardless of this uncertainty**★ — whether the cache is counted in full or discounted, both pin and reuse
win (end of §8).

## 6. Do not decide by exit code

- ○ **The exit code is 0 even with defects.** The output must be parsed.
- ◆ The only documented means to decide limit exhaustion is **the failure event of `--json`**.
  The binary's error classification: usage_limit_exceeded · usage_limit_reached · context_window_exceeded ·
  server_overloaded · quota_exceeded. ★**The exit code mapping is not in the official docs.**★
- ◆ User-facing wording (measured from the binary, excerpt): "You've hit your usage limit." ·
  "Usage limit reached. You've reached your usage limit." · "Your workspace is out of credits."
- ○ The field scrapes stderr with regexes — that is the best state available, and the list is grown by experience, not a contract.
  ★One place **filters out first** the auth noise of a third-party MCP server★ — because it appears on every run and has nothing to do with codex's own
  limit status.
- ○ ★**Without `--json`, the whole agent conversation (10k+ lines) spills onto the terminal**★ — and if that goes into a tool result,
  the caller's context overflows. Always run to a log file.

## 7. Tool-agnostic — 「핀이 이긴다」 comes with conditions

- ◆ **AACR-Bench**: every non-agent method consistently follows the hierarchy **diff > file > repository**
  (e.g., 33.82% → 22.59% → 17.60%). ★**Only the agent method reverses**★ (4.28% → 8.00%), and even that is model-dependent.
- ◆ The comment distribution of the same benchmark (1,505 verified): **diff level 754 (50.1%) · file level 518 (34.4%) ·
  repository level 233 (15.5%)**. ★**The number for 「얼마나 열어줘야 하나」 comes from here**★ —
  giving the full text of the changed files puts about 85% in range, and giving only the hunk leaves up to half out of reach in principle.
- ◆ Agent exploration **raises precision greatly** (one model 39.90% vs 8.70% with no context) while
  recall is low (10.10%) — high confidence · low coverage.
- ◆ **Cloudflare production, 30 days, 131,246 runs** — median **$0.98** · P99 $4.45 · median latency 3 min 39 s ·
  cache hit **85.7%**. Structure = **do not put the diff in the prompt; write per-file patches to disk and pass only the paths**,
  and move the shared context into files too to avoid duplicate injection into the 7 sub-reviewers (in their terms, avoiding a 7x multiplication of token cost).
  The sub-reviewers widen with grep · read **only when needed**. A 3-tier risk branch (2 / 4 / 7+ agents).
- ◆ **Anthropic's Code Review is not diff-only either** — parallel agents look at the **diff + surrounding code**, and
  a **separate verification step** checks the candidates against the actual code behavior to filter false positives.
- ◆ **Long input is not waste but performance degradation** (across 18 frontier models) — a shorter prompt carrying the same information
  beat the longer one in every model. ★But this is a retrieval benchmark, not a code review benchmark★ (transfer is presumed).
- ◆ **Precedent for a cheap-context pattern** = instead of file contents, pick the **file list + key symbol signatures** by dependency graph ranking
  and fit them in a small token budget (default 1k tokens). ★The savings figure is not in that document★.

### ★Counterargument — what the pin actually loses★

- The 15.5% above is **the lower bound of the loss** (when the full text of the changed files is given). With the hunk only, up to 50%.
- In §8 we **actually lost one** — that is our own physical specimen of this counterargument.
- One vendor's own benchmark claims the superiority of full indexing, but ★**the methodology explicitly excludes false positives from the score**★ —
  a benchmark that measures only recall structurally favors a recall-first design. Hard to use as grounds.
- The benefit of adding context **flips sign by model tier** — a gain for closed models (+14.99% / F1 +72.17%),
  a loss for open models (-1.05% ~ -7.98%). ★What gave the biggest gain was not code but **textual context such as the issue · PR description**★.

---

## 8. ★Own measurement — the same review task, four ways★

We ran one and the same target four ways. Target = `git diff 4e8a090~3 4e8a090 -- '*.rs'` (**988 lines · 58,740 bytes · 5 files**).
Model = `gpt-5.6-sol`, effort `high`, the same in all four ways. We put 2 lines to prevent role contamination into all four ways.

| Way | Requests | Input | Cache | Uncached | Output | Wall clock | A real review? |
|---|---|---|---|---|---|---|---|
| **A** exploration allowed · repository cwd | 26 | 2,207,103 | 2,074,752 | 132,351 | 9,528 | 309s | **Yes — 3 defects** |
| **B** pin · neutral cwd · exploration forbidden | 7 | 246,053 | 201,856 | 44,197 | 6,922 | 172s | **Yes — 2 defects** |
| **C** resume round 2 (marginal increment) | 1 | 54,443 | 49,664 | **4,779** | **55** | **8s** | **Yes — 0 tool calls** |
| **C-control** fresh round 2 | 9 | 368,417 | 305,792 | 62,625 | 7,606 | 251s | Yes |

- **All four ways produced a real review. Role contamination 0/4** — the 2 defensive lines worked.
- **The gain of the pin = input 9x · uncached 3x · time 1.8x · round trips 26→7.**
- **The cost of the pin = 1 defect.** What B missed was **a lock-release race that is visible only by reading the call site outside the diff**.
  The two agreed on the top defect. ★n=1 — one target and one round, so it is not a miss rate★.
- **The instruction worked as a control** — all 6 commands B ran were reads of the diff file we handed over. It touched neither git nor the repository.
- **Resume round 2 = cheaper by input 6.8x · uncached 13x · output 138x · time 31x.** Tool calls 0.

### ★The measurement itself nearly went wrong — this trap is the most important★

**`turn.completed.usage` in `--json` is not that turn's value but the session cumulative.**
C had **1 model round trip · 0 tool calls** yet reported `input_tokens: 300,496` — the whole context is about 55k, so
that value is impossible. The session log explains it exactly: the real input of that request is 54,443, and
300,496 = B's 246,053 + 54,443.

- **Where to read per-request values** = in `~/.codex/sessions/YYYY/MM/DD/rollout-*-<uuid>.jsonl`,
  `payload.type == "token_count"` → `info.last_token_usage` (this request) / `info.total_token_usage` (cumulative).
- ★**This misreading produced the previous record's 「resume 은 raw 입력이 더 크다」**★ — it compared the session total against a single request.
  Correction = the real input of resume round 2 is 54,443 and **91% of it is cache**.
- **The old premise of 「호출당 17만」 had the wrong scale too** — a real exploring review is 2.2M cumulative over 26 round trips.
  The empty-directory 16.5k was a single-request value, so subtracting the two never made sense in the first place.

### The conclusion stands regardless of the unconfirmed premise

Whichever way the uncertainty of §5 (whether the cache is counted in full against the limit) turns out:

- If counted in full, **A's 2.2M** is the number that matters → the pin wins by **9x**.
- If discounted, **the 130k uncached** is the number that matters → the pin wins by **3x**.

Resume is the same (6.8x or 13x). **So both can be adopted without waiting for the answer to §5.**

### Reproduction recipe

```sh
# A — 탐색 허용
codex exec --json -o out_A.md --skip-git-repo-check \
  -s read-only -c model_reasoning_effort="high" -C "<repo>" \
  "$(cat prompt_A.txt)" < /dev/null > jsonl_A.jsonl 2> err_A.txt

# B — 핀(중립 cwd · diff 를 ./target.diff 로 건넴 · 프롬프트에서 탐색 금지)
codex exec --json -o out_B.md --skip-git-repo-check \
  -s read-only -c model_reasoning_effort="high" -C "<중립 디렉터리>" \
  "$(cat prompt_B.txt)" < /dev/null > jsonl_B.jsonl 2> err_B.txt

# UUID 회수 = JSONL 첫 줄의 thread.started
UUID=$(head -1 jsonl_B.jsonl | grep -o '[0-9a-f-]\{36\}')

# C — resume. ★-s 도 -C 도 붙이면 안 된다★ (§4). 세션의 디렉터리로 먼저 이동한다.
cd "<중립 디렉터리>" && codex exec resume "$UUID" --json -o out_C.md --skip-git-repo-check \
  -c model_reasoning_effort="high" \
  "$(cat prompt_C.txt)" < /dev/null > jsonl_C.jsonl 2> err_C.txt
```

- The raw artifacts (prompts · diff · JSONL · review bodies) are in the session scratchpad and **disappear with the session.**
- A minor source of confusion: even with `< /dev/null`, `Reading additional input from stdin...` is printed to stderr —
  do not read it as an error signal.

---

## Issues and limits

1. ★**Adversarial review not performed**★ — everything above skipped the confirmation-bias check. This document's status label is that.
2. **Cache tokens and the plan limit** (§5) — only user reports, vendor unconfirmed. But the §8 conclusion does not hinge on it.
3. **Our conditions differ from the benchmark** (key conclusion 4) — it is an agent loop, so 「diff 가 이긴다」 cannot be carried over as is.
4. **The quality axis of §8 is n=1** — one target · one round · one pass. 「핀이 결함을 놓친다」 is a physical specimen, not a rate.
   **A's amount of exploration is itself nondeterministic** — 26 round trips is one draw, not an average.
5. **`fork` not measured** — outside the 4-run budget. We still do not know whether branching per lens after loading the context once works.
6. **The outward appearance of a limit death not measured** — no way died. We still do not know the exit code or wording
   (though we did confirm that **an argument-parsing rejection = exit code 2 · 0 API calls**).
7. **Whether resume stays cheap after round 3 not measured** — the history grows every turn, so there should be a break-even point with fresh,
   but we did not measure it. **The cache retention time is also unknown** — the measurement was at intervals of a few minutes (user judgment: not a priority now).
8. **All web sources are via the collector's summary** (excluding ◆ binary extraction and ✅ local confirmation). We did not
   see the quoted wording directly in the original.
9. One collector **explicitly said not to use one proposal paper with no results as evidence** — we followed that (not cited).

## Next

The canon for the invocation shape is the **global dictionary's** 「비주도 CLI 기본형」, and it lacks the session axis (resume · fork · usage retrieval) and
the scope axis (pin · exploration forbidden) altogether. **Revising the dictionary is a user decision**, so this document gives only the grounds.
The spawn skeleton side (a new slot in the output format for findings outside the diff · structured retrieval · no deciding by exit code) is `/review`'s remit.
