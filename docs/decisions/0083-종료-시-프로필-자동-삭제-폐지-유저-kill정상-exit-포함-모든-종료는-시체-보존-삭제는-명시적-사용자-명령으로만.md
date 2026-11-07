# ADR-0083: Automatic profile deletion on exit discarded — every exit, including a user kill and a normal exit, preserves the corpse, and deletion happens only by an explicit user command

- 상태: 확정 (2026-07-14, basis: user measurement — after terminating an agent from the tree, right-click re-activation immediately reproduces "실패" ("failure") + code confirmation: `UserKill → reaper DeleteProfile → profiles.remove` → on re-activation `profile not found`)
- 관련: Amends ADR-0019 (the clause "user kill, normal exit (code0) → delete the profile" discarded: every exit preserves the profile corpse (KeepDisableAutoRestore), no automatic deletion — deletion only by an explicit user command) · ADR-0082 (activation=resume only, corpse preservation policy — this ADR extends that policy to the kill/exit termination paths and closes the code-0 gap of §open item ②) · ADR-0016 (auto_restore lifetime model) · `agent/reaper.rs::decide`/`apply_disposition` · `daemon/connection_core.rs::{SpawnProfile, DeleteProfile}` · step-log

## 맥락
ADR-0082 fixed the policy "activation = resume only; even on failure the system does not create a new conversation and preserves the profile as a corpse". But that confirmation handled only **the resume failure path (claude exiting with exit≠0 on "No conversation found")** with corpse preservation (`KeepDisableAutoRestore`), and left the **reaper disposition that classifies the termination itself (ADR-0019)** as it was.

The ADR-0019 §decision table sets "an intended exit (user kill, claude `/exit` exit0) = profile **deletion**":

| Exit cause | Disposition (reaper `decide`) |
|---|---|
| User kill (UI/LLM command) | `DeleteProfile` |
| claude `/exit` (exit 0) | `DeleteProfile` |
| Crash (exit≠0/signal/EOF) | `KeepDisableAutoRestore` |
| Daemon shutdown | `KeepAsIs` |

**User-measured bug (reproduced immediately):** after terminating (kill) an agent in the agent tree, right-click → re-activation shows "실패" ("failure"). The cause is not resume — the profile disappears **before even reaching the resume path**. The chain (code-confirmed):
1. Tree termination → `manager.rs::kill_agent` tags `TerminationIntent::UserKill`.
2. The agent dies → pump finish → reaper `decide`: `(UserKill, _) => DeleteProfile` (`reaper.rs`).
3. `apply_disposition` → `profiles.remove(id)` → the profile is deleted from the registry + disk (`agents.json`) (including `claude_session_id`).
4. Re-activation → `connection_core.rs::SpawnProfile` finds `profiles().get(id)` = `None` → `Err("profile not found")` → screen "실패" ("failure").

That is, the user policy of ADR-0082, "nothing must die, do not delete, leave it as a corpse at least", was **reflected only in the resume path and not in the kill/exit termination paths**, so an agent the user explicitly terminated is fully deleted rather than left as a corpse. ADR-0082 §open item ② (deleted on a code-0 early exit so no corpse remains) is exactly part of this gap.

## 결정
**The reaper does not automatically delete the profile on any exit.** Discard the clause "user kill, normal exit (code0) → `DeleteProfile`" of the ADR-0019 §decision table, and lower both cases to **corpse preservation (`KeepDisableAutoRestore` — keep the profile, `auto_restore=false`)**.

- **Every runtime exit (user kill, normal exit, crash, EOF)** = collect the session from the map but **preserve the profile** and lower it to `auto_restore=false` (excluded only from boot auto-restore — it does not come back to life by itself on the next boot). Daemon shutdown stays `KeepAsIs` as before (auto_restore unchanged → boot restore).
- **No automatic deletion path exists.** Profile deletion happens **only by an explicit user command** (`AgentCommand::DeleteProfile` / Tauri `delete_profile`) — the reaper produces no deletion disposition.
- **Re-activation = corpse resume.** A terminated agent remains with its profile + `claude_session_id` intact, so right-click re-activation resumes normally with `--resume <sid>` (if the conversation really exists, the resume succeeds; if not, Failed + cause log as in ADR-0082).

This extends and completes the ADR-0082 policy ("do not kill, do not create anew, preserve the profile, cause log") **down to the termination classification layer**. It closes ADR-0082 §open item ② (the code-0 gap).

