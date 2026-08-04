
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'new-agent' })),
  renameProfile: vi.fn(async () => undefined),
  createClaudeProfile: vi.fn(async () => ({ id: 'new-profile' })),
  // refreshProfiles(eventBus) 가 부르는 listProfiles — 생성 직후 store/tree 반영 검증용. 기본 []
  //   (테스트별로 mockResolvedValueOnce 로 생성 프로필을 실어 반환).
  listProfiles: vi.fn(async () => [] as unknown[]),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: {
    spawnAgent: (...args: unknown[]) => clientMock.spawnAgent(...(args as [])),
    renameProfile: (...args: unknown[]) => clientMock.renameProfile(...(args as [])),
    createClaudeProfile: (...args: unknown[]) => clientMock.createClaudeProfile(...(args as [])),
    listProfiles: (...args: unknown[]) => clientMock.listProfiles(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))

// agentlist.createAgent 는 폴더 다이얼로그(open)로 cwd 를 고른다 — 픽/취소를 테스트별로 제어.
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

import './agentCommands' // side-effect register
import { run } from './registry'
import { buildSlotMenu } from './slotMenu'
import { useAgentStore } from '../store/agentStore'

beforeEach(() => {
  clientMock.spawnAgent.mockClear()
  clientMock.renameProfile.mockClear()
  clientMock.createClaudeProfile.mockClear()
  clientMock.listProfiles.mockClear()
  dialogMock.open.mockReset()
  useAgentStore.setState({ presets: [], profiles: [] })
})
afterEach(() => {
  useAgentStore.setState({ presets: [], profiles: [] })
})

describe('agent.spawn 라우팅', () => {
  it('preset(id) → store.presets 에서 cwd 해소 → spawnAgent(cwd)', () => {
    useAgentStore.setState({ presets: [{ id: 'pr1', cwd: 'C:/work/engram', name: null }] })
    run('agent.spawn', { preset: 'pr1' })
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/work/engram')
  })

  it('raw cwd → spawnAgent(trim 된 cwd)', () => {
    run('agent.spawn', { cwd: '  C:/new/path  ' })
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/new/path')
  })

  it('parent 세팅 → throw(중첩 미지원)', () => {
    expect(() => run('agent.spawn', { cwd: 'C:/x', parent: 'p1' })).toThrow(/parent nesting 미지원/)
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('빈/공백 cwd → throw', () => {
    expect(() => run('agent.spawn', { cwd: '   ' })).toThrow()
    expect(() => run('agent.spawn', {})).toThrow()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('없는 preset id → throw(조용한 no-op 금지)', () => {
    useAgentStore.setState({ presets: [{ id: 'pr1', cwd: 'C:/work', name: null }] })
    expect(() => run('agent.spawn', { preset: 'nope' })).toThrow(/알 수 없는 preset/)
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('preset 이 cwd 보다 우선(둘 다 주면 preset 해소값 사용)', () => {
    useAgentStore.setState({ presets: [{ id: 'pr1', cwd: 'C:/from/preset', name: null }] })
    run('agent.spawn', { preset: 'pr1', cwd: 'C:/ignored' })
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/from/preset')
  })
})

// ── agent.rename(§5 — ADR-0061) ──────────────────────────────────────────────
describe('agent.rename 라우팅', () => {
  it('id + name → renameProfile(id, trimmed)', () => {
    run('agent.rename', { id: '  a1  ', name: '  내 에이전트  ' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', '내 에이전트')
  })
  it('name 생략/빈문자열 → null(override 해제)', () => {
    run('agent.rename', { id: 'a1' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', null)
    clientMock.renameProfile.mockClear()
    run('agent.rename', { id: 'a1', name: '   ' })
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', null)
  })
  it('빈 id → throw(조용한 no-op 금지)', () => {
    expect(() => run('agent.rename', { name: 'x' })).toThrow(/id 가 비어 있음/)
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
  })
})

// ── agent_list 생성 계열 어댑터(ADR-0064 / ADR-0078) ──────────────────────────────
// ★동작 변경★: 옛 즉시 셸(cmd.exe) 스폰(agent.spawn) → claude reserved(비활성) 프로필 등록.
describe('agent_list 생성 계열 라우팅', () => {
  // 생성 프로필의 최소 형태(AgentProfile) — refreshProfiles → setProfiles 로 store/tree 에 실릴 값.
  const createdProfile = { id: 'p-created', cwd: 'C:/work/engram', display_name: null }

  it('createTerminal → createClaudeProfile 를 outputFormat=Terminal 로 호출', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createTerminal', {})

    expect(clientMock.createClaudeProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('createJson → createClaudeProfile 를 outputFormat=StreamJson 로 호출', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createJson', {})

    expect(clientMock.createClaudeProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('createAgent(파라미터형): 인자 없으면 StreamJson 기본, args.outputFormat 주면 그 값 사용', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])
    await run('agentlist.createAgent', {})
    expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    await run('agentlist.createAgent', { outputFormat: 'Terminal' })
    expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
  })

  it('createAgent(파라미터형): 잘못된 outputFormat → 명시 throw + createClaudeProfile 미호출', async () => {
    // 다이얼로그(open)보다 검증이 먼저라 폴더 픽·createClaudeProfile 모두 타지 않는다.
    await expect(run('agentlist.createAgent', { outputFormat: 'invalid' })).rejects.toThrow(/invalid/)
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('취소(다이얼로그 null) → no-op(클라이언트·refetch 미호출)', async () => {
    dialogMock.open.mockResolvedValueOnce(null)
    await run('agentlist.createTerminal', {})
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).not.toHaveBeenCalled()
    expect(useAgentStore.getState().profiles).toEqual([])
  })
})

// ── agent_list pane 메뉴 = 1단 서브메뉴 컨테이너(ADR-0078 / ADR-0065) ──────────────
// side-effect import 로 registerSlotMenu 가 이미 컨테이너를 기여했다.
describe('agent_list 생성 서브메뉴(ADR-0078)', () => {
  it('"에이전트 생성" 컨테이너 + 자식 2개(선언 순서 Json→Terminal)', () => {
    const items = buildSlotMenu('agent_list')
    const container = items.find(i => i.title === '에이전트 생성')
    expect(container).toBeDefined()
    expect(container?.children?.length).toBe(2)
    expect(container?.children?.map(c => c.id)).toEqual([
      'agentlist.createJson',
      'agentlist.createTerminal',
    ])
  })
})
