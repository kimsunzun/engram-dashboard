// ADR-0055 / ADR-0011: only routes to the agentClient (single control surface) spawn path (zero new state paths).
//
// ★ADR-0064 slot menu contribution co-location★: knowledge of the tree (agent_list) content is gathered in this module (the common part is owned by '*').

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import type { AgentProfile, AgentOutputFormat } from '../api/types'
import { agentClient } from '../api/clientFactory'
import { INPUT_LOCKED_REFUSAL } from '../api/agentClient'
import { useAgentStore } from '../store/agentStore'
import { refreshProfiles } from '../store/eventBus'
import { pendingInterrupt, useInterruptStore } from '../store/interruptStore'
import { matchDeclaredSpelling } from './enumArg'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

// ★ADR-0078★: the render mode (Terminal=xterm PTY / StreamJson=headless NDJSON→RichSlot) is fixed at creation time and
//   immutable afterwards — the mode is picked in the pane "Create agent" submenu, which makes a reserved profile (an activation-time override
//   was rejected: the activation row menu keeps a single "Activate"). Only registers a claude reserved (inactive) profile (does not spawn).
async function createReserved(make: (cwd: string) => Promise<AgentProfile>) {
  const picked = await open({ directory: true, multiple: false, title: t('dialog.pickAgentCwd') })
  const cwd = typeof picked === 'string' ? picked : null
  if (!cwd) return // cancelled — no-op
  const profile = await make(cwd)
  // broadcast can be lost (ws queue full, ws.rs:145) and the subscription comes after layout init (eventBus.ts), so right after creation
  // an explicit refetch guarantees the reserved node shows (same shape as activateReserved's .then(refreshProfiles)
  // belt-and-suspenders).
  await refreshProfiles()
  return profile
}

async function createReservedProfile(outputFormat: AgentOutputFormat) {
  return createReserved(cwd => agentClient.createClaudeProfile(cwd, cwd, [], [], false, outputFormat))
}

async function createReservedCodexProfile(outputFormat: AgentOutputFormat) {
  return createReserved(cwd => agentClient.createCodexProfile(cwd, cwd, [], [], false, outputFormat))
}

// ★the `humanOnly` gate on the two codex creation doors was lifted on 2026-09-22★ — the premise the gate was built on (「in a folder it has
//   never seen, codex shows a trust confirmation modal and a non-human caller cannot get past it」) was overturned by measurement,
//   because codex's `app-server` path answers with success for a folder it has never seen and writes that folder into
//   its trust list by itself. The folder trust modal was a TUI-only device.
// ★The canonical place for the reason and the rationale is not here★ — it is the codex row of `commands::LLM_BACKEND_POLICY` in
//   `engram-dashboard-agent`, and the doors that look at that table are two: `agent.new` and the daemon's `SpawnByCwd` handler (the spawn
//   packet of `agent.spawnInto` goes there · ADR-0279). Do not copy that text here — no gate keeps a copy in sync with the table.
// ★To revert, start from that table, not here★ — an edit that closes these doors again must close the codex row of that table
//   before reviving `humanOnly` (otherwise the same backend can be made through `agent.new` but not through this door, a
//   mismatch; the places that measure it are `agentCommands.test.ts` and the policy tests in agent `commands.rs` and the daemon's
//   `connection_core.rs`).

// ★ADR-0078★: AgentOutputFormat boundary validator — a compile-time union is no runtime defence, so narrow to an allowlist
//   of valid values. Unspecified (undefined/null) defaults to 'StreamJson' (back-compat). Specified but neither of the two valid values
//   → an explicit throw instead of a silent no-op or passing it to the backend (includes the bad value — §5 LLM/cdp debugging).
// ★Callers may use any letter case; the outgoing value may not★: removes the mismatch where the same word works via the CLI
//   but not via this surface (user decision 2026-09-23 — the same decision as the Rust-side `AgentBackendKind` deserialization and the command bus enum
//   folding). What goes out after folding here is always the declared spelling below, so the wire value is byte-identical to before.
const VALID_OUTPUT_FORMATS: readonly AgentOutputFormat[] = ['Terminal', 'StreamJson']
function coerceOutputFormat(raw: unknown): AgentOutputFormat {
  if (raw === undefined || raw === null) return 'StreamJson'
  const matched = matchDeclaredSpelling(raw, VALID_OUTPUT_FORMATS)
  if (matched) return matched
  throw new Error(`agentlist.createAgent: invalid outputFormat: ${String(raw)} (valid: 'Terminal' | 'StreamJson')`)
}

