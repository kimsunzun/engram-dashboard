
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'new-agent' })),
  renameProfile: vi.fn(async () => undefined),
  createClaudeProfile: vi.fn(async () => ({ id: 'new-profile' })),
  createCodexProfile: vi.fn(async () => ({ id: 'new-codex-profile' })),
  // listProfiles as called by refreshProfiles(eventBus) — for checking the store/tree update right after creation. Default []
  //   (each test loads the created profile with mockResolvedValueOnce).
  listProfiles: vi.fn(async () => [] as unknown[]),
  cancelQueuedInput: vi.fn(async (): Promise<string> => 'requested'),
  interruptAgent: vi.fn(async (): Promise<void> => undefined),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: {
    spawnAgent: (...args: unknown[]) => clientMock.spawnAgent(...(args as [])),
    renameProfile: (...args: unknown[]) => clientMock.renameProfile(...(args as [])),
    createClaudeProfile: (...args: unknown[]) => clientMock.createClaudeProfile(...(args as [])),
    createCodexProfile: (...args: unknown[]) => clientMock.createCodexProfile(...(args as [])),
    listProfiles: (...args: unknown[]) => clientMock.listProfiles(...(args as [])),
    cancelQueuedInput: (...args: unknown[]) => clientMock.cancelQueuedInput(...(args as [])),
    interruptAgent: (...args: unknown[]) => clientMock.interruptAgent(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))

// agentlist.createAgent picks the cwd with the folder dialog (open) — pick/cancel is controlled per test.
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

import './agentCommands' // side-effect register
import { getCommand, list, run, runAsHuman } from './registry'
import { INPUT_LOCKED_REFUSAL } from '../api/agentClient'
import { fireAndForget } from './dispatch'
import { buildSlotMenu } from './slotMenu'
import { useAgentStore } from '../store/agentStore'
import { pendingInterrupt, useInterruptStore } from '../store/interruptStore'

beforeEach(() => {
  clientMock.spawnAgent.mockClear()
  clientMock.renameProfile.mockClear()
  clientMock.createClaudeProfile.mockClear()
  clientMock.createCodexProfile.mockClear()
  clientMock.listProfiles.mockClear()
  clientMock.cancelQueuedInput.mockReset()
  clientMock.cancelQueuedInput.mockImplementation(async () => 'requested')
  clientMock.interruptAgent.mockReset()
  clientMock.interruptAgent.mockImplementation(async () => undefined)
  dialogMock.open.mockReset()
  useAgentStore.setState({ profiles: [] })
  useInterruptStore.setState({ pending: {}, views: {} })
})
afterEach(() => {
  useAgentStore.setState({ profiles: [] })
})

