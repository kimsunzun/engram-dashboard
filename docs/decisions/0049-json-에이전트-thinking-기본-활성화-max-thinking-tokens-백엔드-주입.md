# ADR-0049: Enable thinking by default for JSON agents — the backend injects MAX_THINKING_TOKENS

- 상태: 확정 (2026-07-06, basis: CLI spike measurement + process env measurement + live ThinkingRow measurement)
- 관련: ADR-0004 (backend knowledge isolation) · ADR-0045 (decoder — thinking→Structured mapping; the stream-json origin is ADR-0044) · `crates/engram-dashboard-core/src/agent/backend/claude.rs` (`// ADR-0049` anchor) · step-log 2026-07-06 section (Cline visual fidelity + enabling thinking)

## 맥락
The thinking display pipe was already complete — the decoder maps a `"type":"thinking"` block to `Structured{kind:"thinking"}` (ADR-0045 — ADR-0044 was the text MVP, and the thinking mapping is the part of 0045 that moved into the decoder backend), and the frontend ThinkingRow renders it as a collapsed row (ADR-0048 dispatch). Yet in real use thinking was never displayed. Cause: the claude CLI **does not turn extended thinking on by default** in headless (stream-json) mode — it emits thinking blocks only if the env `MAX_THINKING_TOKENS` is present at spawn (measured: claude 2.1.170, 2026-07-06). The dashboard did not pass this env at spawn, so the blocks never arrived.

## 결정
`build_spec` in `backend/claude.rs` injects the env `MAX_THINKING_TOKENS=8000` by default, **only in json (stream-json) mode**.
- **Profile wins (explicit-skip):** if the profile env already has the same key, do not inject — a deterministic approach that does not rely on the merge order (last-wins).
- **Case-insensitive:** Windows environment variables are case-insensitive — compare with `eq_ignore_ascii_case` so that a lowercase profile key also prevents a duplicate injection (reflects a cross-family review finding).
- **Do not inject in terminal/interactive mode** — keep behavior the same as the bare CLI (parity).
- Normalizing duplicate keys inside the profile env is out of scope — follow the standard last-wins semantics common to all keys.

## 거부한 대안
- **Add env only to profile data (agents.json)** — no code change, but it has to be repeated by hand for every agent, needs a daemon restart, and a new JSON agent does not get thinking by default. The mismatch of shipping with the default off although the display pipe already exists would continue.
- **CLI flag approach** — claude 2.1.170 has no flag dedicated to enabling thinking (at the time of measurement). The env is the only confirmed mechanism.
- **Inject in terminal mode too** — interactive claude has its own thinking toggle (Tab) and settings. If the dashboard interferes, behavior splits from the bare CLI and causes confusion (parity violation).

## 근거
- **Spike (2026-07-06, claude 2.1.170):** without the env, zero thinking blocks; with `MAX_THINKING_TOKENS=8000`, `"type":"thinking"` blocks are emitted.
- **Process measurement:** confirmed that the key exists in the env of the claude child process spawned by the daemon (QA full).
- **Live measurement:** with a sonnet-4-6 agent, confirmed ThinkingRow collapse/expand and plain-text reasoning rendering (`qa-thinking-sonnet.png`).
- **Known limit (upstream):** opus-4-8 emits thinking in headless stream-json **only as encrypted (signature)** — the plain text is empty and the UI suppresses the empty row as designed, so nothing shows on screen. The thinking_delta of `--include-partial-messages` was also measured to have empty plain text — it cannot be solved by our code. It becomes visible only with models that give plain text (sonnet, etc.).

## 영향 / 불변식
- The claude knowledge (the env key and injection condition) exists only in `backend/claude.rs` (the ADR-0004 isolation is kept — it does not leak into manager/transport).
- Four regression-guard tests: default injection / profile wins (exactly one effective value) / no duplicate with a lowercase profile key / no injection in terminal mode.
- The default 8000 is a budget cap, not forced usage — the model uses only as much as it needs. When changing it, update this ADR and the tests together.
