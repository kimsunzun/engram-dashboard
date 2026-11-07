# Showing the reason when a launch is blocked · bubbling an inner error up to the outer row (practice survey)

- **Status:** light survey · 2026-10-18 · 1 collector (documents) + main grounding (checked against the NN/g · Unreal originals) · no adversarial review
- **Questions:** ① When a launch is blocked because an attached component is broken, where is the reason shown (modal · in place · notification)? ② Is an inner item's error shown on the collapsed outer row?
- **Context:** In the agent preset slot, a preset that holds a broken setting must be visible at a glance and must be followable by clicking (user 2026-10-18).

## Conclusions

- **A modal is only for when the user must stop everything and respond; any other error is shown near its source and lets the user follow it to the source.** NN/g 「Display the error message close to the error's source.」 · 「modal dialogs … should be reserved for severe errors.」 (checked against the original). Microsoft 「Dialogs block interactions with the app window until being explicitly dismissed.」 — only app-wide errors (a lost connection and the like) get a dialog. A launch blocked by a single component is not a modal case (inference). — likely
- **The Unreal Blueprint compile results panel is the closest precedent** — it opens by itself when there is an error (「The panel will also open automatically whenever a Blueprint compile results in errors or warnings.」), and clicking an item goes to the problem node (「Click on a message to jump the graph view directly to the problematic node」). — certain (checked against the original)
- **The VS Code explorer bubbles an inner file's error up to the parent folder** — a count badge (「9+」 for 10 or more) · the color of the most severe level · the setting `problems.decorations.enabled`. Release notes 「The number of errors/warnings is shown in the decoration.」 The bubbling up to the parent (`bubble: true`) is confirmed only from a source summary. — likely
- The run-configuration error screens of VS Code · JetBrains · Docker Desktop · the Windows Terminal settings-error banner · the parent marking in JetBrains/Visual Studio — no primary material found (abstain).

## Sources

| Source | Content | Confidence |
|---|---|---|
| https://www.nngroup.com/articles/error-message-guidelines/ | near the source · a modal only for severe errors | certain (checked against the original) |
| https://learn.microsoft.com/en-us/windows/apps/design/controls/dialogs-and-flyouts/ | a dialog blocks · only app-wide errors | certain (collector quote) |
| https://dev.epicgames.com/documentation/en-us/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | opens automatically on error · clicking goes to the problem node | certain (checked against the original) |
| https://code.visualstudio.com/updates/v1_20 · `markersFileDecorations.ts` | error count badge · bubbling up to the parent folder | likely |