describe('agent.spawn routing', () => {
  it('raw cwd → spawnAgent(trimmed cwd)', () => {
    run('agent.spawn', { cwd: '  C:/new/path  ' })
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/new/path', 'claude')
  })

  it('parent set → throw (nesting not supported)', () => {
    expect(() => run('agent.spawn', { cwd: 'C:/x', parent: 'p1' })).toThrow(/parent nesting not supported/)
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('empty/blank cwd → throw', () => {
    expect(() => run('agent.spawn', { cwd: '   ' })).toThrow()
    expect(() => run('agent.spawn', {})).toThrow()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })
})

// ── agent.rename(§5 — ADR-0061) ──────────────────────────────────────────────
describe('agent.rename routing', () => {
  it('id + name → renameProfile(id, trimmed)', () => {
    run('agent.rename', { id: '  a1  ', name: '  my agent  ' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', 'my agent')
  })
  it('name omitted/empty string → null (clear the override)', () => {
    run('agent.rename', { id: 'a1' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', null)
    clientMock.renameProfile.mockClear()
    run('agent.rename', { id: 'a1', name: '   ' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', null)
  })
  it('empty id → throw (no silent no-op)', () => {
    expect(() => run('agent.rename', { name: 'x' })).toThrow(/id is empty/)
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
  })
})

// agentlist.create* registers a reserved (inactive) profile; it does not spawn.
describe('agent_list creation-family routing', () => {
  // Minimal shape of the created profile (AgentProfile) — the value loaded into the store/tree via refreshProfiles → setProfiles.
  const createdProfile = { id: 'p-created', cwd: 'C:/work/engram', display_name: null }

  it('createTerminal → calls createClaudeProfile with outputFormat=Terminal', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createTerminal', {})

    expect(clientMock.createClaudeProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('createJson → calls createClaudeProfile with outputFormat=StreamJson', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createJson', {})

    expect(clientMock.createClaudeProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  // ★The door where a human picks the codex interactive TUI★ — like its siblings it only registers and does not spawn; it launches on activation (double click). ★`runAsHuman` here measures the human path, not a gate★ — `humanOnly` was lifted and the two entry points are the same; the LLM side is measured by the entry below.
  it('createCodex (human path) → calls createCodexProfile with outputFormat=Terminal', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await runAsHuman('agentlist.createCodex', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  // ★The place that measures that the mode axis really splits★ — the two codex doors call the same `createCodexProfile`, so
  //   if the mode is not carried or both carry the same value, one of the entry above and this entry goes red.
  it('createCodexJson (human path) → calls createCodexProfile with outputFormat=StreamJson', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await runAsHuman('agentlist.createCodexJson', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  // Same gate as sibling `createCodex` (open) — only one side opening or closing is the regression this entry blocks.
  it('createCodexJson: the LLM entry point (registry.run) also makes the same profile as the human path', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createCodexJson', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
  })

  // ★The third creation door — both entry points open the same door★ (the old `humanOnly` gate was lifted).
  // This entry is **a human menu and an LLM command at once**. What kept that double role closed was the **caller axis**, whose premise (the LLM cannot get past codex's trust confirmation modal) died by measurement — canonical rationale = the codex row of `commands::LLM_BACKEND_POLICY` in `engram-dashboard-agent`.
  // ★So what these two measure was flipped★: reviving the gate turns them red. If it ever needs reviving, close that table first — closing only here leaves it makeable through `agent.new` but not this door.
  it('createCodex: the LLM entry point (registry.run) also makes the same profile as the human path', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createCodex', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
  })

  it('createCodex: the human path (tree menu → fireAndForget) still makes codex', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    // This is the very call `SlotContextMenu` uses for menu items (`fireAndForget(id, …)` in that file).
    fireAndForget('agentlist.createCodex')

    await vi.waitFor(() => expect(clientMock.createCodexProfile).toHaveBeenCalled())
    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
  })

  it('createAgent (parametric): StreamJson by default without arguments, the given value when args.outputFormat is set', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])
    await run('agentlist.createAgent', {})
    expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    await run('agentlist.createAgent', { outputFormat: 'Terminal' })
    expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
  })

  // ★Case-insensitive acceptance + normalization to the declared spelling★: the value sent to the backend must be 'Terminal'·'StreamJson', not the caller's spelling — so the wire is byte-identical to before.
  it('createAgent (parametric): outputFormat accepted case-insensitively → normalized to the declared spelling and passed on', async () => {
    for (const [given, expected] of [
      ['terminal', 'Terminal'],
      ['TERMINAL', 'Terminal'],
      ['streamjson', 'StreamJson'],
      ['StreamJSON', 'StreamJson'],
    ] as const) {
      dialogMock.open.mockResolvedValueOnce('C:/work/engram')
      clientMock.listProfiles.mockResolvedValueOnce([createdProfile])
      await run('agentlist.createAgent', { outputFormat: given })
      expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
        'C:/work/engram', 'C:/work/engram', [], [], false, expected,
      )
    }
  })

  it('createAgent (parametric): invalid outputFormat → explicit throw + createClaudeProfile not called', async () => {
    // Validation comes before the dialog (open), so neither the folder pick nor createClaudeProfile runs.
    await expect(run('agentlist.createAgent', { outputFormat: 'invalid' })).rejects.toThrow(/invalid/)
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('cancel (dialog null) → no-op (client and refetch not called)', async () => {
    dialogMock.open.mockResolvedValueOnce(null)
    await run('agentlist.createTerminal', {})
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).not.toHaveBeenCalled()
    expect(useAgentStore.getState().profiles).toEqual([])
  })
})

// ── agent_list pane menu = one-level submenu container (ADR-0078 / ADR-0065) ──────────────
// registerSlotMenu has already contributed the container through the side-effect import.
describe('agent_list creation submenu (ADR-0078)', () => {
  it('"Create agent" container + 4 children (declaration order Json→Terminal→Codex→CodexJson)', () => {
    const items = buildSlotMenu('agent_list')
    const container = items.find(i => i.title === '에이전트 생성')
    expect(container).toBeDefined()
    expect(container?.children?.length).toBe(4)
    expect(container?.children?.map(c => c.id)).toEqual([
      'agentlist.createJson',
      'agentlist.createTerminal',
      'agentlist.createCodex',
      'agentlist.createCodexJson',
    ])
  })
})

// ── ADR-0231: the queued input ✕ and the LLM shake the same handle ─────────────────────────────────────
describe('agent.cancelQueuedInput', () => {
  it('registered and has no help — not on the bus list (it is a name the daemon answers, so if listed the whole registration bundle is bounced)', () => {
    expect(getCommand('agent.cancelQueuedInput')).toBeDefined()
    expect(getCommand('agent.cancelQueuedInput')?.help).toBeUndefined()
    expect(list().find((c) => c.id === 'agent.cancelQueuedInput')?.help).toBeUndefined()
  })

  it('calls agentClient.cancelQueuedInput with agentId·inputId and returns the outcome word', async () => {
    await expect(run('agent.cancelQueuedInput', { agentId: ' a1 ', inputId: 'q1' })).resolves.toEqual({
      outcome: 'requested',
    })
    expect(clientMock.cancelQueuedInput).toHaveBeenCalledWith('a1', 'q1')
  })

  it('returns an unknown outcome word as is too', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => 'deferred')
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).resolves.toEqual({
      outcome: 'deferred',
    })
  })

  it('a lease refusal text that came without a code reads as CONFLICT', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error(INPUT_LOCKED_REFUSAL)
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      `CONFLICT: ${INPUT_LOCKED_REFUSAL}`,
    )
  })

  it('a lease refusal text that came as a bare string like the operating carrier also reads as CONFLICT', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw INPUT_LOCKED_REFUSAL
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      `CONFLICT: ${INPUT_LOCKED_REFUSAL}`,
    )
  })

  it('another refusal (NOT_FOUND) that came as a bare string is rethrown as the string', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw 'NOT_FOUND: no waiting input'
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toBe(
      'NOT_FOUND: no waiting input',
    )
  })

  it('a refusal that is neither a string nor an Error is left alone', async () => {
    const odd = { code: 'X' }
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw odd
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toBe(odd)
  })

  it('a failure with a code (NOT_FOUND) and another code-less failure (disconnect) are left as they are', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error('NOT_FOUND: no waiting input')
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      /^NOT_FOUND: no waiting input$/,
    )
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error('connection lost')
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      /^connection lost$/,
    )
  })

  it('throws without sending for an empty agentId·inputId', async () => {
    await expect(run('agent.cancelQueuedInput', { agentId: '  ', inputId: 'q1' })).rejects.toThrow(/agentId/)
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1' })).rejects.toThrow(/inputId/)
    expect(clientMock.cancelQueuedInput).not.toHaveBeenCalled()
  })

  it('the human path (✕ click → fireAndForget) calls the same command too', async () => {
    fireAndForget('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q2' })
    await Promise.resolve()
    expect(clientMock.cancelQueuedInput).toHaveBeenCalledWith('a1', 'q2')
  })
})

