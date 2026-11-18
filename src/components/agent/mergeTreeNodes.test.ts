import { describe, expect, it } from 'vitest'

import type { AgentInfo, AgentProfile, Capabilities } from '../../api/types'
import { agentPresence, mergeTreeNodes } from './mergeTreeNodes'

const caps = (interrupt: boolean): Capabilities => ({
  input: { raw: true, message: false, attachment: false },
  output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
  control: { resize: true, interrupt, cancel: false, graceful_shutdown: false },
  session: { resume: true, snapshot: false, cwd_env: true },
  model: { select: false, temperature: false, max_tokens: false },
})

function agent(
  id: string,
  name = '',
  interrupt = true,
  status: AgentInfo['status'] = { type: 'Running' },
): AgentInfo {
  return {
    id,
    name,
    cwd: 'C:/x',
    status,
    cols: 80,
    rows: 24,
    epoch: 1,
    capabilities: caps(interrupt),
  }
}

function profile(
  id: string,
  name = '',
  createdAt = 0,
  displayName: string | null = null,
  parentId: string | null = null,
  lastFailure: AgentProfile['last_failure'] = null,
): AgentProfile {
  return {
    id,
    name,
    display_name: displayName,
    parent_id: parentId,
    command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd: 'C:/x',
    env: [],
    backend_session_id: null,
    old_session_ids: [],
    epoch: 0,
    auto_restore: false,
    restart_policy: 'Never',
    restart_count: 0,
    failed_reason: null,
    last_failure: lastFailure,
    created_at: createdAt,
    last_active: 0,
    last_start_at: null,
  }
}

