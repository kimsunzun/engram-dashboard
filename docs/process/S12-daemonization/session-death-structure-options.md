# Structure options — a backend-agnostic abstraction of "agent death" (Slot ≠ Run ≠ Transport)

> **Conclusion (2026-06-16): the user adopted (A) → finalized as ADR-0017.** Slot = one session of one mode; when it ends, the slot ends too (no backend swap inside a slot, no zombie slot). A terminal (shell) is not saved/restored (no session to resume). The (B) slot-persist container model pushed by the consult was **rejected for domain confusion (in-place mode swap, dead pane).** However, from the consult agreement, "per-backend definition of death + exit-reason classification + KilledByUser excluded from restart + API stream ≠ death" is adopted. The (A)/(B) comparison and the OSS survey below are preserved as history. Details: ADR-0017.

- 상태: **Resolved → ADR-0017 (A adopted)**
- Basis: `/consult` job `20260616-011502-consult-death-abstraction` — GPT, Gemini and Claude (opus) blind + judge. Includes a survey of well-known OSS. Raw material `I:\Engram\agents\web-runner\shared\<job>\`.
- Link: the "sid instance" of ADR-0016 (agent lifecycle model) corresponds to the **Run** here — a structural correction that adds a **Slot** layer on top of it.

## Background — why "death" was ambiguous
Currently it is `cmd.exe /c claude`, so when claude exits cmd ends too and the PTY hits EOF → the **session (slot) disappears entirely.** So "claude leaving" and "slot disappearing" become one event. The user pointed out that "even if claude dies, the terminal/slot must remain, and it must be possible to start codex in that place instead" → "death" must be redefined as a backend-agnostic abstraction.

## ★ Cross-validation agreement (3 models + judge agree — nearly certain) ★
1. **Data model = three-layer separation Slot ≠ Run ≠ Transport.**
   - **Slot**: the UI/domain slot. Held by the daemon, persisted, long-lived. A stable `slot_id`.
   - **Run (= the sid instance of ADR-0016)**: a single execution (claude/codex/shell/api). Start to end (including code/signal). Short-lived.
   - **Transport**: PTY / HTTP stream / None. Bound to the Run.
2. **Define "death" not as the Slot's but as the Run's "end event".** A bad abstraction = "Session is dead" / a good abstraction = "the Slot is alive, the Run ended with a reason, and the Transport is Open/Eof/Closed."
3. **Exit-reason classification is mandatory** (EOF alone is insufficient to define death): Completed(0) / Failed(≠0) / Signaled / KilledByUser / InterruptedByUser / TransportLost / StartupFailed / RestoreFailed. (systemd·supervisord precedent)
4. **For an API, stream-end ≠ death** — a normal stream end = Completed, not Dead. (The core of backend-agnosticism)
5. **KilledByUser / Interrupted are excluded from respawn and automatic restart.** interrupt (Ctrl-C) and kill (process level) stay separate (ADR-0001 kept).
6. **Automatic restart is a future option** — under the constraint "UI = viewer · no automatic restore", for now only death is observed and respawn is an explicit trigger. (Turning it on needs backoff/FATAL/rate-limit — supervisord/systemd precedent)
7. **The abstraction (types/events) is laid down now as (B) three layers** (low risk, long-term interface, CLAUDE.md §0). **The actual behavior may stay at (A)** (gradual migration).
8. **★Windows implementation = not cmd /k, but "elevate the Rust Slot lifetime"★** — `cmd /k`, which keeps the shell alive, is dangerous (TUI raw-mode contamination · PTY EOF does not come → a child watcher is needed · the JobObject also catches the shell). Instead, **keep `cmd /c` as is**, and when the PTY hits EOF because the inner process ended, **do not drop Rust's `AgentSlot` struct and keep it as `state: Dead`** (the PTY is closed). This **preserves** the invariants of the EOF signal source, kill causality (ADR-0001) and finalize once (ADR-0005), while getting slot retention + respawn UX. (Isomorphic to tmux dead-pane — what died is the process; the slot remains in memory.)

## OSS precedents (survey)
| OSS | After the inner process exits | Re-run | Note |
|---|---|---|---|
| tmux `remain-on-exit` | keeps the dead pane | `respawn-pane -k` | SlotId kept, only the Run replaced (closest) |
| GNU screen `zombie` | dead window | resurrect key, `onerror` = abnormal only | precedent for distinguishing normal/abnormal |
| Zellij command pane | shows exit status · hold | Enter reruns, (A) via `close_on_exit=true` | (B) default + (A) option |
| VS Code terminal | keeps the (Terminated) tab | Relaunch button | dead slot + explicit re-run handle |
| abduco/dtach | session `+` (inner dies, session stays) | user re-spawns | **closest to the persist daemon model** |
| supervisord/systemd | Unit failed/dead, separated from PID | `restart` (explicit) | precedent for exit-reason classification and automatic restart policy |

## ★ Awaiting user decision — choices ★
Lay down the agreement (above), but the following must be chosen:

**S-1. Scope of adopting actual behavior ((A) vs (B), timing)**
- (a) **Abstraction only now** (types·events = (B)), keep actual behavior at (A) (inner exit = slot closes) → real dead-slot implementation is a follow-up. *Conservative.*
- (b) **Introduce real KeepDeadSlot behavior starting with claude/codex slots** (elevate the Rust Slot) → the slot is kept even on death · respawn. *Aggressive.*

**S-2. Default of `on_exit_policy` (may differ per backend)**
- claude/codex = `KeepDeadSlot` / shell = `CloseSlot` or KeepDead / api = `Idle` (stream end = Completed). → Whether to differentiate or go uniform.

**S-3. Scope of first-phase respawn support**
- same command (claude→claude resume) / different command (replace claude→codex) / drop-to-shell / restore-with-resume — how far for the first phase.

**S-4. Level of dead-slot retention**
- How much of the last screen (scrollback), layout and exit status to hold in the dead slot.

## Recommendation (not a verdict — choices)
The judge's synthesized recommendation = **the data model and exit-reason classification exactly as in the GPT proposal, the implementation path "keep cmd /c + elevate the Rust AgentSlot lifetime"** (Gemini's correct answer). This is the only way to get the UX of (B) without damaging the ADR-0001/0005 invariants or the Windows PTY. Start S-1 with (a) (abstraction first) to fit §0 (low-risk long-term things laid down early), and S-2 leans toward claude/codex = KeepDeadSlot.

## Next
User chooses S-1 to S-4 → **ADR-0017** (session/death structure: Slot≠Run≠Transport, death = Run end event, Windows = Rust Slot elevation) finalized → merge the variable/type schema with ADR-0016 and implement via a coder (behavior in stages).
