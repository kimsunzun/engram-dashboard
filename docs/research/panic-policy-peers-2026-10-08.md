# Panic Handling Policy — Peer Survey (2026-10-08)

- **Status:** research done · **decision = ADR-0288 · ADR-0290** (2026-10-10 — contain one request · one agent, and a panic from shared state exits normally as is the practice · a panic that could not be contained has the hook pull the exit · abort just exits. In between there was ADR-0289 (keep alive with a strong warning), which ADR-0290 discarded). The result of the codex adversarial review is in the section 「적대 리뷰」.
- **Method:** /research medium · design-decision mode. Three tracks (A direct peers = local clones + 9 scratch shallow clones · B mechanism = primary docs · C user-facing behavior = product docs · source) → main synthesis + grounding spot checks → codex adversarial review.
- **Confidence legend:** certain (independently cross-confirmed) · likely (a single source that passed grounding) · uncertain (unchecked · inferred).
- **Trigger:** while sorting out the scope of boundary refactoring A, we were about to decide 「락 오염 때 패닉할지 복구할지」, and the user proposed 「패닉이 나면 메시지를 띄워 종료를 안내하고 살려 두면 사용자가 저장할 건덕지라도 있지 않나」. This report measures the precedents and costs of that proposal.

## Our current state (code measured · 2026-10-08 `aef34d4`)

- Workspace `[profile.release]` = `panic = "abort"` (`Cargo.toml:41`) — every daemon · shell · CLI distribution. A panic in any thread = that process dies instantly. So lock poisoning does not occur in production builds (base `sync` header contract 3).
- The daemon panic hook = leaves one `tracing::error!` line and then calls the previous hook (the default hook — stderr diagnostics) (`crates/engram-dashboard-daemon/src/lib.rs:133-147`). No user notice.
- The shell = no panic hook (0 `set_hook` in `src-tauri/src`).
- If the daemon dies, all agents die too — agent processes are in a `KILL_ON_JOB_CLOSE` Job (`crates/engram-dashboard-platform/src/group/mod.rs:38,53`). If the shell dies, the daemon and agents live (separate processes).
- Mixed lock handling: about 117 places recover from poisoning (ADR-0275 measured) · about 250 places of `lock().unwrap()`/`expect` (including tests · counted by the survey worker on 2026-10-08).

## Conclusion

