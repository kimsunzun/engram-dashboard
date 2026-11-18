# ADR-0380: The profile-edit port reaches the library through a once-set weak cell filled right after the manager is built; an empty or dead cell answers profile-edit-unavailable, never a silent Noop

- 상태: 확정 (2026-11-17, 근거: S25 step-9a TRD §8 (b) B17, PRD+TRD review PASS 2026-11-17 (full)) · 부분 폐기 by ADR-0384 (relink cell check before any write)
- 관련: ADR-0311 (crate edges · ports · the daemon only wires) · ADR-0335 (references fixed presets-then-agents) · ADR-0379 (`profile-edit-unavailable` is one of the two `INTERNAL` exceptions) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §0 · §1 · §2 · §3.5 · §3.6 · §4 · §5 · §8 (b) B17 · §9 U3 · parent TRD `docs/process/S25-preset-library/trd.md` §1-4 assembly order · §1-5 profile-edit port paragraph · Amended by ADR-0384 (relink cell check before any write)

## 맥락
Agent edits go only through the profile-edit port (M18); asset-library never mutates `ProfileRegistry` and reads it directly (M5). The port is agent `ProfileEditPort`, implemented by `AgentManager` and unchanged by 9a. The port (the manager) is filled late into the library service, before `command_table_slot.set` (parent §1-4 assembly order ② ④ ⑥; handed over by step 8). Step 9a's agent-preset bodies (`attach` · `detach` · `reorder`), the `agent.presets` view (`workspace_lock` through the port) and relink's agent rewrite (`rewrite_preset_paths`) need it.

## 결정
1. **The port cell is `OnceLock<Weak<dyn ProfileEditPort>>`** held by the library service.
2. **It is set once in `build_daemon_wiring_with_store`, right after `Arc::new(manager)`**, and before `command_table_slot.set`.
3. **An empty or dead cell answers `Internal` · `profile-edit-unavailable`** (args `{}`) — never a silent Noop. It is a wiring bug, one of the two `INTERNAL` exceptions of ADR-0379. In relink's agent rewrite it gives `stopped: {code}` with no `path` (TRD §3.6).

## 거부한 대안
- **Hold the port as a strong `Arc` in the cell** — it would make an `Arc` cycle (TRD B17 「no `Arc` cycle」). Basis: descriptive (weak).
- **An empty or dead cell falls back to a Noop port** — the TRD rules it out (「never a silent Noop」, §3.5, B17); the QA mutation 「the cell check folded into a Noop」 must be caught (TRD §9 U3). Basis: descriptive (weak).

## 근거
- B17: no `Arc` cycle; filled before `command_table_slot.set` (parent §1-4 ⑥); no silent Noop.
- Step-9a PRD+TRD review (full, Claude lead + codex) PASS on `43f64c4f`, 2026-11-17 (TRD §13).

## 영향 / 불변식
- Agent-preset edits validate against the catalog and then write through the port inside one `change`: the port's last-write-wins contract relies on all writers queueing under the library write lock (agent `profile_edit.rs` `set_attachments` doc) (TRD §3.5).
- Lock order: library lock → manager / registry locks, never the reverse; port calls happen inside `change`, and the port impl never calls back into the library (TRD §4).
- Tests that must hold (TRD §9 U3): attach / detach / reorder only through the port, each call from inside a change; an empty cell ⇒ `profile-edit-unavailable`; the daemon fills the cell before the table is set (source-order guard). QA mutations: the registry mutated directly; the cell check folded into a Noop.
- Implemented in step 9a U3 (asset-library `lib.rs` plus an agent-edit module; daemon `lib.rs`). U3 fixes the `docs/reference/architecture.md` daemon ↔ library wiring sentence and the `DaemonWiring` doc if it changes.
