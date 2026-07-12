// AgentList 드래그 재부모화(onMove → reparentProfile) 배선 테스트 (ADR-0072, §5).
//
// ★왜 별도 파일 + Tree mock★: 실제 드래그는 jsdom 에서 시뮬레이션이 어렵고, react-arborist <Tree> 를 통째
//   mock 하면 렌더 스모크 테스트(AgentList.test.tsx)와 충돌한다. 그래서 이 파일에서만 react-arborist 를
//   mock 해 <Tree> 가 받은 onMove 콜백을 캡처하고, 사람 드래그가 부르는 것과 동일 인자로 직접 호출해
//   컴포넌트의 재부모화 배선(reparentProfile 커맨드 형태 · null=루트 승격 · no-op 억제 · refreshProfiles
//   안전망)을 검증한다. 사람 드래그와 LLM 호출이 같은 reparentProfile 핸들을 쓴다(§5 손발/두뇌 분리).

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { MoveHandler } from 'react-arborist'

// ── clientFactory stub ──────────────────────────────────────────────────────────
const clientMock = vi.hoisted(() => ({
  spawnProfile: vi.fn(async () => ({ id: 'a' })),
  killAgent: vi.fn(async () => undefined),
  deleteProfile: vi.fn(async () => undefined),
  renameProfile: vi.fn(async () => undefined),
  reparentProfile: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    spawnProfile: (...a: unknown[]) => clientMock.spawnProfile(...(a as [])),
    killAgent: (...a: unknown[]) => clientMock.killAgent(...(a as [])),
    deleteProfile: (...a: unknown[]) => clientMock.deleteProfile(...(a as [])),
    renameProfile: (...a: unknown[]) => clientMock.renameProfile(...(a as [])),
    reparentProfile: (...a: unknown[]) => clientMock.reparentProfile(...(a as [])),
  },
  getAgentClient: vi.fn(),
}))
const refreshProfilesMock = vi.hoisted(() => vi.fn(async () => undefined))
vi.mock('../../store/eventBus', () => ({ refreshProfiles: refreshProfilesMock }))
vi.mock('../../store/viewStore', () => ({
  useViewStore: Object.assign(
    (sel: (s: unknown) => unknown) => sel({ assignAgent: vi.fn() }),
    { getState: () => ({ assignAgent: vi.fn() }) },
  ),
  currentViewId: () => 'main-view',
  selectView: () => ({ focusedSlotId: 'slot-1' }),
}))

// ── react-arborist Tree mock — onMove·disableDrag·disableDrop 캡처 ─────────────────
//   실제 트리 렌더는 필요 없다. Tree 가 받은 콜백들을 밖에서 잡아 사람 드래그와 동일 인자로 호출한다.
type MoveArgs = Parameters<MoveHandler<unknown>>[0]
// disableDrag = (data) => boolean · disableDrop = ({parentNode, dragNodes, index}) => boolean.
type DropArgs = { parentNode: unknown; dragNodes: unknown[]; index: number }
const captured = vi.hoisted(() => ({
  onMove: undefined as ((a: MoveArgs) => void | Promise<void>) | undefined,
  disableDrag: undefined as ((data: unknown) => boolean) | undefined,
  disableDrop: undefined as ((a: DropArgs) => boolean) | undefined,
}))
vi.mock('react-arborist', () => ({
  Tree: (props: {
    onMove?: (a: MoveArgs) => void | Promise<void>
    disableDrag?: (data: unknown) => boolean
    disableDrop?: (a: DropArgs) => boolean
  }) => {
    captured.onMove = props.onMove
    captured.disableDrag = props.disableDrag
    captured.disableDrop = props.disableDrop
    return null
  },
}))

import AgentList from './AgentList'
import { useAgentStore } from '../../store/agentStore'
import type { AgentProfile } from '../../api/types'

function profile(id: string, cwd: string, createdAt = 0, parentId: string | null = null): AgentProfile {
  return {
    id, name: '', display_name: null, parent_id: parentId,
    command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd, env: [], claude_session_id: null, old_session_ids: [], epoch: 0, auto_restore: false,
    restart_policy: 'Never', restart_count: 0, failed_reason: null, created_at: createdAt,
    last_active: 0, last_start_at: null,
  }
}

// onMove 인자 헬퍼(react-arborist MoveHandler 형태 — dragIds/parentId 만 컴포넌트가 소비).
function moveArgs(dragId: string, parentId: string | null): MoveArgs {
  return { dragIds: [dragId], dragNodes: [], parentId, parentNode: null, index: 0 } as unknown as MoveArgs
}

