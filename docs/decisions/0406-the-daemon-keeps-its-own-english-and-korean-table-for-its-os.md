# ADR-0406: The daemon keeps its own English and Korean table for its OS approval dialogs and is told ui.language by the shell

- 상태: 확정 (2026-11-23, 근거: S37 U2 piece C3 code review — codex must, lead hold; the main followed the standing rule in CLAUDE.md 「언어」)
- 관련: Amends ADR-0372 (scope of the Rust tables now includes the daemon) · ADR-0373 · ADR-0394 (the dialog text lives on the daemon side) · ADR-0383 (protocol bump criterion) · `docs/reference/structure/i18n.md` · `docs/reference/structure/peer-link.md`

## 맥락
S37 C3 added a native OS approval dialog shown by the daemon (ADR-0394: humans approve trust changes; the question text is written by the daemon). ADR-0372 made English the source with Korean selectable through `ui.language` and gave the shell its own small en/ko table, but the daemon had no table and no way to know the language. CLAUDE.md 「언어」 requires every new user-visible string in English and Korean. `ui.language` is a shell-owned setting (`src-tauri/src/settings/`).

## 결정
1. **The daemon keeps its own en/ko table for the texts it shows the human** (today: the approval dialog — questions, buttons, the waiting count, the system questions), in `link/peer/approvals/text.rs`. It does not read the frontend or shell tables.
2. **The shell tells the daemon the language** with a shell→daemon message `AgentCommand::SetUiLanguage { language }` (no request id, no answer): on every connection it opens and on every change. The daemon keeps the last value in `ControlRegistry` and reads it when a dialog is about to show; English until told, an unknown word means English. The webview cannot send this frame (`forward_daemon_command` drops it).
3. **No `PROTOCOL_VERSION` bump for it**: version 8 is unreleased (v0.4.0 = 7), an older daemon answers the unknown frame with an uncorrelated error the shell only logs, and a newer daemon behind an older shell shows English, the documented default.
4. **An open dialog keeps its language**; the next one uses the new value (a Task Dialog cannot relabel its buttons in place) — an accepted limit to ADR-0372's "applies immediately".
5. An agent can switch the language through the LLM-only `settings.set ui.language` (ADR-0372); accepted — both texts carry the same request id, full IDs, URLs and the default Deny.

## 거부한 대안
- **English-only daemon dialogs as an exception to ADR-0372** — CLAUDE.md 「언어」 requires both languages for new user-visible strings; the user reads Korean. Not put to the user: the standing rule decides it (main, 2026-11-22). (weak — no measurement; the user may still choose it)
- **The daemon reads the shell's `settings.json`** — the settings are shell-owned state (CLAUDE.md 「모듈」); a second reader of that file would couple the daemon to the shell's file format and location.
- **A protocol bump** — nothing released speaks version 8, so no released pair can meet the silent case; a bump would also trigger ADR-0383's legacy-marker removal for no benefit (codex withdrew its finding after checking the tags).
- **Re-show an open dialog in the new language** — the question would flash and its 1 s button hold restart; rare and cosmetic.

## 근거
Code review deep, 4 rounds (codex must → implemented; codex's later bump finding withdrawn after `git show v0.4.0:…` = 7). QA GUI run: the daemon logged `the shell set the UI language language="en"`, then `"ko"` after `settings.set ui.language ko`, and the dialog read through UI Automation showed the Korean question and buttons.

## 영향 / 불변식
- A new daemon-shown human text is added to the daemon table in both languages (`approvals/text.rs`, Korean inside one `hangul-ok` block).
- Any future daemon UI text follows the same path (the shell's `SetUiLanguage`), never a read of the shell's settings file.
- The shell must send the language on every new connection; a missing send shows English, not an error.