register({
  id: 'agent.spawn',
  title: () => t('agent.spawn'),
  category: 'agent',
  //   - when preset(id) is given, cwd is resolved from store.presets (preset = cwd bookmark, ADR-0061). In the end it
  //     flows to agentClient.spawnAgent(cwd) (daemon SpawnByCwd).
  //   - parent is SIGNATURE-ONLY: nesting (spawning under a parent) is not supported yet — when set, an explicit throw (no silent
  //     ignoring, for LLM/cdp debugging). Wire it here when tree nesting is introduced.
  run: (args) => {
    const preset = args?.preset as string | undefined
    const rawCwd = args?.cwd as string | undefined
    const parent = args?.parent as string | undefined

    if (parent !== undefined && parent !== null) {
      throw new Error('agent.spawn: parent nesting not supported')
    }

    let cwd: string | undefined
    if (preset) {
      const found = useAgentStore.getState().presets.find(p => p.id === preset)
      if (!found) {
        throw new Error(`agent.spawn: unknown preset id: '${preset}'`)
      }
      cwd = found.cwd
    } else {
      cwd = rawCwd
    }

    if (!cwd || !cwd.trim()) {
      throw new Error(`agent.spawn: cwd is empty: ${String(cwd)}`)
    }
    // ★This door emits only claude — on 2026-09-22 **the reason changed**★. The old reason (「codex shows a trust confirmation modal
    //   and the LLM cannot get past it」) died by measurement (where that rationale lives = this file's head comment). The one
    //   remaining reason is **not widening the argument surface**: exposing a backend argument on this command would
    //   split its vocabulary from the backend door with the same id (`agent.spawn` in `engram-dashboard-agent`) — that one is also pinned
    //   to claude for the same reason (`create_and_start` in `commands.rs`).
    // ★The LLM path that makes codex is `agent.new`★ — backend and output_format are picked there together.
    return agentClient.spawnAgent(cwd.trim(), 'claude')
  },
})

register({
  id: 'agent.rename',
  title: () => t('agent.rename'),
  category: 'agent',
  // ADR-0061 enrichment (§5 LLM control): set/clear the agent (profile) display-name override.
  //   - name string → store the override. name null/empty string/unspecified → clear the override (back to the cwd basename derivation).
  //   Reflected through the ProfileListUpdated broadcast (no optimistic update). An unknown id is a backend Error. This is why rename is
  //   a backend save rather than frontend-only — the LLM can change the tree display name through the same surface (command).
  run: (args) => {
    const id = args?.id as string | undefined
    if (!id || !id.trim()) {
      throw new Error(`[agent.rename] id is empty: ${String(id)}`)
    }
    const raw = args?.name
    const name = typeof raw === 'string' && raw.trim().length > 0 ? raw.trim() : null
    return agentClient.renameProfile(id.trim(), name)
  },
})

// ADR-0231 · ADR-0237: calls the WS commands that affect input (cancel queued input · interrupt) and unfolds their refusals into
//   the same code as the same refusal on the bus side. ★Only the input lease refusal comes without a code★ (the WS text is the same as the `WriteStdin` refusal) — only that one gets
//   `CONFLICT`. It is recognized by the text: telling them apart by the presence of a prefix would also catch code-less failures such as a disconnect.
// ★The operating carrier (the shell's `forward_daemon_command`) returns refusals as bare strings, not `Error`★ — telling them apart only by `instanceof
//   Error` never matches in operation. Both are unfolded to text and then compared; the rest is thrown in its original shape.
// The argument is a function rather than a promise so that a synchronous throw on the caller side passes the same mapping too.
async function withLeaseConflict<T>(call: () => Promise<T>): Promise<T> {
  try {
    return await call()
  } catch (e) {
    const message = e instanceof Error ? e.message : typeof e === 'string' ? e : null
    if (message === INPUT_LOCKED_REFUSAL) {
      throw new Error(`CONFLICT: ${message}`)
    }
    throw e
  }
}

