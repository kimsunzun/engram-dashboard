# Validation result display — the failure indication and the 「고친 뒤 다시 검사 전」 (after fixing, before re-validating) state (practice survey)

- **Status:** light survey · 2026-10-18 · 1 collector (document type) + main-session grounding (checked against the Unreal original text) · no adversarial review
- **Question:** When config assets are validated with a CLI (about 2 seconds · no model call · OK recorded per config), is a failure shown as ✗, and what is shown after the user fixes it but before it is validated again?
- **Context:** there is no watching — edits inside the app are known immediately, and edits outside are known on refresh. Validation currently runs only right before execution.

## Conclusion

- **Showing a failure explicitly is the practice** (Unreal 「Compile Failed」 · JetBrains run configuration red X · IDE error badges). — likely (only Unreal was checked against the original)
- **The state after fixing goes two ways.** Cheap validations that run in-process (VS Code language server · JetBrains editor analysis) re-validate immediately on change, so there is no stale window. Expensive validations that run explicitly (Unreal Blueprint · Xcode/VS build) keep a separate 「다시 해야 함」 (must be redone) state and give a 「지금 검사」 (validate now) button. — likely
- **We found no case where leaving the old indication as is was an intended design** — such cases (Xcode stale issues · JetBrains red file name · rust-analyzer diagnostics on save) are reported as bugs. — likely
- **Unreal's 「Compile Needed」 is the closest shape** — it has four states: success · needed · warning · failure, and in any state pressing it recompiles. — certain (checked against the original)

## By peer

| Peer | Failure indication | After fixing | Confidence | Source |
|---|---|---|---|---|
| Unreal Blueprint Compile button | Compile Failed | Compile Needed「The Blueprint needs to be recompiled.」 · pressing it compiles | certain (checked against the original) · icon shape · failure→needed transition unconfirmed | https://dev.epicgames.com/documentation/unreal-engine/toolbar-in-the-blueprints-visual-scripting-editor-for-unreal-engine |
| VS Code | Problems · error badge | the language server re-emits while typing / on save · stale diagnostics are treated as bugs | likely (secondary sources) | https://github.com/microsoft/vscode-go/issues/2907 |
| JetBrains | run configuration red X | re-decided when the configuration is edited (inferred) · a red file name that does not clear is reported as a bug | uncertain | intellij-support forum post |
| Xcode · Visual Studio | issue list | build errors remain until the next build (VS 「Build Only」 filter) · Xcode stale issues are bug reports | likely (forum) | https://developer.apple.com/forums/thread/709069 |
| GitHub Actions | 「Workflow file is not valid」 | remote validation at push · editor extensions while typing | likely (thin) | — |
| Unity Asset Manager | imported with error | shows three branches: latest · stale · error | uncertain (summary only) | https://docs.unity3d.com/Packages/com.unity.asset-manager-for-unity@1.7/manual/import-assets.html |
| Terraform saved plan | — | rejected when used, with 「Saved plan is stale」 (key invalidated with no indication) | likely | https://discuss.hashicorp.com/t/question-on-error-saved-plan-is-stale/52912 |

## Gaps

- For VS Code · JetBrains · Xcode · VS · GitHub we could not read the primary docs (search summaries · forums).
- We found no peer that put a 「검사 필요」 (validation needed) indication on a list of config files validated by an external CLI — the closest is Unreal.
