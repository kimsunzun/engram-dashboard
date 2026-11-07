# ADR-0166: UI settings are owned by the disk file, and ui.refresh re-reads it

- 상태: 확정 (2026-08-21, basis: user decision + GUI measurement) · 부분 폐기 by ADR-0265 (decisions 1, 3, 5, 7, 8 and 9) · decision 6 amended — user decision 2026-10-07 · no new ADR: for `window.setTheme`, decision 6 applies only to the window that was named (whether it applies to a `theme.default` write is still open — TRD §5-6). If the named window did not receive the theme, that command answers with an error, and a delivery failure confined to other windows is success + a warn log. Source of truth = `docs/process/S21-storage/trd.md` §5-6
- 관련: ADR-0149(no slot unmount) · ADR-0062(e-ink intent) · ADR-0012(seam isolation) · CLAUDE.md 「LLM-우선 제어」 · `src-tauri/src/ui_settings.rs` · `src/theme/uiSettings.ts` · step-log S20 · Amended by ADR-0265 (decisions 1, 3, 5, 7, 8 and 9)

## 맥락

There was no way to change the display policy from outside. The theme (`src/store/themeStore.ts`) has **no persistence at all**, so when the app is turned off and on it unconditionally returns to `dark`, and the font and chat display values live in `localStorage` (key `engram.chatStyle`, a 10-key whitelist), so they are **inside the WebView2 profile and a process outside cannot open them as a file.** The repository had 0 disk files for frontend settings.

「LLM-우선 제어」 requires every feature to be controllable by the LLM, and the display policy is hard to carry that requirement by creating a command per value — the font keys alone number 10.

This ADR pins a **pilot with a single first key (`theme`)** in that direction. A multi-key settings file and a font export are follow-ups.

## 결정

1. **The settings source of truth = a disk file.** `<data_dir>/ui-settings.json`, key `theme` ∈ `dark`|`light`|`e-ink`. `data_dir` = `engram_dashboard_discovery::default_data_dir()` (release = next to the exe, debug = `.engram-data` inside the repo, environment-variable override).
2. **The read owner = the shell (`src-tauri`).** At boot the frontend pulls (`get_ui_settings`), and for updates the shell pushes (`ui:settings-updated`). The two paths share the same payload struct.
3. **There is one command, `ui.refresh`.** Do not create a command per value. The one who edits the file is an outside agent, and **the shell only reads** — the on-screen theme toggle stays in memory and the next refresh overwrites it (an intended asymmetry of the pilot).
4. **An update is re-applying a value, not a remount.** It swaps a single `data-theme` attribute.
5. **There are only two branches of reason in the answer** (as the file says / folded to the default). The five folding reasons (absent · unreadable · corrupt · unknown value · over the limit) are left only in the log.
6. **If the notification cannot go out, do not answer success** — return an error.
7. **Unknown keys are ignored** (forward compatibility).
8. **A value is put in the log only when it has the shape of a theme name** (ASCII alphanumerics + hyphen, ≤24 bytes) + what passes `mask_secrets`. Anything else gets only the kind and length.
9. **If the file is absent or corrupt, fold to `dark` and leave only a log.** Do not make a `.corrupt` copy.

## 거부한 대안

- **Command as the source of truth — create a command per value** (e.g. `ui.setFont`, extending `theme.set`). Theme, font, color and spacing are **of a settings nature, so there is no need to create a command for each one** (user decision 2026-08-21). The font keys number 10 (measured from the `chatStyleStore` whitelist), so the command arguments get heavy, and the vocabulary grows every time a key is added.
- **A full-remount refresh.** The slot's conversation is component state and the daemon history has already been released, so **a remount is permanent loss** (the place ADR-0149 pinned as no unmount).
- **The daemon owns the file.** Step 3 has already opened the shell's declaration, registration and inbound receive path (`declare_commands!` with 16 in `src-tauri/src/layout/commands.rs` + `src-tauri/src/daemon_client/inbound.rs`), so with the shell there is **zero new wiring**. With the daemon that path would have to be dug anew.
- **Expose the five folding reasons in the answer.** User instruction ("사유 때문에 로직 키우지 마라" — do not grow the logic because of reasons, 2026-08-21). In addition, if the reasons are exposed, **callers branch per reason and the five branches become the contract** — changing the log wording and changing the contract become the same weight.
- **Carry no reason at all** (return only `{theme}`). The first implementation did that and the user reversed it (2026-08-21) — even if the agent makes a typo in the file a **success response** comes back, so it cannot tell whether its own edit took. If the reason is only in the log file, the side that called the command cannot see it.
- **Push a corrupt file aside as `.corrupt`** (the convention of the registry loader). **★Weak ground for rejection — prose only★**: it is a property argument that a settings file has nothing to recover even if lost, and has no quantitative, measured or code ground. The registry (`agents.json`) holds profiles and session ids, so a copy is worth something, but this file is a single key.
- **Put no value in the log at all** (length + hash only). Typo diagnosis dies — the test `typo.contains("Dark")` encodes that requirement. Instead it was narrowed with a shape gate (decision 8).
- **Reject unknown keys and fold to the default.** ★Weak ground for rejection — prose only★: this pilot is the first key of a multi-key settings file, so it is a forward-compatibility argument that rejecting would make it impossible to add keys later, and a second key does not exist yet.

## 근거

- **Putting uppercase letters in the shape gate is intentional.** The theme value comparison is case-sensitive (`from_wire`), so `Dark` is the most common typo — a lowercase-only gate cannot satisfy the blocking purpose (email · URL · token) and typo diagnosis at the same time. The blocked targets get caught on `@ . / : _ 공백` (the last being whitespace), or on length.
- **`catalog_version` 3 → 4, `since` stays at 3.** The answer shape is also a declaration, so the generation goes up, but `since` is the generation in which the **name** came in, so it does not move (`catalog_version_is_per_block` in `crates/engram-dashboard-core/tests/command_alphabet.rs` pins the independence of the two).
- **The reason for not swallowing a notification failure** — `LayoutEvents` swallows it. There the mutation has already happened, so it recovers by pull, but for **this command the notification is the whole result**, so if it could not be sent nothing happened.
- **4 kinds of GUI measurement passed** (2026-08-21): boot with a saved theme (`light`) · switching to `e-ink` with `ui.refresh` leaves 41 DOM nodes identical (0 remounts) · corrupt file → `dark` with no crash · no file → `dark`.
- 5 review rounds (doc-aware PASS 3 times / cross-family blind FIX) — the opposing items were closed by the user's decision.

## 영향 / 불변식

- **Do not put a remount on the `ui.refresh` path** — including key swaps, router navigation and store resets. If breached, the slot conversation is permanently lost (ADR-0149).
- **Do not grow the reason slot of the answer into five branches** — the moment it grows, callers write per-reason branches and those five become the contract.
- **Do not attach `#[serde(rename)]` to the `ThemeSource` variant names** — the bus reply and the Tauri payload must use the same spelling, and the test that measures that match has the expected strings hand-written.
- **Do not widen the shape gate** — a value that passes the gate stays in the log. Widening it leaks that much.
- **Known residuals (do not read them as fixed):** a notification that reaches 0 subscribers still answers success (the boot query overwrites the screen) · a stale notification can overwrite a newer boot value (it heals at the next update) · a non-atomic external write window · if only the file changes without a refresh, only a newly opened window sees the new value · a `listen()` that gets no response is not covered by retry (no timeout) · there is no harness that asserts the log level.
