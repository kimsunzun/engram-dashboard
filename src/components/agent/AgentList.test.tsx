// AgentList 단위테스트 — statusGlyph 전 분기(pure, ADR-0062) + 평면 목록 렌더 스모크 + 픽커 상호작용.
//
// ★검증 불변식★:
//   1. statusGlyph: Running/Exiting/Exited/Failed/Killed/Reserved + 미지 status 전 분기 고정.
//      ◐(입력대기)는 어떤 입력으로도 반환되지 않는다(어휘로만 존재, 미점등).
//   2. 평면 렌더: running ∪ reserved 행이 뜨고, 표시명 = cwd basename(이름 미저장), glyph 가 상태 모양.
//   3. 배경 우클릭 → "에이전트 생성" → 픽커(프리셋 목록 + 경로 입력) → agent.spawn command 라우팅.
//   4. 스타일 = 변수-only(하드코딩 색 없음).

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── clientFactory / registry stub ────────────────────────────────────────────
// AgentList 는 agentClient(spawnProfile/killAgent) + commands/registry.run('agent.spawn') 를 부른다.
//   registry.run 을 mock 해 command 라우팅만 검증한다(실제 agentCommands 배선은 agentCommands.test 담당).
const clientMock = vi.hoisted(() => ({
  spawnProfile: vi.fn(async () => ({ id: 'a' })),
  killAgent: vi.fn(async () => undefined),
  deleteProfile: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    spawnProfile: (...args: unknown[]) => clientMock.spawnProfile(...(args as [])),
    killAgent: (...args: unknown[]) => clientMock.killAgent(...(args as [])),
    deleteProfile: (...args: unknown[]) => clientMock.deleteProfile(...(args as [])),
  },
  getAgentClient: vi.fn(),
}))
const runMock = vi.hoisted(() => vi.fn(() => Promise.resolve({ id: 'spawned' })))
vi.mock('../../commands/registry', () => ({
  run: (...args: unknown[]) => runMock(...(args as [])),
}))
vi.mock('../../store/eventBus', () => ({ refreshProfiles: vi.fn() }))
// viewStore 는 assignAgent(열기 경로)만 참조 — getState 로 접근하므로 실제 store 를 얕게 stub.
//   assignAgent 는 hoisted 한 곳에 두고 getState/셀렉터가 동일 인스턴스를 반환하게 한다(호출 검증용).
const assignAgentMock = vi.hoisted(() => vi.fn(async () => undefined))
vi.mock('../../store/viewStore', () => ({
  useViewStore: Object.assign(
    (sel: (s: unknown) => unknown) => sel({ assignAgent: assignAgentMock }),
    { getState: () => ({ assignAgent: assignAgentMock }) },
  ),
  currentViewId: () => 'main-view',
  selectView: () => ({ focusedSlotId: 'slot-1' }),
}))

import AgentList, { statusGlyph } from './AgentList'
import { useAgentStore } from '../../store/agentStore'
import type { AgentInfo, AgentProfile, Capabilities, Preset } from '../../api/types'

const caps = (): Capabilities => ({
  input: { raw: true, message: false, attachment: false },
  output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
  control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: false },
  session: { resume: true, snapshot: false, cwd_env: true },
  model: { select: false, temperature: false, max_tokens: false },
})

function agent(id: string, cwd: string, status: AgentInfo['status'] = { type: 'Running' }): AgentInfo {
  return { id, name: '', cwd, status, cols: 80, rows: 24, epoch: 1, capabilities: caps() }
}
function profile(id: string, cwd: string, createdAt = 0): AgentProfile {
  return {
    id, name: '', command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd, env: [], claude_session_id: null, old_session_ids: [], epoch: 0, auto_restore: false,
    restart_policy: 'Never', restart_count: 0, failed_reason: null, created_at: createdAt,
    last_active: 0, last_start_at: null,
  }
}