// ── NodeApi 최소 fake(disableDrop/disableDrag 가 소비하는 필드만) ────────────────────
//   disableDrop 이 읽는 것: parentNode.{isRoot, level, id, data.hasProfile} · dragNodes[i].{data.children,
//   parent}. parent 는 다시 NodeApi(루트면 isRoot=true). 실제 NodeApi 전부를 흉내 내지 않고 소비 필드만 채운다.
type FakeNode = {
  id: string
  isRoot: boolean
  level: number
  data: { hasProfile?: boolean; children?: unknown[] }
  parent: FakeNode | null
}
// 내부 루트 pseudo-node(react-arborist canDrop 이 루트 드롭 때 넘기는 것 — data={id:ROOT_ID}, level=-1).
const rootNode: FakeNode = { id: '__ROOT__', isRoot: true, level: -1, data: {}, parent: null }
// 실 노드. parent 기본 = 루트 노드(= 루트 레벨 노드). 자식이면 parent 를 명시.
function node(
  id: string,
  opts: { hasProfile?: boolean; children?: unknown[]; parent?: FakeNode; level?: number } = {},
): FakeNode {
  const parent = opts.parent ?? rootNode
  return {
    id,
    isRoot: false,
    level: opts.level ?? (parent.isRoot ? 0 : parent.level + 1),
    data: { hasProfile: opts.hasProfile ?? true, children: opts.children ?? [] },
    parent,
  }
}
function callDrop(parentNode: FakeNode, dragNodes: FakeNode[]): boolean {
  return captured.disableDrop!({ parentNode: parentNode as unknown, dragNodes, index: 0 })
}

beforeEach(() => {
  clientMock.reparentProfile.mockClear()
  refreshProfilesMock.mockClear()
  captured.onMove = undefined
  captured.disableDrag = undefined
  captured.disableDrop = undefined
  useAgentStore.setState({ agents: [], profiles: [], presets: [], selectedAgentId: null })
})
afterEach(() => {
  cleanup()
  useAgentStore.setState({ agents: [], profiles: [], presets: [], selectedAgentId: null })
})

describe('드래그 재부모화 배선(onMove → reparentProfile, ADR-0072 §5)', () => {
  it('자식을 부모 밑으로 드롭 → reparentProfile(childId, parentId)', async () => {
    // A 루트, B 루트(부모 없음). B 를 A 밑으로 드래그.
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2)] })
    render(<AgentList />)
    expect(captured.onMove).toBeTypeOf('function')
    await act(async () => {
      captured.onMove!(moveArgs('B', 'A'))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).toHaveBeenCalledWith('B', 'A')
    // 성공 시 refreshProfiles 안전망(rename 과 동형 — 낙관 갱신 X).
    expect(refreshProfilesMock).toHaveBeenCalledTimes(1)
  })

  it('루트로 드롭(parentId=null) → reparentProfile(childId, null) 루트 승격', async () => {
    // B 가 A 의 자식 상태 → 루트로 뺀다(parentId null).
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, 'A')] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('B', null))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).toHaveBeenCalledWith('B', null)
  })

  it('이미 그 부모면 no-op(불필요 command 억제)', async () => {
    // B 가 이미 A 의 자식 → A 밑으로 다시 드롭해도 발화 안 함.
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, 'A')] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('B', 'A'))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).not.toHaveBeenCalled()
  })

  it('이미 루트인 노드를 루트로 드롭 → no-op', async () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('A', null))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).not.toHaveBeenCalled()
  })
})

// ── 드래그/드롭 UI 가드(disableDrag / disableDrop, ADR-0072 폴리시) ────────────────────
//   백엔드가 무효 op 를 권위로 거부하지만(데이터 손상 없음), 프론트 가드는 흔한 오조작에서 불필요한 실패
//   커맨드·에러 토스트·혼란을 pre-filter 한다. Tree mock 이 캡처한 콜백을 NodeApi fake 로 직접 호출해 검증.
describe('드래그 가드(disableDrag = 프로필 없는 노드, #1)', () => {
  it('프로필 있는 노드(reserved·매칭 running)는 드래그 가능(disableDrag=false)', () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    expect(captured.disableDrag).toBeTypeOf('function')
    // disableDrag 는 node.data(AgentTreeNode) 를 받는다 — hasProfile=true → 드래그 허용.
    expect(captured.disableDrag!({ hasProfile: true })).toBe(false)
  })

  it('프로필 없는 ad-hoc 노드는 드래그 불가(disableDrag=true)', () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    // ad-hoc(SpawnByCwd) = hasProfile:false → 드래그 차단(reparent 대상 프로필 부재).
    expect(captured.disableDrag!({ hasProfile: false })).toBe(true)
  })
})

