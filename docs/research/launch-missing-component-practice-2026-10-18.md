# When a component attached at launch cannot be read — block vs. notify and launch anyway (practice survey)

- **Status:** light research · 2026-10-18 · collector 1 (document type) + main grounding (checked the two key quotes against the source text) · no adversarial review
- **Question:** When launching an agent, if the library folder of the attached preset is missing, or the instruction/skill files cannot be placed in the codex working folder, does it block the launch, or launch without them and notify?
- **Decision:** User, 2026-10-18 — block (no matter who launches · show the reason). This research supports that decision.

## Conclusion

- **A component the user explicitly attached is by default 「필수」 (required), and if it is missing, block and raise an error.** Docker Compose · systemd · Claude Code's own `--agents` · `--add-dir` do this. — likely
- **Launching without it is an 「선택」 (optional) marker that the author turns on explicitly per component** (Compose `required: false` · systemd `EnvironmentFile=-경로` · Unreal plugin `Optional`). No peer was found that defaults to a global 「관대 모드」 (lenient mode). — likely
- **When a component is missing and launched anyway through an optional marker, peers mostly do not even notify** (Compose 「silently ignores」 · Unreal 「silently ignore」 · the systemd `-` form). Peer precedent for 「알리고 띄우기」 (notify and launch anyway) is weak. — likely
- **No peer was found that splits behavior between human and headless.** — uncertain (not found ≠ does not exist)

## Per peer

| Peer | Behavior | Mechanism | Confidence | Source |
|---|---|---|---|---|
| Docker Compose `env_file` | Blocks by default · can be made optional | `required` defaults to `true` · with `false` it 「silently ignores the entry」 | Certain (checked against source text) | https://docs.docker.com/reference/compose-file/services/ |
| systemd | Blocks by default · optional per item | `EnvironmentFile=-path` 「not considered an error if the file does not exist」 · `Requires=` vs `Wants=` | Certain (collector quote) | https://man7.org/linux/man-pages/man5/systemd.exec.5.html · https://man7.org/linux/man-pages/man5/systemd.unit.5.html |
| Unreal plugin | If optional, silently omitted · behavior when required is unconfirmed | `FPluginReferenceDescriptor.bOptional` 「silently ignore it not being present」 | Flag certain · required behavior uncertain (forum only) | https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Projects/FPluginReferenceDescriptor |
| Claude Code CLI | Explicit arguments are validated at startup, then it exits | `--agents` 「validates the value at startup and exits on an invalid one」 · `--add-dir` 「Validates that each path exists as a directory」 · global bypass = `--safe-mode` · `--bare` (explicit choice) | Certain (checked against source text) | https://code.claude.com/docs/en/cli-reference |
| Claude Code `--settings` · `--plugin-dir` · `--mcp-config` with a missing file | Unknown | — | Abstain | Not in the docs |
| VS Code `launch.json` `envFile` missing | Blocks (ENOENT) | Language extension behavior | Likely (issue snippets only) | dotnet/vscode-csharp · microsoft/vscode-python issues |
| JetBrains run configuration error | Only confirmed up to the red marker · unknown whether it blocks | — | Abstain | https://www.jetbrains.com/help/idea/run-debug-configuration.html |
| codex CLI · tmuxinator/tmuxp | Unknown | — | Abstain | — |

## Implications for this decision (inference)

- Engram's presets are attached explicitly by the user and carry no 「선택」 marker → by convention they are **required → block and show the reason**.
- If 「빠져도 띄우기」 (launch even when missing) is needed later, the peer form is a **「선택」 marker per attachment**, not a global switch.
- Measured note: if even one value in `--settings` has the wrong type, claude silently discards the whole thing (`settings-asset-validation-spike-2026-10-17.md`) — 「조용한 빠짐」 (silent omission) on the CLI side really exists, so it is worth Engram blocking up front.

## Gaps

- Unreal's default behavior on a missing required plugin · whether JetBrains blocks · claude's missing-path behavior for `--settings`/`--plugin-dir`/`--mcp-config` · codex — not confirmed.