register({
  id: 'agent.cancelQueuedInput',
  title: () => t('agent.cancelQueuedInput'),
  category: 'agent',
  // ★Do not attach `help`★ — the daemon already answers the same name on the bus (the declaration in `engram-dashboard-agent`). With help
  //   the shell puts this name into the window command registration bundle, and its name collision check refuses **the whole bundle**, so every
  //   command of this window drops off the bus. It is for inside the window only (✕ · `window.__engramCmd`).
  // ADR-0231: the list ✕ and the LLM shake the same handle. It does not hide itself — the outcome comes back to the list as a ring event.
  run: async (args) => {
    const agentId = args?.agentId
    const inputId = args?.inputId
    if (typeof agentId !== 'string' || !agentId.trim()) {
      throw new Error(`[agent.cancelQueuedInput] agentId is empty: ${String(agentId)}`)
    }
    if (typeof inputId !== 'string' || !inputId) {
      throw new Error(`[agent.cancelQueuedInput] inputId is empty: ${String(inputId)}`)
    }
    // The outcome word is returned as a string as is — a newer daemon's words are not dropped either.
    return { outcome: await withLeaseConflict(() => agentClient.cancelQueuedInput(agentId.trim(), inputId)) }
  },
})

register({
  id: 'agent.interrupt',
  title: () => t('agent.interrupt'),
  category: 'agent',
  // ★Do not attach `help`★ — same reason as `agent.cancelQueuedInput` above (the daemon answers the same name on the bus).
  // ADR-0237: the chat area's Esc and the LLM shake the same handle. `requested` = the interrupt was sent — whether the turn actually stopped is told by the turn-end
  //   event (the same as the outcome word of the bus-side command with the same name).
  // ADR-0244: as soon as it sends, it marks that agent as 「interrupting」 — the area draws that and ignores Esc meanwhile. If refused,
  //   it is cleared here; clearing it at turn end is the area's job (`store/interruptStore.ts`).
  //   ★While interrupting, it does not send again and returns the outcome of the held request★ — a second interrupt could land on the turn opened by
  //   the next queued message. It does not answer with a refusal because the interrupt is already out, so `requested` is true. If that request is still waiting for an answer,
  //   it follows that outcome (refusal included) as is.
  run: async (args) => {
    const agentId = args?.agentId
    if (typeof agentId !== 'string' || !agentId.trim()) {
      throw new Error(`[agent.interrupt] agentId is empty: ${String(agentId)}`)
    }
    const id = agentId.trim()
    const inFlight = pendingInterrupt(useInterruptStore.getState(), id)
    if (inFlight !== undefined) {
      await inFlight
      return { outcome: 'requested' }
    }
    const request = withLeaseConflict(() => agentClient.interruptAgent(id))
    useInterruptStore.getState().begin(id, request)
    try {
      await request
    } catch (e) {
      useInterruptStore.getState().end(id, request)
      throw e
    }
    return { outcome: 'requested' }
  },
})

