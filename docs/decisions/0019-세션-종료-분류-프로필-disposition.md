# ADR-0019: Session end classification — intent is decided at the "point of action", the profile disposition separates death/reserved/restore

- 상태: 확정 (2026-06-16, dashboard4 session — user decision)
- 관련: ADR-0016(lifetime model — the runtime interpretation of restart_policy **partly discarded**), ADR-0017(death definition — **made concrete**), ADR-0001(kill 2 verbs), ADR-0005(finalize once), ADR-0015(daemon persist) · Amended by ADR-0083 (the clause "user kill · normal exit (code0) → profile deletion" removed: every end preserves the profile corpse (KeepDisableAutoRestore), no automatic deletion — deletion only by an explicit user command) · Amended by ADR-0084 (apply_disposition epoch-guard added — so a stale reap cannot demote a re-activated live session to auto_restore=false (applied only when the reaped msg.epoch matches the current session epoch). Re-activation (resume) is a map replacement, so the epoch is bumped (ADR-0007 reconfirmed).) · Amends ADR-0016 (the runtime interpretation of restart=Always)
- Scope: when a claude session "has ended", what counts as death, whether to delete or keep the profile, and how to restore it. Separate in-progress death from boot restore.

## 맥락
There are several paths by which a session ends — user kill / claude `/exit` / a crash / a forced daemon termination. If we **classify only by observing the end (the exit signal)**, they get mixed, for example the Job-kill of a daemon shutdown is misclassified as a user kill. Also ADR-0016 treated "runtime restart (restart_policy Always)" and "boot restore" as one lump, but they are different logic.

## 결정
**Principle: intent is not inferred by observing the end; it is decided at the "point of action" that caused the end.** Split into four channels:

| End cause | Detection | Death? | Session | Profile | auto_restore |
|---|---|---|---|---|---|
| User kill (UI/LLM command) | intent tagged in the command handler | death | cleaned up | **deleted** | — |
| claude `/exit` (normal, exit 0) | PTY EOF + `Exited{0}`, not our kill | death | cleaned up (the terminal ends together) | **deleted** | — |
| claude crash (exit≠0/signal) | PTY EOF + abnormal | not death (an accident) | cleaned up | **kept** | →false (back to reserved) |
| Daemon/app forced termination · reboot | daemon shutdown = no death recorded | not death | (cut off together with the daemon) | **kept** | stays true → boot restore |

1. **The core rule = the profile disposition.** An intended end (user kill · `/exit`) = clean up the session + **delete the profile** (remove it completely). An unintended end (crash · forced termination) = clean up the session + **keep the profile**.
2. **A crash → back to reserved (waiting)** (auto_restore=false). **No runtime auto-restart** — a human re-runs it manually. (It immediately shows as `대기` (waiting).)
3. **auto_restore = "should it be up right now".** Reserved (empty shell) = false, **true on activation** (so it becomes a boot-restore target after a forced termination), a crash return = false.
4. **A daemon shutdown does not record the termination of its children as a per-agent death** (shutting-down suppress). So the running ones that were forcibly terminated stay at auto_restore=true and are **restored at boot**. ↔ On a crash the daemon is alive, so the reaper runs and drops auto_restore to false → the two cases are naturally distinguished by "did the reaper run".
5. **Runtime death ≠ boot restore** — separate logic. Boot restore keeps ADR-0016 §boot restore (revive the saved auto_restore=true ones once).
6. **Detection = PTY EOF (current).** With the `cmd /c claude` structure, claude ending (= /exit · crash) → cmd ends too → master reader EOF → pump finish. No separate child watcher is needed.

## Rejected / replaced
- **The interpretation of ADR-0016 restart_policy=Always as "runtime auto-restart" → discarded.** There is no runtime auto-restart. A crash goes back to reserved (manual re-run). "If it dies, revive it" is limited to the **boot restore** context. (Only the runtime meaning of ADR-0016 §decision 3 is superseded; boot restore, the guard counter and Failed persistence are to be reviewed later — this ADR focuses on end classification.)
- **Classify by observing only the exit signal** — it misclassifies the Job-kill of a daemon shutdown as a user kill. So intent is tagged at the source (command handler/shutdown path).
- **cmd /k (zombie terminal)** — ADR-0017 stands (rejected).

## 영향 / 불변식
- **ADR-0001 (kill 2 verbs) and ADR-0005 (finalize once) are unchanged** — the reaper only **consumes** the existing done signal (`OutputCore.finish` → done_tx), removes the session from the sessions map and applies the disposition; it does not change the kill causality or the finalize count.
- **User kill = profile deletion**, so ADR-0016's "kill ≠ deletion, no pause" becomes "kill = full end = deletion" in this model (no pause is kept — if it is not used for a while, it idles).
- A "restart" (an intentional restart = keep the profile and re-spawn) can be a **separate command** from kill (deletion) — later, if needed.

## Implementation refinement (consult cross-validation `20260616-215346`, judge top trust = GPT → TRD `reaper-trd.md`)
The reaper concurrency design was blind cross-validated by GPT, Gemini and Claude. The synthesis added two things to the decision:
1. **Intent is a snapshot at the moment of finish.** If the reaper reads intent live at reap time, a race appears: "after a crash, before the reaper handles it, the user kills → the crash is misclassified as a user kill → profile deleted (data loss)" (singled out by GPT alone; Gemini and Claude missed it). So when the pump is the finish winner, it **snapshots intent and shutting_down at that moment, puts them in the end event (ReapMsg)** and emits it, and the reaper decides from those frozen values.
2. **Verify the epoch match before reaping (reusing ADR-0007).** A single supervisor + the `sessions.remove()` Some-winner guarantees idempotency, but we compare the epoch before the remove so that a late-arriving old done does not wrongly delete a new session re-spawned under the same AgentId (included by GPT and Claude; omitted by Gemini → a zombification bug). The existing epoch is reused instead of introducing a new RunId type.

For the detailed interfaces, flow and tests see `docs/process/S12-daemonization/reaper-trd.md`.

## Follow-ups (raised by reviewer-deep — resolve before daemonization, not a blocker now)
Items from the post-implementation reviewer-deep inspection that are latent at this stage, but **must be pinned down before daemonization (S12) · introducing auto-restart (OnCrash)**:
1. **`shutting_down` lifetime.** Currently there is no path that sets it back to false after `shutdown_all` sets it. If AgentManager is 1:1 with the process lifetime this is harmless, but in the **daemon-resident model** (ADR-0013, tmux style), if shutdown_all changes meaning to "clean up this client", a flag set once turns every later end into KeepAsIs (skipping the disposition). Before daemonization, settle the policy "manager single-use vs reused" in an ADR.
2. **Status misclassification (Killed vs Exited).** If it is a user kill but the watcher sees the child's death before the shutdown flag is set and drops the master, the pump may report the status as `Exited{code}`. **The disposition is safe because intent takes priority** (zero impact), but the moment `RestartPolicy::OnCrash` auto-restart uses status as an input, the branch goes wrong. When introducing OnCrash, reflect intent in the reason/status branch.
3. **Unify the watchers.** The natural-exit detection watcher is a thread per session (50ms polling) — asymmetric with the reaper being a single supervisor. For multi-session scalability, merge the watchers into a single polling thread as well (long term).
4. **Reaper test coverage gap.** The epoch race (preventing an old ReapMsg from wrongly deleting a re-spawned new session) and idempotency have only unit decide + structural guarantees and no integration measurement — the regression net needs reinforcing when reap_one is modified.
