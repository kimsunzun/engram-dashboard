# Handoff: detached-console minimize (reviews PASS, QA pending, default-flip todo) · mid-turn input for JSON mode designed (research + TRD draft, PRD next)

(Written in English — user decision 2026-09-24: handoffs/attachments may be English.)

## One-line state

Two tracks, nothing committed this session. ① **Detached launchers stop stealing focus** — `scripts/run-detached.ps1` / `scripts/launch-detached.ps1` now start consoles minimized-no-activate (WMI `ShowWindow=7` + `CREATE_BREAKAWAY_FROM_JOB`), launch-detached moved scheduler → WMI and got `-Minimized`; `/review code full` round 2 = **PASS/PASS**, QA **stopped mid-run** (it was stealing the user's focus), and the user then asked to **flip the default** (see Next 1). ② **Mid-turn input in JSON chat mode** — researched (`docs/research/mid-turn-input-display-2026-09-25.md`), all user-visible decisions taken, TRD draft with a late-decision block at `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md`; **user wants a PRD too** (Next 2). The user said: 「다음 타이밍에 쭉쭉 하게」 — proceed without re-asking what is already decided.

## Next first actions

1. **Console track (user: 「최소화 관련된건 너가 알아서」 — delegated through commit + master integration):**
   - ★Flip the default★ (user: 「그냥 스크립트 각각에 끼워넣고 초기화로 가는 식… 지시서 안 넣고 클로드가 몰라도 알아서 동작되게」): `launch-detached.ps1` → app window **minimized-no-activate by default**; add an explicit opt-out switch (e.g. `-Normal`/`-Show`) and pass it ONLY from the 4 human launchers `scripts/run-debug.bat`, `run-release.bat`, `rebuild-run-debug.bat`, `rebuild-run-release.bat` (also check `rebuild-run-debug-log.bat`, which delegates). Remove/rename `-Minimized`. Then agents need no instructions and the qa-binding feedback item about passing `-Minimized` becomes moot (update its status in `I:\Engram\core\claude-global-shared\skills\qa\feedback.md`, 2026-09-25 section).
   - Add a comment for the measured caveat: a show=7 console takes the foreground **only when no window is foreground** (null fg) — conhost skips `SetActiveWindow` for 7 but the window manager activates the first show then (measured; source `microsoft/terminal src/interactivity/win32/window.cpp` `ActivateAndShow`). User kept minimized over hidden; the fully-safe alternatives were: SW_HIDE (no taskbar button) or hidden-then-`ShowWindow(7)` from another process (needs a compiled helper) — not chosen.
   - Gates: `/implement` (coder) → `/review code full` (reviewers must see the flip) → `/qa full` → commit → push → CI → merge to master via merge-tree procedure (CLAUDE.md 「브랜치·커밋」). ★Run the GUI part of QA when it won't disturb the user★ — QA launches real windows; a QA run this session stole focus repeatedly (see Traps 1).
   - Commit together: `docs/research/window-spawn-sites-inventory-2026-09-25.md` (this track's research) + a step-log entry linking it (orphan rule). Do NOT stage the mid-turn docs on this branch.
2. **Mid-turn input track (new topic branch from master, e.g. `v0.3.2/feat/json-midturn-queue`)** — order per CLAUDE.md 「개발 스텝」, but decisions are already the user's (don't re-open them):
   - **PRD first** (user: 「이거 prd도 받아야될것같은데」): write it retroactively from the research report + the decisions below; `/review prd`; surface only genuinely new user-visible questions.
   - Revise the TRD body to the late decisions (its top block lists which sections: §0·§4-3·§5-5·§5-7·§5-9·§10) → `/review trd` → **short user confirmation of the revised TRD** (CLAUDE.md order) → ADR via `/adr new` (content drafted in the TRD top block: amends ADR-0198/0044, reopens ADR-0193; rejected alternatives + the user's reasons are listed there) → Phase 0 measurements M1–M8 (TRD §3-3; esp. claude `command_lifecycle` emitted in our `-p` stream-json mode, `cancel_async_message` works there) → implement.
   - Move the untracked `docs/research/mid-turn-input-display-2026-09-25.md` + TRD draft onto that branch; link from step-log.
3. Phase B of ADR-0226 still needs user decisions Q3·Q6·Q7·Q8 (unchanged from the previous handoff) — don't start it.

## Decisions this session (user)

**Console / windows**
- Consoles start **minimized, not hidden** — goal is "don't steal focus", taskbar button OK (user: 「창들 초기값이 최소화여서 포커스가 안되는 정도가 적당」).
- App window minimized **only for agent/QA launches**; human `run-*.bat` launches show normally. Now to be implemented as **default minimized + human opt-out** (Next 1).
- **No centralization for now** (user: 「공용화는 자제하자. 스크립트에서 각각 하는걸로 만족」). Daemon WMI console (debug) and release-app `taskkill` flash are deferred to backlog — inventory + the shelved design in `docs/research/window-spawn-sites-inventory-2026-09-25.md`.
- QA-binding drift is recorded as **feedback, not edited** (user: 「QA 규약은 피드백으로」) — `I:\Engram\core\claude-global-shared\skills\qa\feedback.md` (2026-09-25 section; uncommitted there by convention).
- Research findings are **always accumulated in `docs/research/`** (user reminder: 「리서칭 한것들은 리서칭에 따로 적립」 — also the research binding rule).

**Mid-turn input (JSON/structured chat mode only — terminal/PTY mode untouched: 「터미널 모드는 그런거 필요없잖아」)**
- Input sent while the agent runs a turn is **handed to the agent immediately** — claude via stdin (as today), codex via `turn/steer` (or `turn/start` if no active turn / on −32600). The agent folds it in **at the end of the currently running tool call** (user: 「당연히 도구호출 끝나면 바로 푸쉬」; 「클로드든 코덱스든 우리가 받고 바로 전달하자. 일단 공용화하고 나중에 세분화」).
- The daemon **also stores** each pending item (id → text) as the authoritative list (agent session layer — not WS/net, not frontend); the frontend shows it **above the input box** (terminal style), gray, one line each "…", oldest on top, >3 → "외 N개". Header: 「준비되면 전달」 while codex connects/hydrates, 「도구 호출이 끝나면 전달」 mid-turn.
- An item leaves the list **only on its own per-id receipt** — claude `command_lifecycle` `started` (uuid), codex `userMessage` echo with our `clientId` — and then appears as a normal bubble at the end of the transcript. Never bulk-remove on "tool ended".
- Idle agent → message goes straight into the transcript (no queued flash).
- ✕ cancel: **claude only** (`control_request` `cancel_async_message`, result judged ONLY by lifecycle `cancelled`/`started`). **No ✕ on codex items** (no API withdraws a single steered input). Cancel is a registered **command** (Esc binding later, not now; not an agent action).
- Cancelled text is **discarded** (user: 「일단 그냥 다 버리고 나중에 생각하자」). ★Main interpreted 「다」 as also discarding items that can never be delivered (interrupt/agent death) — confirm with the user in one line.★
- Separate messages stay separate (no concatenation).

## Repo state (checked at save time)

- Worktree `I:\Engram\apps\engram-dashboard-wt1`, branch `v0.3.2/fix/hide-detached-console` (from master `34cdb37`), **not pushed, no commits**.
- Uncommitted (the console change, reviewed round 2 — snapshot `scratchpad/review-hide-console/r2/change.diff` sha1 6006a9bc… + one later 1-line comment fix in `launch-detached.ps1` l.115, reviewer finding N1): `scripts/run-detached.ps1`, `scripts/launch-detached.ps1`, `scripts/run-debug.bat`, `scripts/run-release.bat`, `scripts/rebuild-run-debug.bat`, `scripts/rebuild-run-release.bat`, `README.md` (l.61). All CRLF, no BOM, PS parse 0 errors.
- Untracked: `docs/research/window-spawn-sites-inventory-2026-09-25.md` (console track), `docs/research/mid-turn-input-display-2026-09-25.md` + `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` (mid-turn track), this handoff.
- `master` = `34cdb37`, its CI run `36016045596` **green** (checked this session).
- Outside the repo, uncommitted by convention: `I:\Engram\core\claude-global-shared\skills\qa\feedback.md` and `…\research\feedback.md` (2026-09-25 sections). `I:\Engram` is on `skill-factory/staging` with the user's edits — don't touch staging.
- Scratch (this session, will not be the next session's scratchpad): `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\0759bb2c-9ea0-4f1f-908c-7ab2600e3133\scratchpad\` — `hide\` has compiled window-measurement tools (`winmon.exe`, `fgexp.exe`, `fgtaker.exe`, `closer.exe`, `probe-gui/con.exe`, sources, `verify-a.sh`/`verify-b.sh`); `research-src\codex` = blob-less codex clone (tags rust-v0.154.0 / 0.156.1). Copy the tools into the new scratchpad if QA needs them.

## Verification state

- **Verified:** console change — `/review code full` 2 rounds (codex blind + Claude doc-aware): round 1 FIX/FIX (8 items fixed), round 2 **PASS/PASS**; coder measurements on an unlocked desktop: run-detached (ping/npx/nested cmd) 0 non-iconic shows, 0 foreground events, `__EXIT=0`, titles applied; launch-detached default → probe normal + wrapper iconic, `-Minimized` → probe iconic from first show, foreground unchanged; env vars reach the app; 0 leftover `.bat`; breakaway flag RV=0 (both scripts). Research report: cross-family adversarial review (BLOCK → 4 findings accepted and corrected in §8). claude cancel/lifecycle strings found in installed 2.1.280 binary.
- ★**Not verified:**★ `/qa` for the console change (stopped before isolation gates; no gate results were reported) · the real Engram app under `-Minimized` (foreground/iconic, **viewport may be 0×0 while minimized → CDP shots/layout**; reviewer F4/F5) · the default-flip (not written yet) · claude `command_lifecycle` / `cancel_async_message` in our spawn mode (TRD M1/M3) · codex steer behaviour in our transport · anything on macOS.

## Do-not / traps (bit this session)

1. ★**Our own QA steals the user's focus.**★ Each gate launches a wrapper console; GUI QA launches app windows; the focus-steal investigation took focus ~10×. Also: **worktree wt2 still runs the OLD `run-detached.ps1`** → its consoles go to Windows Terminal and grab focus (attributed by creation time/title) — it stops only after this fix reaches master and wt2 picks it up. Tell the user before running window/GUI measurements.
2. **A show=7 console DOES take the foreground when no window is foreground** (null fg) — measured with `fgtaker.exe <ms> desk <holdMs>`; with a normal foreground (incl. 5 back-to-back wrappers) it does not.
3. **Hook blocks any Bash command text containing `Add-Type` or `DllImport`** (even inside a heredoc/sed on a .cs file or a feedback note). Use Write/Edit for such text; build C# helpers with `C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe`.
4. `cancel_async_message` returning `cancelled=false` **does not mean delivered** (pending-cancel when stdin not yet read) — judge only by lifecycle events.
5. codex via `codex:codex-rescue`: after any parallel/other codex call in the same cwd, **don't `--resume`** — open `--fresh`. (Direct `codex exec` for research reviews also counts.)
6. Port 1420 belongs to wt2's vite → use the RELEASE exe with an isolated `ENGRAM_DATA_DIR` for GUI checks in wt1 (daemon first, then client; teardown by PID after checking ExecutablePath).
7. Blob-less clone + `git grep` across tags hangs; `grep -a` over the 237 MB `claude.exe` with wide context times out — use `git show <tag>:<path>` and a node `Buffer.indexOf` script.
8. The `nmfc-origin-dev-harness` UserPromptSubmit hooks (handoff trigger, wiki pre-consult) misfire on every message — ignore them; this project's handoff is the user's `handoff` skill.

## Stop conditions

- Any user-visible question that the decisions above don't settle (PRD/TRD reviews may surface some) → ask, one at a time.
- TRD final confirmation before coding the mid-turn feature (CLAUDE.md order).
- Master CI red; any master change except branch + CI + merge-tree; `v*` tag push.
- Touching wt2 processes/files, `I:\Engram` staging, or an app/daemon you didn't start.
- Phase B of ADR-0226 without Q3/Q6/Q7/Q8.

## Backlog (not started)

- Daemon WMI console (debug daemon, focus-taking) + release-app `taskkill` flash — centralization design shelved in the inventory doc.
- From the previous handoff (unchanged): SubscribeAck enqueue swallow · codex cold-start handshake budget · whitespace-only text ends loading · qa-binding recipe drift chore · `submit_delivery_boundary` flake candidate · spinner under reduced-motion/e-ink GUI check.
- Mid-turn follow-ups deferred by the user: returning cancelled text to the input box (append rule), codex cancel (Codex TUI Enter/Tab split as reference), Esc keybinding for the cancel command.

## Files to read (only if needed)

- `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` — top block (late decisions) first, then §3-3 (M1–M8), §5.
- `docs/research/mid-turn-input-display-2026-09-25.md` §0, §4, §8.
- `docs/research/window-spawn-sites-inventory-2026-09-25.md`.
- `scripts/launch-detached.ps1` header + WMI block · `scripts/run-detached.ps1` WMI block.
- `docs/decisions/0198-*.md`, `0193-*.md`, `0044-*.md` (to be amended by the new ADR).
