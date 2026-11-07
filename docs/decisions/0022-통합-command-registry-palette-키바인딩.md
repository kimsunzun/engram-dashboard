# ADR-0022: Unified command registry — a single source for palette + keybindings + LLM + menu/tray

- 상태: **제안(Proposed)** (2026-06-17, only the direction is fixed — implementation comes later. A record so that later sessions build commands in this shape.) · 부분 폐기 by ADR-0155 (three unresolved forks exhausted)
- 관련: CLAUDE.md §5 (LLM-first control, every menu programmable) · ADR-0011 (agentClient control surface) · ADR-0020 (single dispatch) · ADR-0021 (daemon lifecycle command) · backlog "data-driven right-click menu" · Amended by ADR-0155 (three unresolved forks exhausted)
- Scope: the architectural direction of deriving user actions, LLM control, keybindings, and menu/tray/palette from **one command registry**. The aim is to drive the blast radius of adding future features toward zero.

## 맥락

Three requirements in fact converge on **the same one thing**:
1. **§5 LLM-first control** — every feature (backend + UI/layout) must be callable by the LLM.
2. **A VS Code-style Command Palette** — searching shows many commands, and you run them.
3. **Custom keybindings** — the user maps a shortcut to every action (keybindings.json style).

All three presuppose the same structure: "every action is a command registered by ID, and the various surfaces only call it." Today: the backend is already commands (invoke/agent_command/daemon_*), but **UI/layout actions (split, slot placement, layout save, theme, tree manipulation) are frontend (Zustand) only** — there is no single entry point that the LLM, palette, keys, and tray can reach (the §5 gap). If features keep being added this way, "UI first, control later" repeats and each time has a ripple effect.

## 결정 (Proposed)

**Register every action in the command registry as `id + 메타 + handler`, and make the palette, keybindings, menu, tray, and LLM all consumers (derivatives) of that registry.**

- command = `{ id, title, category, handler, when?(context), defaultKeybinding? }`.
- **A single entry point:** a human click = a command call, an LLM call = the same command call, a palette search-and-run = a registry lookup and then a call, a key = a `키→command id` map, tray/menu = a command id reference.
- **Adding a new feature = registering one command** → it automatically shows up in the palette, can be given a keybinding, can be called by the LLM, and can be put on the menu/tray. **The ripple of an addition converges to zero** (the finished form of the blast-radius goal).

## 거부한 대안
- **Implement UI and control separately per feature (the current gap):** every feature needs a direct frontend edit plus a separate control path → large ripple, violates §5, and the palette/keybindings are wired by hand for each feature.
- **Make only the backend commands and keep the UI frontend-only:** the LLM cannot touch the UI/layout (falls short of §5). UI actions do not show up in the palette.
- **A hardcoded menu/keymap:** every new feature means editing the menu and keymap by hand → rot and ripple. (Rejected in the same spirit as the CLAUDE.md ADR-rot prevention.)

## 근거
- The VS Code proven model: `contributes.commands` (registry) → the Command Palette + `keybindings.json` (key → id) + the menu (`when` clause) are all derived from the registry. An extensibility pattern run for 10+ years.
- ADR-0020/0021 already applied "a single entry point reduces the ripple of additions" to the backend — this ADR extends that principle to **all actions, including UI**.

## Impact / direction (to be followed from now on, even before implementation)
- The commands built from now on (daemon lifecycle, UI/layout, etc.) are built in a **registry-compatible form (a stable id + metadata)** — so that existing ones are not shaken when the palette/keybindings are added later.
- The premise is to absorb the **§5 UI/layout control-surface gap** into this registry (promote the frontend Zustand actions to commands).
- **#2 tray** is the first case of this direction — a tray item = a registry command reference (zero separate logic).

## Unresolved (decide at implementation — do not fix arbitrarily)
- Where the registry lives: frontend (JS) alone vs a backend mirror vs both (the LLM introspects on the backend side). The path by which the LLM discovers the command list.
- The command id namespace convention, the `when`-context model, where keybindings are stored (frontend localStorage vs backend config).
- How to represent backend commands (invoke/agent_command) and frontend UI commands in one registry (a bridge).
- → When implementation starts, re-survey the prior art (VS Code command/keybinding/when) and put the options to a user decision.
