# PRD — S14 module ① output-plane redesign: output management unit = independent per View

**Status:** review reflected (`/review prd full` FIX reflected) — awaiting ADR-0040 numbering
**Written:** 2026-06-30
**Basis ADRs:** ADR-0035 (layout authority = src-tauri) · ADR-0036 (transport relay unification) · ADR-0037 (transport semantics Rust alone) · **ADR-0040 (the decision this PRD will produce — output unit = independent per View, not yet numbered)**
**Basis research:** `.claude/skills/research/study-notes/20260630-multiview-relay-2layer-medium.md` (medium, cross-family) · `20260629-multi-viewer-terminal-buffer.md` (light, first diagnosis of N1)
**Review:** `/review prd full` = Codex (User lens blind) + opus reviewer-deep (Tester lens blind) **both FIX, no BLOCK** — the two families **independently converged** on the contradiction "everything from the start ↔ buffer cap". This rev reflects the FIX (behavior correction + missing scenarios/risks stated).
**Continuation:** a **unit** redesign of the S14 module ① T5~T7 output plane (fan-out). T7d (GUI measurement) · thinning are postponed until after this decision.

---

## 1. What / why

**Background.** In S14 module ① we built an output plane that fans a single daemon connection out to N windows (T5~T7). But when the same agent is viewed in several Views, a defect where output is lost has been reproduced repeatedly.

**The nature of the defect (structural).** The current output management unit is *"a single subscription per agent + per-window progress tracking + resubscribe based on the most-lagging window `min(render_seq)`"*. A freshly opened window is **missing** from the min computation (the asymmetry of key-absent vs explicit `None` — an unrendered window cannot elicit a "full replay"). As a result the daemon receives a high seq based on the leading window, and the new window **permanently loses** the output before that. This is not a specific bug but a **limit of the "per-agent bundle" unit itself** — it does not go away by changing only the authority axis (who subscribes).

> The adversarial review of 2 families (opus FIX + Codex BLOCK) confirmed this loss by **independent convergence** (a strong signal). Location: `crates/engram-dashboard-core/src/output_window_seq.rs` `min_render_seq` · call `connection.rs` Subscribe after_seq.

**Why change the unit.** The industry standard (research cross-family convergence, certain) is *"content is stored centrally once + progress is a per-subscriber independent index, transport sequence ≠ consumption progress separated"* — VS Code pty host (the relay solely owns the replay buffer, each renderer has an independent cursor) · Kafka/Redis (shared log + per-consumer offset) are the canonical form.

---

## 2. Core decision (user confirmed — direction)

**Output management unit = independent per View.** The relay hub (src-tauri) **collects and keeps one copy** of the output per agent (shared buffer), and each View (window) keeps only *how far it has read* (an index) of that buffer **independently**. It subscribes to the daemon **only once** per agent.

= a combination of VS Code pty host (relay single buffer) + Kafka/Redis (shared log + per-subscriber offset). Consistent with the standard pattern the research verified.

**Role switch of "the most-lagging window min"** — now: *the daemon resubscribe point* (causes the defect) → new model: *the client buffer retention lower bound (eviction)*. A new window fills itself from the client buffer with its own index, without re-requesting from the daemon.

---

## 3. Behavior specification (user decisions)