beforeEach(() => {
  clientMock.spawnProfile.mockClear()
  clientMock.killAgent.mockClear()
  clientMock.deleteProfile.mockClear()
  assignAgentMock.mockClear()
  runMock.mockClear()
  runMock.mockImplementation(() => Promise.resolve({ id: 'spawned' }))
  useAgentStore.setState({ agents: [], profiles: [], presets: [], selectedAgentId: null })
})
afterEach(() => {
  cleanup()
  useAgentStore.setState({ agents: [], profiles: [], presets: [], selectedAgentId: null })
})

// ── statusGlyph: 전 분기(pure, ADR-0062) ─────────────────────────────────────
describe('statusGlyph (pure, 전 분기)', () => {
  it('Running → ● (작업중)', () => expect(statusGlyph('Running')).toBe('●'))
  it('Exiting → ◻ (멈춤 전이)', () => expect(statusGlyph('Exiting')).toBe('◻'))
  it('Exited → ◻ (멈춤)', () => expect(statusGlyph('Exited')).toBe('◻'))
  it('Killed → ◻ (멈춤)', () => expect(statusGlyph('Killed')).toBe('◻'))
  it('Failed → ✗ (에러)', () => expect(statusGlyph('Failed')).toBe('✗'))
  it('Reserved → ○ (유휴/미spawn)', () => expect(statusGlyph('Reserved')).toBe('○'))
  it('미지 status → ○ (degrade, 빈 글리프 방지)', () => expect(statusGlyph('???')).toBe('○'))
  it('◐(입력대기)는 어떤 입력으로도 반환되지 않는다(어휘로만 존재, 미점등)', () => {
    const inputs = ['Running', 'Exiting', 'Exited', 'Failed', 'Killed', 'Reserved', 'unknown', '']
    for (const s of inputs) expect(statusGlyph(s)).not.toBe('◐')
  })
})

// ── 평면 목록 렌더 ─────────────────────────────────────────────────────────
describe('AgentList 평면 렌더', () => {
  it('빈 목록 → 안내 문구', () => {
    render(<AgentList />)
    expect(screen.getByText(/에이전트 없음/)).toBeTruthy()
  })

  it('running ∪ reserved 행 렌더 + 표시명 = cwd basename(이름 미저장)', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/work/engram')],
      profiles: [profile('p1', '/home/me/reserved-proj')],
    })
    render(<AgentList />)
    // 실행중 행(basename) + 예약 행(basename) 둘 다 마운트.
    expect(document.querySelector('[data-agent-row="a1"]')).toBeTruthy()
    expect(document.querySelector('[data-agent-row="p1"]')).toBeTruthy()
    expect(screen.getByText('engram')).toBeTruthy()
    expect(screen.getByText('reserved-proj')).toBeTruthy()
  })

  it('상태 글리프가 모양으로 뜬다(running=● / reserved=○)', () => {
    useAgentStore.setState({
      agents: [agent('a1', 'C:/w', { type: 'Failed', message: 'x' })],
      profiles: [profile('p1', 'C:/r')],
    })
    render(<AgentList />)
    const runGlyph = document.querySelector('[data-agent-row="a1"] [data-agent-glyph]') as HTMLElement
    const resGlyph = document.querySelector('[data-agent-row="p1"] [data-agent-glyph]') as HTMLElement
    expect(runGlyph.textContent).toBe('✗') // Failed
    expect(resGlyph.textContent).toBe('○') // Reserved
  })

  it('예약 행 더블클릭 → spawnProfile(restore UX 유지)', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.doubleClick(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    expect(clientMock.spawnProfile).toHaveBeenCalledWith('p1', false)
  })

  it('스타일 = 변수-only(루트 background 가 var 참조)', () => {
    render(<AgentList />)
    const root = document.querySelector('[data-agent-list]') as HTMLElement
    expect(root.style.background).toContain('var(')
  })
})

