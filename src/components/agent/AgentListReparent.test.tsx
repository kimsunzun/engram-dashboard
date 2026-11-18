// ★Why a separate file + Tree mock★: real dragging is hard to simulate in jsdom, and mocking the whole react-arborist <Tree> conflicts with the render smoke test (AgentList.test.tsx). Here react-arborist is mocked to capture the `onMove` the <Tree> received and call it with a human drag's arguments, verifying the reparent wiring (reparentProfile command shape · null=root promotion · no-op suppression · refreshProfiles safety net). A human drag and an LLM call use the same reparentProfile handle (§5).

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

// ── react-arborist Tree mock — captures onMove·disableDrag·disableDrop ──
//   No real tree render needed: the callbacks the <Tree> received are caught outside and called with a human drag's arguments.
type MoveArgs = Parameters<MoveHandler<unknown>>[0]
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
    cwd, env: [], backend_session_id: null, old_session_ids: [], epoch: 0, auto_restore: false,
    restart_policy: 'Never', restart_count: 0, failed_reason: null, last_failure: null,
    created_at: createdAt,
    last_active: 0, last_start_at: null,
  }
}

// The component consumes only dragIds/parentId (the other MoveHandler fields are dummies).
function moveArgs(dragId: string, parentId: string | null): MoveArgs {
  return { dragIds: [dragId], dragNodes: [], parentId, parentNode: null, index: 0 } as unknown as MoveArgs
}

// ── NodeApi minimal fake (only the fields disableDrop/disableDrag consume) ──
//   disableDrop reads: parentNode.{isRoot, level, id, data.hasProfile} · dragNodes[i].{data.children, parent}; parent is again a NodeApi (isRoot=true if root).
type FakeNode = {
  id: string
  isRoot: boolean
  level: number
  data: { hasProfile?: boolean; children?: unknown[] }
  parent: FakeNode | null
}
// Internal root pseudo-node (what react-arborist canDrop passes on a root drop — data={id:ROOT_ID}, level=-1).
const rootNode: FakeNode = { id: '__ROOT__', isRoot: true, level: -1, data: {}, parent: null }
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
  useAgentStore.setState({
    agents: [], agentsLoaded: true, profiles: [], profilesLoaded: true, presets: [], selectedAgentId: null,
  })
})
afterEach(() => {
  cleanup()
  useAgentStore.setState({ agents: [], profiles: [], presets: [], selectedAgentId: null })
})

describe('drag reparent wiring (onMove → reparentProfile, ADR-0072 §5)', () => {
  it('drop a child under a parent → reparentProfile(childId, parentId)', async () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2)] })
    render(<AgentList />)
    expect(captured.onMove).toBeTypeOf('function')
    await act(async () => {
      captured.onMove!(moveArgs('B', 'A'))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).toHaveBeenCalledWith('B', 'A')
    expect(refreshProfilesMock).toHaveBeenCalledTimes(1)
  })

  it('drop to root (parentId=null) → reparentProfile(childId, null) root promotion', async () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, 'A')] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('B', null))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).toHaveBeenCalledWith('B', null)
  })

  it('if it is already that parent it is a no-op (suppresses a needless command)', async () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1), profile('B', 'C:/b', 2, 'A')] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('B', 'A'))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).not.toHaveBeenCalled()
  })

  it('dropping a node that is already root onto root → no-op', async () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    await act(async () => {
      captured.onMove!(moveArgs('A', null))
      await Promise.resolve()
    })
    expect(clientMock.reparentProfile).not.toHaveBeenCalled()
  })
})

// ── Drag/drop UI guards (disableDrag / disableDrop, ADR-0072 polish) ──
//   The backend authoritatively rejects invalid ops; the frontend guard pre-filters needless failed commands, error toasts and confusion from common mis-operations.
describe('drag guard (disableDrag = a node without a profile, #1)', () => {
  it('a node with a profile (reserved · matching running) is draggable (disableDrag=false)', () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    expect(captured.disableDrag).toBeTypeOf('function')
    expect(captured.disableDrag!({ hasProfile: true })).toBe(false)
  })

  it('an ad-hoc node without a profile is not draggable (disableDrag=true)', () => {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    expect(captured.disableDrag!({ hasProfile: false })).toBe(true)
  })
})

describe('drop guard (disableDrop, #1·#5 + the existing one-level cap)', () => {
  function mountTree() {
    useAgentStore.setState({ profiles: [profile('A', 'C:/a', 1)] })
    render(<AgentList />)
    expect(captured.disableDrop).toBeTypeOf('function')
  }

  it('[#1] drop under a parent without a profile (ad-hoc) → disabled', () => {
    mountTree()
    const adhocParent = node('adhoc', { hasProfile: false })
    const drag = node('B') // a root-level child (current parent = root) → the parent changes so #5 does not catch it.
    expect(callDrop(adhocParent, [drag])).toBe(true)
  })

  it('[#1] drop under a parent with a profile (root level) → allowed (when the parent actually changes)', () => {
    mountTree()
    const realParent = node('A', { hasProfile: true })
    const drag = node('B')
    expect(callDrop(realParent, [drag])).toBe(false)
  })

  it('[#5] drop onto the same parent (A\'s child → under A again) → disabled (reorder unsupported)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const child = node('B', { parent: parentA })
    expect(callDrop(parentA, [child])).toBe(true)
  })

  it('[#5] root↔root drop (dropping a current root node onto root) → disabled (the parent does not change)', () => {
    mountTree()
    const drag = node('B')
    expect(callDrop(rootNode, [drag])).toBe(true)
  })

  it('[existing] drop under a child (parent level>0) → disabled (prevents two levels)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const childC = node('C', { parent: parentA, hasProfile: true })
    const drag = node('B')
    expect(callDrop(childC, [drag])).toBe(true)
  })

  it('[existing] dropping a node that already has children under another → disabled (prevents two levels)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const dragWithChildren = node('P', { children: [{ id: 'x' }] })
    expect(callDrop(parentA, [dragWithChildren])).toBe(true)
  })

  it('a root child (currently root) under a parent with a profile → allowed (normal reparent)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const drag = node('B')
    expect(callDrop(parentA, [drag])).toBe(false)
  })

  // ★isRoot normalization regression guard★: the 'root↔root' test above drops a current-root node onto root expecting true (blocked), so even if the parentNode.isRoot normalization breaks, the pseudo-root's missing data.hasProfile (②) blocks independently and it passes anyway. Here a legal drop 'promoting' a child (level 1) to root must be allowed (false): isRootDrop takes dropParentId as null, differing from currentParentId('A'). If the normalization breaks (e.g. the pseudo-root treated as a real parent) it turns true via ② or a dropParentId misjudgment and this test fails.
  it('[promote] drop child B of A (level 1) onto root → allowed (disableDrop=false)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const childB = node('B', { parent: parentA, hasProfile: true })
    expect(callDrop(rootNode, [childB])).toBe(false)
  })

  // A regression guard on the allowing side of a parent-changing move (symmetric with same-parent no-op).
  it('[move] drop child B of A under another parent C → allowed (disableDrop=false)', () => {
    mountTree()
    const parentA = node('A', { hasProfile: true })
    const childB = node('B', { parent: parentA, hasProfile: true })
    const parentC = node('C', { hasProfile: true })
    expect(callDrop(parentC, [childB])).toBe(false)
  })
})
