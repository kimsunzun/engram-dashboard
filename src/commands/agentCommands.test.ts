
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'new-agent' })),
  renameProfile: vi.fn(async () => undefined),
  createClaudeProfile: vi.fn(async () => ({ id: 'new-profile' })),
  createCodexProfile: vi.fn(async () => ({ id: 'new-codex-profile' })),
  // refreshProfiles(eventBus) 가 부르는 listProfiles — 생성 직후 store/tree 반영 검증용. 기본 []
  //   (테스트별로 mockResolvedValueOnce 로 생성 프로필을 실어 반환).
  listProfiles: vi.fn(async () => [] as unknown[]),
  cancelQueuedInput: vi.fn(async (): Promise<string> => 'requested'),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: {
    spawnAgent: (...args: unknown[]) => clientMock.spawnAgent(...(args as [])),
    renameProfile: (...args: unknown[]) => clientMock.renameProfile(...(args as [])),
    createClaudeProfile: (...args: unknown[]) => clientMock.createClaudeProfile(...(args as [])),
    createCodexProfile: (...args: unknown[]) => clientMock.createCodexProfile(...(args as [])),
    listProfiles: (...args: unknown[]) => clientMock.listProfiles(...(args as [])),
    cancelQueuedInput: (...args: unknown[]) => clientMock.cancelQueuedInput(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))

// agentlist.createAgent 는 폴더 다이얼로그(open)로 cwd 를 고른다 — 픽/취소를 테스트별로 제어.
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

import './agentCommands' // side-effect register
import { getCommand, list, run, runAsHuman } from './registry'
import { INPUT_LOCKED_REFUSAL } from '../api/agentClient'
import { fireAndForget } from './dispatch'
import { buildSlotMenu } from './slotMenu'
import { useAgentStore } from '../store/agentStore'

