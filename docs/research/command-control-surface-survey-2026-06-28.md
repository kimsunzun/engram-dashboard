# Research — LLM control surface (intent command bus) OSS survey

**Status:** design-decision mode survey (OSS reference before starting). Cross-family check (Claude Sonnet fan-out + Codex blind) + opus adversarial verification. **medium.**
**Date:** 2026-06-28 · Author: dashboard1 (wip/a1) · **Reference** (adoption = the owner's PRD decision).
**Confidence legend:** certain / likely / uncertain.

> Purpose: §5 LLM-first control — a single intent bus through which the LLM (primary) and the human (secondary) call all UI + backend functions (layout split · slot placement · popups · agent spawn/placement · theme, etc.) through **the same handle**. A reference for "how to build this feature".

## Candidates × core (summary)

| Candidate | Core mechanism | Fit for engram | License / maturity |
|---|---|---|---|
| **VS Code Commands API** | `registerCommand(id, handler)` → palette · keybindings · `executeCommand` **all use the same handler** | ★ The textbook single control surface. Adopt the "human click = LLM call = keybinding, same handler" pattern | MIT (Code-OSS) / very high |
| **Zellij actions/CLI/plugin** | The CLI `zellij action` and plugins share **the same Action enum**, `--json` · `subscribe` | ★ "Every UI operation = a named action + a target handle" + a permission scheme. Rust native | MIT / high |
| **tmux control mode `-C`** | stdin/stdout text protocol, `%begin/%end` + `%` async notifications, target ID | The async notifications and request-ID structure are a reference for the daemon WS events. The text protocol and no Windows support rule out direct adoption | ISC / standard-grade |
| **WezTerm cli** | External control of a running instance, returns a pane-id on spawn | Unrelated to the app's own bus — dev/verification tool level (used alongside cdp.mjs) | MIT / high |
| **CQRS/Redux dispatch (Rust)** | Typed command envelope → handler/registry dispatch, replay/audit | ★ Internal architecture. But `cqrs-es`/`qonduit` are too much event sourcing → **implement only the trait registry pattern ourselves** | MIT/Apache · medium |
| Google A2UI | The agent sends declarative JSON component descriptors, mapped through a catalog | Adopt only the "set of approved command IDs" security boundary concept. A Tauri native app does not need the JSON round trip | Apache-2.0 / early |

## Cross-check table (Claude ↔ Codex)

| Claim | Claude | Codex | Verdict |
|---|---|---|---|
| A single handler registry (human = LLM, same path) = the VS Code model | ✓ | ✓ | **Converged · certain** |
| Every feature = a typed action/intent enum (Zellij style) | ✓ `EngramCommand` enum | ✓ `Intent{id,version,actor,target,payload,correlation_id}` | **Converged · certain** |
| Dispatch adopts the CQRS/Redux pattern, but excludes heavy crates | ✓ (own trait implementation) | ✓ (cqrs-es is optional, traits recommended) | **Converged · certain** |
| Frontend = pure I/O (one invoke line), the store only receives events | ✓ | ✓ | **Converged · certain** |
| command ID ↔ LLM tool schema 1:1 | ✓ | (implied) | Converged · likely |
| Swappability = separate by controller trait | (implied) | ✓ `IntentHandler/AgentSpawner/LayoutController/...` | Converged |

**No disagreement.** Codex is more granular (correlation_id · versioning · `IntentResult{events,handles}` · explicit controller traits). Claude stresses "command ID = LLM tool 1:1".

## Adversarial verification (key claims)

- **Is "command ID = LLM tool 1:1" over-coupling?** → Concern about tool explosion and rigid evolution. Mitigation: Codex's `Intent{id, **version**}` + grouping absorbs evolution. **Kept (likely).**
- **The stringly-typed ID trap?** → The weakness of string IDs that VS Code suffers from. engram fixes this with a **Rust typed schema** (enum/struct), with the ID only as the surface → mitigated. **Kept.**

## Recommendation (converged proposal)

A **typed intent registry** in the Rust core daemon: `Intent{ id, version, actor, target, payload, correlation_id } → IntentResult{ events, handles }`.
- **Single handler** (VS Code): human click and LLM invoke go through the same handler path.
- **Typed action enum** (Zellij): layout · spawn · placement · theme all in one enum, serialized as protocol crate wire types.
- **Dispatch** (CQRS/Redux): implement the trait registry ourselves (exclude event sourcing crates). Command log = audit/replay.
- **Frontend = one invoke line**, the store receives events (extends ADR-0011 agentClient).
- **Swappability**: `IntentHandler` · `LayoutController` · `AgentSpawner` · `WindowController` · `ThemeController` traits.
- The command ID list ≈ the LLM tool list (a new feature = register the command + define the tool at the same time).

## Rejected candidates → candidate rejected alternatives for the ADR

- Adopting `cqrs-es`/`qonduit` directly — over-design with event sourcing · saga, too much for a desktop app.
- WezTerm cli / tmux text protocol — unsuitable as the app's internal bus (external control tool · Windows · no type safety).
- Adopting A2UI directly — a Tauri native app does not need the JSON round trip (adopt the concept only).

## Gaps and limits

- Unanimity ≠ correct: both families converge on the "typed command bus + traits" pattern — a well-established pattern, so low risk, but be aware of possible shared training bias.
- The real activity level of Rust CQRS crates (cqrs-es etc.) is unverified — low impact since we recommend our own implementation anyway.
- The interface with the real S14 ViewManager API (layout commands driving the ViewManager) **needs separate confirmation** (dashboard2's domain). This report goes only as far as the pattern.

## Sources

VS Code Commands https://code.visualstudio.com/api/extension-guides/command · Zellij https://zellij.dev/documentation/cli-actions.html · https://zellij.dev/documentation/plugin-api-commands.html · tmux control mode https://github.com/tmux/tmux/wiki/Control-Mode · WezTerm cli https://wezterm.org/cli/cli/index.html · CQRS https://learn.microsoft.com/en-us/azure/architecture/patterns/cqrs · cqrs-es https://docs.rs/cqrs-es/ · mediator https://docs.rs/mediator/ · Redux https://redux.js.org/tutorials/fundamentals/part-2-concepts-data-flow · A2UI https://developers.googleblog.com/introducing-a2ui-an-open-project-for-agent-driven-interfaces/ · Tauri invoke https://v2.tauri.app/develop/calling-rust/
