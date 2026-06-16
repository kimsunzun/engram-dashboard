// mergeTreeNodes 단위테스트 (ADR-0018) — 빈 프로필 / 실행중만 / 둘 다 / 중복 id.

import { describe, expect, it } from 'vitest'

import type { AgentInfo, AgentProfile, Capabilities } from '../../api/types'
import { mergeTreeNodes } from './mergeTreeNodes'

const caps = (interrupt: boolean): Capabilities => ({
  input: { raw: true, message: false, attachment: false },
  output: { terminal_bytes: true, markdown: false, tool_events: false, usage: false },
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

function profile(id: string, name = '', createdAt = 0): AgentProfile {
  return {
    id,
    name,
    command: { kind: 'Claude', extra_args: [] },
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
    // name 비면 id 앞 8자
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
    // same 은 1개만, running 으로
    expect(out.filter(n => n.id === 'same')).toHaveLength(1)
    const same = out.find(n => n.id === 'same')!
    expect(same.kind).toBe('running')
    expect(same.name).toBe('실행중이름')
    // 안 겹친 p2 는 예약으로 남음
    expect(out.find(n => n.id === 'p2')).toMatchObject({ kind: 'reserved' })
  })

  // ── MINOR-2: 결정적 정렬 회귀 가드 ──────────────────────────────────────────
  // 백엔드 listProfiles/agents 가 HashMap iteration(비결정적) 순서로 올 수 있어,
  // 정렬이 없으면 refetch 마다 노드가 튄다. "같은 입력 집합이면 항상 같은 순서" 보장.
  describe('결정적 정렬(MINOR-2)', () => {
    it('reserved 는 created_at 오름차순, 입력 순서와 무관', () => {
      const p1 = profile('zzz', '나중', 200)
      const p2 = profile('aaa', '먼저', 100)
      // 입력 순서를 뒤집어도 created_at 기준 동일 결과여야 한다.
      const a = mergeTreeNodes([p1, p2], []).map(n => n.id)
      const b = mergeTreeNodes([p2, p1], []).map(n => n.id)
      expect(a).toEqual(['aaa', 'zzz']) // created_at 100 < 200
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
        // 같은 id 는 정확히 1개, 여전히 running-kind (예약으로 강등되지 않음).
        expect(out.filter(n => n.id === 'x')).toHaveLength(1)
        const node = out.find(n => n.id === 'x')!
        expect(node.kind).toBe('running')
        // status 문자열은 terminal type 을 그대로 노출(Reserved 아님).
        expect(node.status).toBe(status.type)
        expect(node.name).toBe('세션이름')
      })
    }
  })
})
