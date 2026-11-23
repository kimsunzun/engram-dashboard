// peerCommands — the PC list slot's commands (ADR-0394): every trust action is human-only and off the bus, and each runs the
// right daemon `peer.*` with the daemon's argument names. The screen that calls them is measured by `PeersSlot.test`.

import { beforeEach, describe, expect, it, vi } from 'vitest'

const client = vi.hoisted(() => ({
  peerCommand: vi.fn(async (_name: string, _args: Record<string, unknown>) => ({}) as unknown),
}))
vi.mock('../api/clientFactory', () => ({ agentClient: client }))

import './peerCommands' // side-effect: register
import { getCommand, list, run, runAsHuman } from './registry'

/** Every slot action that changes trust plus the slot's other writes. */
const HUMAN_ONLY = [
  'peers.add',
  'peers.rename',
  'peers.setAddress',
  'peers.remove',
  'peers.approve',
  'peers.deny',
  'peers.setNetwork',
  'peers.regenerateKey',
] as const

beforeEach(() => {
  client.peerCommand.mockClear()
})

describe('the human-only gate', () => {
  it.each(HUMAN_ONLY)('%s is human-only, has no help and is refused on the LLM surface', id => {
    const cmd = getCommand(id)
    expect(cmd?.humanOnly).toBeTruthy()
    expect(cmd?.help).toBeUndefined()
    expect(() => run(id, { id: 'x', name: 'n', request: 'r' })).toThrow(/not run through this surface/)
    expect(client.peerCommand).not.toHaveBeenCalled()
  })

  it('no peers.* command is offered to the bus (no help on any)', () => {
    const offered = list().filter(c => c.id.startsWith('peers.') && c.help !== undefined)
    expect(offered).toEqual([])
  })

  it('reading stays open — it changes no trust', async () => {
    expect(getCommand('peers.read')?.humanOnly).toBeUndefined()
    client.peerCommand.mockResolvedValueOnce({ state: 'ready' })
    await run('peers.read')
    expect(client.peerCommand).toHaveBeenCalledWith('peer.list', { agents: false })
  })
})

describe('the daemon command each runs', () => {
  const cases: [string, Record<string, unknown>, string, Record<string, unknown>][] = [
    ['peers.add', { id: ' abcd ', name: 'office', address: '' }, 'peer.add', { id: 'abcd', name: 'office' }],
    ['peers.add', { id: 'abcd', name: 'office', address: '10.0.0.2:47120' }, 'peer.add', { id: 'abcd', name: 'office', address: '10.0.0.2:47120' }],
    ['peers.rename', { id: 'abcd', name: 'home' }, 'peer.edit', { id: 'abcd', new_name: 'home' }],
    ['peers.setAddress', { id: 'abcd', address: '' }, 'peer.edit', { id: 'abcd', address: 'none' }],
    ['peers.remove', { id: 'abcd' }, 'peer.remove', { id: 'abcd' }],
    ['peers.approve', { request: 'r1' }, 'peer.approve', { request: 'r1' }],
    // The adopt question's third answer is an approve with start_fresh — never a deny.
    ['peers.approve', { request: 'r1', startFresh: true }, 'peer.approve', { request: 'r1', start_fresh: true }],
    ['peers.deny', { request: 'r1' }, 'peer.deny', { request: 'r1' }],
    ['peers.deny', { request: 'r1', stopAsking: true }, 'peer.deny', { request: 'r1', stop_asking: true }],
    [
      'peers.setNetwork',
      { port: 'default', relay: 'none', address_lookup: 'https://lookup.example', mdns: false },
      'peer.setNetwork',
      { port: 'default', relay: 'none', address_lookup: 'https://lookup.example', mdns: false },
    ],
    ['peers.regenerateKey', {}, 'peer.regenerateKey', {}],
  ]
  it.each(cases)('%s %j → %s %j', async (id, args, daemon, daemonArgs) => {
    await runAsHuman(id, args)
    expect(client.peerCommand).toHaveBeenNthCalledWith(1, daemon, daemonArgs)
  })

  it('a missing required argument throws before anything is sent', async () => {
    await expect(runAsHuman('peers.add', { id: 'abcd' }) as Promise<unknown>).rejects.toThrow(/name required/)
    expect(client.peerCommand).not.toHaveBeenCalled()
  })
})
