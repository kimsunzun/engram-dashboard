# ADR-0070: Display name = backend-stored override (Preset.name / AgentProfile.display_name, serde default, no migration)

- 상태: 확정 (2026-07-12, basis: implementation + review deep 3-reviewer PASS · qa full PASS)
- 관련: CLAUDE.md §5 (LLM-first control) · ADR-0061 (preset persistence · cwd basename name derivation) extension · mirrors ADR-0057 (tab rename) · `crates/engram-dashboard-core/src/agent/preset.rs` (`Preset.name`, `rename`) · `.../agent/profile.rs` (`display_name`, `rename`) · `crates/engram-dashboard-protocol/src/messages.rs` (`RenamePreset`/`RenameProfile`) · `crates/engram-dashboard-daemon/src/connection_core.rs` (dispatch) · step-log

## 맥락

The names shown in the preset palette (`PresetPalette`) and the agent tree (`AgentList`) have so far been **derived** from the basename of the registered cwd (ADR-0061), so the user could not change them directly. Tabs can already be renamed inline (ADR-0057), but presets and agents cannot, so it is asymmetric.

The key constraint is that existing fields cannot be reused as a display name:
- **`AgentProfile.name`** is not a display name — the `CreateProfile` path puts in the claude profile name, and the ad-hoc `SpawnByCwd` path puts in **the whole cwd string**, so it is a polluted axis (`connection_core.rs`). That is why the frontend tree has ignored this value and drawn the cwd basename.
- **`Preset`** has no name field at all (cwd basename derivation, ADR-0061).

## 결정

Store the display-name override **in the backend**. Add two fields:
- `Preset.name: Option<String>` (`#[serde(default)]`)
- `AgentProfile.display_name: Option<String>` (`#[serde(default)]`) — a **separate axis** from the existing `name`

Display rule: `Some` → show as is, `None` → derive the cwd basename (**existing behavior unchanged**). rename is exposed as the protocol commands `RenamePreset`/`RenameProfile` and sets (`Some`) / clears (`None`) the override. Normalization (trim, reject an empty string, skip when unchanged) is handled **by the frontend right before confirmation** (the same shape as the TabBar rename) — only a valid value or an explicit `None` reaches the backend. On success, `PresetListUpdated`/`ProfileListUpdated` is broadcast to every connection (no optimistic update — all windows sync).

## 거부한 대안

- **Frontend-only localStorage storage** — a §5 violation. If rename lives only in the frontend state, there is no control handle the backend-side LLM/orchestrator can reach, so renaming happens **only by human click**, and it is not persistent across windows or machines.
- **Reuse the existing `AgentProfile.name`** — it is a polluted axis, so it cannot be used (see Context). Overwriting `name` with a display name destroys its original meaning of profile identification → add a separate axis `display_name`.
- **Add fields + a migration script** — unnecessary. Because of `#[serde(default)]`, an old `presets.json`/`agents.json` without this field is absorbed as `None` on load → no migration.

## 근거

§5 (LLM-first control) is the core reason for backend storage — rename is exposed as a command so the backend-side LLM/orchestrator also changes names through the same handle (a human click is an auxiliary input to the same handle). The serde default no-migration was confirmed by a unit test (JSON without the field → `None` round-trip). Review deep 3-reviewer PASS · qa full PASS (GUI measurement: for both presets and agents, a real daemon round trip `null↔override↔null` through the §5 handle, broadcast reflected).

## 영향 / 불변식

- **Display-name priority = override (`Some`) > cwd basename.** The frontend tree and palette do not use `AgentProfile.name` (the profile identification axis) as a display name — `display_name`/`preset.name` take priority.
- If you delete these two fields or merge them into `name`, the §5 rename and the no-migration absorption break.
- Do not remove `#[serde(default)]` — loading an old JSON (without the field) would break.
- The persistence consistency of concurrent renames is guaranteed by ADR-0071 (lock discipline) — exposing it as a §5 command is what opens that concurrency window, so the two ADRs are one slice.
