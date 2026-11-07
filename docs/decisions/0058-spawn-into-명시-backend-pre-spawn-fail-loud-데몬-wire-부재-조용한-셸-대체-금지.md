# ADR-0058: spawn_into with an explicit backend = pre-spawn fail-loud (the daemon wire is absent — no silent shell substitution)

- 상태: 확정 (2026-07-09, basis: stage 5 commit 88134e2 + /review code full closed with PASS)
- 관련: CLAUDE.md 「백엔드 확장」 (backend knowledge isolation) · ADR-0004 (backend seam) · `src-tauri/src/commands/layout.rs:346-374` (backend fail-loud guard) · `crates/engram-dashboard-daemon/src/connection_core.rs:852` (SpawnByCwd = the default shell) · TRD `docs/process/B-wezterm-tabs/TRD.md` §6 D-7 · step-log Phase 2 stage 5

## 맥락
`spawn_into(window, tab?, slot?, backend?, cwd)` of Phase 2 tabs stage 5 is a command that assembles spawn + create_tab + slot assignment in one shot (TRD D-7). The signature has a `backend` argument, but the actual daemon spawn wire, `SpawnByCwd{cwd}` (`connection_core.rs:852`), **takes only cwd and has no backend selection argument** — the daemon always spawns the default backend (currently `default_shell()`). That is, the command surface has a backend choice, but there is no channel to carry that intent to the daemon. Even if the caller passes `backend="claude"`, a shell actually comes up.

## 결정
An explicit `backend` request is **rejected before spawning** (fail-loud). The only thing that passes is an unspecified `backend` (`None`/empty/whitespace), and **any explicit value, including `"claude"`, is rejected** (what is spawned now is a shell, so "claude supported" would be a lie). The rejection happens at a stage before agent creation, so it has no side effects (nothing has died yet → `alive_err` is unnecessary). Actual backend selection needs, as a **follow-up**, an extension of `SpawnByCwd` in the protocol crate + an extension of daemon dispatch, and it will be recorded in a separate ADR then.

## 거부한 대안
- **Extend the protocol now (1b) — add the backend selection wire immediately.** The spawn_into stage is a thin assembly slice (TRD D-7), and digging into the protocol crate `SpawnByCwd` + daemon dispatch would greatly exceed the stage scope. There is no real demand for backend selection yet (codex/gemini are not connected), so extending now is an investment on an unverified assumption (CLAUDE.md architecture §0 — high cost and uncertain = empty shell only). → Leave the shell (the argument only) and defer filling it until there is real demand.
- **Silently ignore the requested backend and spawn the default shell.** It was an early candidate, but it causes a malfunction where the caller silently receives a different agent from the one they wanted — if the LLM believes it spawned with `backend="claude"` and continues operating afterwards, everything is off. Fail-loud is safer than a silent malfunction.

## 근거
- Daemon wire measurement: `SpawnByCwd` consumes only cwd and ignores backend (`connection_core.rs:852`) — confirmed in code that there is no path for the backend argument of the command surface to reach the daemon.
- In the `/review code full` 2-family adversarial review, the doc-aware reviewer pointed out that the initial "ignore backend" implementation was "a lie (claims claude support)" → corrected to fail-loud, then closed with PASS.
- User decision: "1-a" (deferral · fail-loud) was settled in this session's conversation.

## 영향 / 불변식
- **Do not force backend into SpawnByCwd** — the wire does not exist. Until the protocol extension (deferred), keep fail-loud for an explicit backend, and do not silently substitute the default spawn.
- The guard lives in the pre-spawn validation block of `src-tauri/src/commands/layout.rs` (currently `:366-374`). When the daemon spawn protocol is later extended so that backend selection actually works, supersede this ADR and relax the guard.
- Consistent with ADR-0004 (backend knowledge isolation): claude-specific arguments exist only in `backend/claude.rs`, and spawn_into handles backend only as a dispatch and does not know it directly.
