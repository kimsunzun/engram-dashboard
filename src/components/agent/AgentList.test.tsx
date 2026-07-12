// AgentList 단위테스트 — statusGlyph 전 분기(pure, ADR-0062) + 평면 목록 렌더 스모크 + 행 메뉴 상호작용.
//
// ★검증 불변식★:
//   1. statusGlyph: Running/Exiting/Exited/Failed/Killed/Reserved + 미지 status 전 분기 고정.
//      ◐(입력대기)는 어떤 입력으로도 반환되지 않는다(어휘로만 존재, 미점등).
//   2. 평면 렌더: running ∪ reserved 행이 뜨고, 표시명 = cwd basename(이름 미저장), glyph 가 상태 모양.
//   3. ★ROW 메뉴만 유지(ADR-0064)★: 배경(bg) 메뉴 + 프리셋 픽커는 제거됐다(배경 우클릭 = 통합 슬롯 메뉴로
//      버블 — agentlist.createAgent command). 행 우클릭 메뉴(활성화/예약취소 · 열기/종료/이름변경/재시작)는
//      item-targeted 라 그대로 유지되고 여전히 stopPropagation 한다.
//   4. 스타일 = 변수-only(하드코딩 색 없음).

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── clientFactory stub ────────────────────────────────────────────────────────
// AgentList 는 agentClient(spawnProfile/killAgent/deleteProfile) 를 부른다(spawn 은 이제 통합 슬롯 메뉴의
//   agentlist.createAgent command 로 이전 — AgentList 는 더 이상 registry.run 을 부르지 않는다, ADR-0064).
const clientMock = vi.hoisted(() => ({
  spawnProfile: vi.fn(async () => ({ id: 'a' })),
  killAgent: vi.fn(async () => undefined),
  deleteProfile: vi.fn(async () => undefined),
  renameProfile: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    spawnProfile: (...args: unknown[]) => clientMock.spawnProfile(...(args as [])),
    killAgent: (...args: unknown[]) => clientMock.killAgent(...(args as [])),
    deleteProfile: (...args: unknown[]) => clientMock.deleteProfile(...(args as [])),
    renameProfile: (...args: unknown[]) => clientMock.renameProfile(...(args as [])),
  },
  getAgentClient: vi.fn(),
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
import type { AgentInfo, AgentProfile, Capabilities } from '../../api/types'

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
function profile(id: string, cwd: string, createdAt = 0, displayName: string | null = null): AgentProfile {
  return {
    id, name: '', display_name: displayName,
    command: { kind: 'Claude', extra_args: [], output_format: 'Terminal' },
    cwd, env: [], claude_session_id: null, old_session_ids: [], epoch: 0, auto_restore: false,
    restart_policy: 'Never', restart_count: 0, failed_reason: null, created_at: createdAt,
    last_active: 0, last_start_at: null,
  }
}

beforeEach(() => {
  clientMock.spawnProfile.mockClear()
  clientMock.killAgent.mockClear()
  clientMock.deleteProfile.mockClear()
  clientMock.renameProfile.mockClear()
  assignAgentMock.mockClear()
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

  // ★FIX-A: 빈 상태 안내는 ScrollArea 밖의 flex-1 센터링 div★ — Radix Viewport 자식이 display:table 로
  //   감싸져 height:100% 세로 중앙이 WebView2 에서 안 먹던 회귀 방지. 행이 없으면 스크롤 표면도 마운트 안 함.
  it('빈 목록 → 안내 문구는 ScrollArea(스크롤 표면) 밖에 있고 스크롤 표면은 마운트되지 않는다(FIX-A)', () => {
    render(<AgentList />)
    const scroll = document.querySelector('[data-testid="agent-list-scroll"]')
    expect(scroll).toBeNull() // 스크롤할 행이 없으면 ScrollArea 자체가 없다
    const empty = screen.getByText(/에이전트 없음/)
    // 안내 문구는 바깥 컬럼(data-agent-list)의 직속 자식(스크롤 표면 안이 아님).
    expect(empty.closest('[data-testid="agent-list-scroll"]')).toBeNull()
    expect(empty.closest('[data-agent-list]')).toBeTruthy()
  })

  it('비빈 목록 → ScrollArea(스크롤 표면) 마운트 + 행이 그 안에 있다(FIX-A)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    const scroll = document.querySelector('[data-testid="agent-list-scroll"]')
    expect(scroll).toBeTruthy()
    expect(scroll?.querySelector('[data-agent-row="a1"]')).toBeTruthy()
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

// ── ★배경(bg) 메뉴 제거 = 통합 슬롯 메뉴로 버블(ADR-0064)★ ──────────────────────
describe('배경 우클릭 = 자체 메뉴 없음(통합 슬롯 메뉴로 버블)', () => {
  it('배경 우클릭 → 옛 bg "에이전트 생성" 메뉴/픽커가 뜨지 않는다(제거됨)', () => {
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-list]') as HTMLElement)
    // 옛 자체 배경 메뉴·픽커 요소는 더 이상 없다(agentlist.createAgent = 통합 메뉴 command 로 이전).
    expect(document.querySelector('[data-agent-create]')).toBeNull()
    expect(document.querySelector('[data-agent-picker]')).toBeNull()
  })

  it('배경 우클릭 이벤트는 stopPropagation 하지 않는다(상위 통합 슬롯 메뉴가 받게 버블)', () => {
    render(<AgentList />)
    const pane = document.querySelector('[data-agent-list]') as HTMLElement
    // stopPropagation 여부를 관측 — 배경(pane) 우클릭은 버블해야 한다(옛 stopPropagation 제거 회귀 안전망).
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    const stopSpy = vi.spyOn(ev, 'stopPropagation')
    pane.dispatchEvent(ev)
    expect(stopSpy).not.toHaveBeenCalled()
  })
})

// ── 행 우클릭 메뉴: 종료·이름변경 wired / 재시작 disabled(ADR-0061 리치화) ──────────
describe('행 우클릭 메뉴', () => {
  it('실행중 행: 종료는 killAgent 호출, 재시작은 "준비 중" 비활성(no-op)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    // 재시작은 여전히 준비 중(백엔드 command 부재) — 클릭해도 no-op.
    fireEvent.click(screen.getByText('재시작 (준비 중)'))
    expect(clientMock.killAgent).not.toHaveBeenCalled()
    // 종료 → killAgent.
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('a1')
  })

  it('실행중 행 이름변경 → 인라인 입력 → Enter 확정 → renameProfile(id, trimmed) 호출', () => {
    // 프로필 없는 ad-hoc running 이면 rename 대상이 없지만, 여기선 매칭 프로필을 둬 override 저장 경로를 검증.
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')], profiles: [profile('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="a1"]') as HTMLInputElement
    expect(input).toBeTruthy()
    fireEvent.change(input, { target: { value: '  내 에이전트  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    // trim 후 값으로 renameProfile 발화(§5 백엔드 저장 — 낙관 갱신 X).
    expect(clientMock.renameProfile).toHaveBeenCalledWith('a1', '내 에이전트')
  })

  it('예약 행 이름변경 → Esc 취소 → renameProfile 미발화(revert)', () => {
    useAgentStore.setState({ profiles: [profile('p1', 'C:/r')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p1"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p1"]') as HTMLInputElement
    fireEvent.change(input, { target: { value: '바뀐이름' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    // Esc = 취소 → 백엔드 발화 없음(revert).
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
    expect(document.querySelector('[data-agent-rename-input="p1"]')).toBeNull() // 편집 종료
  })

  it('이름변경 미변경(표시명과 동일) → renameProfile 미발화', () => {
    // display_name override 가 이미 "고정명" → 같은 값으로 확정하면 발화 안 함(불필요 command 억제).
    useAgentStore.setState({ profiles: [profile('p2', 'C:/x', 0, '고정명')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="p2"]') as HTMLElement)
    fireEvent.click(screen.getByText('이름 변경'))
    const input = document.querySelector('[data-agent-rename-input="p2"]') as HTMLInputElement
    // 시드된 draft = 현재 표시명('고정명'). 그대로 Enter → 미변경이라 발화 없음.
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(clientMock.renameProfile).not.toHaveBeenCalled()
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

// ── 행(ROW) 메뉴는 stopPropagation 유지(item-targeted, ADR-0064) ─────────────────
describe('행 우클릭 stopPropagation(통합 메뉴 대신 행 메뉴가 이긴다)', () => {
  it('행 우클릭 이벤트는 stopPropagation 한다(상위 통합 슬롯 메뉴가 안 뜨게)', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    const row = document.querySelector('[data-agent-row="a1"]') as HTMLElement
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    const stopSpy = vi.spyOn(ev, 'stopPropagation')
    row.dispatchEvent(ev)
    expect(stopSpy).toHaveBeenCalled() // 행 메뉴가 pane 통합 메뉴를 가로챈다(의도)
  })
})

// ── Escape 로 열린 행 메뉴 닫기 ──────────────────────────────────────────────
describe('Escape 로 행 메뉴 닫기', () => {
  it('행 메뉴 열림 → Escape → 닫힘', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByText('열기')).toBeNull()
  })
})

// ── 타깃이 목록에서 사라지면 행 메뉴가 닫힌다(stale-target 가드) ──────────────────
//   rowMenu 상태는 대상 행보다 오래 산다: 목록이 바뀌어 대상 agentId 가 rows 에서 빠져도(kill/삭제/
//   마지막 에이전트 제거) rowMenu 는 남는다. empty 전이로 ScrollArea 가 언마운트돼 메뉴가 잠깐 사라졌다가
//   새 에이전트 등장으로 non-empty 재마운트되면 stale 좌표에 떠난 agentId 를 겨눈 메뉴가 되살아난다.
describe('타깃 사라지면 행 메뉴 닫힘(stale-target 가드)', () => {
  it('행 메뉴 열림 → 대상 에이전트 제거(목록 empty) → 메뉴 사라짐', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy() // 메뉴 열림
    // 마지막 에이전트 제거 → empty 전이(ScrollArea 언마운트). act 로 감싸 리셋 effect 를 flush.
    act(() => useAgentStore.setState({ agents: [] }))
    expect(screen.queryByText('열기')).toBeNull() // 메뉴 사라짐
  })

  it('대상 제거 후 다른 에이전트 등장(non-empty 재마운트) → stale 메뉴가 되살아나지 않는다', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    // a1 제거 → empty → 새 에이전트 a2 등장(non-empty 재마운트). 각 전이를 act 로 flush.
    act(() => useAgentStore.setState({ agents: [] }))
    act(() => useAgentStore.setState({ agents: [agent('a2', 'C:/x')] }))
    // 떠난 a1 을 겨눈 stale 메뉴가 되살아나면 안 된다(rowMenu 가 리셋됐어야 함).
    expect(screen.queryByText('열기')).toBeNull()
    expect(document.querySelector('[data-agent-row="a2"]')).toBeTruthy() // 새 행은 렌더
  })

  it('여러 행 중 대상만 제거(목록은 non-empty 유지) → 메뉴 사라짐', () => {
    useAgentStore.setState({ agents: [agent('a1', 'C:/w'), agent('a2', 'C:/x')] })
    render(<AgentList />)
    fireEvent.contextMenu(document.querySelector('[data-agent-row="a1"]') as HTMLElement)
    expect(screen.getByText('열기')).toBeTruthy()
    // a1 만 제거(a2 는 남음 — empty 전이 없이도 대상 부재로 닫혀야 함). act 로 flush.
    act(() => useAgentStore.setState({ agents: [agent('a2', 'C:/x')] }))
    expect(screen.queryByText('열기')).toBeNull()
  })
})