describe('mergeTreeNodes', () => {
  it('empty input → empty array', () => {
    expect(mergeTreeNodes([], [])).toEqual([])
  })

  it('only running (no profile) → all running nodes, ad-hoc shell shown as is', () => {
    const out = mergeTreeNodes([], [agent('a', '코더'), agent('b')])
    expect(out).toHaveLength(2)
    expect(out[0]).toMatchObject({ id: 'a', name: '코더', kind: 'running', status: 'Running' })
    expect(out[1]).toMatchObject({ id: 'b', kind: 'running' })
  })

  it('only reserved (none running) → all reserved nodes (status=Reserved, canInterrupt=false)', () => {
    const out = mergeTreeNodes([profile('p1', '예약1'), profile('p2', '예약2')], [])
    expect(out).toHaveLength(2)
    expect(out[0]).toMatchObject({ id: 'p1', name: '예약1', kind: 'reserved', status: 'Reserved', canInterrupt: false })
    expect(out[1]).toMatchObject({ id: 'p2', kind: 'reserved' })
  })

  it('both (not overlapping) → running first, reserved after', () => {
    const out = mergeTreeNodes([profile('p1', '예약')], [agent('a', '실행')])
    expect(out.map(n => n.id)).toEqual(['a', 'p1'])
    expect(out[0].kind).toBe('running')
    expect(out[1].kind).toBe('reserved')
  })

  it('duplicate id (profile spawned and running) → running wins, reserved absorbed (no duplicate)', () => {
    const out = mergeTreeNodes(
      [profile('same', '프로필이름'), profile('p2', '여전히예약')],
      [agent('same', '실행중이름')],
    )
    expect(out.filter(n => n.id === 'same')).toHaveLength(1)
    const same = out.find(n => n.id === 'same')!
    expect(same.kind).toBe('running')
    expect(same.name).toBe('실행중이름')
    expect(out.find(n => n.id === 'p2')).toMatchObject({ kind: 'reserved' })
  })

  // ── ADR-0061 enrichment: display_name override propagation ──────────────────────────────
  it('display_name override → a reserved node takes it directly from the profile, a running node inherits it from the matching profile', () => {
    const out = mergeTreeNodes(
      [profile('p-run', '', 0, '실행override'), profile('p-res', '', 0, '예약override')],
      [agent('p-run'), agent('adhoc')],
    )
    expect(out.find(n => n.id === 'p-run')?.displayName).toBe('실행override')
    expect(out.find(n => n.id === 'adhoc')?.displayName).toBeNull()
    expect(out.find(n => n.id === 'p-res')?.displayName).toBe('예약override')
  })

  it('without display_name the node displayName=null (basename derivation, existing behavior unchanged)', () => {
    const out = mergeTreeNodes([profile('p1', '예약1')], [agent('a', '실행')])
    expect(out.find(n => n.id === 'p1')?.displayName).toBeNull()
    expect(out.find(n => n.id === 'a')?.displayName).toBeNull()
  })

  // ── ADR-0072 drop guard: hasProfile (whether a profile exists) ──
  //   Reparent needs a real profile on both child and parent (the backend rejects otherwise); hasProfile is the basis of the frontend drag/drop pre-filter.
  describe('hasProfile (drop guard, ADR-0072)', () => {
    it('reserved node → hasProfile=true (made from a profile)', () => {
      const out = mergeTreeNodes([profile('p1', '예약')], [])
      expect(out.find(n => n.id === 'p1')?.hasProfile).toBe(true)
    })

    it('running node with a matching profile → hasProfile=true', () => {
      const out = mergeTreeNodes([profile('a', '', 1)], [agent('a')])
      expect(out.find(n => n.id === 'a')?.hasProfile).toBe(true)
    })

    it('ad-hoc running node without a profile (SpawnByCwd) → hasProfile=false', () => {
      const out = mergeTreeNodes([], [agent('adhoc')])
      expect(out.find(n => n.id === 'adhoc')?.hasProfile).toBe(false)
    })

    it('mixed: ad-hoc (false) + matching running (true) + reserved (true) judged together', () => {
      const out = mergeTreeNodes(
        [profile('run', '', 1), profile('res', '', 2)],
        [agent('run'), agent('adhoc')],
      )
      expect(out.find(n => n.id === 'run')?.hasProfile).toBe(true)
      expect(out.find(n => n.id === 'adhoc')?.hasProfile).toBe(false)
      expect(out.find(n => n.id === 'res')?.hasProfile).toBe(true)
    })
  })

  // ── MINOR-2: deterministic sort regression guard ──
  // The backend listProfiles/agents may arrive in HashMap order, so unsorted nodes would jump on every refetch; "the same input set always gives the same order".
  describe('deterministic ordering (MINOR-2)', () => {
    it('reserved by created_at ascending, regardless of input order', () => {
      const p1 = profile('zzz', '나중', 200)
      const p2 = profile('aaa', '먼저', 100)
      const a = mergeTreeNodes([p1, p2], []).map(n => n.id)
      const b = mergeTreeNodes([p2, p1], []).map(n => n.id)
      expect(a).toEqual(['aaa', 'zzz'])
      expect(b).toEqual(a)
    })

    it('on created_at ties the id tiebreaker (deterministic)', () => {
      const out = mergeTreeNodes(
        [profile('b', '', 50), profile('a', '', 50)],
        [],
      ).map(n => n.id)
      expect(out).toEqual(['a', 'b'])
    })

    it('running by id ascending (AgentInfo has no created_at), regardless of input order', () => {
      const a = mergeTreeNodes([], [agent('b'), agent('a')]).map(n => n.id)
      const b = mergeTreeNodes([], [agent('a'), agent('b')]).map(n => n.id)
      expect(a).toEqual(['a', 'b'])
      expect(b).toEqual(a)
    })
  })

  // ── MINOR-3 / MAJOR-1: exited session + same-id profile coexist (pinning current behavior) ──
  // Known limitation (MAJOR-1): the reap/restart of ADR-0016 is unimplemented, so an exited session (Exited/Killed/Failed) stays in agents[] (T-4); even with a same-id profile the merge keeps that id running-kind and does not revert it to reserved → an exited empty shell cannot be reactivated by double-click. Pinned below as a regression guard (no frontend workaround).
  describe('exited session + same-named profile coexist (pins current behavior, MAJOR-1 known limitation)', () => {
    const terminalStatuses: AgentInfo['status'][] = [
      { type: 'Exited', code: 0 },
      { type: 'Killed' },
      { type: 'Failed', message: 'boom' },
    ]

    for (const status of terminalStatuses) {
      it(`an agent with status=${status.type} and a profile with the same id → kept as 1 running (does not return to reserved)`, () => {
        const out = mergeTreeNodes(
          [profile('x', '깡통이름')],
          [agent('x', '세션이름', true, status)],
        )
        expect(out.filter(n => n.id === 'x')).toHaveLength(1)
        const node = out.find(n => n.id === 'x')!
        expect(node.kind).toBe('running')
        expect(node.status).toBe(status.type)
        expect(node.name).toBe('세션이름')
      })
    }
  })

  // ── ADR-0072: parent_id hierarchy (1-level nesting) ──
  //   A forest plugging children into the parent's children by parent_id; only 1 level (a child's children is always empty). The backend enforces cycle/2-level, but the frontend defensively promotes to root on out-of-step data.
  describe('hierarchy (parent_id, ADR-0072)', () => {
    it('without parent_id all are roots (children empty array)', () => {
      const out = mergeTreeNodes([profile('p1', 'A'), profile('p2', 'B')], [])
      expect(out).toHaveLength(2)
      expect(out.every(n => n.children.length === 0)).toBe(true)
    })

    it('children nest as the parent\'s children (A > B·C) — roots are parents only', () => {
      const out = mergeTreeNodes(
        [
          profile('A', '부모', 1),
          profile('B', '자식B', 2, null, 'A'),
          profile('C', '자식C', 3, null, 'A'),
        ],
        [],
      )
      expect(out.map(n => n.id)).toEqual(['A'])
      expect(out[0].children.map(n => n.id)).toEqual(['B', 'C'])
    })

    it('running parent + reserved child mixed (merge hierarchy kept, ADR-0018 ⊕ ADR-0072)', () => {
      // parent_id comes only from profiles, so the child profile points at A.
      const out = mergeTreeNodes(
        [profile('A', '', 1, null, null), profile('B', '', 2, null, 'A')],
        [agent('A', '실행부모')],
      )
      expect(out.map(n => n.id)).toEqual(['A'])
      expect(out[0].kind).toBe('running')
      expect(out[0].children.map(n => n.id)).toEqual(['B'])
      expect(out[0].children[0].kind).toBe('reserved')
    })

    it('a running child also inherits the matching profile\'s parent_id and nests (AgentInfo has no parent_id)', () => {
      const out = mergeTreeNodes(
        [profile('A', '', 1), profile('B', '', 2, null, 'A')],
        [agent('A'), agent('B')],
      )
      expect(out.map(n => n.id)).toEqual(['A'])
      expect(out[0].children.map(n => n.id)).toEqual(['B'])
      expect(out[0].children[0].kind).toBe('running')
    })

    it('a nonexistent parent_id → the child is promoted to root (orphan defense)', () => {
      const out = mergeTreeNodes([profile('B', '', 1, null, 'GHOST')], [])
      expect(out.map(n => n.id)).toEqual(['B'])
      expect(out[0].children).toHaveLength(0)
    })

    it('self-parent → promoted to root (cycle defense)', () => {
      const out = mergeTreeNodes([profile('B', '', 1, null, 'B')], [])
      expect(out.map(n => n.id)).toEqual(['B'])
      expect(out[0].children).toHaveLength(0)
    })

    it('two-level prevention (A>B>C) — if B is already a child, C does not attach under B and is promoted to root', () => {
      const out = mergeTreeNodes(
        [
          profile('A', '', 1),
          profile('B', '', 2, null, 'A'),
          profile('C', '', 3, null, 'B'),
        ],
        [],
      )
      expect(out.map(n => n.id).sort()).toEqual(['A', 'C'])
      const a = out.find(n => n.id === 'A')!
      expect(a.children.map(n => n.id)).toEqual(['B'])
      expect(a.children[0].children).toHaveLength(0)
    })

    it('child ordering is also deterministic (created_at ascending → id tiebreaker), regardless of input order', () => {
      const cx = profile('x', '', 30, null, 'A')
      const cy = profile('y', '', 20, null, 'A')
      const cz = profile('z', '', 10, null, 'A')
      const parent = profile('A', '', 1)
      const forward = mergeTreeNodes([parent, cx, cy, cz], [])[0].children.map(n => n.id)
      const reversed = mergeTreeNodes([parent, cz, cy, cx], [])[0].children.map(n => n.id)
      expect(forward).toEqual(['z', 'y', 'x'])
      expect(reversed).toEqual(forward)
    })
  })
})

