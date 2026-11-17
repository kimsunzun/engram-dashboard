# ADR-0061: Preset persistence = daemon-owned (presets.json, mirrors the profile pattern)

- 상태: 확정 (2026-07-10, basis: pre-PRD design-decision research + user decision, step-log 2026-07-10) · 부분 폐기 by ADR-0291 (the clause where the presets store duplicated profiles)
- 관련: CLAUDE.md §5(LLM-first control)·CLAUDE.md 「백엔드 모듈 맵」(AgentManager ownership = daemon) · ADR-0024(location of `.engram-data/`) · ADR-0029(embedded removal, single ownership of daemon data) · ADR-0028(backend event bus ownership) · `crates/engram-dashboard-core/src/persistence/mod.rs`(the target FileProfileStore mirrors) · step-log "Agent tree·presets" · Amended by ADR-0291 (the clause where the presets store duplicated profiles)

## 맥락

In the agent tree + presets MVP, we had to decide where to store **presets** (the list of "registered cwd paths"). A preset is the list of registered paths that shows up in the "Create agent" picker when you right-click the background, and holds only `{ id, cwd }` (model · icon · backend · inject come later). This is the **first backend-persisted user data of this project** — the only persistent data users had made until now was profiles (agents.json), and frontend state (theme · chat style) lived in localStorage.

Where it is stored decides the "authority": several windows (main + popout webviews) must see the same preset list (multi-monitor), and per §5 (LLM-first control) preset CRUD must be a handle held by the backend-side LLM (the brain).

## 결정

**Presets are owned by the daemon.** Mirror the profile (agents.json) system as it is:

- **Storage:** `.engram-data/presets.json` (ADR-0024 `default_data_dir()`). In core, `FilePresetStore` (atomic write: tmp → sync_all → rename) + `PresetRegistry` (in-memory + store delegation) — copies the `FileProfileStore`/`ProfileRegistry` pattern.
- **Ownership:** at startup the daemon builds `FilePresetStore::new(data_dir)` → `PresetRegistry` in `build_manager` and `AgentManager` holds it (ADR-0029: the daemon owns user data singly).
- **wire contract:** `AgentCommand::{ListPresets, CreatePreset{cwd}, DeletePreset{preset_id}}` + `AgentEvent::{PresetList{request_id, presets}(조회 응답), PresetListUpdated{presets}(CRUD 후 broadcast)}` — in 1:1 correspondence with the profile's `ListProfiles`/`CreateProfile`/`DeleteProfile` + `ProfileList`/`ProfileListUpdated`.
- **broadcast:** after create/delete, push to all connections with `broadcast_preset_list()` (ADR-0028 single push channel) → every window syncs automatically.
- **Data model:** `Preset { id: Uuid, cwd: PathBuf }`. The name is derived (cwd basename) — not stored.

## 거부한 대안

- **Frontend localStorage** — storage splits per webview (window), so it desyncs across popout multi-monitor. Violates §5 (the frontend holds data authority = the brain cannot reach it). Frontend state is used only for window-local UI taste (theme · font), and for shared user data the backend is the answer.
- **Owned by src-tauri (the client shell)** — src-tauri is a single process so windows sync with each other, but ADR-0029 nailed down single ownership of user data to the daemon and the §5 brain (= the daemon-side LLM) has to control presets. Putting it in src-tauri revives the root of double-ownership risk (the very problem ADR-0029 eliminated).
- **Reusing the profile (agents.json) with a preset field** — a profile = a spawned/reserved agent instance (has sid · epoch · restart policy, ADR-0016). A preset = a pre-spawn "path bookmark" (not an instance). Mixing two things with different meanings in one store tangles lifetime and deletion semantics (deleting a preset ≠ stopping an agent). Separated into its own store.
- **Preset only as new SlotContent variant data** — the preset list is persistent state, "the set of registered paths", separate from slot rendering (PresetPalette). Tying it to UI variant config entangles lifetime with layout save/restore — separate data (daemon) from presentation (SlotContent).

## 근거

- **The profile precedent is already verified** — `FileProfileStore`'s atomic write · version check · corruption preservation, the `ProfileListUpdated` broadcast, and ProtocolClient query/subscribe are working in practice. Presets only need to copy this verified path (not built from the ground up — the reference implementation principle).
- **Consistent with §5 · ADR-0029** — daemon = brain + single owner of user data. Putting presets here brings multi-window sync and LLM control for free (reuses the broadcast channel).
- **Minimal schema** — only `{id, cwd}`. Add model/icon/inject fields when real demand appears (avoiding over-engineering, but the store/wire seam is laid down now so extension is cheap).

## 영향 / 불변식

- **Only the daemon writes `presets.json`** — the frontend and src-tauri do not touch the file directly. The frontend does CRUD only over the ProtocolClient wire (ADR-0011 control surface).
- **create/delete must always lead to a broadcast** — otherwise other windows go stale. Same as the profile `broadcast_profile_list` convention.
- **Deleting a preset ≠ terminating an agent** — a preset is a pre-spawn bookmark. An agent already spawned from that preset lives regardless of the preset deletion (lifetime separation).
- **The name is not stored (derived from cwd basename)** — so that adding a field in a later preset enrichment (user-specified name) does not conflict with this derivation rule, extend with a `name: Option<String>` override (derivation is the default).
