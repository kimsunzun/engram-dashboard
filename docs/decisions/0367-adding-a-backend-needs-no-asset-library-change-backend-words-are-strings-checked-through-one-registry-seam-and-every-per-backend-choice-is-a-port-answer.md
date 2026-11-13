# ADR-0367: Adding a backend needs no asset-library change: backend words are strings checked through one registry seam, and every per-backend choice is a port answer

- 상태: 확정 (2026-11-13, 근거: user decision 2026-11-11 (S25 step-7 TRD §1-1) · child TRD review findings B20 · must 9 · must 10 · landed P1 (`42cede81` — `Backends`) through P9)
- 관련: Amends ADR-0334 (decision 3 and 영향, the port members and what a new backend adds) · ADR-0004 (backend knowledge isolation) · ADR-0311 (the asset-library crate) · ADR-0359 (decision 4 — a new backend gets no schema check) · ADR-0329 (backend tags on preset items) · ADR-0364 (notice codes) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` §1-1 · §3-3a · §8-3 · §10-2 B20

## 맥락
ADR-0334 puts per-backend settings knowledge in the agent backend's settings-check port and says a new backend adds `materialize` and a port implementation. It does not say how asset-library learns which backends exist, how it compares a backend tag or a raw settings field name, or which per-backend choices it may make itself. The user set the rule for step 7: 「나중에 백엔드 계속 추가될텐데 최대한 공용화로 하고(예를들어 ENUM 추가하면 모두 자동으로 되게 한다던가.)」 (2026-11-11).
The TRD drafts broke it four times: draft 1 had asset-library pick the three codex-named notice codes by knowing the backend is codex, and named the working-folder refusal code as if asset-library owned it (B20); draft 2 called the backend registry as a global, so no test could inject a fake third backend (review round 1 must 10), and told claude's natively loaded folder files from codex's carried ones by the backend (must 9).

## 결정
1. **Adding a backend to agent — a new `AgentCommand` variant, its `backend_word`, its settings check in `settings_checks()` — makes tags, settings raw fields, the selection of what an agent loads, the hand-over checks and the notices work with no asset-library change.**
2. **A backend tag and a settings raw-field name are strings.** asset-library accepts a word only if the backend registry knows it, and compares it case-insensitively through the registry. asset-library has no `enum` over backends.
3. **The registry is reached through one seam, `Backends`** (`for_word`, `all`). Production = `AgentBackends`, the only asset-library caller of agent `settings_check_for` / `settings_checks()`; tests inject a registry with a fake third backend. Parsing, settings checks, resolution and the service take `&dyn Backends`.
4. **Every per-backend or per-mode choice is a port answer** — `loads`, `accepts_skill_name`, `command_line_args`, `command_line_fits`, `skill_format`, `folder_entry_blocks`, `working_folder_blocks`, `folder_listing_at_launch`, `folder_load_set` (with its `carried` rows), `native_skill_roots`, `root_skills`, `notice_codes`. asset-library branches only on those answers, never on the backend word. This adds to the port members ADR-0334 decision 3 lists: the notice codes, the working-folder rule, whether a launch lists folder entries, and what of a folder the launch carries.
5. **No `claude` / `codex` literal in asset-library** outside tests, doc comments that describe today's port answers, and the built-in content files.

## 거부한 대안
- **An `enum Backend` in asset-library** — every new backend would need an asset-library change; the user asked for the opposite (「ENUM 추가하면 모두 자동으로」).
- **asset-library picks backend-named notice codes and refusal codes itself** (TRD draft 1) — the codes `codex-terminal-omitted` etc. would be chosen by knowing the backend; a new backend would need asset-library code (B20).
- **Call the backend registry as a global** (TRD draft 2) — the fake-third-backend tests could not be written, so nothing would prove that a new backend works without a change (review round 1 must 10).
- **Tell what a launch carries from a folder by the backend** (TRD draft 2 — claude's native set vs codex's carried files) — a branch on the backend word; replaced by `folder_listing_at_launch` and `FolderLoadSet.carried` (must 9).

## 근거
- User decision 2026-11-11 (quoted above) — child TRD §1-1.
- Child TRD review: B20 (draft 1), round 1 must 9 and must 10 (draft 2).
- Code: asset-library `backends.rs` (`Backends`, `AgentBackends`); agent `backend` `SettingsCheck` (the added methods `working_folder_blocks`, `notice_codes`, `folder_listing_at_launch` and `FolderLoadSet.carried`); the fake-third-backend tests of P3, P4, P8 and P9.

## 영향 / 불변식
- Every step-7 unit's code review runs the TRD §8-3 check: `rg -n -i '\b(claude|codex)\b' crates/engram-dashboard-asset-library/src` has no hit in a branch, a match arm, an emitted string or a field name; `rg -n 'settings_check_for|settings_checks\(' crates/engram-dashboard-asset-library/src` hits only `backends.rs`. Main may turn these into a CI gate once tests live in separate files (B21).
- A unit that branches on backend behaviour has a test with a fake third backend injected through `Backends`.
- A new per-backend difference is a new port answer in agent `backend`, not a branch in asset-library (ADR-0004). ADR-0334's 「영향」 now reads: a new backend adds `materialize` and a settings-check implementation, and nothing in asset-library.
- The step-13 screen lists backend choices from the same registry (per-backend rows appear only when the user adds them).
