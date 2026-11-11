# ADR-0334: Launch materials are passed through a port — resolution in asset-library, flag translation and backend settings knowledge in backend

- 상태: 제안 (2026-10-31, basis: main decisions M2 · M16 · M18 (reported — the user may object) · M63 · M72 (re-review r7 · r8 — reported) · TRD review #2 · #3 · CLAUDE.md 「백엔드 확장」) · 부분 폐기 by ADR-0359 (decision 3 schema source and unknown-key handling)
- 관련: ADR-0004 (backend knowledge isolation) · ADR-0311 · ADR-0327 · ADR-0316 · ADR-0337 · TRD §1-4 port table · §1-5 · §7 3 · Amended by ADR-0359 (decision 3 schema source and unknown-key handling)

## 맥락
When an agent starts, the attached presets are expanded and the instructions, skills and settings are turned into CLI flags and loaded. Who knows the expansion, the flags, and the settings format that differs per backend?

## 결정
1. The launch-materials port `LaunchKitSource` (`check` · `prepare` · `release`) is defined by agent and implemented directly by the asset-library service. The manager calls it.
2. Flag translation is in backend `materialize` (`backend/claude/launch.rs` · `backend/codex/launch.rs` — flag knowledge lives only in these two).
3. The settings knowledge that differs per backend (schema source · CLI version check · raw text → JSON · claude merging · unknown-key handling · codex skill format · command-line guard) is gathered into the single settings-check port of agent `backend` (M18). Downloading goes through the fetch port the daemon plugs in.
4. `check` has no run-origin argument (M16 — the next topic adds it) · `prepare` has no priming argument (ADR-0327).
5. **The settings-check port does not read the YAML of the codex skill front matter** — the one place that knows YAML is asset-library `format.rs`, and the caller (`validate.rs`) passes that parsing function to the port as a closure. The repair (text level · codex knowledge) lives in the codex backend and calls the same parsing function again with the repaired text. agent has no YAML dependency (main 2026-10-31 — M63).
6. That parsing function deserializes the front matter into a **serde type defined by agent** — a value of the wrong type is also counted as a parse failure, so the repair is called as for codex. The type is owned by agent, and `format.rs` only runs serde-saphyr (main 2026-10-31 — M72).

## 거부한 대안
- **The manager resolves the preset** — basis (main 2026-10-31 — reported): the conflict lines of the edit area · the problem list · the tree ⛔ · the run-block reason · the launch materials are all produced by a single resolution function of asset-library — so that the screen and the run do not diverge on different rules (the 「같은 함수가 … 모두를 만든다」 paragraph of TRD §1-3). If the manager resolves the run share separately, there are two resolutions. Also, agent does not know asset-library and only defines ports (ADR-0311 Decision 2 — the crate split of user G16) — if the manager knows the preset format, that knowledge leaks into agent and the condition of the crate decision collapses (decision table — ADR-0311).
- **backend reads the library** — basis (main 2026-10-31 — reported): reading and writing the library is done only by the asset-library inside the daemon (ADR-0314 Decision 2 · TRD §7 1) · backend is inside the agent crate and does not know asset-library (ADR-0311 Decision 2). What CLAUDE.md 「백엔드 확장」 gathers into backend is knowledge that differs per backend, and reading the library is unrelated to the backend — if it were in backend, the same reading would stand twice, for claude and codex.
- **Adding a field to `ControlEndpoint`** — basis (main 2026-10-31 — reported): `ControlEndpoint` is the descriptor of what is loaded only together with the control channel — `Option<ControlEndpoint>` answers 「제어 채널이 있나」 as a type (ADR-0099 · that struct comment in `crates/engram-dashboard-agent/src/types.rs`) and if there is no endpoint, what is loaded there is not loaded either (ADR-0217 Decision 5). Launch materials are loaded regardless of the presence of a control channel and have their own lifetime — `LaunchKitGuard` is independent of `ProvisionGuard` and the material descriptor is a separate argument in the same way as `ControlEndpoint` (TRD §1-5 (b) 5 · 7).
- **Settings-check knowledge in the library (`validate.rs`)** — TRD review #2 · #3 withdrew it and moved it to the backend port (M2). If left as is, the condition of the crate split decision does not hold (decision table conclusion — ADR-0311).
- **Taking the front matter as a generic YAML tree** — a type mismatch is not counted as a parse failure, so the repair is not called, and a skill that codex would load is blocked with ⛔ (main 2026-10-31 — M72 · TRD §1-5 settings-check port).

## 근거
CLAUDE.md 「백엔드 확장」 (backend-specific code is gathered in one place, `backend`) · M18 · M63 · M72 (re-review r7 · r8 — reported). The basis of the three rejected alternatives = above (main 2026-10-31 — TRD §1-3 · ADR-0311 Decision 2 · ADR-0314 Decision 2 · ADR-0099 · ADR-0217 Decision 5).

## 영향 / 불변식
- It is a violation if flag or backend settings format knowledge leaks into asset-library or the manager. A new backend adds `materialize` and a settings-check port implementation.
