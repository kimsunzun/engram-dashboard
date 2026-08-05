import { describe, expect, it } from 'vitest'

import type { AgentInfo, AgentProfile, Capabilities } from '../../api/types'
import { mergeTreeNodes } from './mergeTreeNodes'

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
): AgentProfile {
  return {
    id,
    name,
    display_name: displayName,
    parent_id: parentId,
    command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd: 'C:/x',
    env: [],
    claude_session_id: null,
    old_session_ids: [],
    epoch: 0,
    auto_restore: false,
    restart_policy: 'Never',
    restart_count: 0,
    failed_reason: null,
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

  // ── ADR-0061 리치화: display_name override 전파 ──────────────────────────────
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

  // ── ADR-0072 드롭 가드: hasProfile(프로필 유무) ─────────────────────────────────
  //   reparent 는 child·parent 둘 다 실 프로필이 있어야 성립(백엔드가 no-profile op 를 거부).
  //   hasProfile 은 프론트 드래그/드롭 pre-filter 의 근거 필드다.
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

  // ── MINOR-2: 결정적 정렬 회귀 가드 ──────────────────────────────────────────
  // 백엔드 listProfiles/agents 가 HashMap iteration(비결정적) 순서로 올 수 있어,
  // 정렬이 없으면 refetch 마다 노드가 튄다. "같은 입력 집합이면 항상 같은 순서" 보장.
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

  // ── MINOR-3 / MAJOR-1: 종료 세션 + 같은 id 프로필 공존(현재 동작 박제) ───────
  // 알려진 한계(MAJOR-1): ADR-0016 의 reap/재시작이 미구현(파킹, 사용자 결정 대기)이라,
  // 세션이 종료(Exited/Killed/Failed)되어도 agents[] 에서 사라지지 않는다(T-4). 그래서
  // 같은 id 의 프로필이 있어도 머지는 그 id 를 여전히 running-kind 로 분류하고 reserved 로
  // 되돌리지 않는다 → 종료된 깡통을 더블클릭으로 재활성화할 수 없다.
  // 아래는 그 "현재 동작"을 회귀 가드로 박제한다(동작 변경 아님 — 프론트 우회 금지).
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

  // ── ADR-0072: parent_id 계층화(1단 중첩) ──────────────────────────────────────
  //   parent_id 로 자식을 부모 children 에 꽂는 forest 반환. 1단만(자식의 children 은 항상 빈 배열).
  //   백엔드가 cycle/2단을 강제하지만 프론트는 데이터가 어긋나도 방어적으로 루트 승격(2단 금지).
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
      // parent_id 는 프로필에서만 오므로 자식 프로필이 A 를 가리킨다.
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
