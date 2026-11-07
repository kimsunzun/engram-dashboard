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
  it('빈 입력 → 빈 배열', () => {
    expect(mergeTreeNodes([], [])).toEqual([])
  })

  it('실행중만(프로필 없음) → 전부 running 노드, ad-hoc 셸 그대로 표시', () => {
    const out = mergeTreeNodes([], [agent('a', '코더'), agent('b')])
    expect(out).toHaveLength(2)
    expect(out[0]).toMatchObject({ id: 'a', name: '코더', kind: 'running', status: 'Running' })
    expect(out[1]).toMatchObject({ id: 'b', kind: 'running' })
  })

  it('예약만(실행중 없음) → 전부 reserved 노드(status=Reserved, canInterrupt=false)', () => {
    const out = mergeTreeNodes([profile('p1', '예약1'), profile('p2', '예약2')], [])
    expect(out).toHaveLength(2)
    expect(out[0]).toMatchObject({ id: 'p1', name: '예약1', kind: 'reserved', status: 'Reserved', canInterrupt: false })
    expect(out[1]).toMatchObject({ id: 'p2', kind: 'reserved' })
  })

  it('둘 다(겹치지 않음) → running 먼저, reserved 뒤', () => {
    const out = mergeTreeNodes([profile('p1', '예약')], [agent('a', '실행')])
    expect(out.map(n => n.id)).toEqual(['a', 'p1'])
    expect(out[0].kind).toBe('running')
    expect(out[1].kind).toBe('reserved')
  })

  it('중복 id(프로필이 spawn되어 실행중) → 실행중 우선, 예약 흡수(중복 없음)', () => {
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
  it('display_name override → reserved 노드는 프로필 직접, running 노드는 매칭 프로필에서 이어받음', () => {
    const out = mergeTreeNodes(
      [profile('p-run', '', 0, '실행override'), profile('p-res', '', 0, '예약override')],
      [agent('p-run'), agent('adhoc')],
    )
    expect(out.find(n => n.id === 'p-run')?.displayName).toBe('실행override')
    expect(out.find(n => n.id === 'adhoc')?.displayName).toBeNull()
    expect(out.find(n => n.id === 'p-res')?.displayName).toBe('예약override')
  })

  it('display_name 없으면 노드 displayName=null(basename 파생, 기존 동작 불변)', () => {
    const out = mergeTreeNodes([profile('p1', '예약1')], [agent('a', '실행')])
    expect(out.find(n => n.id === 'p1')?.displayName).toBeNull()
    expect(out.find(n => n.id === 'a')?.displayName).toBeNull()
  })

  // ── ADR-0072 drop guard: hasProfile (whether a profile exists) ─────────────────────────────────
  //   reparent holds only when both child and parent have a real profile (the backend rejects a no-profile op).
  //   hasProfile is the basis field of the frontend drag/drop pre-filter.
  describe('hasProfile(드롭 가드, ADR-0072)', () => {
    it('reserved 노드 → hasProfile=true(프로필에서 생성)', () => {
      const out = mergeTreeNodes([profile('p1', '예약')], [])
      expect(out.find(n => n.id === 'p1')?.hasProfile).toBe(true)
    })

    it('매칭 프로필 있는 running 노드 → hasProfile=true', () => {
      const out = mergeTreeNodes([profile('a', '', 1)], [agent('a')])
      expect(out.find(n => n.id === 'a')?.hasProfile).toBe(true)
    })

    it('프로필 없는 ad-hoc running 노드(SpawnByCwd) → hasProfile=false', () => {
      const out = mergeTreeNodes([], [agent('adhoc')])
      expect(out.find(n => n.id === 'adhoc')?.hasProfile).toBe(false)
    })

    it('혼합: ad-hoc(false) + 매칭 running(true) + reserved(true) 동시 판정', () => {
      const out = mergeTreeNodes(
        [profile('run', '', 1), profile('res', '', 2)],
        [agent('run'), agent('adhoc')],
      )
      expect(out.find(n => n.id === 'run')?.hasProfile).toBe(true)
      expect(out.find(n => n.id === 'adhoc')?.hasProfile).toBe(false)
      expect(out.find(n => n.id === 'res')?.hasProfile).toBe(true)
    })
  })

  // ── MINOR-2: deterministic sort regression guard ──────────────────────────────────────────
  // The backend listProfiles/agents can arrive in HashMap iteration (non-deterministic) order,
  // so without sorting, nodes jump on every refetch. Guarantees "the same input set always gives the same order".
  describe('결정적 정렬(MINOR-2)', () => {
    it('reserved 는 created_at 오름차순, 입력 순서와 무관', () => {
      const p1 = profile('zzz', '나중', 200)
      const p2 = profile('aaa', '먼저', 100)
      const a = mergeTreeNodes([p1, p2], []).map(n => n.id)
      const b = mergeTreeNodes([p2, p1], []).map(n => n.id)
      expect(a).toEqual(['aaa', 'zzz'])
      expect(b).toEqual(a)
    })

    it('created_at 동률이면 id tiebreaker(결정적)', () => {
      const out = mergeTreeNodes(
        [profile('b', '', 50), profile('a', '', 50)],
        [],
      ).map(n => n.id)
      expect(out).toEqual(['a', 'b'])
    })

    it('running 은 id 오름차순(AgentInfo 엔 created_at 없음), 입력 순서와 무관', () => {
      const a = mergeTreeNodes([], [agent('b'), agent('a')]).map(n => n.id)
      const b = mergeTreeNodes([], [agent('a'), agent('b')]).map(n => n.id)
      expect(a).toEqual(['a', 'b'])
      expect(b).toEqual(a)
    })
  })

  // ── MINOR-3 / MAJOR-1: exited session + same-id profile coexist (pinning current behavior) ───────
  // Known limitation (MAJOR-1): the reap/restart of ADR-0016 is unimplemented (parked, awaiting a user decision), so
  // even when a session exits (Exited/Killed/Failed) it does not disappear from agents[] (T-4). So
  // even if a profile with the same id exists, the merge still classifies that id as running-kind and does not
  // revert it to reserved → an exited empty shell cannot be reactivated by double-click.
  // Below we pin that "current behavior" as a regression guard (not a behavior change — no frontend workaround).
  describe('종료 세션 + 동명 프로필 공존(현재 동작 박제, MAJOR-1 알려진 한계)', () => {
    const terminalStatuses: AgentInfo['status'][] = [
      { type: 'Exited', code: 0 },
      { type: 'Killed' },
      { type: 'Failed', message: 'boom' },
    ]

    for (const status of terminalStatuses) {
      it(`status=${status.type} 인 agent 와 동 id 프로필 → running 1개로 유지(reserved 로 안 돌아옴)`, () => {
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

  // ── ADR-0072: parent_id hierarchy (1-level nesting) ──────────────────────────────────────
  //   Returns a forest that plugs children into the parent's children by parent_id. Only 1 level (a child's children is always an empty array).
  //   The backend enforces cycle/2-level, but the frontend defensively promotes to root even if data is out of step (2-level forbidden).
  describe('계층화(parent_id, ADR-0072)', () => {
    it('parent_id 없으면 전부 루트(children 빈 배열)', () => {
      const out = mergeTreeNodes([profile('p1', 'A'), profile('p2', 'B')], [])
      expect(out).toHaveLength(2)
      expect(out.every(n => n.children.length === 0)).toBe(true)
    })

    it('자식이 부모 children 으로 중첩(A > B·C) — 루트는 부모만', () => {
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

    it('running 부모 + reserved 자식 혼합(머지 계층 유지, ADR-0018 ⊕ ADR-0072)', () => {
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

    it('running 자식도 매칭 프로필의 parent_id 를 이어받아 중첩(AgentInfo 엔 parent_id 없음)', () => {
      const out = mergeTreeNodes(
        [profile('A', '', 1), profile('B', '', 2, null, 'A')],
        [agent('A'), agent('B')],
      )
      expect(out.map(n => n.id)).toEqual(['A'])
      expect(out[0].children.map(n => n.id)).toEqual(['B'])
      expect(out[0].children[0].kind).toBe('running')
    })

    it('존재하지 않는 parent_id → 자식이 루트로 승격(고아 방어)', () => {
      const out = mergeTreeNodes([profile('B', '', 1, null, 'GHOST')], [])
      expect(out.map(n => n.id)).toEqual(['B'])
      expect(out[0].children).toHaveLength(0)
    })

    it('self-parent → 루트로 승격(cycle 방어)', () => {
      const out = mergeTreeNodes([profile('B', '', 1, null, 'B')], [])
      expect(out.map(n => n.id)).toEqual(['B'])
      expect(out[0].children).toHaveLength(0)
    })

    it('2단 방지(A>B>C) — B 가 이미 자식이면 C 는 B 밑에 안 붙고 루트로 승격', () => {
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

    it('자식 정렬도 결정적(created_at 오름차순 → id tiebreaker), 입력 순서 무관', () => {
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

// ADR-0148: the form that asks the tree's "running vs reserved" decision by a single id. The slot render gate uses the same function —
// if the rule diverges in two places, the slot presentation of an exited agent goes out of step with the tree.
describe('agentPresence', () => {
  it('명부에 있으면 running — terminal 상태여도 세션이 아직 있으므로 running 이다', () => {
    const dead = agent('A', '', true, { type: 'Killed' })
    expect(agentPresence('A', [dead], [])).toBe('running')
  })

  it('명부엔 없고 프로필만 있으면 reserved(= 종료 후 수거된 모습)', () => {
    expect(agentPresence('A', [], [profile('A')])).toBe('reserved')
  })

  it('둘 다 없으면 unknown', () => {
    expect(agentPresence('A', [agent('B')], [profile('C')])).toBe('unknown')
  })

  it('트리의 예약 노드 집합과 같은 판정이다(같은 입력에서 서로 어긋나지 않는다)', () => {
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

// ── last failure axis (ADR-0172) ──────────────────────────────────────────────────
// It is an axis absent from the AgentInfo wire, so a running node must also inherit it from the matching profile — if it does not,
// the screen has no material to decide the state "came up again but before the first conversation".
describe('mergeTreeNodes — lastFailure', () => {
  it('예약 노드는 프로필의 값을 그대로 싣는다', () => {
    const out = mergeTreeNodes([profile('p1', '', 0, null, null, 'NoConversationToResume')], [])
    expect(out[0].lastFailure).toBe('NoConversationToResume')
  })

  it('실행중 노드도 매칭 프로필에서 이어받는다(도는 중 + 마지막 실패 조합이 표현돼야 한다)', () => {
    const out = mergeTreeNodes(
      [profile('a', '', 0, null, null, 'SpawnFailed')],
      [agent('a', '코더')],
    )
    expect(out[0]).toMatchObject({ kind: 'running', lastFailure: 'SpawnFailed' })
  })

  it('프로필이 없는 ad-hoc 실행중 노드는 null', () => {
    const out = mergeTreeNodes([], [agent('adhoc')])
    expect(out[0].lastFailure).toBeNull()
  })

  it('기록이 없으면 null(기존 노드 모양 불변)', () => {
    const out = mergeTreeNodes([profile('p1')], [])
    expect(out[0].lastFailure).toBeNull()
  })
})
