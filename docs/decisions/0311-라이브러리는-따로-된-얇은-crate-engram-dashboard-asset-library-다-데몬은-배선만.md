# ADR-0311: The library is a separate thin crate `engram-dashboard-asset-library` — the daemon only wires it

- 상태: 제안 (2026-10-31, basis: user decision G16 (2026-10-20 「라이브러리를 별도 create로 확정하는건 맞는데」) · G24 (2026-10-29 「asset 으로해」) + main decisions M1 · M17 · M18 · M25 (reported) · decision table `docs/research/crate-split-checklist-2026-10-20.md`)
- 관련: replaces the old memo 96 (a module inside the daemon — user 2026-10-14) · ADR-0151 (criteria for splitting a crate) · ADR-0155 Decision 1 (vocabulary lives next to its producer — `library.*` is the agent block) · ADR-0251 (synchronous port + `blocking_handler` precedent) · ADR-0175 · ADR-0269 (conditions for moving into base) · CLAUDE.md 「백엔드 확장」 · ADR-0334 · TRD `docs/process/S25-preset-library/trd.md` §1-1 rejected alternatives · §1-4 · §7 30

## 맥락
Where to put the library (scan, read, write, resolve and relink for instruction, skill, settings and preset files). Memo 96 (2026-10-14) decided on a dedicated module inside the daemon crate — the trait seam is placed from the start, base was ruled out because it fails the move-in conditions (domain knowledge, one consumer), and a separate crate was deferred to 「다른 crate 가 필요하거나 프리셋 로직이 커지거나 격리 시험이 필요할 때」. The old rejection reason in the TRD was 「소비자가 하나다」.

## 결정
1. The library is a new crate `crates/engram-dashboard-asset-library/` (Rust `engram_dashboard_asset_library`). Name = what it holds is the asset loaded onto an agent, so asset (matches the TRD vocabulary `AssetRef`). The bus command `library.*` and the screen name 「라이브러리」 stay as they are.
2. Dependencies = exactly agent · platform · command · base. The daemon depends on this crate and **only wires** it (creating, plugging in ports, passing values). agent does not know this crate and only defines ports.
3. The outside contact points are ports. This crate's service directly implements `LaunchKitSource` · `LibraryCommandHost` (a typed method per command). The settings knowledge that differs per backend goes to the settings-check port of agent `backend` (ADR-0334). Roster changes go through the manager's profile-edit port — roster changes and spawn reservation use one lock order (M18).
4. It has no network client — fetching the schema goes through a fetch port the daemon plugs in (the trait is defined by this crate — M25). It has no test feature (`test-support`) — daemon-side tests use the real `FsStore` + a temporary folder (M17).
5. It does not call daemon internals (`connection_core` · `command_delivery` · `control`). The daemon passes values (the place `DataLayout` produced · `daemon_id`).
6. Four gates (M17): the test line · the dependency cap = the name **set** {agent · asset-library · base · command · platform} (not a line count) · `asset_library` in the messaging isolation regex · zero `use tauri` lines. The async import gate will be decided when fetching the schema comes in.

## 거부한 대안
- **A module inside the daemon** (memo 96 — user 2026-10-14 「애매하긴 하지만 그렇게 해」) — the user settled on a separate crate on 2026-10-20 (G16 — 「분리할수 있으면 분리 … 데몬안에 있는 조건이 데몬에 있는걸 너무 많이 연관시켜야되고 상호참조가 필요해서 어쩔수 없을때」). The old rejection reason 「소비자가 하나다」 is a criterion that ADR-0151 Decision 1 discarded (decision table conclusion · TRD §1-1).
- **Putting it in base** — it has domain knowledge and one consumer, so it fails the move-in conditions (memo 96 · ADR-0269).
- **The crate name `engram-dashboard-library`** (the memo's example) — the user chose asset (G24 — what it holds is the asset · `AssetRef` vocabulary).

## 근거
- Decision table: all five questions of 「분리할 수 있나」 are 「예」 — but only after moving the backend check knowledge to the backend port (M2). The cost is only fixed items (manifest, gate registration, public surface), and it is cheapest to add now, when no incoming edges exist (TRD §1-1). The decision table's confidence = medium.
- There is no type to move (decision table C6 = 0) · no test feature is needed (C5 = 0) (TRD §1-4).

## 영향 / 불변식
- If this crate comes to call daemon internals, the crate decision is overturned (TRD §1-4 · decision table §3 「뒤집는 것」).
- Daemon assembly order (TRD §1-4): registry → library service (the profile-edit port = a blank filled in late · a weak reference after it is filled) → manager → fill the blank → schema refresh thread. Filling the blank comes **before** the command table slot `command_table_slot.set` — the control route (MCP server) starts before the daemon wiring, so it is that table slot that guards the blank (a route before the table is up = 503), and the accept loop opens after it (M31 — the old 「채운 다음에 수락 루프 · 제어 라우트를 연다」 did not match the code). Touching the blank answers with an error — it is not folded into a Noop.
- The daemon's `DataLayout` passes the `daemon\state\` folder, and the `library.json` name inside it is owned by asset-library (ADR-0264 Decision 4 · M30).
- The new lines of CLAUDE.md 「백엔드 모듈 맵」 · 「빌드·검증 명령」 are added at implementation time after user confirmation (M1 · M17).
- Memo 96 is marked as replaced (TRD §5 0c).
