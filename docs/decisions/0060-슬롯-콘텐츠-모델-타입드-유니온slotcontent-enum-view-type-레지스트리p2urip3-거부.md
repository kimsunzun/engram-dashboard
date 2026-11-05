# ADR-0060: Slot content model = typed union (SlotContent enum) — the view-type registry (P2) and URI (P3) rejected

- 상태: 확정 (2026-07-09, basis: /research medium design-decision mode OSS survey + Codex cross-family adversarial review with FIXes applied + user decision)
- 관련: CLAUDE.md §5 (LLM control) · §0 (low-risk, long-term seam) · ADR-0035 (layout authority = src-tauri) · ADR-0057 (tab ownership) · ADR-0041/0042/0046 (output routing) · ADR-0002/0030 (capability composition) · `src-tauri/src/layout/types.rs` (LayoutNode::Slot) · TRD `docs/process/C-slot-content/TRD.md` · step-log 2026-07-09

## 맥락
The leaf slot of the layout tree (`LayoutNode::Slot { id, agent_id: Option<Uuid> }`) is hardened to hold **only a single agent**. The user wants a slot to hold heterogeneous content (a live agent + a file/entity tree + a custom control button set, etc.), but the current model has no concept of "the kind of slot occupant" at all (Some/None of agent_id is only occupied/unoccupied). Since this is a coarse-grained design, before starting we surveyed how OSS models "the kind of content in a cell" (VS Code, JupyterLab/Lumino, Theia, Obsidian, Tabby, tmux, Zellij).

## 결정
Model the slot occupant as a **typed union `SlotContent`** (a Rust enum, internally tagged with `#[serde(tag="type")]`) — the same as the existing `LayoutNode` tagging convention. The `Agent` variant absorbs today's `agent_id`, and non-agent content is added as separate variants. ts-rs generates this enum automatically as a TypeScript discriminated union → the frontend (`ViewLayoutRenderer`, a pure renderer) branches in a type-safe way with `switch(content.type)`.

```rust
#[serde(tag = "type", rename_all = "snake_case")]
enum SlotContent {
    Empty,
    Agent { agent_id: String },   // 스트림은 OutputRouter가 별도 라우팅(여기엔 바인딩만)
    // 후속: FileTree { .. }, ControlPanel { config } — variant 추가로 확장
}
```

**Key insight (preventing enum explosion):** the user's customization (a preset button set) is expressed **not as a new variant but as config data inside the `ControlPanel` variant** (data-driven). Even if hundreds of button sets are made, the Rust variants and the frontend switch cases do not grow. A new case is added only when a "structurally completely different kind" appears.

## 거부한 대안
- **P2 — a view-type registry (`{ viewType: string, state: opaque }` + a factory registry).** The dominant practice of large GUI apps (VS Code EditorInput typeId · Obsidian registerView · Theia factoryId · JupyterLab command+args). **Reason for rejection:** the premise of that practice = **a plugin ecosystem** (third parties register content kinds → the core must not know the types → type safety is given up with opaque state). In engram the **backend controls all the content kinds** (no plugin extension), and getting **end-to-end type safety** with Rust + ts-rs is a design goal (§5 · ADR-0035). The premises differ, so it is the minority approach (P1), not the practice (P2), that fits the constraints. If it becomes necessary, a `Custom { schema_id, state: JsonValue }` escape hatch can be added inside P1 later (YAGNI for now).
- **P3 — a URI + scheme resolver (`agent://…`, the scheme selects the handler).** The file-centric model of VS Code `EditorResolverService`. **Reason for rejection:** it does not fit pure views without a URI (control panel, tree), and since it is string parsing, type errors are not caught until runtime — losing the Rust authority and type-safety benefits. (It may be partly useful as the external notation of LLM commands, but is unsuitable as the internal representation.)

## 근거
- **Zellij (a Rust production precedent):** it expresses the kind of pane content as the `Run` enum (`Plugin | Command | EditFile | Cwd`), and `PaneLayoutManifest.run: Option<Run>` preserves the discriminator through session serialization too (`zellij-utils/src/input/layout.rs`, `session_serialization.rs`). But Zellij's `Run` is an "execution instruction" enum, so it supports a general heterogeneous-content model only **weakly** (beware of over-analogy — Codex's point).
- **Tagging consistency measured:** the current `LayoutNode` already uses `#[serde(tag="type", rename_all="snake_case")]` internal tagging (`types.rs:26`) → internal tagging of `SlotContent` is fully consistent, and ts-rs generates a discriminated union the same way. (Resolves Codex's point on the ts-rs tagging pitfall — a tuple variant cannot be internally tagged, but we use only struct/unit variants.)
- **No persistence:** the layout tree currently has no disk persistence (memory only, `new()` on restart). So this refactor requires no serde schema migration — Codex's versioning concern is deferred as a **requirement at the point persistence is introduced** (invariant below).
- **Judgment against the practice:** after explicitly confirming that the premise of the practice (P2, plugin apps) differs from ours, we adopted the minority approach (P1) — not "ignoring the practice" but "premise mismatch".

## 영향 / 불변식
- **Content kind and output routing are mostly orthogonal but not completely orthogonal (Codex's point):** the byte routing of `OutputRouter` only needs to pull the agent_id out of `SlotContent::Agent { agent_id }`, so it is orthogonal (ADR-0041/0042/0046 unchanged). **But lifetime and meaning are governed by the content kind** — closing a slot vs agent kill/detach, reassignment, the render default (Agent→xterm/rich · non-agent→dom), title/focus. The claim of "completely orthogonal" is discarded. The live stream is not held in `SlotContent`; only the binding (agent_id) is (epoch is outside the layout tree, owned by agentStore — ADR-0007/0046).
- **Capability composition (follow-up requirement):** behaviors that cut across variants such as closable/killable/serializable/commandable are to be localized in a **capability table** rather than scattered across exhaustive matches (consistent in idiom with engram's existing backend capability matrix, ADR-0002/0030). Out of scope for this seam slice — introduce it when the variants grow.
- **Requirement when persistence is introduced (deferred):** when the layout is persisted to disk, put into `SlotContent` together a **version envelope + an `Unknown` variant (forward-compat for old versions) + a migration pass** (Theia precedent). Not applied now because there is no persistence.
- **Room for nested layouts (open):** if a future "dashboard-style" content wants its own tabs/splits, we must reconsider whether `SlotContent` is leaf-only or compositional — this time it is leaf-only (Agent/Empty).
- **Invariants kept:** the `resolve_spawn_slot` occupancy decision (ADR-0059) moves over to `SlotContent::Empty` = empty / `Agent{..}` = occupied. The `assign_agent`/`assign_in_tree` overwrite semantics (the related invariant of ADR-0058) are kept — the occupancy defense is at the resolve layer.
