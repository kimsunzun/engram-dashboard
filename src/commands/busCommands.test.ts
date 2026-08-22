// ★버스에 오르는 화면 command 의 **명단 자물쇠**★.
//
// 왜 필요한가: 무엇이 밖으로 나가나는 `help` 한 칸이 정하고(`viewCommandBridge.offeredCommands`), 그 칸을
// 더하거나 빼는 것은 **한 줄 편집**이다. 그런데 그 한 줄은 데몬 명부의 어휘를 바꾼다 — 명단을 아무 데서도
// 안 물면 「등록되던 command 가 조용히 사라졌다」·「의도 없이 하나가 밖으로 나갔다」가 diff 에 안 보인다.
// 손으로 적은 이 배열이 그 편집을 **눈에 보이는 변경**으로 만든다(테스트를 함께 고쳐야 초록이 된다).
//
// ★help 를 전 command 에 강제하지 않는 이유(사용자 결정)★: 이번 스텝의 범위가 증명용 넷이고 나머지 29개는
// 이연이다. 그래서 `help` 는 선택 칸으로 두고, 대신 **어느 것이 그 칸을 갖는지**를 여기서 못 박는다.
//
// ★셸·데몬이 답하는 이름과의 충돌은 여기서 안 잰다★ — 그 판정은 Rust 한 곳에 산다
// (`src-tauri/src/view_commands.rs` 의 `reserved_names`, 하네스는 `tests/layout_commands.rs`).

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

import './contributions' // side-effect: 전 command 등록
import { list } from './registry'
import { offeredCommands } from './viewCommandBridge'

/** 오늘 버스에 오르는 화면 command 전량 — 늘리거나 줄이면 이 줄을 함께 고친다. */
const ON_THE_BUS = ['slot.empty', 'tab.next', 'theme.set', 'theme.toggle']

describe('화면 command 중 버스에 오르는 것', () => {
  it('명단이 정확히 이 넷이다', () => {
    expect(offeredCommands().map(c => c.name).sort()).toEqual([...ON_THE_BUS].sort())
  })

  it('오르는 것은 전부 summary 와 effect 를 갖는다', () => {
    for (const offered of offeredCommands()) {
      expect(offered.help.summary.trim().length, `${offered.name}: summary`).toBeGreaterThan(0)
      // ★effect 를 안 실으면 셸이 등록에서 뺀다★ — 기본값을 고르면 명부가 거짓 표식을 광고하기 때문이다
      //   (`src-tauri/src/view_commands.rs` 의 `ViewEffect`). 타입이 이미 강제하지만, 그 타입이 느슨해지는
      //   날 이 단언이 먼저 걸린다.
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
