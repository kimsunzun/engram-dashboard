# ADR-0061: Preset persistence = daemon-owned (presets.json, mirroring the profile pattern)

- 상태: 확정 (2026-07-10, basis: pre-PRD design-decision research + user decision, step-log 2026-07-10) · 부분 폐기 by ADR-0291 (the clause where the presets store duplicated profiles)
- 관련: CLAUDE.md §5 (LLM-first control) · CLAUDE.md 「백엔드 모듈 맵」 (AgentManager owner = daemon) · ADR-0024 (`.engram-data/` location) · ADR-0029 (embedded removed, daemon is the single owner of data) · ADR-0028 (backend owns the event bus) · `crates/engram-dashboard-core/src/persistence/mod.rs` (FileProfileStore, the mirror target) · step-log "에이전트 트리·프리셋" · Amended by ADR-0291 (the clause where the presets store duplicated profiles)

## 맥락

For the agent tree + presets MVP, we had to decide where to store **presets** (the list of "registered cwd paths"). Presets are the list of registered paths shown in the "에이전트 생성" (create agent) picker on background right-click, and they hold only `{ id, cwd }` (model, icon, backend and inject come later). This is **the first backend-persistent user data in this project** — until now the only persistent data a user created was profiles (agents.json), and frontend state (theme, chat style) lived in localStorage.

The storage location decides the "authority": multiple windows (main + popout webviews) must see the same preset list (multi-monitor), and under §5 (LLM-first control) preset CRUD must be a handle held by the backend-side LLM (the brain).

## 결정

**The daemon owns presets.** They mirror the profile (agents.json) system as is:

- **Storage:** `.engram-data/presets.json` (ADR-0024 `default_data_dir()`). In core, `FilePresetStore` (atomic write: tmp → sync_all → rename) + `PresetRegistry` (in-memory + delegation to the store) — a copy of the `FileProfileStore`/`ProfileRegistry` pattern.
- **Ownership:** at startup the daemon builds `FilePresetStore::new(data_dir)` → `PresetRegistry` in `build_manager`, and `AgentManager` holds it (ADR-0029: the daemon is the single owner of user data).
- **Wire contract:** `AgentCommand::{ListPresets, CreatePreset{cwd}, DeletePreset{preset_id}}` + `AgentEvent::{PresetList{request_id, presets}(조회 응답), PresetListUpdated{presets}(CRUD 후 broadcast)}` — a 1:1 match with the profile's `ListProfiles`/`CreateProfile`/`DeleteProfile` + `ProfileList`/`ProfileListUpdated`.
- **Broadcast:** after create/delete, `broadcast_preset_list()` pushes to every connection (ADR-0028 single push channel) → all windows sync automatically.
- **Data model:** `Preset { id: Uuid, cwd: PathBuf }`. The name is derived (cwd basename) — not stored.

## 거부한 대안

- **Frontend localStorage** — storage splits per webview (window), so popout multi-monitor desyncs. Violates §5 (the frontend would hold data authority = the brain cannot reach it). Frontend state is used only for window-local UI preferences (theme, font); shared user data is correctly owned by the backend.
- **src-tauri (client shell) ownership** — src-tauri is a single process, so its windows stay in sync with each other, but ADR-0029 nailed down the daemon as the single owner of user data, and the §5 brain (= the daemon-side LLM) must control presets. Putting them in src-tauri would bring back the root of the double-ownership risk (the very problem ADR-0029 eliminated).
- **Reuse a preset field in the profile (agents.json)** — a profile = a spawned/reserved agent instance (it has sid, epoch, restart policy, ADR-0016). A preset = a "path bookmark" before spawn (not an instance). Mixing two things with different meanings into one store tangles lifetime and deletion semantics (deleting a preset ≠ turning off an agent). Split into a separate store.
- **Keep presets only as data of a new SlotContent variant** — the preset list is persistent state, "the set of registered paths", separate from slot rendering (PresetPalette). Tying it to the UI variant config entangles its lifetime with layout save/restore — separate data (daemon) from presentation (SlotContent).

## 근거

- **The profile precedent is already proven** — `FileProfileStore`'s atomic write, version check and corruption preservation, the `ProfileListUpdated` broadcast, and ProtocolClient query/subscribe are working in practice. Presets only need to copy this proven path (not built from the ground up — the reference-implementation principle).
- **Consistent with §5 and ADR-0029** — daemon = brain + single owner of user data. Putting presets here gives multi-window sync and LLM control for free (the broadcast channel is reused).
- **Minimal schema** — only `{id, cwd}`. Fields for model/icon/inject are added when real demand appears (avoids over-engineering, but the store/wire seam is laid down now so extension is cheap).

## 영향 / 불변식

- **Only the daemon writes `presets.json`** — the frontend and src-tauri never touch the file directly. The frontend does CRUD only through the ProtocolClient wire (ADR-0011 control surface).
- **Create/delete must always be followed by a broadcast** — otherwise other windows go stale. Same as the profile `broadcast_profile_list` convention.
- **Deleting a preset ≠ terminating an agent** — a preset is a pre-spawn bookmark. An agent already spawned from that preset lives on regardless of the preset deletion (lifetimes are separate).
- **The name is not stored (derived from the cwd basename)** — when a field is added later in preset enrichment (user-specified names), extend it with a `name: Option<String>` override so it does not conflict with this derivation rule (derivation is the default).
