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

// ── react-arborist Tree mock — onMove 캡처 ────────────────────────────────────────
//   실제 트리 렌더는 필요 없다. Tree 가 받은 onMove 를 밖에서 잡아 사람 드래그와 동일 인자로 호출한다.
type MoveArgs = Parameters<MoveHandler<unknown>>[0]
const captured = vi.hoisted(() => ({ onMove: undefined as ((a: MoveArgs) => void | Promise<void>) | undefined }))
vi.mock('react-arborist', () => ({
  Tree: (props: { onMove?: (a: MoveArgs) => void | Promise<void> }) => {
    captured.onMove = props.onMove
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

beforeEach(() => {
  clientMock.reparentProfile.mockClear()
  refreshProfilesMock.mockClear()
  captured.onMove = undefined
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