## 거부한 대안
- **Keep user kill and normal exit = profile deletion (ADR-0019 as is)** — the old premise "the user explicitly killed it, so deleting is fine". Rejected: the user policy was reversed ("do not delete, leave it as a corpse at least"). If this premise remains, (a) terminate→re-activate breaks with `profile not found`, (b) the conversation context (`claude_session_id`) is lost the moment of termination, and (c) ADR-0082's corpse preservation is nullified on the kill path.
- **Preserve only UserKill, keep deletion for a normal exit (code0)** — /exit was intentionally ended by the user inside claude, so leave it as deletion. Rejected: the user's explicit decision = **preserve both**. A code-0 exit is also a corpse that can be resumed because the conversation really exists, and keeping deletion leaves the ADR-0082 §open item ② gap as it is. Splitting deletion by "was it an intended exit" is itself the thing to discard.
- **At the re-activation entry point (`SpawnProfile`), recreate on the spot if there is no profile** — leave deletion as is and recreate and attach the profile on re-activation. Rejected: (1) at the time of deletion `claude_session_id` and the conversation are already lost, so there is nothing to resume even if recreated (an empty new conversation = a recurrence of the fresh-fallback that ADR-0082 discarded) (2) it is a workaround that covers only the symptom while leaving the root cause (automatic deletion).
- **Discard the explicit deletion command too (accumulate forever only)** — nothing can be erased. Rejected: the user's original plan is "rather than creating a new one, **delete and** recreate" — deletion remains an act the user does **intentionally** (only automatic deletion is discarded). Keep the `AgentCommand::DeleteProfile` path.

## 근거
- **User measurement:** tree termination → right-click re-activation → "실패" ("failure") reproduced immediately. `agents.json` = `{"profiles": []}` (physical evidence that the profile was actually deleted after termination).
- **Code confirmation:** `reaper.rs::decide` `(UserKill,_)=>DeleteProfile` / `(None,Exited{0})=>DeleteProfile` → `apply_disposition` → `profiles.remove`. `profiles().get()==None → Err("profile not found")` in `connection_core.rs::SpawnProfile`.
- **The user's original plan (inherited from ADR-0082):** "Nothing must die. Even if the session blows up, do not create a new one. Rather than creating a new one, delete and recreate. Leave it as a corpse at least." → discard automatic deletion entirely + deletion is manual. In this session the user explicitly decided the preservation scope (both UserKill and code0).
- **§5 consistency:** the exit cause is preserved as a corpse + `auto_restore=false` so that the LLM (the brain) reads the state and decides re-activation/deletion (the human is auxiliary). The system does not erase the profile without the user knowing and remove the basis for the decision.

## 영향 / 불변식
- **Simplify the reaper `decide`** — `shutting_down => KeepAsIs`, every other exit → `KeepDisableAutoRestore`. The reaper no longer produces `Disposition::DeleteProfile` (now a crash and a user kill/normal exit get the same disposition = preserve the profile, `auto_restore=false`). Only "intended exit = deletion" of ADR-0019 §decision 1 and the §decision table is discarded, and the rest (the intent frozen snapshot, epoch verification, shutting_down suppress, the auto_restore lifetime) is kept.
- **A single deletion path** — profile deletion = only an **explicit user command** that calls `ProfileRegistry::remove` (`AgentCommand::DeleteProfile`/Tauri `delete_profile`). The reaper only collects sessions and downgrades `auto_restore`.
- **ADR-0082 completed** — the corpse preservation on a resume failure holds on the kill/normal-exit termination paths too. §open item ② (the code-0 gap) closed.
- **Accumulation caution (an accepted tradeoff)** — since every exit leaves a corpse, profiles can accumulate without bound. This is the intended behavior under the user policy, and cleanup is done by explicit deletion. The visual display of the corpse (a separate "죽음/실패" ("death/failure") marker vs the current reserved node) is the **user's later decision** as in ADR-0082 §open item ① (outside this ADR's scope).
- **Anchor:** `// ADR-0083` (`reaper.rs::decide`). **Regression tests (mandatory):** ① `decide(UserKill) == KeepDisableAutoRestore` (update the existing `decide_user_kill_deletes`) ② `decide(Exited{0}) == KeepDisableAutoRestore` (update the existing `decide_clean_exit_deletes`) ③ integration: agent kill → profile preserved (`profiles.get(id).is_some()` + `auto_restore==false` + `claude_session_id` kept) → re-activation enters resume without `profile not found`.
