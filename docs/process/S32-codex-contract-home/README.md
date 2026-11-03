# S32 — codex contract harness on codex 0.163.0

The `#[ignore]` codex contract test (`crates/engram-dashboard-agent/tests/backend_contract.rs`, the qa codex lane) failed 6/11 after codex 0.163.0: the temp CODEX_HOME path made codex's app-server socket exceed SUN_LEN, and the folder-trust prompt was reworded. The harness now uses a short temp home with the socket path budget asserted, passes the first screen without depending on its wording, suppresses update/login screens through the temp home's config, and requires a trust record for the cwd after any accept. Production code is unchanged.

- Related ADR: none (test harness only).
- Tracking: none.
- Timeline: `docs/process/step-log.md` `## S32`.
