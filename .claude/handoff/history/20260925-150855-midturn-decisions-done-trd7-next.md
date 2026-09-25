# Handoff: console fix landed on master · mid-turn input — all user Q&A answered (codex ✕ via hold-until-earliest-boundary), TRD 7판 next

(Written in English — user decision 2026-09-24: handoffs/attachments may be English.)

## One-line state

Two tracks. ① **Console minimization: DONE** — merged to master (`b008b37`), branch + master CI green; the app-window part was dropped (a script cannot minimize the Engram app — measured). ② **Mid-turn input (JSON chat mode): design phase, no code.** PRD 4판 + TRD 6판 + Phase 0 measurements are committed **locally** on `v0.3.2/feat/json-midturn-queue` (not pushed). This morning the user answered **every** open question (PRD §9 Q1–Q9, TRD §10-2/§10-4 N1–N6) — **those answers are NOT yet written into PRD/TRD.** Next = TRD 7판 (+ PRD 5판) with the answers → `/review trd` → user confirms TRD → `/adr new` → `/implement`.

## Next first actions

1. **Write today's decisions (below) into the docs** — delegate (fresh `worker-senior`; TRD is 773 lines, give section line ranges): PRD 5판 (§3 rules, §5 ACs — esp. AC10 codex ✕, AC22 mail, header removal; §9 answered → closed) and TRD 7판. Biggest TRD change = **codex "hold until the earliest boundary" (B)** replacing codex immediate steer as the target design, built as a swappable policy inside the codex transport (proposed name `HandOverPolicy`: `Immediate` | `AtEarliestBoundary`). Also: remove header line everywhere; no notices + remove a rejected Direct bubble; Q7 optimistic per-window removal; Q8 codex rule; mail idle rule; N1 (a), N4 bump, N5 (a) closed; Q5 out of scope.
2. **Measurements before/with TRD 7판** (implementation depends on them): **M15 daemon reaction latency** (codex tool `item/completed` → our steer written; window measured 6–64 ms) · **codex text-only "answer end" boundary** (is there a window like the tool one? if holding there would delay delivery, hand over immediately there, no ✕) · the TRD §3-3 Phase 0b list (M9–M14) as needed. Harness + raw logs preserved at `.claude/handoff/attachments/20260925-midturn-phase0/` (copied from the volatile scratchpad).
3. `/review trd full` on 7판 → short user confirmation of the TRD (CLAUDE.md order) → `/adr new` (amends/links per TRD §8; rejected alternatives per TRD §10-5 + today's) → `/implement` (tier **critical** — concurrency/lifetime) in the order: common layer + claude path → codex with `Immediate` policy skeleton → measure → codex hold policy + ✕.
4. Push `v0.3.2/feat/json-midturn-queue` when implementation starts (docs-only now).

## Decisions — 2026-09-25 morning Q&A (user; authoritative — don't re-open)

Shared model the user confirmed: **one daemon-held pending-input list for both backends** (both "cache" it; every window + LLM reads it; reattach = one `listQueuedInputs`, then live events). Same cancel entry for both. Per-backend differences only: **when to send** (claude = immediately to stdin; codex = held by us until the agent can take it) and **cancel internals** (claude = vendor cancel request → result; codex = delete the held item).

1. **codex ✕ is IN scope (B).** Principle (user): **"deliver as early as the agent can take it"** — hold only while holding does not delay delivery; ✕ while held; hand over at the boundary instantly. If holding would delay in some segment, hand over immediately there (no ✕ there). Tool end = measured OK (10/10, window 6–64 ms); daemon latency unmeasured (M15); text-only answer-end unmeasured. If M15 can't meet the window → stop and ask. Esc-to-cancel later. User explicitly corrected me: B is not "a later release", it's part of this feature; order is mine.
2. **Q1 notices: none** (add later if needed). Items that can't be delivered just leave the list. **Q1④:** an idle-sent message is drawn immediately (codex synthetic echo per ADR-0198; claude after write accepted) — if it is then rejected, **remove that bubble silently** (user: "거절하면 우리쪽 채팅도 지워져야"). Premise that such a bubble stays today = unverified.
3. **Q2:** dissolved by B (items held during codex connect/restore get ✕ the same way).
4. **Q3:** 「외 N개」 expandable · collapsed shows oldest 3 · tooltip full text. "나중에 조정".
5. **Q4 + N2: NO header line at all** ("한 줄 낭비") — gray items only, Claude Code style. ★Reverses last session's header decision (「준비되면 전달」/「도구 호출이 끝나면 전달」).★
6. **Q5: turn-stop control = separate feature, next after this one.** Facts found: codex transport implements `interrupt` (`turn/interrupt`) but no chat UI caller; claude `StdioTransport::interrupt` returns Unsupported (`crates/engram-dashboard-agent/src/transport/stdio.rs:371`, ADR-0044 MVP); claude CLI advertises `interrupt_receipt_v1` / `interrupt_cancel_queued_v1` (Phase 0 M4 init) → probably one control line away (unmeasured).
7. **Q6:** codex — conversation resumed but on-screen history restore failed → hand over anyway. claude — input while a resumed conversation loads → list + hand over immediately (claude handles after load; M14 unmeasured).
8. **Q7:** ✕ → item disappears **immediately** in the clicking window, never reappears; late case (agent took it first) → it just shows up as a chat bubble (user: fine). Other windows/LLM keep "cancelling" until the result. Tab + tooltip 「취소 — 글은 버려짐」: yes. codex hint line: no.
9. **Q8 error-ended turn:** codex — pending items are **not** auto-sent; they wait (✕ available) and go **with the user's next input**, in one turn, pending first. No auto-push after token recharge (user: doesn't want the session touched). claude — **follow the vendor naturally, don't force it** ("억지로 구현하지 마"); list stays in sync via per-item lifecycle events. Already-bubbled unanswered messages → leave.
10. **Q9:** leftovers at turn end → **one next turn together**, separate messages (codex; claude = vendor decides).
11. **N1:** old vendor versions → today's behaviour, no special support ("알아서 업데이트").
12. **N4:** `PROTOCOL_VERSION` 5 → 6 approved.
13. **N5:** client-side seq re-ordering (keep the ADR-0006 invariant) — user agreed after the concrete example (pump thread `Delivered{X}` #51 vs input thread `Queued{Y}` #52 arriving reversed).
14. **N6 / mail:** **user input has priority over mail.** Mail "idle" = **no turn AND user pending list empty** (same idle definition as user input) → mail waits until user items are in and that turn ends; mail may be delayed (OK — incl. after an error while user items wait). **Terminal (PTY) mode unchanged** — mail poured in as today. PRD AC22 changes from "same timing as today" to "mail may be delayed behind user input".
15. (Night, confirmed) undeliverable items (interrupt/agent end) are discarded.
16. **Console:** app-window minimization → later (user: "나중에 보고").

## Repo state (checked at save time)

- wt1 on `v0.3.2/feat/json-midturn-queue` (from master `b008b37`), HEAD `0c91640`, **local only**. Commits: `6a88aaf` (PRD 4판 · research `mid-turn-input-display` · Phase 0 report), `0c91640` (TRD 6판 · step-log entry). This handoff + the Phase 0 attachment are committed on the same branch (next commit).
- **master moved on since my merge:** now `7a14b29` = `origin/master` (another worktree merged ADR-0227 flat split renderer). The midturn branch is based on `b008b37`; merge master in before implementing.
- `v0.3.2/fix/hide-detached-console` merged (`b008b37`), kept (not deleted). CI: branch run `36063506903`, master run `36064539524` — both green.
- Outside the repo, uncommitted by convention: `I:\Engram\core\claude-global-shared\skills\qa\feedback.md` (2026-09-25 items: -Minimized void, WebView2 profile not isolated in §full, installed-release single-instance capture, probe-vs-real-app lesson, statuses) and `...\skills\implement\feedback.md` (grep CR miscount, probe lesson). Don't touch `I:\Engram` staging.

## Verification state

- **Verified:** console fix — `/review code full` 4 rounds (final PASS/PASS) · `/qa full` PASS (separate-run gates fmt/tsc/vitest 62 files 1010 tests/exit-3 propagation, isolation gates, release-app GUI: wrapper consoles iconic, app normal 1280×800, CDP eval+shot OK, log redirected) · CI green. Phase 0 M1–M8 measured (claude 2.1.280, codex 0.156.1; ≈$0.28). PRD `/review prd full` 3 rounds; TRD `/review trd full` 4 rounds (4th = "no structural problem, converging" + closure PASS).
- ★**Not verified:**★ foreground (focus) behaviour of any window under WMI launch on an active desktop — the user session was disconnected all night (`GetForegroundWindow` = NULL) · M15 daemon latency · codex text-only boundary · M9–M14 · TRD 6판's claim that the replay-complete marker stays fixed-length (marked 미확인 in the TRD) · everything on macOS · today's Q&A decisions are not reviewed as a design yet (they land in TRD 7판).

## Do-not / traps (bit this session)

1. ★**Git Bash `grep -c $'\r$'` lies about line endings**★ — coder and a reviewer both reported ".bat all CRLF" while they were LF (0 CR bytes). Use `git ls-files --eol <file>` (want `w/crlf`) or `tr -cd '\r' < f | wc -c`. LF `.bat` = cmd executes mid-comment text (`.gitattributes` header).
2. ★**A stand-in probe must be checked once against the real app**★ — the "tao-like" probe skipped tao's first `SW_HIDE`, so `-Minimized` "worked" on the probe and not on the app (STARTUPINFO show value is consumed by the first ShowWindow). Cost: 2 coder + 2 review rounds + a 220k-token spike.
3. **TRD editing:** no per-판 tags in the body — history goes in the header 「판 이력」 block, rejected alternatives in §10-5 (the 6판 editor spent most effort sweeping ~100 such tags). Give editors section line ranges; the whole TRD read is ~40k tokens.
4. **Worker context:** TRD writers reached 410–450k tokens per round; don't resume one near that (571k death precedent) — spawn fresh per round.
5. **API session limit** killed two agents mid-run overnight; `SendMessage` resume worked. After such a stop, check for leftover processes/temp files before resuming.
6. The `nmfc-origin-dev-harness` UserPromptSubmit hooks (handoff trigger, wiki pre-consult) misfire on every message — ignore; this project's handoff is the user's `handoff` skill.
7. `codex:codex-rescue`: always `--fresh`; never `--resume` after parallel codex calls in the same cwd.
8. Hook blocks Bash text containing `Add-Type`/`DllImport` → compiled C# helpers via `C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe` (spike helper `wp4.exe` source was in the old scratchpad `...6847046e...\scratchpad\spike-minimized\winprobe.cs` — volatile).
9. Release-build GUI QA: set `WEBVIEW2_USER_DATA_FOLDER` to an isolated folder (else it writes into the installed app's WebView profile) and make sure no app with the same identifier is running (single-instance hand-off hijacks the launch). Port 1420 = wt2's vite.
10. When explaining to this user: one question at a time, plain words, diagrams help; they catch loose wording (e.g. my "claude doesn't need a list" confused them — say "both cache the list; only the unsent payload differs").

## Stop conditions

- M15 (or the text-only boundary) shows our daemon can't hand over within the codex window → ask the user (B vs fallback) before coding codex hold.
- Any measurement contradicting a decision (PRD R-clauses) → ask.
- TRD 7판 must get the user's confirmation before `/implement`.
- Master CI red; any master change except branch + CI + merge-tree; `v*` tag push.
- Touching wt2 processes/files, `I:\Engram` staging, or apps not started by us.
- ADR-0226 Phase B still needs Q3·Q6·Q7·Q8 (older, unrelated to today's Q-numbers).

## Backlog (not started)

- Turn-stop control in the chat view — **next feature after mid-turn input** (see decision 6 facts). Esc to cancel a queued item.
- App window start minimized/unfocused for agent/QA launches (needs app code — tao `focused(false)` / show-then-minimize; the launcher cannot).
- Debug daemon console spawned by the app via WMI is still a normal window (inventory doc `docs/research/window-spawn-sites-inventory-2026-09-25.md`).
- `scripts/run-detached.ps1` never deletes its `%TEMP%\detached-cmd-<tag>.bat` (875 piled up).
- Other worktrees get the console fix only after merging master.
- qa-binding drift (scheduler wording l.198/l.215, WebView2 isolation, single-instance pre-check) — in qa feedback.md.
- From older handoffs: SubscribeAck enqueue swallow · codex cold-start handshake budget · whitespace-only text ends loading · `submit_delivery_boundary` flake candidate · spinner reduced-motion/e-ink GUI check.

## Files to read (only if needed)

- `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` (6판) — header 판 이력, §5-0 (ordered input path, `MidTurnPolicy`), §5-5 (codex), §5-7 (frontend, seq ordering), §10.
- `docs/process/S21-codex-backend/prd-mid-turn-input-queue.md` (4판) — §3, §5, §9 (all answered above).
- `docs/research/mid-turn-phase0-measurements-2026-09-25.md` + `.claude/handoff/attachments/20260925-midturn-phase0/`.
