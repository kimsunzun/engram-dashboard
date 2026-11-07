# Do apps notify the user when a state or settings file cannot be read or is broken — practice (light · 2026-10-05)

- **Status:** decided (user 2026-10-05) — ① if `state.json` cannot be read at boot, **only notify, and do not save for the rest of that run** (do not add retries or re-read during the run — 「가장 깔끔한 구현」 (the cleanest implementation)) · ② if a broken file is copied aside and the default screen is used, notify the same way (practice · session recommendation) · no automatic-recovery button · an LLM also learns of it through a state query · implementation = P3c2 (TRD §9-2).
- **Method:** `/research` light — 1 collector (document type) · about 12 searches and reads · main grounding = the Windows Terminal and VS Code wording checked directly against the original source (2026-10-05). The rest is from secondary sources.
- **Decision question:** do we show an on-screen notice in the two cases of storage management (TRD `docs/process/S21-storage/trd.md` §6-5) — ① `state.json` cannot be read, so this run does not save the screen state (guard i) ② the broken file is copied aside as `.corrupt` and we start with the default screen.
- **Confidence legend:** certain (checked directly against the original source) · likely (several secondary sources) · uncertain (a single user report · wording not checked).

## Findings

| App | Case | What the user sees | Wording | Confidence · source |
|---|---|---|---|---|
| Windows Terminal | settings.json fails to parse → continues with defaults | a warning (form not confirmed — dialog or InfoBar) | title "Failed to load settings" · body "Settings could not be loaded from file. Check for syntax errors, including trailing commas." + "Temporarily using the Windows Terminal default settings." · reload failure = "Failed to reload settings" | **certain** (wording) — [Resources.resw](https://raw.githubusercontent.com/microsoft/terminal/main/src/cascadia/TerminalApp/Resources/en-US/Resources.resw) `InitialJsonParseErrorTitle/Text` · `UsingDefaultSettingsText` · `ReloadJsonParseErrorTitle` |
| Windows Terminal | not writable | a warning | "We could not write to your settings file. Check the permissions …" | likely (collector summary · not checked by main) |
| VS Code | when it tries to write a setting while settings.json is broken | a notification + an 「Open Settings」 button · the write is refused (neither overwritten nor copied aside) | "Unable to write into user settings. Please open the user settings to correct errors/warnings in it and try again." | **certain** — [configurationEditing.ts](https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/services/configuration/common/configurationEditing.ts) `errorInvalidConfiguration` · `open` |
| Chrome | detects (HMAC) that the settings were changed by another program → resets to defaults | a notification + a choice to undo | "Some settings were reset …" (secondary quotation · original not checked) | uncertain — [chalmers paper](https://odr.chalmers.se/items/412c8886-a931-4f3f-8219-6561e2f82c65) · [jamf forum](https://community.jamf.com:443/t5/jamf-pro/google-chrome-settings-from-user-template-quot-some-settings-were/td-p/180177) |
| Chrome | Preferences cannot be read or is broken | a modal error dialog · recovery is by the user by hand | "Your preferences file is corrupt or invalid. Google Chrome is unable to recover your settings." | uncertain — [user report](https://www.wintips.org/?p=16845) |
| JetBrains | the settings file cannot be read (parse error · size limit) | a notification | "Cannot load settings from file '<path>': …" | uncertain — [support forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/115000008604/comments/115000035464) |
| Firefox | prefs.js · sessionstore corrupted | no basis for an automatic notice — the user recovers from a backup by hand | — | uncertain — [SUMO](https://support.mozilla.org/questions/1245632) · [discourse](https://discourse.mozilla.org/t/session-file-corruption/110112) |

## Interpretation (not fact)

- **No app that goes through silently is within the range confirmed by evidence.** The two confirmed ones (Windows Terminal · VS Code) both notify on screen, and put the cause (file · syntax) and the next action (open the file · defaults 「임시」 (temporary)) in the wording.
- **The precedent that states 「이번엔 기본값으로 임시 동작」 (this time it runs temporarily on defaults) explicitly = Windows Terminal** — it is the closest to our ① (do not save this run).
- **Automatic-recovery buttons are rare** (only Chrome's tamper reset). The rest are a notice + the user by hand.
- **We did not find a precedent that separates a read failure (locked · permission) from corruption and notifies them differently.**

## Gaps

- Wording precedent for read failure alone (locked · permission) — most cases were corruption · write failure.
- The UI form of the Windows Terminal warning · the originals for Chrome/Firefox/JetBrains · VS Code `state.vscdb` · terminal session-restore tools (tmux-resurrect · zellij · wezterm · iTerm2) · Sublime · Obsidian — no evidence.
