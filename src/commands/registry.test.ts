
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, getCommand, list, register, run, runAsHuman } from './registry'

beforeEach(() => {
  __resetRegistryForTest()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('command registry (ADR-0055)', () => {
  it('register + run: calls the handler and passes the return value through as is', () => {
    register({ id: 'a.b', title: () => 'A B', run: () => 42 })
    expect(run('a.b')).toBe(42)
  })

  it('run (unknown id): throws clearly (not a silent no-op)', () => {
    expect(() => run('nope.nope')).toThrow(/unknown command id: 'nope\.nope'/)
  })

  it('arguments = passed as a single object bag (not variadic)', () => {
    const spy = vi.fn((args?: Record<string, unknown>) => args?.theme)
    register({ id: 'demo.theme', title: () => 'set', run: spy })
    const result = run('demo.theme', { theme: 'light', extra: 1 })
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith({ theme: 'light', extra: 1 })
    expect(result).toBe('light')
  })

  it('run (no args): the handler receives undefined', () => {
    const spy = vi.fn(() => 'ok')
    register({ id: 'x', title: () => 'x', run: spy })
    run('x')
    expect(spy).toHaveBeenCalledWith(undefined)
  })

  it('a handler returning a Promise: run returns that Promise as is (awaitable)', async () => {
    register({ id: 'async.cmd', title: () => 'async', run: () => Promise.resolve('done') })
    const result = run('async.cmd')
    expect(result).toBeInstanceOf(Promise)
    await expect(result as Promise<string>).resolves.toBe('done')
  })

  it('duplicate id: warns but registers anyway (the last one wins)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'dup', title: () => 'first', run: () => 1 })
    register({ id: 'dup', title: () => 'second', run: () => 2 })
    expect(warn).toHaveBeenCalledOnce()
    expect(warn.mock.calls[0][0]).toContain("'dup'")
    expect(run('dup')).toBe(2)
  })

  it('list: returns a metadata snapshot of the registered commands (excluding the run function)', () => {
    register({ id: 'c1', title: () => 'C1', category: 'cat', keybinding: 'Ctrl+K', run: () => {} })
    register({ id: 'c2', title: () => 'C2', run: () => {} })
    const items = list()
    expect(items).toHaveLength(2)
    const c1 = items.find(i => i.id === 'c1')!
    expect(c1).toEqual({ id: 'c1', title: 'C1', category: 'cat', keybinding: 'Ctrl+K' })
    expect('run' in c1).toBe(false)
    const c2 = items.find(i => i.id === 'c2')!
    expect(c2.category).toBeUndefined()
  })

  it('list: an empty registry gives an empty array', () => {
    expect(list()).toEqual([])
  })

  // ★help must be carried in the projection for there to be a way onto the bus★ (TRD §6 Step 4): the shell unfolds this value into a catalog entry
  //   and puts it on the daemon registry. If missing, only the name is registered and a caller that finds it has no material to fill the arguments (ADR-0156).
  it('list: carries help as is (undefined when absent)', () => {
    const help = {
      summary: 'Changes the theme',
      effect: 'write' as const,
      args: { theme: { type: 'string' as const } },
    }
    register({ id: 'demo.theme', title: () => 'set', help, run: () => {} })
    register({ id: 'local.only', title: () => 'local', run: () => {} })

    const items = list()
    expect(items.find(i => i.id === 'demo.theme')!.help).toEqual(help)
    expect(items.find(i => i.id === 'local.only')!.help).toBeUndefined()
  })

  // ★The snapshot contract breaks most easily at the help field★: with value fields alone a shallow copy was enough, but
  //   help is two levels of object (args → enum array), so handing it over as is lets a consumer directly edit a registered command.
  //   toEqual alone passes even with an alias, so we measure by **re-reading after tampering**.
  it('list/getCommand: help is a copy, so a consumer changing it does not change the registry', () => {
    register({
      id: 'demo.theme',
      title: () => 'set',
      help: {
        summary: 'original',
        effect: 'write',
        args: { theme: { type: 'string', enum: ['dark'] } },
        required: ['theme'],
      },
      run: () => {},
    })

    const leaked = list().find(i => i.id === 'demo.theme')!.help!
    leaked.summary = 'tampered'
    leaked.effect = 'read'
    leaked.args!.theme.type = 'number'
    leaked.args!.theme.enum!.push('intruder')
    leaked.required!.push('ghost')
    const viaGet = getCommand('demo.theme')!.help!
    viaGet.summary = 'tampered via get'
    viaGet.args!.theme.description = 'intruder'

    const fresh = list().find(i => i.id === 'demo.theme')!.help!
    expect(fresh.summary).toBe('original')
    expect(fresh.effect).toBe('write')
    expect(fresh.args!.theme).toEqual({ type: 'string', enum: ['dark'] })
    expect(fresh.required).toEqual(['theme'])
  })

  it('getCommand: returns a copy → tampering with cmd.run does not leak into the registry (FIX-C)', () => {
    register({ id: 'guarded', title: () => 'g', run: () => 'original' })
    getCommand('guarded')!.run = () => 'hijacked'
    expect(run('guarded')).toBe('original')
    expect(getCommand('guarded')!.id).toBe('guarded')
    expect(getCommand('missing')).toBeUndefined()
  })
})

// ── humanOnly — a door closed on the caller axis (2026-09-08) ─────────────────────────
//
// One registry entry serves both the human menu and the LLM surface. An action that must be open to only one of the two cannot be
// told apart by `when` (`when` does not know the caller, and neither entry point reads it) — so the entry point was split in two, and this section
// is the place that measures that split.
describe('humanOnly gate', () => {
  it('run (LLM entry point): throws carrying the reason and does not call the handler', () => {
    const spy = vi.fn(() => 'ran')
    register({ id: 'closed.door', title: () => 'c', humanOnly: 'a door a human must pass', run: spy })
    expect(() => run('closed.door')).toThrow(/a door a human must pass/)
    expect(spy).not.toHaveBeenCalled()
  })

  it('runAsHuman (human entry point): runs the same entry as is', () => {
    const spy = vi.fn(() => 'ran')
    register({ id: 'closed.door', title: () => 'c', humanOnly: 'a door a human must pass', run: spy })
    expect(runAsHuman('closed.door')).toBe('ran')
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('for an entry without humanOnly, the two entry points are the same', () => {
    register({ id: 'open.door', title: () => 'o', run: () => 'ran' })
    expect(run('open.door')).toBe('ran')
    expect(runAsHuman('open.door')).toBe('ran')
  })

  it('runAsHuman also throws on an unknown id (no silent no-op)', () => {
    expect(() => runAsHuman('nope.nope')).toThrow(/unknown command id/)
  })

  it('list: also gives the humanOnly reason (so it is known to be closed before calling)', () => {
    register({ id: 'closed.door', title: () => 'c', humanOnly: 'reason', run: () => 1 })
    register({ id: 'open.door', title: () => 'o', run: () => 1 })
    const items = list()
    expect(items.find(i => i.id === 'closed.door')!.humanOnly).toBe('reason')
    expect(items.find(i => i.id === 'open.door')!.humanOnly).toBeUndefined()
  })
})