register({
  id: 'agentlist.createAgent',
  title: () => t('agent.create'),
  category: 'agent',
  // ★ADR-0064★: parametric §5 LLM primitive of the agent_list (tree) slot pane menu "Create agent" family —
  //   picks a cwd with the folder dialog, then registers a claude reserved (inactive) profile (does not spawn). It shows in the tree as a reserved
  //   node, and only activation (double click / right-click Activate → spawnProfile) spawns claude.
  // ★Behaviour change (WHY)★: the old flow launched a shell (cmd.exe) agent *immediately* via agent.spawn({cwd}) → SpawnByCwd
  //   (kind='running', not claude, not reserved) — it was off from the user's confirmed intent (create = claude reserved, activate = claude spawn).
  //   So instead of spawning immediately here, it only registers through createReservedProfile. The agent.spawn primitive
  //   stays as is (valid as the LLM/cdp immediate-spawn primitive — a separate concern).
  // ★ADR-0078★: outputFormat is taken from args.outputFormat, defaulting to 'StreamJson' when unspecified (back-compat — old
  //   calls, tests and LLM references calling without the argument keep the previous behaviour). The human menu path has the two leaf commands below
  //   (createTerminal/createJson) fix the mode explicitly. The command id is preserved (backward compatibility).
  //   ★Boundary validation (§5 LLM/cdp primitive)★: outputFormat is external input, so no unvalidated cast — it is filtered by a runtime
  //   allowlist (AgentOutputFormat is a compile-time union, no runtime enum): unspecified → default,
  //   a bad value → an explicit throw instead of silently flowing to the backend (the same fail-loud as agent.spawn/rename — includes the bad value
  //   for LLM/cdp debugging).
  run: async (args) => createReservedProfile(coerceOutputFormat(args?.outputFormat)),
})

register({
  id: 'agentlist.createTerminal',
  title: () => t('agent.createTerminal'),
  category: 'agent',
  // ★ADR-0078★: creation with the render mode fixed to Terminal (xterm PTY) — submenu leaf. The mode is settled at creation time and immutable afterwards.
  run: async () => createReservedProfile('Terminal'),
})

register({
  id: 'agentlist.createCodex',
  title: () => t('agent.createCodex'),
  category: 'agent',
  // ★The door where a human picks the codex interactive TUI★ — activating the reserved node made here launches codex in that
  //   folder. ★In a first-visit folder, codex's own trust confirmation modal shows on screen as is and the human
  //   gets past it (user decision 2026-09-07 — we do not plant trust ahead to bypass it). That modal exists **only on the TUI
  //   path** — so it was no reason to keep this door closed to the LLM, and on 2026-09-22
  //   `humanOnly` was lifted (the head comment above points to where the rationale lives).
  run: async () => createReservedCodexProfile('Terminal'),
})

register({
  id: 'agentlist.createCodexJson',
  title: () => t('agent.createCodexJson'),
  category: 'agent',
  // ★The door where a human picks the codex resident JSON server (`codex app-server`)★ — the same backend as the sibling `createCodex`;
  //   only the output mode tells them apart. The place where that mode also decides the link, the input encoding and the output decoder
  //   is `backend::codex::is_app_server` in `engram-dashboard-agent`.
  // ★Same contract as ADR-0078★: the mode is fixed at creation time and immutable afterwards (no activation-time override).
  // ★`humanOnly` was lifted the same day as the sibling `createCodex` (2026-09-22)★ — the two doors carry the same gate, so
  //   opening only one of them is itself a regression (where that is measured = `agentCommands.test.ts`).
  run: async () => createReservedCodexProfile('StreamJson'),
})

register({
  id: 'agentlist.createJson',
  title: () => t('agent.createJson'),
  category: 'agent',
  // ★ADR-0078★: creation with the render mode fixed to StreamJson (headless NDJSON→RichSlot) — submenu leaf. Settled at creation time, immutable.
  run: async () => createReservedProfile('StreamJson'),
})

// ★ADR-0078★: the agent_list pane menu "Create agent" as a one-level submenu container — the render mode is picked *at creation time*
//   (immutable afterwards). children are flat leaves (declaration order kept, ADR-0065) — an activation-time override was rejected.
// ADR-0064: group='content' — rendered above the common slot-ops.
registerSlotMenu('agent_list', [
  {
    id: 'agentlist.create',
    title: () => t('agent.create'),
    group: 'content',
    order: 10,
    children: [
      { commandId: 'agentlist.createJson', group: 'content', order: 10 },
      { commandId: 'agentlist.createTerminal', group: 'content', order: 20 },
      { commandId: 'agentlist.createCodex', group: 'content', order: 30 },
      { commandId: 'agentlist.createCodexJson', group: 'content', order: 40 },
    ],
  },
])
