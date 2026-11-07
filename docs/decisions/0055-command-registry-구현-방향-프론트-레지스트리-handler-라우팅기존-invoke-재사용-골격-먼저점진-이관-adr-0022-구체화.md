# ADR-0055: Command registry implementation direction — a frontend registry + handler routing (reusing the existing invoke), skeleton first and gradual migration (makes ADR-0022 concrete)

- 상태: 확정 (2026-07-09, basis: design agreement with the user — the forks were resolved in this session's conversation) · 부분 폐기 by ADR-0169 (the clause of decision 3 on keeping __engramLayout (agent invoke stays))
- 관련: ADR-0022 (unified command registry direction — this ADR makes its unresolved forks concrete) · CLAUDE.md §5 (LLM-first control) · ADR-0020 (single dispatch) · ADR-0011 (agentClient control surface) · `docs/process/B-wezterm-tabs/PRD.md` (tabs = the first adapter) · step-log 2026-07-09 · Amended by ADR-0169 (the clause of decision 3 on keeping __engramLayout (agent invoke stays))

## 맥락
ADR-0022 fixed only the direction "every action = a command registered by id, and palette, keybinding, LLM and menu are consumers (derived)" and left the implementation forks (registry location, backend mirror, keybinding storage, when-context, enum vs registry) **unresolved**. Implementation now starts — the minimal skeleton must be fixed. User policy: **"Don't tear everything up into commands at once; build the system (skeleton) and migrate one by one."**

## 결정
- **command = `{ id, title, category?, keybinding?, when?, run(args) }`**, registered in a **frontend Map registry** (`register`/`run(id,args)`/`list`).
- **Execution = the handler routes to the existing entry points** — a backend action goes through `invoke('cmd', args)` (existing typed dispatch → the `ViewManager`/`AgentManager` singletons), and a pure frontend action calls the store. **Do not create new singletons or new arg-parsing logic** (state authority stays as it is, ADR-0035 kept).
- **Consumers = all unified under the single `run(id, args)`:** human click · a global `keydown` (★a focus guard is mandatory — never hijack shortcuts while typing in `input`/`textarea`/`.xterm`★) · `window.__engramCmd.{list, run}` (the LLM/cdp §5 entry point). The palette is a separate UI that eats `list()`, as a **follow-up**.
- **Skeleton first + gradual migration:** **leave the existing ad-hoc surfaces** (`window.__engramLayout`, agent `invoke`) **as they are**, and register **new features first** in the registry. No big-bang full migration.
- **Args = a single object (a bag)**; no variadic arguments. Each handler destructures only the keys it needs. TS type safety comes from an `id → 인자타입` lookup that gives a compile check at call sites (optional; this is where enum-exhaustive safety is obtained).
- **Registry location = frontend only** (the LLM introspects `__engramCmd.list()` through cdp). A backend mirror = follow-up.

## 거부한 대안
- **A single enum-data command (variant + central dispatch):** types and exhaustiveness are strong, but **every new command requires editing the central enum + match + metadata map** → violates the ADR-0022 goal of "zero ripple on addition · a feature registers its own command itself (decentralized)". (The backend `AgentCommand` enum is the **wire contract and the execution layer**, so it stays as it is — it coexists with this ADR's frontend registry as a separate layer.)
- **A backend mirror registry (now):** ideal for LLM introspection, but the current cdp `list()` is enough → follow-up (avoids the bridge and duplication cost).
- **Big-bang full migration:** turning all the existing surfaces into commands immediately = risky and large → gradual (user policy).
- **Custom keymap storage (localStorage) · a full when-context model · palette UI:** outside the skeleton → follow-up (build up from the minimum and add as layers).

## 근거
- The VS Code-proven model of ADR-0022 (registry → palette/keybinding/menu derived). An extensibility pattern for 10+ years.
- **Reuses the existing invoke/`AgentCommand` typed dispatch as the execution layer** → zero new singletons, and the registry is a thin discovery/routing/metadata layer.
- The hot path (agent output) passes binary raw, so using JSON for commands, which are a cold path, has a negligible cost (human hand speed).
- The skeleton is small and low risk (pure frontend, additive), so laying down the seam now is advantageous (CLAUDE.md §0 — low risk + long term → over-engineering allowed).

## 영향 / 불변식
- **New UI/layout features are registered as commands** (§5 — the LLM, keybinding and palette absorb them automatically). **Tabs (PRD B-tabs, Phase 2) are the first adapter** — `tab.switch`/`tab.create`/`tab.close`, etc.
- **The registry is not a state authority** — discovery/routing/metadata only. Execution goes through the existing invoke → singletons (ADR-0035 layout authority kept).
- **Keybinding focus-guard invariant:** without it, shortcuts hijack typing in the terminal/input box, a regression (load-bearing).
- It does not supersede ADR-0022 — it is a **concretization (Amends)** of that direction. Follow-up extensions (backend mirror, palette, custom keymap, when) are layered on top of this skeleton.
