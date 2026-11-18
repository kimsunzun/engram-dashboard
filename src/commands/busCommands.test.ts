// ★The **list lock** of the screen commands that go on the bus★.
// Why: what goes out is decided by one field, `help` (`viewCommandBridge.offeredCommands`), and adding or removing it is a **one-line edit** that changes the daemon registry's vocabulary — with nothing pinning the list, "a command that used to register silently vanished" · "one went out unintentionally" do not show in the diff. This hand-written array makes that edit a **visible change** (the test must be fixed along with it).
// ★Why help is not forced on every command★: only a few for proof go up and the rest are deferred, so `help` stays optional and **which ones have it** is pinned here.
// ★Collisions with names the shell and daemon answer are not measured here★ — shell table names are filtered in one place in Rust (`reserved_names` in `src-tauri/src/view_commands.rs`, harness `tests/layout_commands.rs`), and the wall for names the daemon answers is the daemon's registration bounce (ADR-0270 decision 2).

import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main', close: vi.fn(async () => undefined) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('../api/clientFactory', () => ({
  agentClient: new Proxy({}, { get: () => vi.fn(async () => undefined) }),
  getAgentClient: vi.fn(),
  bootstrapDaemonIfNeeded: vi.fn(async () => undefined),
}))

import './contributions' // side-effect: register all commands
import { list } from './registry'
import { offeredCommands } from './viewCommandBridge'

/**
 * All the screen commands that go on the bus today — fix this line when adding or removing one.
 * ★It must not be empty★: otherwise the shell has no screen share to put on the registry and **the whole webview delivery path goes unverified** (left by ADR-0167 removing the two theme commands — these two took their place).
 */
const ON_THE_BUS = ['slot.empty', 'tab.next']

describe('screen commands that go on the bus', () => {
  it('the list is exactly these two', () => {
    expect(offeredCommands().map(c => c.name).sort()).toEqual([...ON_THE_BUS].sort())
  })

  it('every one that goes on has a summary and an effect', () => {
    for (const offered of offeredCommands()) {
      expect(offered.help.summary.trim().length, `${offered.name}: summary`).toBeGreaterThan(0)
      // ★If effect is not carried the shell drops it from registration★ — picking a default would make the registry advertise a false marker (`ViewEffect` in `src-tauri/src/view_commands.rs`). The type already enforces it; on the day it loosens, this assertion is hit first.
      expect(['read', 'write'], `${offered.name}: effect`).toContain(offered.help.effect)
    }
  })

  it('the remaining screen commands have no help and stay inside this window only', () => {
    const local = list().filter(cmd => !ON_THE_BUS.includes(cmd.id))
    expect(local.length).toBeGreaterThan(0)
    for (const cmd of local) {
      expect(cmd.help, `${cmd.id} got a help — fix the list (ON_THE_BUS) along with it`).toBeUndefined()
    }
  })
})