beforeEach(() => {
  clientMock.spawnAgent.mockClear()
  clientMock.renameProfile.mockClear()
  clientMock.createClaudeProfile.mockClear()
  clientMock.createCodexProfile.mockClear()
  clientMock.listProfiles.mockClear()
  clientMock.cancelQueuedInput.mockReset()
  clientMock.cancelQueuedInput.mockImplementation(async () => 'requested')
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
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/work/engram', 'claude')
  })

  it('raw cwd → spawnAgent(trim 된 cwd)', () => {
    run('agent.spawn', { cwd: '  C:/new/path  ' })
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/new/path', 'claude')
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
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/from/preset', 'claude')
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

  // ★사람이 codex 대화형 TUI 를 고르는 문★ — 형제와 마찬가지로 등록만 하고 스폰하지 않는다 — 뜨는 것은
  //   활성화(더블클릭)에서다.
  // ★`runAsHuman` 인 것은 이제 **게이트 때문이 아니라 사람 경로를 재려고**다★ — 2026-09-22 에
  //   `humanOnly` 가 걷히면서 두 진입점이 같아졌고, LLM 쪽은 아래 별도 항목이 따로 잰다.
  it('createCodex(사람 경로) → createCodexProfile 를 outputFormat=Terminal 로 호출', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await runAsHuman('agentlist.createCodex', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  // ★모드 축이 실제로 갈리는 것을 재는 자리★ — 두 codex 문이 같은 `createCodexProfile` 을 부르므로,
  //   모드를 안 싣거나 둘 다 같은 값을 실으면 위 항목과 이 항목 중 하나가 빨개진다.
  it('createCodexJson(사람 경로) → createCodexProfile 를 outputFormat=StreamJson 로 호출', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await runAsHuman('agentlist.createCodexJson', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
    expect(clientMock.listProfiles).toHaveBeenCalledTimes(1)
    expect(useAgentStore.getState().profiles).toEqual([createdProfile])
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
  })

  // 형제 `createCodex` 와 **같은 게이트를 진다** — 한쪽만 열리거나 한쪽만 닫히는 것이 이 항목이 막는
  //   회귀다(그 게이트는 지금 열려 있다).
  it('createCodexJson: LLM 진입점(registry.run)도 사람 경로와 같은 프로필을 만든다', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createCodexJson', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'StreamJson',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
  })

  // ★세 번째 생성 문 — 두 진입점이 같은 문을 연다★(2026-09-22: 옛 `humanOnly` 게이트를 걷었다).
  //
  // 이 항목은 **사람 메뉴이면서 동시에 LLM command** 다. 그 겸직을 닫고 있던 것은 백엔드 낱말이 아니라
  // **호출자 축**이었고, 그 축을 세운 전제(codex 의 신뢰 확인 모달을 LLM 이 못 지난다)가 실측으로
  // 죽었다 — 근거의 정본 = `engram-dashboard-agent` 의 `commands::LLM_BACKEND_POLICY` 의 codex 줄.
  // ★그래서 이 둘이 재는 것이 뒤집혔다★: 게이트를 되살리면 이 두 항목이 빨개진다. 되살릴 일이 생기면
  // 그 표부터 닫을 것 — 표는 열린 채 여기만 닫으면 `agent.new` 로는 만들고 이 문으로는 못 만든다.
  it('createCodex: LLM 진입점(registry.run)도 사람 경로와 같은 프로필을 만든다', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    await run('agentlist.createCodex', {})

    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
    expect(clientMock.createClaudeProfile).not.toHaveBeenCalled()
  })

  it('createCodex: 사람 경로(트리 메뉴 → fireAndForget)는 그대로 codex 를 만든다', async () => {
    dialogMock.open.mockResolvedValueOnce('C:/work/engram')
    clientMock.listProfiles.mockResolvedValueOnce([createdProfile])

    // `SlotContextMenu` 가 메뉴 항목에 쓰는 바로 그 호출이다(그 파일의 `fireAndForget(id, …)`).
    fireAndForget('agentlist.createCodex')

    await vi.waitFor(() => expect(clientMock.createCodexProfile).toHaveBeenCalled())
    expect(clientMock.createCodexProfile).toHaveBeenCalledWith(
      'C:/work/engram', 'C:/work/engram', [], [], false, 'Terminal',
    )
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

  // ★대소문자 무관 수용 + 선언 철자 정규화(사용자 결정 2026-09-23)★: 백엔드로 나가는 값은 호출자가 쓴
  //   철자가 아니라 'Terminal'·'StreamJson' 이어야 한다 — 그래야 wire 가 종전과 바이트 동일하다.
  it('createAgent(파라미터형): outputFormat 대소문자 무관 수용 → 선언 철자로 정규화해 전달', async () => {
    for (const [given, expected] of [
      ['terminal', 'Terminal'],
      ['TERMINAL', 'Terminal'],
      ['streamjson', 'StreamJson'],
      ['StreamJSON', 'StreamJson'],
    ] as const) {
      dialogMock.open.mockResolvedValueOnce('C:/work/engram')
      clientMock.listProfiles.mockResolvedValueOnce([createdProfile])
      await run('agentlist.createAgent', { outputFormat: given })
      expect(clientMock.createClaudeProfile).toHaveBeenLastCalledWith(
        'C:/work/engram', 'C:/work/engram', [], [], false, expected,
      )
    }
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
  it('"에이전트 생성" 컨테이너 + 자식 4개(선언 순서 Json→Terminal→Codex→CodexJson)', () => {
    const items = buildSlotMenu('agent_list')
    const container = items.find(i => i.title === '에이전트 생성')
    expect(container).toBeDefined()
    expect(container?.children?.length).toBe(4)
    expect(container?.children?.map(c => c.id)).toEqual([
      'agentlist.createJson',
      'agentlist.createTerminal',
      'agentlist.createCodex',
      'agentlist.createCodexJson',
    ])
  })
})

// ── ADR-0231: 대기 입력 ✕ 와 LLM 이 같은 핸들을 흔든다 ─────────────────────────────────────
describe('agent.cancelQueuedInput', () => {
  it('등록돼 있고 help 가 없다 — 버스 명단에 안 오른다(데몬이 답하는 이름이라 오르면 등록 묶음 전체가 반려된다)', () => {
    expect(getCommand('agent.cancelQueuedInput')).toBeDefined()
    expect(getCommand('agent.cancelQueuedInput')?.help).toBeUndefined()
    expect(list().find((c) => c.id === 'agent.cancelQueuedInput')?.help).toBeUndefined()
  })

  it('agentId·inputId 로 agentClient.cancelQueuedInput 을 부르고 결말 낱말을 돌려준다', async () => {
    await expect(run('agent.cancelQueuedInput', { agentId: ' a1 ', inputId: 'q1' })).resolves.toEqual({
      outcome: 'requested',
    })
    expect(clientMock.cancelQueuedInput).toHaveBeenCalledWith('a1', 'q1')
  })

  it('모르는 결말 낱말도 그대로 돌려준다', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => 'deferred')
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).resolves.toEqual({
      outcome: 'deferred',
    })
  })

  it('코드 없이 온 임대 거절 문구는 CONFLICT 로 읽힌다', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error(INPUT_LOCKED_REFUSAL)
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      `CONFLICT: ${INPUT_LOCKED_REFUSAL}`,
    )
  })

  it('운영 carrier 처럼 맨 문자열로 온 임대 거절 문구도 CONFLICT 로 읽힌다', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw INPUT_LOCKED_REFUSAL
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      `CONFLICT: ${INPUT_LOCKED_REFUSAL}`,
    )
  })

  it('맨 문자열로 온 다른 거절(NOT_FOUND)은 문자열 그대로 다시 던진다', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw 'NOT_FOUND: no waiting input'
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toBe(
      'NOT_FOUND: no waiting input',
    )
  })

  it('문자열도 Error 도 아닌 거절은 건드리지 않는다', async () => {
    const odd = { code: 'X' }
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw odd
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toBe(odd)
  })

  it('코드를 단 실패(NOT_FOUND)와 코드 없는 다른 실패(끊김)는 그대로 둔다', async () => {
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error('NOT_FOUND: no waiting input')
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      /^NOT_FOUND: no waiting input$/,
    )
    clientMock.cancelQueuedInput.mockImplementation(async () => {
      throw new Error('connection lost')
    })
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q1' })).rejects.toThrow(
      /^connection lost$/,
    )
  })

  it('빈 agentId·inputId 는 보내지 않고 던진다', async () => {
    await expect(run('agent.cancelQueuedInput', { agentId: '  ', inputId: 'q1' })).rejects.toThrow(/agentId/)
    await expect(run('agent.cancelQueuedInput', { agentId: 'a1' })).rejects.toThrow(/inputId/)
    expect(clientMock.cancelQueuedInput).not.toHaveBeenCalled()
  })

  it('사람 경로(✕ 클릭 → fireAndForget)도 같은 명령을 부른다', async () => {
    fireAndForget('agent.cancelQueuedInput', { agentId: 'a1', inputId: 'q2' })
    await Promise.resolve()
    expect(clientMock.cancelQueuedInput).toHaveBeenCalledWith('a1', 'q2')
  })
})
