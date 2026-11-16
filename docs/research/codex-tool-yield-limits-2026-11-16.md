# Codex tool yield limits and the launch wait (S25 step 8, C1)

Status: light check, 2026-11-16 — source read at `openai/codex` tag `rust-v0.167.0` (commit `086ccc83`) plus one measurement in S25 step 8 QA (L8). Decision at the end.

## Question

A codex agent runs `engram agent spawn …`, which waits up to 94 s (ADR-0360 / ADR-0369 chain). How long does codex's own tool wait for one call, and can a config value make it wait the whole launch?

## Measurement (S25 step 8 QA, L8 — codex-cli 0.167.0, JSON mode, real kits)

- The model ran a 100 s sleep through the JS `exec` tool with `yield_time_ms: 10000` (copied from the tool description's pragma example).
- The call returned after 10.0 s with no output and a live cell id; the command kept running.
- The model polled with `wait` (5 s, 31 s, 11 s ×3) and received `done` at about 102.6 s. Nothing was cut.

## Source (paths relative to `codex-rs/` at `rust-v0.167.0`)

| Tool | Default | Per-call cap | Config |
|---|---|---|---|
| shell `exec_command` (unified exec) | 10 000 ms (`tools/handlers/unified_exec.rs` `default_exec_yield_time_ms`) | clamped 250..30 000 ms, Windows floor 10 000 (`core/src/unified_exec/mod.rs` `MAX_YIELD_TIME_MS`, `clamp_yield_time`) | none — hard-coded |
| `write_stdin` with input | 250 ms | 30 000 ms | none |
| `write_stdin` empty poll | — | 5 000 .. `background_terminal_max_timeout` (default 300 000 ms) | `background_terminal_max_timeout` (`config/src/config_toml.rs`) |
| JS `exec` (code mode) | 30 000 ms (`core/src/config/mod.rs` `DEFAULT_CODE_MODE_EXEC_YIELD_TIME_MS`) | none by default (`code-mode-runtime/src/service.rs` `resolve_yield_timeout` caps only when `max_yield_time_ms` is set; core never sets it) | `[features.code_mode] default_exec_yield_time_ms` — the default only |
| JS `wait` | 10 000 ms | none by default | none |

Unverified: that both config keys take effect in app-server (JSON) mode at runtime (same `ConfigBuilder` — likely); which tools codex terminal mode uses.

## Reading

- A codex call never waits the full 94 s in one go: the shell tool is capped at 30 s, and the JS tools use whatever the model writes per call (it wrote 10 s). A config default does not override a value the model writes.
- Yielding is not failure: the command keeps running and a later poll (empty `write_stdin` up to 5 min, or `wait`) returns the result. The spawned agent comes up either way; only the caller's knowledge of the result depends on polling.
- Claude Code's Bash tool waits up to its timeout (default 2 min, from the tool contract — not measured here), which already exceeds 94 s.

## Decision (user, 2026-11-16)

C1 (step-8 TRD §8 (c)): **no change** — no priming line, no codex config. Waiting or polling is the backend's own tool behaviour ("최소한만 막기"). Revisit only if a codex agent is seen misreporting a spawn it did not poll for; the remedy would then be on Engram's side or in the user's shared agent rules, not in the built-in priming.

## Sources

- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/unified_exec/mod.rs
- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/unified_exec/process_manager.rs
- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/tools/handlers/unified_exec.rs
- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/core/src/config/mod.rs
- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/config/src/config_toml.rs
- https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/code-mode-runtime/src/service.rs
