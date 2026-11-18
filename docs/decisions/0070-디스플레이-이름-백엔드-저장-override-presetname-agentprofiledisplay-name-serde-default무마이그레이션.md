# ADR-0070: Display name = backend-stored override (Preset.name / AgentProfile.display_name, serde default · no migration)

- 상태: 확정 (2026-07-12, basis: implementation + review deep 3 reviewers PASS · qa full PASS)
- 관련: CLAUDE.md §5(LLM-first control) · ADR-0061(preset persistence · cwd basename name derivation) extended · ADR-0057(tab rename) mirrored · `crates/engram-dashboard-core/src/agent/preset.rs`(`Preset.name`·`rename`) · `.../agent/profile.rs`(`display_name`·`rename`) · `crates/engram-dashboard-protocol/src/messages.rs`(`RenamePreset`/`RenameProfile`) · `crates/engram-dashboard-daemon/src/connection_core.rs`(dispatch) · step-log

## 맥락

The names shown in the preset palette (`PresetPalette`) and the agent tree (`AgentList`) were until now **derived** from the basename of the registered cwd (ADR-0061), so the user could not change them directly. Tabs can already be renamed inline (ADR-0057) but presets and agents cannot, so it is asymmetric.

The key constraint is that existing fields cannot be reused as the display name:
- **`AgentProfile.name`** is not a display name — in the `CreateProfile` path it holds the claude profile name, and in the ad-hoc `SpawnByCwd` path it holds **the whole cwd string**, so it is a polluted axis (`connection_core.rs`). So the frontend tree ignored this value and drew the cwd basename.
- **`Preset`** has no name field at all (cwd basename derivation, ADR-0061).

## 결정

Store the display-name override **in the backend**. Two fields are added:
- `Preset.name: Option<String>` (`#[serde(default)]`)
- `AgentProfile.display_name: Option<String>` (`#[serde(default)]`) — an axis **separate** from the existing `name`

Display rule: `Some` → show as is, `None` → derive the cwd basename (**existing behavior unchanged**). rename is exposed as the protocol command `RenamePreset`/`RenameProfile` and sets (`Some`)/clears (`None`) the override. Normalization (trim · reject empty string · skip unchanged) is handled **by the frontend right before confirming** (same shape as the TabBar rename) — only a valid value or an explicit `None` reaches the backend. On success, `PresetListUpdated`/`ProfileListUpdated` is broadcast to all connections (no optimistic update — all windows sync).

## 거부한 대안

- **Frontend-only localStorage storage** — violates §5. If rename lives only in frontend state, the backend-side LLM/orchestrator has no control handle to reach it, so renaming happens **only by human click**, and it is not persistent across windows or machines.
- **Reusing the existing `AgentProfile.name`** — it is a polluted axis so it cannot be used (see the context above). Overwriting `name` with the display name destroys its original meaning of profile identification → add a separate axis `display_name`.
- **Adding fields + a migration script** — unnecessary. Because of `#[serde(default)]`, old `presets.json`/`agents.json` without this field are absorbed as `None` on load → no migration.

## 근거

§5 (LLM-first control) is the key reason for backend storage — rename is exposed as a command, so the backend-side LLM/orchestrator also renames through the same handle (human click is an auxiliary input to the same handle). The serde default no-migration was confirmed by a unit test (JSON without the field → `None` round-trip). Review deep 3 reviewers PASS · qa full PASS (GUI measurement: a real daemon round trip `null↔override↔null` through the §5 handle for both preset and agent, broadcast reflected).

## 영향 / 불변식

- **Display name priority = override (`Some`) > cwd basename.** The frontend tree and palette do not use `AgentProfile.name` (the profile identification axis) as the display name — `display_name`/`preset.name` take priority.
- Deleting these two fields or merging them into `name` breaks §5 rename and no-migration absorption.
- Removing `#[serde(default)]` is forbidden — loading old JSON (no field) breaks.
- The persistence consistency of concurrent rename is guaranteed by ADR-0071 (lock discipline) — exposing the §5 command is what opens that concurrency window, so the two ADRs are one slice.
