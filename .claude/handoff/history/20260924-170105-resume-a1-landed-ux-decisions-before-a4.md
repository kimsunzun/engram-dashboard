# Handoff: resume-after-first-turn — TRD + ADR-0226 committed, Phase A step A1 landed (CI green); two new UX decisions pending before A3/A4

(Written in English — user decision 2026-09-24: handoffs/attachments may be English.)

## One-line state

★**Design is done and committed (TRD `767a774` after 5 review rounds · ADR-0226 `3d16a3c`). Phase A step A1 (behaviour-neutral skeleton) landed as `56a4e98` after `/review code deep` ×2 + `/qa full` PASS; master (`16e1bca` layout-bugs) merged in as `451ac84`; branch CI green. Next = A2, but the user raised two UX changes at the very end that must be decided before A3/A4.**★

## Next first actions

1. ★**Settle the two user items first (they change A3/A4, not A2)**★ — ask one at a time:
   - **U-A — termination veil lets interaction through.** `src/components/slot/SlotUnavailableVeil.tsx:36-37` is `pointer-events-none absolute inset-0 … bg-background/70` by design (ADR-0148 — 「아래 조작을 막지 않는다 — 입력 비활성은 각 슬롯의 disabled 가 담당」, RichSlot `:389` `disabled={agentUnavailable}`, `:405` veil). User (2026-09-24): 「전원 표시 나오는 거는 뒤에 막아야되는데 안막고 있어 이거 수정해야」. Decision point = what to block (clicks/buttons behind vs keep scroll/copy of a dead agent's transcript?). ADR-0148 amendment. Ask the user what they were able to operate behind it before designing. Applies to all three slot kinds (the veil is shared).
   - **U-B — loading look + abnormal case.** User leans: full-slot veil like the termination veil (not the TRD's centred icon with active input), input blocked while loading, and for the abnormal case (history never arrives) **guide "끄고 다시 켜기" instead of inducing typing** (「입력해서 유도하기 보다는 그냥 끄고 다시키는걸 유도하는게 더 좋지 않나」). This conflicts with committed decisions Q2=(a) (loading until input) and the layout (가) (ADR-0226 decisions 8·9, TRD §3-5 ~l.244-263, §3-6). It needs an exit/detection signal without a timer (ADR-0038 · `0145:26`). Options to present:
     - (i) lift the veil on the first live frame of any kind — codex sends `Usage` right after `thread/resume`, history follows +20–70 ms (measured 2026-09-23), and that frame arrives even if the history fetch fails → no stuck state for codex. ★Unverified★: whether a claude resumed incarnation with 0 history (transcript unreadable) emits any frame before input. Cannot detect "abnormal" → no restart guidance.
     - (ii) reopen Q2 (b): a per-incarnation "history done" signal (wire field → changes the A3 contract in TRD §6-5 and needs an ADR amending ADR-0226 d8·d9). Enables the "불러오지 못했습니다 — 끄고 다시 켜 주세요" guidance. Restart itself already has an LLM path (`agent.kill` + `spawnProfile`), so a button adds no new control surface (CLAUDE.md 「LLM-우선 제어」).
     - (iii) keep the committed design (conversation-area veil, input bright).
     - After the decision: amend TRD §3-5/§3-6/§6-5 and record via `/adr` (new ADR amending 0226 — don't overwrite 0226 in place).
2. **A2 can start in parallel with the U-A/U-B discussion** (agent crate only — `/implement … critical`). Scope = TRD §6-1 A2 row (l.413). Read only: §3-2 (l.59-171), §4-1 (l.303-344), §4-2 (l.345-358), §6-3 (l.429-443), §6-5 (l.448-463) — the A1 coder spent ~67k tokens reading the whole 537-line TRD.
   - ★Run the Q4 pilot first★ — does a PATH-override fake `claude.cmd`/`codex.cmd` work under `cmd /c` for the D1 end-to-end test? If not → user decision on a production test seam (ADR-0012; TRD §4-3 currently says "none").
   - A2 must: delete the four `#[cfg_attr(not(test), expect(dead_code, …))]` attributes it starts using (otherwise `unfulfilled_lint_expectations` warns — intended tripwire); map `release_session_id` → `None` to `profile_vanished_mid_spawn`; update the `replace_session_id` doc paragraph (`profile.rs` ~855-859) when `clear_session_id`/`new_session_id` are removed or migrated; reconsider `AgentManager::agent_epoch` (no production caller after A1 — only `tests/ws_e2e.rs`).
3. A3 (wire+shell) and A4 (frontend) in parallel after U-B is decided. A5 = docs + CLAUDE.md (ownership-split invariant line; workspace test count now **58 / 2764 / 0 / 20**) + anchors + GUI trials 1–9 (TRD §4-4). ADR-0226 is already written — A5 no longer writes it.
4. When Phase A is complete: integrate to master with the CLAUDE.md merge-tree procedure (no checkout of master).

## Repo state (checked at save time)

- Branch **`v0.3.2/fix/resume-after-first-turn`**, pushed. Commits: `767a774` TRD · `b593d30` ADR-0226 number reservation (+8 amend stamps) · `3d16a3c` ADR-0226 body + step-log · `56a4e98` A1 · `451ac84` merge of master `16e1bca` · + the handoff commit that carries this file.
- CI run `35971548680` on `451ac84`: frontend / backend / fmt+isolation all **success** (the A1-only run was cancelled by the merge push — the merge run covers it).
- This handoff commit also adds the previously untracked handoff files: attachments `20260924-resume-trd-{context,review-r1,-r2,-r3-findings}.md`, history `20260923-144358-…md` and `20260924-111837-…md`.
- ★Shared config repo `I:\Engram` is on `skill-factory/staging` — user: 「staging 수정중이야 건들지마」. I did not touch it this session. The previous session's uncommitted edits there remain (see `history/20260924-111837-…md`).★
- Scratchpad of this session holds nothing needed (review diffs/payloads only).

## Done this session

- Q5 marked decided; TRD round-2..5 reviews (`/review trd deep`): r2 FIX → r3 FIX → r4 FIX → r5 PASS ×3. Consolidated findings = attachments r2/r3 (r4/r5 were small and are in the TRD/commit message).
- User decisions: Q1=(a) `/clear` new id persisted immediately (the one deliberate D1 exception) · Q2=(a) · layout (가) · Q5=(a) · handle-less Resume → Fresh as a consequence of D1 (user 「ㅇㅇ」). ★Q2 and the layout may be reopened by U-B.★
- ADR-0226 written ahead of TRD step A5 (「결정 즉시 ADR」 + number reservation), `/review doc light` FIX → PASS. Stamps on 0008·0076·0082·0145·0185·0216·0217·0218 relabelled so header-only readers aren't misled.
- A1 (`56a4e98`): `session_id_latch.rs` (new), `submits_turn`, `commit_session_id`/`mint_session_id`/`release_session_id`, `Incarnation`/`SubscribeReply`, session builders + note sites (no-op without a latch), `subscribe_from` reply, daemon `handle_subscribe` epoch from the reply (SubscribeFailed reason text for a missing agent changed — human-only string). Production behaviour unchanged.
- Merged master into the branch; conflicts only in `docs/decisions/README.md` (index — regenerated) and `docs/process/step-log.md` (kept both). adr lint errors 5 → 1 (0170 gap only, pre-existing).

## Verification state

- **Verified:** A1 `/review code deep` round 2: codex PASS · doc-aware PASS · concurrency PASS (mutation tests confirmed the new latch tests fail on the targeted regressions; ~32k stress rounds 0 failures). `/qa full` PASS: workspace 58 result lines / 2764 passed / 0 failed / 20 ignored (+40) · shell targets 5 · fmt · isolation gates · ts-rs sync · tsc · npm 953 · GUI probe on an isolated instance with real claude + codex, 6 scenarios, behaviour unchanged (codex resume splash ≈1.37 s — expected until A4). Branch CI green on `451ac84`.
- ★**Not verified:**★ the merged tree was not run locally (CI only) · claude 0-history incarnation: does any live frame arrive before input (U-B option i) · the Q4 PATH trick · real codex cold start > 10 s (carried) · a `session_tracker: 세션 파일 미발견 — 추적 degraded` WARN appeared twice in the QA probe (file untouched by A1; not compared with HEAD).

## Do-not / traps (bit this session)

1. ★**ADR numbering sees only this worktree's files.**★ Before `/adr new|supersede`, scan every ref: `for r in $(git for-each-ref --format='%(refname)' refs/heads refs/remotes); do git -c core.quotepath=false ls-tree -r --name-only "$r" docs/decisions | grep -oE '^docs/decisions/[0-9]{4}' | sort | tail -1; done` (without `core.quotepath=false` the Korean filenames are octal-quoted and silently skipped). If another branch holds higher numbers, copy those files in temporarily, run the script, delete them — then commit + push the scaffold at once.
2. **Reviewer/QA subagents leave temp worktrees and multi-GB target dirs in the scratchpad** (their cleanup is permission-denied). Main must `git worktree remove --force <path>` + `rm -r <dir>` afterwards (removed ~8 GB this session).
3. **Blind codex reviewers flag accepted design residuals** (TRD r5 RichSlot draft loss; A1 note-before-send). Resolve by showing the rationale AFTER its blind review and asking PRE-EXISTING/WITHDRAWN — it withdrew both times. Don't decide it yourself; don't feed rationale before the blind pass.
4. **GUI probe isolation** (the qa binding §full still lacks it): `scripts/launch-detached.ps1 -Exe <daemon> -EnvVars 'ENGRAM_DATA_DIR=…','RUST_LOG=…'` FIRST, then the client with the same `ENGRAM_DATA_DIR`; teardown client first, then daemon by PID. ★`RUST_LOG=debug` on the daemon is inherited by the codex child → 30 MB in 4 min; scope it to `engram_dashboard_agent=debug,engram_dashboard_daemon=debug,engram_dashboard_lib=debug`.★ (`launch-detached.ps1` header says `-Env` but the parameter is `-EnvVars`.) `MSYS_NO_PATHCONV=1 taskkill /PID <pid> /T /F` worked for the QA worker this time.
5. The `nmfc-origin-dev-harness` UserPromptSubmit hooks (handoff trigger, wiki pre-consult) misfire constantly — ignore (handoff flow §0).
6. Don't fix the flicker with frontend-only signals (carried — ADR-0226 「거부한 대안」 now records both reverted attempts).
7. For doc reviews, have the author emit a claims TSV (`path<TAB>line<TAB>token`) and verify it with a shell loop — it removed pointer checking from reviewer cost this session.

## Stop conditions

- U-A / U-B decisions (user-visible). Any new production test seam (Q4 → ADR-0012).
- Reviewer lenses in direct conflict. `v*` tag push. Killing a daemon that hosts the user's agents.
- Phase B needs Q3/Q6/Q7/Q8 first.

## User check list (after A4 — tell the user then)

1. codex resume (kill → reactivate): no "new agent" splash flash; loading visual per the U-B decision, then the history.
2. An agent killed before any conversation: reactivates as a new agent instead of failing (from A2).
3. A fresh agent still shows the "new agent" first screen.
4. claude resume: unchanged, no flicker.

## Side notes from QA (not A1)

- ADR-0130 re-open trigger fires: production edges in `crates/engram-dashboard-daemon/src/control/` (`commands.rs:27`, `mcp_server.rs:41`, `registry.rs:246`) — pre-existing; the user decides whether to re-open ADR-0130.
- qa binding drift to fix via `/review doc`: isolation recipe, teardown order, scoped `RUST_LOG`, a read-only ts-rs sync variant (`git add -N -f` touches the index), a JSON-agent chat probe recipe (fresh-screen marker `data-rich-empty`, textarea native setter + Enter keydown, re-attach via `slot.empty` → `layout.setSlotContent`).

## Improvement notes (NOT recorded — the skill folders live in the staging repo the user said not to touch)

- `/adr`: numbering should scan all refs, not the local folder.
- `/review`: author-emitted claims TSV (applied, worked); an "ADR clause ↔ change" matrix for ADR amend lists.
- `/implement`: give coders TRD section line ranges instead of the whole TRD.
- `/qa` binding: the GUI chat probe recipe above.

## Carried over, untouched

- Previous handoff decision 7: priming draft attachment disposal · leader hold-queue cap/overflow · stale failure records drawn as 「막힘」 · deleting real `~/.codex` session records · ADR-0014/0022 still 「제안」.
- Close ADR-0220's open item via `/adr` (MCP tool descriptions stay Korean — user decision 2026-09-23).
- Backlog: slot content/focus read-back command (PRD first) · main-window webview ghost (investigate on recurrence) · check `gpt-6-sol` effort on next codex call · TRD §3-6 PRE-EXISTING: a rejected chat submission clears the draft with no visible error (RichSlot `:249-277`).
- TRD §3-2-2 residual ① wording nit: the window is "intent check → one agents.json commit write → send", not "one check".

## Files to read

- `docs/process/S21-codex-backend/trd-resume-after-first-turn.md` — §6-1 (l.408-416), §6-5 (l.448-463), §3-5 (l.244-263) for U-B, §7 (l.467+)
- `docs/decisions/0226-세션-id-는-첫-대화-뒤에만-영속하고-이어받는-화신은-구독-응답으로-로딩을-띄운다.md` — decisions 8·9 (U-B), 「영향」 first bullet
- `docs/decisions/0148-*.md` + `src/components/slot/SlotUnavailableVeil.tsx` — U-A
- `docs/process/step-log.md` tail — this round's entry