1. **There is no precedent in the range we surveyed for the user's proposal (wherever a panic comes from, keep the whole process alive and guide the user to wrap up).** Likely — surveyed = Rust peers zellij · herdr · wezterm · rust-analyzer · sccache · vector · Zed · Alacritty · nushell · Deno · Tauri · atuin · helix (+ vibe-kanban) · products VS Code (extension host · pty host) · Zed · tmux · zellij · Chrome · JetBrains. ★But 「요청 · 작업 하나를 가두고 서버는 산다」 (contain one request · one job and the server lives) is common★ (see (b) below · adversarial review 1). The closest is a JetBrains IDE (shows a caught exception as a notification and keeps running), but it is an exception in a managed runtime, so it differs in nature from a Rust panic (knowledge-based · unchecked).
2. **Peers split into four groups.** Certain (local source confirmed) — but (d) and tower-http in (b) were added by the adversarial review (docs.rs basis). We did not put herdr · atuin · helix in any group — they have no server-side panic hook or isolation boundary, so they rely on the std default behavior that 「비주 스레드 패닉은 그 스레드만 죽는다」 (a panic in a non-main thread kills only that thread) (herdr likely · the rest uncertain).
   - **(a) panic = exit, but report and leave a record** — zellij · Zed · Alacritty · wezterm. For the Deno · Tauri repositories we confirmed only that the distribution uses `panic = "abort"` (panic hook · actual behavior unchecked — uncertain). A zellij server, on a panic in any thread, sends `Exit{Error(backtrace)}` to every attached client and then ends (`zellij-server/src/lib.rs:1596-1607` — main checked). Alacritty on Windows shows a `MessageBoxW` 「Alacritty: Runtime Error」 and dies (`alacritty/src/panic.rs:17-20` — main checked). Zed has a sidecar leave a minidump and report it at the next launch.
   - **(b) contain one request · job and live** — rust-analyzer (only read-only requests get `catch_unwind` · `on_sync_mut`, which changes state, is deliberately not contained — 「they don't modify the state, so it's OK to recover」 · 「don't make bugs :-)」, `crates/rust-analyzer/src/handlers/dispatch.rs:34-44` main checked) · sccache (one compile request · `src/server.rs:1530` main checked) · nushell (one REPL turn) · tower-http `CatchPanicLayer` (turns a handler panic into HTTP 500 — it also contains handlers that hold shared state · adversarial review 1). ★「공유 상태를 안 바꾸는 단위만」 (only units that do not change shared state) is the criterion rust-analyzer states explicitly, not the rule of the whole group★ — web servers contain without that criterion and leave a poisoned lock to the code above.
   - **(c) catch, but catch in order to shut down in an orderly way** — vector (catches per component and brings down the whole topology).
   - **(d) supervision tree** — ractor (a child actor's panic is reported to the supervisor) · ractor-supervisor (restart policy · cap). A Rust port of the Erlang-style 「죽게 두고 감독자가 다시 띄운다」 (let it die and the supervisor brings it back). The process lives and only the dead unit comes up anew — but there is no guarantee that a panic from anywhere is safely revived (adversarial review 2).
3. **Every peer that survives is an unwind build.** Certain. `catch_unwind` is powerless under `panic = "abort"` (std docs). With our distribution setting, (b) does not hold either.
4. **The mainstream answer of mature products is not 「살려 두기」 (keep alive) but 「상태 주인을 버그 나기 쉬운 코드와 다른 프로세스에 두기 + 자동 재기동 + 복원」 (put the state owner in a different process from bug-prone code + automatic restart + restore).** Likely.
   - VS Code: puts terminals in a separate pty host process so terminals live even if the renderer or extension host dies. The pty host itself, if it dies, has a cap on restart count (`_restartCount <= MaxRestarts`, `MaxRestarts = 5` — counted from 0, so at most six times · adversarial review 6), and **creates the terminals anew** from the layout · cwd (the old processes are lost · `ptyHostService.ts`). The extension host restarts automatically up to 3 times in 5 minutes and then shows a notice with a button (`abstractExtensionService.ts`).
   - zellij: serializes the session layout · commands to a cache and revives them by reattaching. Revived commands run only after you press Enter (accident prevention).
   - tmux: if the server dies, everything dies — restore is third-party (tmux-resurrect).
5. **Our structure is on the tmux side.** The daemon is the state owner and also holds all the code that can have bugs. The shell side is already VS Code-shaped (if the shell dies, the daemon and agents live).

## Mechanism facts (track B)

| Item | Content | Confidence |
|---|---|---|
| The panic hook runs even under abort | After that it ends without destructors. On Windows the fast-fail exit code is `0xc0000409` | certain (std `process::abort` docs · MS fastfail docs) |
| `catch_unwind` | Catches only unwind panics — it cannot catch abort · double panic · foreign exceptions. It is not a general try/catch (docs) | certain |
| tokio task panic | If unwind, it is caught as a `JoinError` and the runtime lives (default `UnhandledPanic::Ignore`). `ShutdownRuntime` is unstable · current_thread only | certain (docs.rs) |
| std thread panic | If unwind, only that thread ends and `join` returns `Err` | likely (knowledge-based) |
| Lock poisoning | std keeps poisoning. `std::sync::nonpoison` is unstable (#134645 open). The discussion about changing the default lock in Edition 2027 (#149359) is at an exploratory stage | certain (existence · unstable) |
| parking_lot | No poisoning — the lock is simply released on panic. wezterm mux · Zed · helix · atuin use it | certain |
| Tauri v2 | No `catch_unwind` in command dispatch (tauri 2.11.3 source grep — only one mobile entry point). Synchronous commands run on the main thread. No doc guiding panic policy | certain (grep) · uncertain (actual behavior) |
| What the user sees on abort | Whether a WER dialog appears depends on system settings — unknown | uncertain |

## Options × constraint fit

| | Chance to rescue the user | Risk of state corruption | Precedent | Cost | Consequence for lock-poisoning policy |
|---|---|---|---|---|---|
| **1. Current (abort) + better notification** — the daemon hook leaves a crash record, and the shell detects the daemon's death and shows 「내부 오류로 데몬이 종료됨 · 로그 위치」. Polish the restore path after restart | none (instant death) — filled by restart · restore instead | none | many in (a) · zellij is closest | small | panic (A) — poisoning does not arise in production |
| **2. unwind + contain one request · command** (rust-analyzer · sccache · tower-http style) — e.g. the handling of one query request · one bus command. On a panic, only that request gets an error response | partial (only panics from a contained unit) | low to medium — if you contain even state-changing units, a half-changed state remains. How far to contain is the crux of the judgment | (b) | medium — a per-unit audit | where not contained = exit → panic (A). Locks in a contained place stay poisoned, so decide per unit |
| **3. unwind + contain one agent + supervision** ((d) style) — if the thread or task of one agent panics, only that agent is wound down as failed and the rest live | large (other agents live) | medium — ★a pump's own panic is already contained inside the pump★ (`crates/engram-dashboard-agent/src/transport/pty.rs:274` 「B-2」 — `core.finish` after `catch_unwind` gives `Failed` · same shape at `stdio.rs:305`. Powerless now because of the abort build). What is open is agent-only code outside the pump (writes · the output interpreter · latches). Making a supervisor the third caller of cleanup is the causal branching that ADR-0127 forbids | (d) · Erlang style | medium — audit the boundary outside the pump · re-examine invariants | locks inside the agent = discarded with that agent. Shared locks get the same judgment as 2 |
| **4. unwind + the whole survives + guide the wrap-up** (the user's proposal) | yes | **high** — what the panicking thread was doing is left half-done (e.g. if it dies while changing the shared registry, an agent that is in the registry but not running is left, and the poisoned registry keeps being used · the pump already contains itself as in 3 above) | none | medium to large — define which features die and live · the shell notice path | recovery (B) forced |
| **5. Separate the state owner** — a process other than the daemon (e.g. a per-agent host) owns the agent PTY so that a daemon panic does not kill agents | yes (agents live) | low — but a restore path of reattaching after the daemon restarts must be designed separately (crash-only design · adversarial review 4) | VS Code pty host · Chrome | **large** — a structural change. On Windows the process that created the PTY must hold it (two agreements in CLAUDE.md 「참조 구현」) — the host must create it directly | unrelated |

- **The agent conversation itself is mostly not lost to a panic** — claude saves the conversation and we persist the session id (ADR-0226). But there are conditions: the id is persisted only after the first submit (if it dies before the first turn, the next activation is a new conversation) and `/clear` tracking is a best-effort exception (adversarial review 5). What a panic loses = the turn in progress · parking mail that exists only in memory (ADR-0105) · screen output.

## Rejection candidates → ADR rejected-alternative candidates

- **4 (the whole survives)** — zero precedent · risk of state corruption. If the user chooses it, it is not a rejection — the decision is the user's.
- **Replacing everything with parking_lot** — erases only the poisoning problem; the problem of 「패닉 뒤 반쯤 바뀐 상태」 (a half-changed state after a panic) stays. Not a standalone solution.

## Adversarial review

codex (`codex exec` · high · web_search) once — verdict **FIX**, all 8 applied.

1. (high · counter-evidence) The over-generalization 「살아남는 피어는 공유 상태를 안 바꾸는 단위만 가둔다」 (surviving peers contain only units that do not change shared state) — tower-http `CatchPanicLayer` also contains handlers that hold shared state (https://docs.rs/tower-http/latest/tower_http/catch_panic/). → fixed the description of (b).
2. (high · omission) The supervision-tree pattern (ractor · ractor-supervisor) was missing → added (d) · option 3.
3. (medium · overstated) 「선례 0」 (zero precedent) has no list of targets → wrote the list in conclusion 1 and narrowed the claim to 「어디서 난 패닉이든 전체 생존」 (the whole survives, whatever the panic's origin).
4. (medium · overstated) 「없음」 (none) for the state-corruption risk of separating the state owner → 「낮음 · 복원 설계 필요」 (low · restore design needed) (crash-only — https://static.usenix.org/events/hotos03/tech/full_papers/candea/candea_html/index.html).
5. (medium · overstated) Conversation is not lost → attached the ADR-0226 conditions.
6. (low · counter-evidence) VS Code pty host restart count = at most six times (`_restartCount <= MaxRestarts`).
7. (low · counter-evidence) The daemon hook calls the previous hook after logging (`lib.rs:147`).
8. (low · misattribution) rust PR #141828 is a `link.exe` diagnostics change, so it is not a basis for the abort path → removed the citation (kept only the MS fastfail docs).

Spot checks passed (reviewer): tokio default `UnhandledPanic::Ignore` · the `nonpoison` tracking issue open · no `catch_unwind` in Tauri's generated wrapper · the repository's release abort · the Job setting.

## Limits · gaps

- For the VS Code pty host's original motivation and the source of the user banner, we could not read primary material (search summaries).
- Deno's panic hook · the default panic setting of the create-tauri-app template unchecked.
- Warp · Windows Terminal · agent managers not surveyed.
- How many units in our daemon can actually be contained in the (b) way was not measured.
