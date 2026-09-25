# 핸드오프: mid-turn input — TRD 7판 through review round 5 · briefing published · 4 user decisions pending (N11·N12·N13·tooltip) · no code yet

(Written in English — user decision 2026-09-24: handoffs/attachments may be English. PRD/TRD stay Korean.)

## One-line state

Mid-turn input (JSON chat mode) is still in the **TRD phase — no code**. Today: PRD 5판 got every user answer; TRD 7판 was rewritten (codex "hold until the earliest boundary", ✕ always = "remove from list", acceptance-unknown items stay listed) and went through `/review trd full` **5 rounds** (r5: codex PASS · Claude FIX-light, applied in `9d8b680` but **not re-reviewed**). A review briefing (HTML) was published; the user is reading it and must answer **4 decisions** before anything else. User said: "핸드오프로 넘어가자" — next session starts from their answers.

## Next first actions (in order — user confirmed this order is not "code yet")

1. **Get the user's answers** to the briefing's 「결정해 주실 것」 (TRD §10-4 N11/N12/N13 + tooltip). Recommendations recorded in the TRD/briefing:
   - **N11** owed follow-up (empty) turn itself fails → (a) give up + warn, mail flows (current text) · **(b) rec** park like an error turn: no retry, paid by the user's next input, mail waits (downside: invisible wait).
   - **N12** ✕ on an `Unconfirmed` (acceptance-unknown) codex item → **(a) rec (tentative)** close at once, mail flows, late echo → bubble + answer · (b) hold as 「취소 중」 until outcome (hidden mail blocker + AC1 loss).
   - **N13** later messages behind an `Unconfirmed` item → (a) wait behind it (current tentative text; FIFO) · **(b) rec** overtake only that item; inversion only if its late echo arrives. PRD §3-4 sentence + AC2 exception needed under (b).
   - **Tooltip** 「취소 — 글은 버려짐」 is wrong for a handed-over codex item → candidates 「목록에서 빼기」 / 「목록에서 빼기 — 이미 넘어간 글은 대화에 나타날 수 있음」 / keep.
2. **Apply answers** to PRD + TRD (delegate `worker-senior`; PRD small → `worker-scout`). TRD edits touch §5-5 mostly (+ §0, §5-1/5-2, §5-9, §7, §8, §9, §10). Remove 「N11/N12/N13 대기」 markers. Add a header 판 이력 line.
3. **Round 6 `/review trd full`** on the result (covers the unreviewed r5 fixes + the answers). New session → codex slot `--fresh` (new thread). Hand reviewers the compact changed-rows file (tool below), not the raw diff.
4. User **TRD confirmation** (CLAUDE.md order) → `/adr new` (decisions + rejected alternatives = TRD §8 new-ADR list + §10-5; amend stamps 0044·0045·0190·0192·0193·0198·0226 etc. per §8).
5. **Merge master into the branch** (branch base `b008b37`; master now `083edba` — moved at least twice today by other worktrees) → push branch.
6. `/implement` tier **critical**, TRD §9 order: **P0 first = Phase 0b measurements M9·M10·M13 (no code; harness in `.claude/handoff/attachments/20260925-midturn-phase0/`)** → P1 → P4 → P5 → P3 → P2 → P7 (M15/M16) → P8 → P6.
7. Update the briefing (artifact + repo copy) if the user wants it kept current.

## Decisions made today (authoritative — recorded with user quotes in PRD §3/§6 and TRD §10-1/§10-4; don't re-open)

