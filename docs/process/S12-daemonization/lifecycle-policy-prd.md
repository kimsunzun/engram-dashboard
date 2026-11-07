# PRD (candidate) — agent/daemon lifecycle policy: variable-schema stage

- Status: **awaiting selection** (consult cross-validation done; PRD to be finalized by user decision)
- Scope: for claude agents held by the persist daemon: (1) boot restore (2) runtime restart (3) crash runaway guard. **This stage = define variables (schema) only; the behavior logic is TODO.**
- Basis: `/consult` job `20260616-001941-consult-lifecycle-policy` — GPT, Gemini and Claude (opus) blind cross-validation + judge decision. Raw material: `I:\Engram\agents\web-runner\shared\<job>\`.

## ★Decisions (2026-06-16, user) — certain/provisional★
- **B-1 restart_policy default = `Always` (certain).** "Always keep it alive" is the product direction. The user overrode the consult's recommendation of Never.
- **B-2 guard = simple counter (certain).** `consecutive_failures` +1 on every attempt; past the limit of 5 it stops in Failed. **The only reset is a manual restart (human/LLM)** — automatic reset, stable-uptime reset, sliding window and `stable_secs` are **all discarded** (over-defense). Because there is no "reset automatically if it survives", even a 35-second runaway loop simply stops after 5 attempts. `last_start_at` is only for recording (debugging) and is not used to decide a reset.
- **B-3 Failed cold boot = persisted, `SkipUntilManualRestart` (provisional recommendation).** Failed is kept across boot and excluded from automatic restore until a manual restart.
- **B-4 restore target = "restore the last running one unless it was turned off" (provisional).** `auto_restore` AND `last_known_state != Stopped`. Anything the user explicitly turned off is not revived.
- **B-5 guard counter persistence = persist in agents.json (provisional).** Prevents the infinite loop where a daemon reboot wipes the counter to 0 (raised by the consult).
- **Discarded:** the "ask (물어보기: ask in the UI whether to restore)" mode of ADR-0008 — the consult decided it is the **anti-pattern where the UI becomes the restore trigger**. The restore default is "automatic" (independent of the UI). The ADR-0008 correction is a follow-up.
- Provisional items (B-3, B-4, B-5) are changed only if the user points out something wrong. Certain items (B-1, B-2) are not shaken.

## Fixed constraints (no reopening)
- Daemon = persist-until-kill, console = detachable viewer (ADR-0015). Opening the UI must not become a restore trigger.
- The scheduler = an independent capsule, out of scope this time.
- Define variables only; behavior is TODO.

## A. Agreed conclusions (3 models + judge agree — nearly certain, reflected in the variables)
1. **`auto_restore` (intent) ≠ "last running" (observation) are orthogonal axes — separate them.** Lumping them into one field causes confusion: "the daemon revives what the user turned off" and "a brief stop becomes a permanent exclusion". → Put the observed state in a separate variable.
2. **Failed must persist across cold boots** — `agents.json` currently has no status field, so on boot Failed evaporates and the agent unconditionally re-enters the restore ladder. A persisted variable is needed to keep "Failed = automatic retry stopped, data preserved" beyond boot.
3. **Daemon guard × agent guard counter volatility interaction** — if a daemon reboot resets the in-memory agent counter to 0, the infinite loop cannot be stopped. Part of the guard state must be persisted (or the daemon guard must catch it).
4. **Global tuning values go in a new `daemon-config.json`** (C2) — `agents.json` is per-agent and does not fit. Include `schema_version`. Isolate it with a trait isomorphic to ProfileStore.
5. **Rename `last_restore` → `last_start_at`** — there are zero consumers, so regenerating ts-rs is enough (no deprecated compatibility layer needed).
6. **The per-agent guard is well separated** (not a global counter), and **the scheduler is well separated** (no time or period mixed into wake/restore).
7. **The subject of the daemon re-spawn guard = Tauri Core (Rust main runtime), not the frontend (React)** — because the daemon can die while the UI is closed. The variables also live on the Rust side.
8. **An exit-kind classification is needed** — `Killed` (killed by a human) is never an automatic restart target (otherwise it is revived even after a kill). This is the precondition for the OnCrash decision.

## B. ★Issues awaiting user decision★ (needed to finalize the PRD)

### B-1. restart_policy default — Never vs OnCrash
- **Never (recommended: safe at the variable stage)**: avoids the semantic switch (migration shock) where, the moment the behavior is implemented, "every existing profile restarts automatically". The exit-kind classification is not implemented yet either, so being conservative is right.
- **OnCrash (product philosophy)**: natural for an autonomous-agent daemon, but risky until the guard and exit decision are verified.
- **Compromise (the one the judge saw as superior)**: set the **schema default = Never** and make only the **preset of the new-creation UI OnCrash** — separate "schema default ≠ creation-form preset". → **Decision needed.**

### B-2. ★Guard reset stable_secs value + sliding-window assist★ (most important — corrects my earlier proposal)
- The **stable_secs=30s I proposed earlier was decided to be dangerous for claude** (quantified by Gemini alone, accepted by the judge): a normal single prompt takes claude **1~2 minutes**, so with a 30-second stability criterion a loop that "lives 35 seconds and dies" (API timeout, insufficient credit) **resets the counter to 0 every time and cannot stop the infinite runaway**.
- The right direction (recommend doing both):
  - (a) **Make stable_secs large to fit the workload** — e.g. **300 seconds (5 minutes)** or more.
  - (b) **Open only a placeholder for the sliding-window assist schema now** — `window_secs: Option`, `window_max: Option` (initially None). For the case of "living slightly longer than stable_secs and dying repeatedly".
- → **Decision needed: the stable_secs value (e.g. 300) + whether to open the sliding-window assist fields now.**

### B-3. Failed cold-boot policy — keep vs one retry
- `SkipUntilManualRestart` (recommended): Failed is kept across boot; excluded from automatic restore until a human/LLM releases it.
- `RetryOnDaemonColdBoot`: one retry after an OS reboot or update (recovers if the cause was environmental). But there is a risk of daemon runaway × agent runaway combining.
- Both choices can be expressed with the enum. → **Default decision needed.**

### B-4. Restore target — simple (all auto_restore) vs observation-coupled (auto_restore AND was_running)
- The agreement is "separate intent/observation" (A.1), but whether the **initial behavior** is simple (restore everything with auto_restore=true) or also coupled to observation (`was_running`) is a choice. Because of persist-until-kill, "last running" only matters at a *daemon crash/OS reboot* (irrelevant in normal operation). → **Decision needed** (the variable is laid down anyway; the question is what the restore function looks at).

### B-5. Whether to persist the guard counter
- The need is acknowledged because of A.3. **Persist per-agent `consecutive_failures` in `agents.json`** (kept across daemon reboots, but with the cost of rewriting the file on every death) vs **in-memory + the daemon guard catches the runaway**. → **Decision needed.**

## C. Seam decisions to hand to the TRD (not closed by variables alone — A = raised by Claude alone)
- **★There is no runtime death-observation hook in the code★**: currently `AgentManager` does **not subscribe** to the terminal signal (`status_changed`, emitted by the pump alone). So the manager has no way to know "the agent died at runtime" → **the restart capsule is not a simple behavior TODO; it presupposes "creating an observation seam".** Candidates: StatusSink feedback / OutputCore done_rx supervisor join / a new LifecycleSink. (Decided in the TRD)
- **Boot restore dual path**: both the daemon and src-tauri (Embedded) call `restore_all` — the "restore does not run in normal operation" assumption breaks in direct Embedded execution. A handling policy is needed until the mode toggle settles.
- daemon_respawn guard: `ensure_daemon` has no retry loop itself → new state + loop needed on the Tauri Core side.
- (Reference) PTY orphans: on a sudden daemon death the JobObject (`KILL_ON_JOB_CLOSE`) already guarantees the children die with it → Gemini's "orphan double-token" concern is mitigated on Windows (watch only non-Windows and abnormal paths).

## D. correctness-merge variable schema draft (add or remove depending on the choices)
> Skeleton = GPT, E values/sliding window = Gemini, death-observation seam and last_restore handling = adopted from Claude. Excludes what the judge said was wrong (the 30s assertion, the PTY-orphan exaggeration, the deprecated layer).

**per-profile (agents.json):**
- `auto_restore: bool` (exists) — restore intent/eligibility.
- `restart_policy: {Never,OnCrash,Always}` (exists) — default decided in B-1.
- `last_known_state: {Stopped,Running,Failed}` — new. Observed state (A.1) + Failed persistence (A.2). "Stopped = the user turned it off / Failed = isolated by the guard".
- `last_start_at: Option<i64>` — `last_restore` renamed (A.5). Measures elapsed time for the stable-uptime reset.
- `consecutive_failures: u32` — new. Guard counter (whether to persist: B-5).
- (Optional) `last_exit_kind: Option<{ManualStop,NormalExit,CrashExit,SpawnFailed,RestoreFailed,DaemonShutdown,Unknown}>`, `failed_at`, `failed_reason` — exit classification and Failed reason (A.8). Decide whether to put them all in now.

**Global (new daemon-config.json):**
- `schema_version: u32`
- `restore_target_policy: {AutoRestoreProfiles, LastRunningOnly}` (B-4, default AutoRestoreProfiles)
- `failed_restore_policy: {SkipUntilManualRestart, RetryOnDaemonColdBoot}` (B-3)
- `agent_restart_guard: { stable_secs(예 300), restart_max(예 5), window_secs: Option, window_max: Option }` (B-2)
- `daemon_respawn_guard: { stable_secs(예 60), respawn_max(예 3) }`

**Tauri Core (Rust, in-memory) — daemon watching (A.7):**
- `DaemonSupervisorState { daemon_respawn_count: u32, last_daemon_start_at: Option<Instant> }` — not the frontend, separate from daemon-config.

## Next
- Once the user picks **B-1 to B-5 + the scope of the (optional) exit-kind fields above** → PRD finalized → TRD (the seam decisions of C) → ADR-0016 → coder adds the fields (behavior 0).
