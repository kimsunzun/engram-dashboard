# ADR-0070: Display name = backend-stored override (Preset.name / AgentProfile.display_name, serde default, no migration)

- 상태: 확정 (2026-07-12, basis: implementation + review deep 3 reviewers PASS, qa full PASS) · 부분 폐기 by ADR-0314 (the Preset.name preset-name clause)
- 관련: CLAUDE.md §5 (LLM-first control) · extends ADR-0061 (preset persistence, name derived from cwd basename) · mirrors ADR-0057 (tab rename) · `crates/engram-dashboard-core/src/agent/preset.rs` (`Preset.name` · `rename`) · `.../agent/profile.rs` (`display_name` · `rename`) · `crates/engram-dashboard-protocol/src/messages.rs` (`RenamePreset`/`RenameProfile`) · `crates/engram-dashboard-daemon/src/connection_core.rs` (dispatch) · step-log · Amended by ADR-0314 (the Preset.name preset-name clause)

## 맥락

The name shown in the preset palette (`PresetPalette`) and the agent tree (`AgentList`) has so far been **derived** from the basename of the registered cwd (ADR-0061), so users could not change it directly. Tabs can already be renamed inline (ADR-0057), but presets and agents cannot, which is asymmetric.

The key constraint is that no existing field can be reused as the display name:
- **`AgentProfile.name`** is not a display name — on the `CreateProfile` path it holds the claude profile name, and on the ad-hoc `SpawnByCwd` path it holds the **whole cwd string**, so it is a polluted axis (`connection_core.rs`). That is why the frontend tree ignored this value and drew the cwd basename.
- **`Preset`** has no name field at all (derived from the cwd basename, ADR-0061).

## 결정

Store the display-name override **in the backend**. Add two new fields:
- `Preset.name: Option<String>` (`#[serde(default)]`)
- `AgentProfile.display_name: Option<String>` (`#[serde(default)]`) — a **separate axis** from the existing `name`

Display rule: `Some` → show as is; `None` → derive from the cwd basename (**existing behavior unchanged**). Rename is exposed as the protocol commands `RenamePreset`/`RenameProfile`, which set (`Some`) or clear (`None`) the override. Normalization (trim, reject empty string, skip when unchanged) is handled **by the frontend right before commit** (same shape as TabBar rename) — only a valid value or an explicit `None` reaches the backend. On success, `PresetListUpdated`/`ProfileListUpdated` is broadcast to all connections (no optimistic update — all windows stay in sync).

## 거부한 대안

- **Frontend-only localStorage storage** — violates §5. If rename lives only in frontend state, there is no control handle the backend-side LLM/orchestrator can reach, so renaming happens **only by human click**, and it is not persistent across windows or machines.
- **Reusing the existing `AgentProfile.name`** — cannot be used because it is a polluted axis (see 맥락 above). Overwriting `name` with the display name destroys its original meaning of identifying the profile → add a separate axis `display_name`.
- **New field + migration script** — unnecessary. Because of `#[serde(default)]`, old `presets.json`/`agents.json` without this field are absorbed as `None` on load → no migration.

## 근거

§5 (LLM-first control) is the core reason for backend storage — rename is exposed as a command, so the backend-side LLM/orchestrator renames through the same handle (human click is a secondary input to the same handle). The serde default no-migration behavior is confirmed by a unit test (JSON without the field → `None` round-trip). Review deep 3 reviewers PASS · qa full PASS (GUI measured: a real daemon round trip `null↔override↔null` through the §5 handle for both presets and agents, broadcast reflected).

## 영향 / 불변식

- **Display name priority = override (`Some`) > cwd basename.** The frontend tree and palette do not use `AgentProfile.name` (the profile identification axis) as the display name — `display_name`/`preset.name` come first.
- Deleting these two fields or merging them into `name` breaks §5 rename and the no-migration absorption.
- Do not remove `#[serde(default)]` — loading old JSON (without the field) would break.
- The persistence consistency of concurrent renames is guaranteed by ADR-0071 (lock discipline) — exposing the §5 command is what opens that concurrency window, so the two ADRs form one slice.