// ── 배경 메뉴 → 픽커 → agent.spawn 라우팅 ────────────────────────────────────
describe('에이전트 생성 픽커', () => {
  it('배경 우클릭 → "에이전트 생성" → 픽커 표시', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    expect(document.querySelector('[data-agent-picker]')).toBeTruthy()
  })

  it('픽커에서 프리셋 선택 → run("agent.spawn",{preset:id})', () => {
    useAgentStore.setState({ presets: [{ id: 'pr1', cwd: 'C:/work/engram' } as Preset] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    // 프리셋 표시명 = basename.
    expect(document.querySelector('[data-agent-picker-preset="pr1"]')?.textContent).toBe('engram')
    fireEvent.click(document.querySelector('[data-agent-picker-preset="pr1"]') as HTMLElement)
    expect(runMock).toHaveBeenCalledWith('agent.spawn', { preset: 'pr1' })
  })

  it('픽커에 경로 입력 + 생성 → run("agent.spawn",{cwd})', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    const input = document.querySelector('[data-agent-picker-input]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '  C:/new/path  ' } })
    fireEvent.click(document.querySelector('[data-agent-picker-go]') as HTMLElement)
    expect(runMock).toHaveBeenCalledWith('agent.spawn', { cwd: 'C:/new/path' })
  })
})

// ── 행 우클릭 메뉴: 종료 wired / 이름변경·재시작 disabled ──────────────────────
describe('행 우클릭 메뉴', () => {
  it('실행중 행: 종료는 killAgent 호출, 이름변경·재시작은 "준비 중" 비활성(no-op)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    // 준비 중 항목 클릭해도 아무 command/agentClient 호출 없음(disabled no-op).
    fireEvent.click(screen.getByText('이름변경 (준비 중)'))
    fireEvent.click(screen.getByText('재시작 (준비 중)'))
    expect(clientMock.killAgent).not.toHaveBeenCalled()
    // 종료 → killAgent.
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('a1')
  })
})

// ── FIX#1: 동기 in-flight 가드(useRef) — 연타 더블파이어 1회로 접힘 ─────────────
describe('동기 in-flight 가드(useRef, FIX#1)', () => {
  // spawnProfile 을 미해결 Promise 로 잡아 두 번째 doubleClick 이 아직 in-flight 인 상태에서 들어오게 한다.
  //   useState 만이면 re-render commit 전 stale closure 로 둘 다 통과하지만, busyRef 동기 가드는 즉시 차단한다.
  it('예약 행 더블클릭 연타 → spawnProfile 은 1회만(busyRef 동기 차단)', () => {
    let resolveSpawn: (() => void) | undefined
    clientMock.spawnProfile.mockImplementationOnce(
      () => new Promise<{ id: string }>(res => { resolveSpawn = () => res({ id: 'p1' }) }),
    )
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    const row = document.querySelector('[data-agent-row="p1"]') as HTMLElement
    fireEvent.doubleClick(row) // 1st — in-flight 진입(미해결)
    fireEvent.doubleClick(row) // 2nd — 같은 tick, busyRef 로 차단돼야 함
    expect(clientMock.spawnProfile).toHaveBeenCalledTimes(1)
    resolveSpawn?.() // cleanup(핸들 미해결 방지)
  })

  it('실행중 행 종료 연타 → killAgent 은 1회만(busyRef 동기 차단)', () => {
    clientMock.killAgent.mockImplementationOnce(() => new Promise<undefined>(() => {})) // 영구 미해결
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    // 메뉴 → 종료 클릭(메뉴는 클릭 시 닫히므로 매번 다시 연다).
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료')) // 2nd — in-flight, busyRef 차단
    expect(clientMock.killAgent).toHaveBeenCalledTimes(1)
  })

  it('열기 연타 → assignAgent 은 1회만(busyRef 동기 차단)', () => {
    assignAgentMock.mockImplementationOnce(() => new Promise<undefined>(() => {})) // 영구 미해결
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('열기'))
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('열기')) // 2nd — in-flight, busyRef 차단
    expect(assignAgentMock).toHaveBeenCalledTimes(1)
  })
})