// ADR-0148: the tree's "running vs reserved" decision by a single id; the slot render gate uses the same function — a divergence would put an exited agent's slot presentation out of step with the tree.
describe('agentPresence', () => {
  it('in the roster = running — even in a terminal state the session still exists so it is running', () => {
    const dead = agent('A', '', true, { type: 'Killed' })
    expect(agentPresence('A', [dead], [])).toBe('running')
  })

  it('not in the roster but only a profile = reserved (= the look after reaping on exit)', () => {
    expect(agentPresence('A', [], [profile('A')])).toBe('reserved')
  })

  it('neither = unknown', () => {
    expect(agentPresence('A', [agent('B')], [profile('C')])).toBe('unknown')
  })

  it('it is the same verdict as the tree\'s reserved node set (they do not diverge on the same input)', () => {
    const agents = [agent('run')]
    const profiles = [profile('run'), profile('dead')]
    const reservedInTree = mergeTreeNodes(profiles, agents)
      .filter(n => n.kind === 'reserved')
      .map(n => n.id)
    const reservedByRule = [...profiles, ...agents]
      .map(x => x.id)
      .filter(id => agentPresence(id, agents, profiles) === 'reserved')
    expect(reservedInTree).toEqual(['dead'])
    expect([...new Set(reservedByRule)]).toEqual(reservedInTree)
  })
})

// ── last failure axis (ADR-0172) ──
// An axis absent from the AgentInfo wire, so a running node must inherit it from the matching profile — else the screen cannot decide "came up again but before the first conversation".
describe('mergeTreeNodes — lastFailure', () => {
  it('a reserved node carries the profile\'s value as is', () => {
    const out = mergeTreeNodes([profile('p1', '', 0, null, null, 'NoConversationToResume')], [])
    expect(out[0].lastFailure).toBe('NoConversationToResume')
  })

  it('a running node also inherits from the matching profile (the running + last failure combination must be expressible)', () => {
    const out = mergeTreeNodes(
      [profile('a', '', 0, null, null, 'SpawnFailed')],
      [agent('a', '코더')],
    )
    expect(out[0]).toMatchObject({ kind: 'running', lastFailure: 'SpawnFailed' })
  })

  it('an ad-hoc running node without a profile is null', () => {
    const out = mergeTreeNodes([], [agent('adhoc')])
    expect(out[0].lastFailure).toBeNull()
  })

  it('without a record it is null (existing node shape unchanged)', () => {
    const out = mergeTreeNodes([profile('p1')], [])
    expect(out[0].lastFailure).toBeNull()
  })
})