// ── ADR-0237: the chat area's Esc and the LLM shake the same handle ─────────────────────────────────────
describe('agent.interrupt', () => {
  it('registered and has no help — not on the bus list (it is a name the daemon answers, so if listed the whole registration bundle is bounced)', () => {
    expect(getCommand('agent.interrupt')).toBeDefined()
    expect(getCommand('agent.interrupt')?.help).toBeUndefined()
    expect(list().find((c) => c.id === 'agent.interrupt')?.help).toBeUndefined()
  })

  it('calls agentClient.interruptAgent with agentId and returns requested', async () => {
    await expect(run('agent.interrupt', { agentId: ' a1 ' })).resolves.toEqual({ outcome: 'requested' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
    expect(clientMock.interruptAgent).toHaveBeenCalledWith('a1')
  })

  it('throws without sending for an empty, missing or non-string agentId', async () => {
    await expect(run('agent.interrupt', { agentId: '  ' })).rejects.toThrow(/agentId/)
    await expect(run('agent.interrupt', {})).rejects.toThrow(/agentId/)
    await expect(run('agent.interrupt', { agentId: 7 })).rejects.toThrow(/agentId/)
    expect(clientMock.interruptAgent).not.toHaveBeenCalled()
  })

  it('a lease refusal text that came without a code reads as CONFLICT whether it comes as an Error or a bare string', async () => {
    clientMock.interruptAgent.mockImplementation(async () => {
      throw new Error(INPUT_LOCKED_REFUSAL)
    })
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toThrow(`CONFLICT: ${INPUT_LOCKED_REFUSAL}`)
    clientMock.interruptAgent.mockImplementation(async () => {
      throw INPUT_LOCKED_REFUSAL
    })
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toThrow(`CONFLICT: ${INPUT_LOCKED_REFUSAL}`)
  })

  it('other refusals (no turn to interrupt · disconnect) are rethrown in their shape', async () => {
    clientMock.interruptAgent.mockImplementation(async () => {
      throw 'no turn to interrupt'
    })
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toBe('no turn to interrupt')
    clientMock.interruptAgent.mockImplementation(async () => {
      throw new Error('connection lost')
    })
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toThrow(/^connection lost$/)
  })

  it('a lease refusal the caller threw synchronously also reads as CONFLICT', async () => {
    clientMock.interruptAgent.mockImplementation((() => {
      throw new Error(INPUT_LOCKED_REFUSAL)
    }) as unknown as () => Promise<void>)
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toThrow(`CONFLICT: ${INPUT_LOCKED_REFUSAL}`)
  })

  it('the human path (Esc → fireAndForget) calls the same command too, and a refusal is only swallowed as a warning', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.interruptAgent.mockImplementation(async () => {
      throw 'no turn to interrupt'
    })
    fireAndForget('agent.interrupt', { agentId: 'a1' })
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    expect(clientMock.interruptAgent).toHaveBeenCalledWith('a1')
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })
})

// ── ADR-0244: 「interrupting」 from sending the interrupt until turn end · no resending meanwhile ───────────────────────────
describe('agent.interrupt — interrupting (ADR-0244)', () => {
  const interrupting = (agentId: string): boolean =>
    pendingInterrupt(useInterruptStore.getState(), agentId) !== undefined

  /** An interrupt request whose answer the test resolves. */
  function deferred(): { resolve: () => void; reject: (e: unknown) => void } {
    const handle = { resolve: () => {}, reject: (_e: unknown) => {} }
    clientMock.interruptAgent.mockImplementation(
      () =>
        new Promise<void>((resolve, reject) => {
          handle.resolve = resolve
          handle.reject = reject
        }),
    )
    return handle
  }

  it('if the agent has a conversation view, it is set as soon as it sends, and stays until turn end even when the interrupt is accepted', async () => {
    useInterruptStore.getState().watch('a1')
    const answer = deferred()
    const result = run('agent.interrupt', { agentId: ' a1 ' }) as Promise<unknown>
    expect(interrupting('a1')).toBe(true)
    expect(interrupting('other')).toBe(false)
    answer.resolve()
    await expect(result).resolves.toEqual({ outcome: 'requested' })
    expect(interrupting('a1')).toBe(true)
    // Clearing at turn end is the conversation view's job (RichSlot.test.tsx).
    useInterruptStore.getState().end('a1')
    expect(interrupting('a1')).toBe(false)
  })

  it.each<[string, () => unknown]>([
    ['lease refusal (CONFLICT)', () => new Error(INPUT_LOCKED_REFUSAL)],
    ['no turn to interrupt (bare string)', () => 'no turn to interrupt'],
    ['disconnect', () => new Error('connection lost')],
  ])('cleared when refused — %s', async (_label, error) => {
    useInterruptStore.getState().watch('a1')
    clientMock.interruptAgent.mockImplementation(async () => {
      throw error()
    })
    await expect(run('agent.interrupt', { agentId: 'a1' })).rejects.toBeDefined()
    expect(interrupting('a1')).toBe(false)
  })

  it('calling again while interrupting does not send and answers with requested', async () => {
    useInterruptStore.getState().watch('a1')
    await expect(run('agent.interrupt', { agentId: 'a1' })).resolves.toEqual({ outcome: 'requested' })
    await expect(run('agent.interrupt', { agentId: 'a1' })).resolves.toEqual({ outcome: 'requested' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
    expect(interrupting('a1')).toBe(true)

    // After the turn ends, the next call sends again.
    useInterruptStore.getState().end('a1')
    await run('agent.interrupt', { agentId: 'a1' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('if the first request is still waiting for an answer, the repeated call follows its outcome — refusal too', async () => {
    useInterruptStore.getState().watch('a1')
    const answer = deferred()
    const first = run('agent.interrupt', { agentId: 'a1' }) as Promise<unknown>
    const second = run('agent.interrupt', { agentId: 'a1' }) as Promise<unknown>
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(1)
    answer.reject('no turn to interrupt')
    await expect(first).rejects.toBe('no turn to interrupt')
    await expect(second).rejects.toBe('no turn to interrupt')
    expect(interrupting('a1')).toBe(false)
  })

  it("a late refusal of an old request does not clear a newer request's interrupting state", async () => {
    useInterruptStore.getState().watch('a1')
    const oldAnswer = deferred()
    const old = run('agent.interrupt', { agentId: 'a1' }) as Promise<unknown>
    // The turn ended while the old request was waiting for an answer, and a new interrupt is sent in the next turn.
    useInterruptStore.getState().end('a1')
    const newAnswer = deferred()
    const fresh = run('agent.interrupt', { agentId: 'a1' }) as Promise<unknown>
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)

    oldAnswer.reject('no turn to interrupt')
    await expect(old).rejects.toBe('no turn to interrupt')
    expect(interrupting('a1')).toBe(true)
    newAnswer.resolve()
    await expect(fresh).resolves.toEqual({ outcome: 'requested' })
  })

  it('if this window has no conversation view of that agent, it is not set, and calling again sends again', async () => {
    await run('agent.interrupt', { agentId: 'a1' })
    expect(interrupting('a1')).toBe(false)
    await run('agent.interrupt', { agentId: 'a1' })
    expect(clientMock.interruptAgent).toHaveBeenCalledTimes(2)
  })

  it('cleared only when the last conversation view is released — stays while another view of the same agent remains', async () => {
    const releaseA = useInterruptStore.getState().watch('a1')
    const releaseB = useInterruptStore.getState().watch('a1')
    await run('agent.interrupt', { agentId: 'a1' })
    releaseA()
    releaseA() // counted once even when called twice.
    expect(interrupting('a1')).toBe(true)
    releaseB()
    expect(interrupting('a1')).toBe(false)
    expect(useInterruptStore.getState().views).toEqual({})
  })

  it("does not read a prototype name ('constructor') as interrupting", async () => {
    await run('agent.interrupt', { agentId: 'constructor' })
    expect(clientMock.interruptAgent).toHaveBeenCalledWith('constructor')
  })
})
