# ADR-0061: Preset persistence = daemon-owned (presets.json, mirrors the profile pattern)

- 상태: **폐기 (Superseded by ADR-0314)** — presets change from working-folder bookmarks (`presets.json`) to YAML assets inside the library folder, so the daemon preset store, wire and broadcast are withdrawn (ADR-0314). ~~확정 (2026-07-10, basis: pre-PRD design-decision research + user decision, step-log 2026-07-10) · 부분 폐기 by ADR-0291 (the clause where the presets store duplicated profiles)~~
- 관련: CLAUDE.md §5 (LLM-first control) · CLAUDE.md 「백엔드 모듈 맵」 (AgentManager owner = daemon) · ADR-0024 (location of `.engram-data/`) · ADR-0029 (embedded removed, daemon is the single owner of data) · ADR-0028 (backend owns the event bus) · `crates/engram-dashboard-core/src/persistence/mod.rs` (FileProfileStore, the mirror target) · step-log "에이전트 트리·프리셋" · Amended by ADR-0291 (the clause where the presets store duplicated profiles)

## 맥락

In the agent tree + presets MVP, we had to decide where to store **presets** (the list of "registered cwd paths"). A preset is a registered path that appears in the "에이전트 생성" (create agent) picker on background right-click, and it holds only `{ id, cwd }` (model, icon, backend and inject come later). This is **the first backend-persisted user data in this project** — until now the only persisted data a user created was profiles (agents.json), and frontend state (theme, chat style) lived in localStorage.

The storage location decides the "authority": several windows (main + pop-out webviews) must see the same preset list (multi-monitor), and under §5 (LLM-first control) preset CRUD must be a handle held by the backend-side LLM (the brain).

## 결정

**The daemon owns presets.** It mirrors the profile (agents.json) system as is:

- **Storage:** `.engram-data/presets.json` (ADR-0024 `default_data_dir()`). In core, `FilePresetStore` (atomic write: tmp → sync_all → rename) + `PresetRegistry` (in-memory + delegation to the store) — copies the `FileProfileStore`/`ProfileRegistry` pattern.
- **Ownership:** at startup, in `build_manager`, the daemon builds `FilePresetStore::new(data_dir)` → `PresetRegistry`, and `AgentManager` holds it (ADR-0029: the daemon is the single owner of user data).
- **Wire contract:** `AgentCommand::{ListPresets, CreatePreset{cwd}, DeletePreset{preset_id}}` + `AgentEvent::{PresetList{request_id, presets}(조회 응답), PresetListUpdated{presets}(CRUD 후 broadcast)}` — maps 1:1 to the profile's `ListProfiles`/`CreateProfile`/`DeleteProfile` + `ProfileList`/`ProfileListUpdated`.
- **Broadcast:** after create/delete, push to all connections through `broadcast_preset_list()` (ADR-0028 single push channel) → all windows sync automatically.
- **Data model:** `Preset { id: Uuid, cwd: PathBuf }`. The name is derived (cwd basename) — not stored.

## 거부한 대안

- **Frontend localStorage** — storage differs per webview (window), so it desyncs across pop-out multi-monitor. Violates §5 (the frontend would hold data authority = the brain cannot reach it). Frontend state is used only for window-local UI preferences (theme, font); shared user data is owned by the backend.
- **Owned by src-tauri (client shell)** — src-tauri is a single process, so its windows stay in sync, but ADR-0029 fixed the daemon as the single owner of user data and under §5 the brain (= the daemon-side LLM) must control presets. Putting them in src-tauri brings back the risk of dual ownership (the very problem ADR-0029 removed).
- **Reusing the profile (agents.json) for preset fields** — a profile = a spawned/reserved agent instance (it has sid, epoch and restart policy, ADR-0016). A preset = a pre-spawn "path bookmark" (not an instance). Mixing two things with different meanings in one store tangles lifetime and deletion semantics (deleting a preset ≠ turning off an agent). Split into a separate store.
- **Preset as data of a new SlotContent variant only** — the preset list is persistent state, "the set of registered paths", separate from slot rendering (PresetPalette). Tying it to UI variant config entangles its lifetime with layout save/restore — separate data (daemon) from presentation (SlotContent).

## 근거

- **The profile precedent is already proven** — atomic write, version check and corruption preservation of `FileProfileStore`, the `ProfileListUpdated` broadcast, and ProtocolClient query/subscribe are running in practice. Presets only need to copy this proven path (not built from scratch — the reference implementation principle).
- **Consistent with §5 and ADR-0029** — the daemon = the brain + single owner of user data. Putting presets here gets multi-window sync and LLM control for free (the broadcast channel is reused).
- **Minimal schema** — `{id, cwd}` only. Fields for model/icon/inject are added when a real need appears (avoids over-engineering, but the store/wire seam is laid down now so extension is cheap).

## 영향 / 불변식

- **Only the daemon writes `presets.json`** — the frontend and src-tauri do not touch the file directly. The frontend does CRUD only through the ProtocolClient wire (ADR-0011 control surface).
- **Create/delete must always be followed by a broadcast** — otherwise other windows go stale. Same as the profile `broadcast_profile_list` convention.
- **Deleting a preset ≠ terminating an agent** — a preset is a pre-spawn bookmark. An agent already spawned from that preset lives on regardless of the preset's deletion (lifetimes are separate).
- **The name is not stored (derived from the cwd basename)** — so that adding a field later, when presets get richer (user-specified names), does not conflict with this derivation rule, extend it with an `name: Option<String>` override (derivation stays the default).
