// 리치 슬롯 백엔드 표기 회귀 — 제목·표식·색조가 claude 와 codex 를 가르나.
//
// 배경: 리치(구조화) 모드는 두 백엔드를 똑같이 그려 누구와 말하는지 화면에 신호가 없었다. 가르는 근거는
//   프로필의 `command.kind` 하나이고, **프로필이 없으면 claude 표기 그대로**가 규약이다(회귀 없는 기본값
//   — ad-hoc 세션과 프로필을 안 채우는 mock 이 그 경로를 실제로 탄다).
//
// 관측 표면: 빈 상태 제목 글자 · 표식의 `data-rich-brand` · 루트의 `data-rich-brand` + 색조 클래스.
//   ★루트 토큰을 따로 재는 이유★: 대화가 시작되면 제목·표식이 함께 접혀(ADR-0145) 그때 백엔드를 말하는
//   DOM 은 루트 속성뿐이다.
//
// 전략: agentClient(clientFactory)·agentStore 를 RichSlot.test.tsx 와 동일 패턴으로 stub 하고,
//   복원 완료 신호('live')를 발화해 빈 상태를 연다.

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// ── subscribeOutput 콜백 캡처(상태 콜백만 쓴다 — 빈 상태 게이트를 여는 신호). ──
const captured = vi.hoisted(() => ({
  onState: null as ((s: ViewPhase) => void) | null,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 시그니처 (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        _onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
      ) => {
        captured.onState = onState ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
    // 등록 즉시 현재 상태로 1회 통지 + disposer 반환(실물 계약).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('connected')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 부재 판정(agents·agentsLoaded)과 표기 판정(profiles)이 여기를 읽는다. ──
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: undefined as unknown[] | undefined,
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// ── 테스트 대상 ────────────────────────────────────────────────────────────────
import RichSlot from './RichSlot'
import { richBranding } from './richBranding'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** 프로필 최소 형태 — 슬롯이 읽는 칸(id·command·cwd·display_name)만 채운다. */
function profile(kind: 'Claude' | 'Codex' | 'Shell'): unknown {
  const command =
    kind === 'Shell'
      ? { kind: 'Shell', program: 'pwsh', args: [] }
      : { kind, extra_args: [], output_format: 'StreamJson' }
  return { id: AGENT, name: 'C:/work', display_name: null, cwd: 'C:/work', command }
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/** 마운트 + 복원 완료 신호 → 빈 상태(표식·제목)가 열린 화면. */
async function renderEmpty(): Promise<void> {
  render(<RichSlot viewId="v1" agentId={AGENT} />)
  await flush()
  act(() => captured.onState!('live'))
}

function root(): HTMLElement {
  const el = document.querySelector('[data-rich-live="1"]')
  if (!el) throw new Error('RichSlot 루트가 없다')
  return el as HTMLElement
}

function mascot(): HTMLElement | null {
  return document.querySelector('[data-rich-mascot="1"]')
}

beforeEach(() => {
  captured.onState = null
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = undefined
})

afterEach(() => {
  cleanup()
})

describe('richBranding — 순수 매핑', () => {
  it('Codex 프로필만 codex 표기로 갈린다', () => {
    const b = richBranding('Codex')
    expect(b.brand).toBe('codex')
    expect(b.title).toBe('Codex')
    expect(b.tintClass).toBe('rich-tint-codex')
  })

  it('Claude 프로필은 기존 표기 그대로다(제목 문자열 불변 — 제품명이라 i18n 을 타지 않는다)', () => {
    const b = richBranding('Claude')
    expect(b.brand).toBe('claude')
    expect(b.title).toBe('Claude Code')
    // 색조 없음 = 지금까지의 모습. 여기 클래스가 생기면 기존 화면이 전부 갈린다.
    expect(b.tintClass).toBe('')
  })

  // ★기본값 규약★: 모르는 것을 중립 표기로 바꾸지 않는다 — 알아낸 것 없이 화면만 갈리는 쪽이 더 나쁘다.
  it('프로필 부재(null·undefined)와 미지 종류는 claude 표기로 떨어진다', () => {
    expect(richBranding(null).brand).toBe('claude')
    expect(richBranding(undefined).brand).toBe('claude')
    expect(richBranding('Shell').brand).toBe('claude')
  })
})

describe('RichSlot — 빈 상태 표기', () => {
  it('codex 프로필: 제목·표식·색조가 모두 codex 로 간다', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()

    expect(screen.getByText('Codex')).toBeTruthy()
    expect(screen.queryByText('Claude Code')).toBeNull()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('codex')
    expect(root().className).toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('codex')
  })

  it('claude 프로필: 표기가 하나도 바뀌지 않는다(색조 클래스 없음)', async () => {
    agentStoreState.profiles = [profile('Claude')]
    await renderEmpty()

    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
    expect(root().getAttribute('data-rich-brand')).toBe('claude')
  })

  it('프로필이 없으면 claude 표기 — 색조도 붙지 않는다(현행 모습 유지)', async () => {
    await renderEmpty() // profiles = undefined(단위테스트 mock·ad-hoc 세션 경로)
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(mascot()?.getAttribute('data-rich-brand')).toBe('claude')
    expect(root().className).not.toContain('rich-tint-codex')
  })

  // 표식은 순수 장식이라 접근성 트리에서 숨긴다(ADR-0146 불변식) — 형제와 같은 규율을 codex 표식도 진다.
  it('codex 표식도 aria-hidden 이고 기존 표식 셀렉터에 그대로 걸린다', async () => {
    agentStoreState.profiles = [profile('Codex')]
    await renderEmpty()
    expect(mascot()).not.toBeNull()
    expect(mascot()?.getAttribute('aria-hidden')).toBe('true')
  })
})

describe('RichSlot — 대화 중에도 백엔드가 관측된다', () => {
  // 빈 상태가 닫히면 제목·표식이 함께 사라진다 — 그 구간의 유일한 근거가 루트 토큰이다.
  it('빈 상태가 아니어도 루트 토큰·색조는 남는다', async () => {
    agentStoreState.profiles = [profile('Codex')]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    act(() => captured.onState!('buffering')) // 복원 중 = 빈 상태 게이트가 닫힌 구간

    expect(mascot()).toBeNull()
    expect(screen.queryByText('Codex')).toBeNull()
    expect(root().getAttribute('data-rich-brand')).toBe('codex')
    expect(root().className).toContain('rich-tint-codex')
  })
})