- ✕ hides the item in **all windows at once**; 「취소 중」 only in the LLM list.
- **✕ is always shown; pressing it removes the item from the list** ("x를 건드릴 이유가 있어? 그냥 x누르면 목록에서 기계적으로 빼는거잖아") — no per-segment ✕ gating; `cancellable` field removed. Handed-over codex item: ✕ removes it, late echo → bubble (late case, user: "상관없어").
- **N8 no cap** on mail blocking by a stuck user item ("사용자 글이 막히면 우편도 막혀야 … 글이 막히는것 자체가 이상한건데").
- **N7** claude input while a resumed conversation loads = today's behaviour (immediate bubble; explicit PRD exception) — no loading signal exists (`backend/claude/mod.rs:1042-1060`).
- **N9** idle-sent bubble whose agent ends before processing → leave it ("resume 하면 어차피 갱신될텐데").
- **N10** codex acceptance-unknown item **stays listed** (leaves only by receipt / ✕ / agent end — "목록에서 뺀다는건 이유가 있어야"). TRD stage `Unconfirmed{debt_turn}`.
- Internal (orchestrator, PRD-driven): a failed cancel request never re-shows the item (stays `Cancelling`); rejection after delivery also removes the bubble; codex held items get ✕ under both policies; failed turn never pays debt with an automatic empty turn; LLM row state gains `unconfirmed`.
- **M13 stop condition added**: if claude slash commands mid-turn never ack → stop before P3 and ask (N8's premise "stuck = abnormal" fails for a normal action).

## Repo state (at save)

- wt1 on `v0.3.2/feat/json-midturn-queue`, HEAD `c72209e` (+ this handoff commit), **local only, not pushed**. 21 commits since `b008b37` (PRD `b9f3021` `15a6796` `c3959be` `1341cac`; TRD `2f456f8`…`9d8b680`; briefing copy `c72209e`). Working tree clean.
- Briefing: artifact https://claude.ai/artifact/XBBqm9Lhz1JLgxaYb4fxHd (**private, bound to the account that published it** — the session switched to the company account `nm-fc.com` mid-way) · durable copy `.claude/handoff/attachments/20260925-midturn-briefing/midturn-briefing.html` (standalone, opens in a browser; reflects TRD `9d8b680`).
- Review helper preserved: `.claude/handoff/attachments/20260925-midturn-briefing/changed_rows.py` — `python changed_rows.py <base> <head> <path> <out>` → one entry per changed line with ±80 chars around each change (cut r5 reviewer input 345 KB → 22 KB).
- Outside the repo, uncommitted by convention: `~/.claude/skills/review/feedback.md` (appended 2026-09-25 wt1 entry: changed-rows payload, codex resume thread-check held 4/4). Earlier-session items in qa/implement feedback.md also still there.

## Verification state

- **Verified:** PRD/TRD text only — `/review trd full` r1–r5 (r1 FIX/FIX · r2 FIX/FIX · r3 FIX/FIX · r4 FIX/FIX · r5 **PASS (codex) / FIX-light (Claude)**); round-N findings were checked resolved by round N+1 reviewers. Briefing quotes were grep-checked against the PRD (one paraphrase marked as such).
- ★**Not verified:**★ the r5 fixes (`9d8b680`) are unreviewed · PRD 5판 never went through `/review prd` (only user answers written in; TRD reviewers read it as authority) · **nothing measured**: M9, M10, M13, M15, M16 (and M11/M12/M14) · whether a reload drops a vendor-unrecorded bubble (R11) · claude `started`→`refused` order (R10) · all code-level claims are code reads at HEAD, not runs · no GUI/QA of anything.

## Do-not / traps (bit this session)

1. ★**The TRD has single lines of 3–5 KB (table rows)**★ — whole file ≈ 90k tokens, §5-5 alone ≈ 30k. Every fix/review worker cost 230–430k tokens. Split fixes by section cluster and run them **sequentially** (same file), tell workers "don't read the whole file; python replace asserting a unique match", ask fixers to return final § + line numbers of every changed sentence, and give reviewers `changed_rows.py` output. Worker-noted improvement: splitting the §7-1 mega-cells into sub-bullets would make every later edit cheaper.
2. **Each fix surface spawned new edge findings** (esp. the `Unconfirmed` stage in r4/r5). Expect at least one more round after applying the answers.
3. ★**Don't settle user-visible behaviour by analogy or PRD literalism**★ — twice this session I did (N11 "give up" by analogy to Q8; N13 FIFO block from AC2) and reviewers escalated both to the user. If the PRD doesn't decide it and the user would see it → ask.
4. When explaining to this user: plain words + small ASCII diagrams; they push back on anything that "disappears without a reason" and on disabled UI. They rejected a publish-early offer ("아냐 다 되고 해") — finish, then show.
5. `codex:codex-rescue` blind slot: `--resume` worked r2–r5 when the prompt starts "state your previous turn in one line; if it isn't X, stop". New session → `--fresh`.
6. `nmfc-origin-dev-harness` UserPromptSubmit hooks (handoff trigger, wiki pre-consult) misfire on every message — ignore; this project's handoff is the user's `handoff` skill.
7. Git Bash python printing Korean: set `PYTHONIOENCODING=utf-8` (cp949 crash otherwise).
8. Artifact pages are account-bound — always keep a repo copy of anything published for the user.

## Stop conditions

- TRD must get the user's confirmation before `/implement`.
- M9 or M10 red → stop before P2 · M13 red → stop before P3 (re-ask N8) · M15 red (tool-end sum ≥ 6 ms) → stop before P8. (TRD §9 「멈춤 조건」.)
- Master CI red; any master change except branch + CI + merge-tree procedure; `v*` tag push.
- Touching wt2 processes/files, `I:\Engram` staging, or apps not started by us.

## Backlog (not started, carried over)

- Turn-stop control in the chat view — next feature after mid-turn input; Esc to cancel a queued item.
- App window start minimized/unfocused for agent/QA launches; debug daemon console via WMI still a normal window; `scripts/run-detached.ps1` leaves `%TEMP%\detached-cmd-*.bat` behind.
- ADR-0226 Phase B still needs its own Q3·Q6·Q7·Q8 (unrelated to today's numbering).
- Older: SubscribeAck enqueue swallow · codex cold-start handshake budget · whitespace-only text ends loading · `submit_delivery_boundary` flake candidate · spinner reduced-motion/e-ink check.

## Files to read (only if needed)

- `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` — header 판 이력 (what each round changed), §0, §9 (order + 멈춤 조건), §10-4 (N1–N13). Don't read whole.
- `docs/process/S21-codex-backend/prd-mid-turn-input-queue.md` (~250 lines) — §3 rules, §5 ACs, §6 rejected alternatives with user quotes.
- `.claude/handoff/attachments/20260925-midturn-briefing/midturn-briefing.html` — what the user is reviewing.