describe('드롭 가드(disableDrop, #1·#5 + 기존 1단 상한)', () => {
  function mountTree() {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    expect(captured.disableDrop).toBeTypeOf('function')
  }

  it('[#1] 프로필 없는 부모(ad-hoc) 밑으로 드롭 → 비활성', () => {
    mountTree()
    const adhocParent = node('adhoc', { hasProfile: false })
    const drag = node('B') // 루트 레벨 자식(현재 부모 = 루트) → 부모가 바뀌므로 #5 에 안 걸림.
    expect(callDrop(adhocParent, [drag])).toBe(true)
  })

  it('[#1] 프로필 있는 부모(루트 레벨) 밑으로 드롭 → 허용(부모가 실제로 바뀔 때)', () => {
    mountTree()
    const realParent = node('A', { hasProfile: true })
    const drag = node('B') // 현재 루트 → A 밑으로 = 부모 변경 → 허용.
    expect(callDrop(realParent, [drag])).toBe(false)
  })

  it('[#5] 같은 부모(A 자식 → 다시 A 밑)로의 드롭 → 비활성(reorder 미지원)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const child = node('B', { parent: parentA }) // 이미 A 의 자식.
    expect(callDrop(parentA, [child])).toBe(true)
  })

  it('[#5] 루트↔루트 드롭(현재 루트 노드를 루트로) → 비활성(부모 안 바뀜)', () => {
    mountTree()
    const drag = node('B') // parent = 루트.
    // 루트 드롭 = parentNode 가 내부 루트 pseudo-node(isRoot). currentParentId(null) == dropParentId(null).
    expect(callDrop(rootNode, [drag])).toBe(true)
  })

  it('[기존] 자식 밑(부모 level>0)으로 드롭 → 비활성(2단 방지)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true }) // 루트 레벨(level 0)
    const childC = node('C', { parent: parentA, hasProfile: true }) // level 1 = 자식.
    const drag = node('B') // 루트 → C 밑으로 = 2단.
    expect(callDrop(childC, [drag])).toBe(true)
  })

  it('[기존] 이미 자식을 가진 노드를 남의 밑으로 드롭 → 비활성(2단 방지)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const dragWithChildren = node('P', { children: [{ id: 'x' }] }) // 자식 보유 노드.
    expect(callDrop(parentA, [dragWithChildren])).toBe(true)
  })

  it('루트 자식(현재 루트)을 프로필 있는 부모 밑으로 → 허용(정상 재부모화)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const drag = node('B') // 현재 루트, 자식 없음, 부모 변경.
    expect(callDrop(parentA, [drag])).toBe(false)
  })

  // ★isRoot 정규화 회귀 가드★: 위 '루트↔루트' 테스트는 현재-루트 노드를 루트로 드롭해 true(차단) 기대라,
  //   parentNode.isRoot 정규화가 깨져도 pseudo-root 의 data.hasProfile 부재(②)가 독립적으로 차단해 통과해버린다.
  //   여기서는 자식(level 1)을 루트로 '승격'하는 합법 드롭 — isRootDrop 정규화가 dropParentId 를 null 로 잡고
  //   currentParentId('A')와 달라 허용(false)이어야 한다. isRoot 정규화가 깨지면(예: pseudo-root 를 실 부모로
  //   취급) ②(hasProfile 부재)나 dropParentId 오판으로 true 가 되어 이 테스트가 실패한다 = 진짜 회귀 가드.
  it('[승격] A 자식 B(level 1)를 루트로 드롭 → 허용(disableDrop=false)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true }) // 루트 레벨(isRoot=false, id='A').
    const childB = node('B', { parent: parentA, hasProfile: true }) // level 1 자식, leaf.
    // rootNode = 내부 루트 pseudo-node(isRoot=true) — 루트 승격 드롭.
    expect(callDrop(rootNode, [childB])).toBe(false)
  })

  // 자식 B 를 다른 실 부모 C 로 이동 — currentParentId('A') ≠ dropParentId('C'), C 는 level 0·hasProfile=true 라
  //   다른 차단조건(①②)에도 안 걸려 허용(false). 부모-변경 이동의 허용측 회귀 가드(same-parent no-op 대칭).
  it('[이동] A 자식 B 를 다른 부모 C 밑으로 드롭 → 허용(disableDrop=false)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const childB = node('B', { parent: parentA, hasProfile: true }) // 현재 부모 = A.
    const parentC = node('C', { hasProfile: true }) // 다른 루트 레벨 부모(level 0, leaf).
    expect(callDrop(parentC, [childB])).toBe(false)
  })
})