1. **A new window = the same as an existing window.** Whether a new View or the first View, without discrimination — it draws all of that agent's output **from the beginning of the retained range** (no variants such as tail-only or one screen only). **Definition of "from the beginning" = the retention lower bound (the oldest retained point)** — anything older has already been discarded by the daemon too and cannot be given from anywhere. This is **identical to the current behavior** (existing windows also received only what the daemon had). Only the source changes from a daemon re-request → the client shared buffer.
2. **Buffer = a mirror of the daemon's lossless range.** The client buffer holds, as one copy, exactly the range the daemon keeps losslessly. The retention limit is a **dual cap (about 2MB AND about 4096 entries, whichever is reached first)** — defense against a 1-byte-chunk runaway (isomorphic to the daemon's ReplayBuffer). Excess over the limit is evicted from the front and **the retention lower bound rises**. The daemon does not have anything older either, so **no 2-tier fallback is needed** (single buffer). **The truncated past is left as is, with no separate indication on screen** (existing behavior kept — user decision. Risk stated in §7). The retained amount is not proportional to the number of windows.
3. **Daemon subscription = 1 per agent.** Since the client holds the buffer + indices, **no viewer identifier leaks to the daemon** (minimal protocol change). The daemon is responsible only for a single lossless stream. **However, what to send as the daemon Subscribe `after_seq` on reconnect (the client buffer's latest seq vs the oldest) is the crux of losslessness — settled up front at §8 decision point 3.**
4. **New output arriving while scrolled into the past = terminal convention (user decision).** A window that is scrolled up looking at the past **keeps its position** (no forced jump to the bottom on new output); if it is at the very bottom (follow) it **follows** the new output (auto-scroll). **Independent per window** (consistent with the per-View independent unit — one window's scroll does not affect other windows).

---

## 4. Scenarios / edges (BDD-style)

- **Simultaneous multi-viewer (2~N windows):** while watching an agent in window A, opening window B → B draws from the client buffer **from the retention lower bound**, A is unaffected, **0 loss within the retained range**.
- **New output arrives:** daemon → client buffer append → every window showing that agent catches up from its own index and draws (the follow/keep-position branch of §3-4 applies).
- **Restart (epoch transition):** it is a new stream, so buffer reset + all window index reset → the new session's output from the beginning, **0 remnants of old output**.
- **epoch transition ∩ new window at the same time:** in the instant the buffer is reset by a restart a new window mounts → **an ordering guarantee is needed** between the buffer reset and the new window's index initialization (blocks the race of reading the old buffer with the pre-reset index, or drawing the empty post-reset buffer as "from the beginning"). The exact order = TRD decision point 4.
- **Opening a new window during reconnect:** if a new window opens while the connection is cut (reconnecting) → **render immediately from the preserved client buffer** (no waiting for a daemon response). This is the core improvement of the new model (previously an empty screen because of the dependence on daemon re-requests). Verified by acceptance criterion 5.
- **New window right after a buffer wrap:** a new window right after the ring just evicted → "from the beginning" = from the current retention lower bound (not the true start). To the user it appears from the retention lower bound (definition §3-1).
- **Window close:** remove that window's index. The buffer is kept as long as another window watches that agent.
- **Last window closed:** 0 windows watching that agent → the buffer becomes an eviction candidate (retention lower bound). The exact timing · policy = TRD decision point 4.
- **Daemon disconnect/reconnect:** the client buffer is preserved, resubscribe once per agent (min summation discarded). The meaning of after_seq = §8 decision point 3.

---

## 5. Acceptance criteria

1. In **2+ windows and 3+ windows simultaneously** on the same agent, **0 loss of output within the retained range** — even if a new window is opened late, everything from the retention lower bound. (The evicted past is outside the guaranteed range — definition §3-1.)
2. **The daemon subscription is 1 per agent** (regardless of window count — confirm with `rg`/logs).
3. On an epoch transition, all windows show the new session's output from the beginning, **0 remnants of old output** (no epoch ∩ new-window race).
4. **Memory = a dual cap per agent (`min(~2MB, ~4096건)`)** (not proportional to window count).
5. **A new window during reconnect wait also renders immediately from the preserved buffer** (0 empty screens).
6. **Screen identity:** two windows of the same agent draw the same screen — ANSI escapes · partial UTF-8 · CR progress bars · alternate screen preserved (raw bytes replay does not create broken sequences).
7. Existing concurrency invariants preserved — ADR-0001 (kill 2 verbs) · 0005 (finalize once) · 0006 (lock order) · 0007 (epoch).
8. `cargo test` (root) + core `rg "use tauri"` 0 + `npm test`/`tsc` + `cdp.mjs` measurement pass.

---

## 6. Rejected alternatives (one line each — moved into the ADR-0040 body)

- **A single subscription per agent + per-window min summation (current)** — loss through the missing new window, a structural defect (independent convergence of 2 families).
- **The frontend subscribing to the daemon directly (plan b)** — violates ADR-0035 "frontend = pure renderer".
- **Re-requesting from the daemon for every new window** — the user's remark "the structure is off", an unnecessary network round trip in the 2-layer setup.
- **Dynamic retention + daemon 2-tier fallback** — no survey of OSS precedents for client↔daemon sync/eviction, high cost · uncertain. **However this rejection is valid only on the premise of lowering §3-1 to "from the retention lower bound"** — if "everything including the truly evicted past" is wanted, 2-tier becomes necessary, so that requirement is an explicit **non-goal** (§7).

---

## 7. Non-goals / risks / open items

- **Local zero-copy / per-environment (mobile remote vs local PC) branching = follow-up.** The daemon↔src-tauri is a separate process even locally (ADR-0029), so the receive structure across the boundary is common, and the memory duplication (~2MB×N) is small too, so the actual gain is small. Consistent with the step-log "daemonization = the same infrastructure for remote/mobile". Absorbed later at the transport seam (ADR after measurement).
- **★Risk (stated): multi-view resize ∩ a single shared buffer.** If two windows of different sizes look at the same raw byte buffer, one window's resize can break the escape sequences embedded in the buffer in the other window (the PTY has a single cols/rows). The resize policy is a separate open item, but we record as a risk that the **"single shared buffer" assumption can break under multi-resize** — if per-viewport buffer branching becomes necessary later, the §2 core decision must be retrofitted. To be reconsidered in the TRD together with the resize model (a point both reviewers share).
- **★Risk (stated): the truncated past is not displayed.** Since the past beyond the retention lower bound silently does not show, the user may mistake it for "loss" — displaying it is a **non-goal** by user decision (§3-2), but kept as a future UX candidate.
- **Multi-view resize policy** (windows of different sizes — tmux window-size style) = a separate open item (`trd.md`).
- **Persistence (restart restore)** = non-goal.

---

## 8. Decision points handed to the TRD (implementation forks — user decision + report)

1. Mapping between the index unit (seq) ↔ the position within the buffer (byte offset).
2. Relation · duplication between each View's xterm scrollback (render result) ↔ the client buffer (raw bytes) (directly tied to screen identity §5-6).
3. **[Crux] Computing the daemon Subscribe `after_seq` on reconnect** — the client buffer's latest seq vs the oldest. This is the core of the role switch of min (daemon re-request point → client eviction lower bound), and the §3-3 promise of "minimal protocol change" depends on it (opus's point: the promise got ahead of the verification → resolve and close it here).
4. Eviction timing · policy — last window closed ↔ buffer discard, epoch reset, **epoch ∩ new-window ordering guarantee**.
5. The scope of reworking/discarding the current `output_window_seq.rs` (per-agent min model).
</content>
