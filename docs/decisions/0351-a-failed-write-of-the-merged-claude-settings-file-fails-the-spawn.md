# ADR-0351: A failed write of the merged claude settings file fails the spawn

- 상태: 확정 (2026-11-09, basis: user decision 2026-11-09 — S25 step 5 decision D7, landed `baf769ff`)
- 관련: Amends ADR-0328 (쓰기 실패 처리) · ADR-0316 (decision 3 — one merged `settings.json`, one `--settings`) · ADR-0334 (flag translation in backend `materialize`) · TRD S25 §5-2 step 5 「Step 5 landed」

## 맥락
ADR-0328 replaced the hidden settings fragment with one merged claude `settings.json` per launch, but left one item open: 「Not in the source (to confirm): what happens to 「쓰기 실패는 warn+계속」 in the new edition」. The old fragment path warned and launched without the fragment when its write failed. With real launch materials the merged file is the only carrier of the preset settings — the model, and the 「기본」 preset's `allowedMcpServers` entry that lets the agent see the engram MCP tools. Step 5 writes it in the claude backend `materialize` (`backend/claude/launch.rs`) through the `LaunchFs` port, inside the volatile folder.

## 결정
With real launch materials, a failed write of the merged claude `settings.json` fails the spawn: the claude `materialize` returns `Err`, which the manager turns into `PtyError::SpawnFailed`; nothing launches. There is no warn-and-continue path. Noop materials (every launch until TRD step 8) are unchanged.

## 거부한 대안
- **Warn and launch without the merged settings (the old fragment behaviour)** — the agent would start silently without the preset's model and without the `allowedMcpServers` entry, so it would run with no engram MCP tools and nothing on screen says why. Basis: code — the merged file is the only place those values travel (ADR-0328 decision 3 drops the fragment for real materials). Offered to the user 2026-11-09 and rejected by the user directly.

## 근거
- User decision 2026-11-09 (D7, chosen over the warn-and-continue option; the recommendation was fail closed).
- A launch that cannot carry its settings is a broken launch, not a degraded one: the missing values change what the agent can do (no MCP tools) without a visible cause.

## 영향 / 불변식
- The claude `materialize` must not swallow a settings write error; the unit test `a_failed_settings_write_fails_the_placement` (`backend/claude/launch.rs`) pins it.
- A partial placement (settings written, a later link failing, or the reverse) is cleaned up by the existing launch-kit guard → `release` path; no new cleanup is needed.
- When real materials are plugged in (TRD step 8), a write failure shows as a spawn failure in the reply and the agent's last-failure field — not as a running agent without tools.
