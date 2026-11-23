// Commands of the PC list slot — each one runs one daemon `peer.*` on this app's own daemon connection, which the
//   daemon types as the human.
//
// ★Every action that changes trust is `humanOnly` and has no `help`★: `run` (the LLM surface — `window.__engramCmd.run`, the bus
//   bridge) refuses them, the bus never lists them, and the shell command table declares none of them — so an agent cannot relay one and
//   have it arrive at the daemon as the human. The slot calls them through `runAsHuman` from its buttons. Agents have the daemon's own
//   `peer.*` instead, where their trust changes only become requests a human approves.
//   The slot's write actions that the daemon leaves open to agents (remove, an address edit) are human-only here too — an agent calls
//   `peer.remove` / `peer.edit` itself; one rule for every slot write keeps the list short to check.
// ★Reading stays open★: `peers.read` (peer.list) is what the slot does on mount — no trust change, so it is not human-only; it has no `help`
//   either, because agents read `peer.list` directly (one query per area — `docs/reference/llm-command-conventions.md`).
// Opening the slot is `slot.fill.peers` (slotContentCommands) and the shell's `layout.setSlotContent content=Peers`.
// ADR-0394

import { agentClient } from '../api/clientFactory'
import type { PeerCommandName } from '../api/agentClient'
import { t } from '../i18n'
import { usePeerStore } from '../store/peerStore'
import { register, type CommandArgs } from './registry'

/** The reason carried by a refusal on the LLM surface. */
export const PEER_HUMAN_ONLY =
  'a trust change for other PCs is a human act (ADR-0394) — agents call the daemon `peer.*` commands, whose changes wait for a human to approve'

function str(args: CommandArgs | undefined, key: string, cmd: string): string {
  const v = args?.[key]
  if (typeof v !== 'string' || v.trim().length === 0) throw new Error(`[${cmd}] ${key} required`)
  return v.trim()
}

function optStr(args: CommandArgs | undefined, key: string, cmd: string): string | undefined {
  const v = args?.[key]
  if (v === undefined || v === null) return undefined
  if (typeof v !== 'string') throw new Error(`[${cmd}] ${key} must be a string`)
  const trimmed = v.trim()
  return trimmed.length === 0 ? undefined : trimmed
}

function optBool(args: CommandArgs | undefined, key: string, cmd: string): boolean | undefined {
  const v = args?.[key]
  if (v === undefined) return undefined
  if (typeof v !== 'boolean') throw new Error(`[${cmd}] ${key} must be a boolean`)
  return v
}

/** One human-only slot action: builds the daemon arguments, runs the command, then reads the state again (the broadcast also comes). */
function registerHumanOnly(
  id: string,
  title: () => string,
  daemon: PeerCommandName,
  build: (args: CommandArgs | undefined) => Record<string, unknown>,
): void {
  register({
    id,
    title,
    category: 'peers',
    humanOnly: PEER_HUMAN_ONLY,
    run: async args => {
      const daemonArgs = build(args)
      const answer = await agentClient.peerCommand(daemon, daemonArgs)
      void usePeerStore.getState().read()
      return answer
    },
  })
}

register({
  id: 'peers.read',
  title: () => t('peers.refresh'),
  category: 'peers',
  run: () => usePeerStore.getState().read(),
})

registerHumanOnly('peers.add', () => t('peers.add'), 'peer.add', args => {
  const cmd = 'peers.add'
  const address = optStr(args, 'address', cmd)
  return { id: str(args, 'id', cmd), name: str(args, 'name', cmd), ...(address ? { address } : {}) }
})

registerHumanOnly('peers.rename', () => t('peers.rename'), 'peer.edit', args => {
  const cmd = 'peers.rename'
  return { id: str(args, 'id', cmd), new_name: str(args, 'name', cmd) }
})

// `none` clears the address (the daemon's word).
registerHumanOnly('peers.setAddress', () => t('peers.editAddress'), 'peer.edit', args => {
  const cmd = 'peers.setAddress'
  return { id: str(args, 'id', cmd), address: optStr(args, 'address', cmd) ?? 'none' }
})

registerHumanOnly('peers.remove', () => t('peers.remove'), 'peer.remove', args => ({
  id: str(args, 'id', 'peers.remove'),
}))

// The adopt question's third answer is `startFresh` — `peer.approve --start_fresh true`, never a deny.
registerHumanOnly('peers.approve', () => t('peers.approve'), 'peer.approve', args => {
  const cmd = 'peers.approve'
  const startFresh = optBool(args, 'startFresh', cmd)
  return { request: str(args, 'request', cmd), ...(startFresh ? { start_fresh: true } : {}) }
})

registerHumanOnly('peers.deny', () => t('peers.deny'), 'peer.deny', args => {
  const cmd = 'peers.deny'
  const stopAsking = optBool(args, 'stopAsking', cmd)
  return { request: str(args, 'request', cmd), ...(stopAsking ? { stop_asking: true } : {}) }
})

// Absent fields keep their value (the daemon merges); `port` takes a number or `default`.
registerHumanOnly('peers.setNetwork', () => t('peers.network'), 'peer.setNetwork', args => {
  const cmd = 'peers.setNetwork'
  const out: Record<string, unknown> = {}
  for (const key of ['port', 'relay', 'address_lookup'] as const) {
    const v = optStr(args, key, cmd)
    if (v !== undefined) out[key] = v
  }
  const mdns = optBool(args, 'mdns', cmd)
  if (mdns !== undefined) out.mdns = mdns
  return out
})

registerHumanOnly('peers.regenerateKey', () => t('peers.regenerate'), 'peer.regenerateKey', () => ({}))
