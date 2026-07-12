// agentCommands 단위테스트 — agent.spawn 어댑터가 preset/cwd 를 해소해 agentClient.spawnAgent 로
//   올바로 라우팅하는지(ADR-0055/0011). parent 는 signature-only(미지원 throw).
//
// ★검증 불변식★:
//   1. preset(id) → store.presets 에서 cwd 해소 → spawnAgent(cwd).
//   2. raw cwd → spawnAgent(trim 된 cwd).
//   3. parent 세팅 → throw(중첩 미지원).
//   4. 빈/공백 cwd → throw. 없는 preset id → throw.

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

// ── agent.rename 어댑터(§5 LLM 제어 — ADR-0061 리치화) ────────────────────────────
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

// ── agentlist.createAgent 어댑터(ADR-0064) ────────────────────────────────────────
// ★동작 변경★: 옛 즉시 셸(cmd.exe) 스폰(agent.spawn) → 이제 claude reserved(비활성) 프로필 등록.
//   폴더 다이얼로그로 cwd 를 고른 뒤 createClaudeProfile(name=cwd, [], [], autoRestore=false, 'StreamJson').
//   활성화(더블클릭/우클릭)에서 비로소 claude 를 spawn 한다.
describe('agentlist.createAgent 라우팅', () => {
  // 생성 프로필의 최소 형태(AgentProfile) — refreshProfiles → setProfiles 로 store/tree 에 실릴 값.
  const createdProfile = { id: 'p-created', cwd: 'C:/work/engram', display_name: null }

  it('폴더 픽 → createClaudeProfile(reserved claude) 로 라우팅 + 생성 직후 store 반영(refetch)', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    // refreshProfiles(eventBus) 가 부르는 listProfiles 가 생성 프로필을 담아 돌려준다 → setProfiles 로 store 반영.
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createAgent', {})

    expect(clientMock.createClaudeProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    // ★핵심 FIX 검증★: broadcast 유실에 대비한 명시 refetch 로 예약 노드가 store(=트리 소스)에 실린다.
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    // 옛 즉시 스폰 경로는 더 이상 타지 않는다.
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  it('취소(다이얼로그 null) → no-op(클라이언트·refetch 미호출)', async () => {
    dialogMock.open.mockResolvedValueOnce(null)
    await run('agentlist.createAgent', {})
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
    // 취소면 refreshProfiles(→listProfiles)도 타지 않고 store 는 그대로.
    expect(clientMock.listProfiles).not.toHaveBeenCalled()
    expect(useAgentStore.getState().profiles).toEqual([])
  })
})