// ── FIX#2: 생성 실패 시 픽커 유지 + 인라인 에러(동기 throw·async reject 모두) ───────
describe('에이전트 생성 실패 표시(FIX#2)', () => {
  it('run async reject → 픽커 유지 + 에러 표시', async () => {
    runMock.mockImplementationOnce(() => Promise.reject(new Error('boom-async')))
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    const input = document.querySelector('[data-agent-picker-input]') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'C:/x' } })
    fireEvent.click(document.querySelector('[data-agent-picker-go]') as HTMLElement)
    await waitFor(() => expect(document.querySelector('[data-agent-picker-error]')).toBeTruthy())
    // 픽커는 닫히지 않는다(재시도 가능).
    expect(document.querySelector('[data-agent-picker]')).toBeTruthy()
    expect(document.querySelector('[data-agent-picker-error]')?.textContent).toContain('boom-async')
  })

  it('run 동기 throw → Promise.resolve.catch 로 못 잡던 케이스도 잡아 픽커 유지 + 에러 표시', async () => {
    // ★핵심★: run 이 동기 throw 하면 옛 Promise.resolve(run()).catch 는 못 잡는다(unhandled). async try/catch 로 잡힘.
    runMock.mockImplementationOnce(() => { throw new Error('boom-sync') })
    useAgentStore.setState({ presets: [{ id: 'pr1', cwd: 'C:/work/engram' } as Preset] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-picker-preset="pr1"]') as HTMLElement)
    await waitFor(() => expect(document.querySelector('[data-agent-picker-error]')).toBeTruthy())
    expect(document.querySelector('[data-agent-picker]')).toBeTruthy()
    expect(document.querySelector('[data-agent-picker-error]')?.textContent).toContain('boom-sync')
  })

  it('성공 시 픽커 닫힘(에러 없음)', async () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    const input = document.querySelector('[data-agent-picker-input]') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'C:/ok' } })
    fireEvent.click(document.querySelector('[data-agent-picker-go]') as HTMLElement)
    await waitFor(() => expect(document.querySelector('[data-agent-picker]')).toBeFalsy())
  })
})

// ── FIX#3: 예약 행 "예약 취소" → deleteProfile(리그레션 복원) ────────────────────
describe('예약 취소(deleteProfile, FIX#3)', () => {
  it('예약 행 우클릭 → "예약 취소" → deleteProfile(id) 호출', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('예약 취소'))
    expect(clientMock.deleteProfile).toHaveBeenCalledWith('p1')
  })

  it('실행중 행 메뉴엔 "예약 취소" 없음(reserved 전용)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.queryByText('예약 취소')).toBeNull()
  })
})

// ── FIX#4: 픽커 ↔ 메뉴 상호배타 ─────────────────────────────────────────────
describe('픽커 ↔ 메뉴 상호배타(FIX#4)', () => {
  it('픽커 열림 상태에서 행 우클릭 → 픽커 닫히고 행 메뉴만', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    // 픽커 연다.
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    expect(document.querySelector('[data-agent-picker]')).toBeTruthy()
    // 행 우클릭 → 픽커 닫힘.
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(document.querySelector('[data-agent-picker]')).toBeFalsy()
    expect(screen.getByText('열기')).toBeTruthy() // 행 메뉴 떠 있음
  })

  it('픽커 열림 상태에서 배경 우클릭 → 픽커 닫히고 배경 메뉴만', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    fireEvent.click(document.querySelector('[data-agent-create]') as HTMLElement)
    expect(document.querySelector('[data-agent-picker]')).toBeTruthy()
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    expect(document.querySelector('[data-agent-picker]')).toBeFalsy()
    expect(document.querySelector('[data-agent-create]')).toBeTruthy() // 배경 메뉴 떠 있음
  })
})

// ── FIX#5: Escape 로 열린 메뉴 닫기 ─────────────────────────────────────────
describe('Escape 로 메뉴 닫기(FIX#5)', () => {
  it('행 메뉴 열림 → Escape → 닫힘', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByText('열기')).toBeNull()
  })

  it('배경 메뉴 열림 → Escape → 닫힘', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    expect(document.querySelector('[data-agent-create]')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(document.querySelector('[data-agent-create]')).toBeFalsy()
  })
})
