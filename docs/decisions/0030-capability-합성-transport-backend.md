# ADR-0030: Capability derivation = transport (physical) ⊕ backend (program) composition

- 상태: 확정 (2026-06-21 dashboard11, basis: coder → reviewer-deep (Blocker 0) → QA measured [shell resume=false / claude resume=true, on the real WS→daemon→frontend IPC path]) · 부분 폐기 by ADR-0245 (the interrupt in the PTY capability list becomes false)
- 관련: ADR-0002 (capability matrix — this ADR makes the derivation location concrete) · ADR-0004 (AgentTransport seam + backend knowledge isolation) · CLAUDE.md 「백엔드 확장」 (capability matrix · backend isolation) · `crates/engram-dashboard-core/src/agent/types.rs`(TransportCaps/BackendCaps/Capabilities::compose)·`agent/backend/mod.rs`(backend_caps dispatch)·`agent/session.rs`(compose) · Amended by ADR-0245 (the interrupt in the PTY capability list becomes false)
- Scope: **who decides** the agent capability (feature matrix). Not the values themselves but the split of derivation responsibility (the seam).

## 맥락

`PtyTransport::capabilities()` **hardcoded `session.resume = true` regardless of backend (claude vs shell).** As a result a generic shell agent also came out as `resume=true` (inaccurate — a shell has no `--resume`). The cause was that capability was derived **in only one place, the transport.** The transport (PtyTransport) is isolated so that it does not know which program was launched (claude/shell) (ADR-0004) — so it cannot fill in backend-dependent capabilities (resume, etc.) accurately.

Looking at the ADR-0002 capability matrix, capability in fact comes from **two sources**:
- **What the physical channel decides:** resize (PTY yes, meaningless for an API) · interrupt · raw input · terminal-bytes output → the *transport* knows.
- **What the program decides:** resume (claude `--resume` yes, shell no) · model selection/options (API only) → the *backend* knows.

Trying to fill both in one place (the transport) got the backend-dependent values wrong.

## 결정

**Derive capability as a composition of two sources. Enforce ownership by type.**

- `TransportCaps { input, output, control }` — the physical-channel caps. Returned by the transport (`AgentTransport::capabilities() -> TransportCaps`). PTY = raw input · terminal-bytes · resize · interrupt.
- `BackendCaps { session, model }` — the program caps. Returned by the backend (`AgentBackend::capabilities() -> BackendCaps`). claude `session.resume=true`, shell `session.resume=false`, codex/gemini stub `false` (a conservative value before the CLI spike).
- `Capabilities::compose(TransportCaps, BackendCaps) -> Capabilities` — `AgentSession` combines the two to build the final 5-area Capabilities. The session receives BackendCaps at spawn via `backend::backend_caps(&AgentCommand)` and holds it.

**Why split the types:** the transport type has no session/model fields and the backend type has no input/output/control fields → mixing of sources (such as the transport filling in resume) is **impossible at compile time**. 1:1 with the matrix (§2): resize = transport / resume = backend / model = backend.

## 거부한 대안

- **The transport takes the backend kind as an argument and derives everything** — breaks the ADR-0004 isolation (the transport does not know claude/codex). If backend knowledge leaks into the transport, the transport must be changed for every backend added.
- **The transport returns full Capabilities + the session overwrites only session/model** — the transport still produces and leaks meaningless session/model values and the session quietly discards them (the type cannot prevent the mixing, room for a drop mistake). Splitting the composition types is safer.
- **Capability as a static table in profile/AgentCommand** — the derivation logic is separated from the backend implementation and drifts. It is more cohesive for the backend to declare its own caps.

## 근거

- reviewer-deep adversarial review: Blocker/Major 0. The compose 5-area 1:1 mapping · the type-split ownership · the resume correction were all confirmed as intended. The findings were only Minor (the spec/caps dispatch are separate calls, so when a future variant is added both must be updated — since `backend_for` is the single entry point, the exhaustive match currently enforces it) · Nit (clone because BackendCaps does not derive Copy).
- QA measured (the real app WS→daemon→frontend ProtocolClient): shell agent `capabilities.session.resume=false`, claude `=true`, branching exactly. Proves it is not the earlier hardcoded true.
- core unit+integration 76 passed (including the claude resume=true · shell resume=false regression tests + the compose ownership composition test).

## 영향 / 불변식

- **Capability is created only by `Capabilities::compose`.** The transport returns only TransportCaps and the backend only BackendCaps — if either side tries to fill the other's area, it is a type error. If this split is broken, the inaccuracy (the past shell resume=true) recurs.
- **session/model = backend-owned, input/output/control = transport-owned.** When adding a new capability field, choose the area by this criterion.
- When adding a backend variant, adding a branch to `backend_for` (dispatch) makes `build_command_spec` · `backend_caps` · `needs_session` see the same command, so caps and the executing backend automatically agree (keeping the single entry is the invariant).
- The wire/protocol `Capabilities` (the ts-rs mirror) stays as the 5 areas — TransportCaps/BackendCaps are for core-internal composition only (no ts-rs attached, not exposed on the wire).
- The codex/gemini BackendCaps are currently dead code because dispatch is not connected (conservative stub). When the variant is connected after the CLI spike, replace with measured values.
