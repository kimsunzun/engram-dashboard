// ★The **list lock** of the screen commands that go on the bus★.
//
// Why it is needed: what goes out is decided by one field, `help` (`viewCommandBridge.offeredCommands`), and
// adding or removing that field is a **one-line edit**. But that one line changes the vocabulary of the daemon registry — if nothing
// anywhere pins the list down, 「등록되던 command 가 조용히 사라졌다」 (a command that used to register silently vanished) · 「의도 없이 하나가 밖으로 나갔다」 (one went out unintentionally) do not show in the diff.
// This hand-written array makes that edit a **visible change** (the test must be fixed along with it to turn green).
//
// ★Why help is not forced on every command (user decision)★: only a few for proof go up and the rest are deferred.
// So `help` is left as an optional field, and instead **which ones have that field** is pinned down here.
//
// ★Collisions with names the shell and daemon answer are not measured here★ — the shell table names are filtered in one place in Rust
// (`reserved_names` in `src-tauri/src/view_commands.rs`, the harness is `tests/layout_commands.rs`), and the wall for names the daemon
// answers is the daemon's registration bounce (ADR-0270 decision 2).

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
 * All the screen commands that go on the bus today — when you increase or decrease them, fix this line along with it.
 *
 * ★It must not be empty★: if this is empty the shell has no screen share at all to put on the registry, so **the whole webview delivery path
 * goes unverified** (a condition left by ADR-0167 when it took down the two theme commands — the place left by removing those two is these two).
 */
const ON_THE_BUS = ['slot.empty', 'tab.next']

describe('화면 command 중 버스에 오르는 것', () => {
  it('명단이 정확히 이 둘이다', () => {
    expect(offeredCommands().map(c => c.name).sort()).toEqual([...ON_THE_BUS].sort())
  })

  it('오르는 것은 전부 summary 와 effect 를 갖는다', () => {
    for (const offered of offeredCommands()) {
      expect(offered.help.summary.trim().length, `${offered.name}: summary`).toBeGreaterThan(0)
      // ★If effect is not carried the shell drops it from registration★ — because picking a default would make the registry advertise a false marker
      //   (`ViewEffect` in `src-tauri/src/view_commands.rs`). The type already enforces it, but on the
      //   day that type loosens, this assertion is hit first.
      expect(['read', 'write'], `${offered.name}: effect`).toContain(offered.help.effect)
    }
  })

  it('나머지 화면 command 는 help 가 없어 이 창 안에만 남는다', () => {
    const local = list().filter(cmd => !ON_THE_BUS.includes(cmd.id))
    expect(local.length).toBeGreaterThan(0)
    for (const cmd of local) {
      expect(cmd.help, `${cmd.id} 가 help 를 갖게 됐다 — 명단(ON_THE_BUS)을 함께 고칠 것`).toBeUndefined()
    }
  })
})
