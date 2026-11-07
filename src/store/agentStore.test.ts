// agentsLoaded wiring regression (ADR-0148).
//
// ★Why this one line gets a test★: this flag is the only signal that tells "the authoritative registry not yet received" from "truly empty". If it does not
//   stand, the slot of an agent reaped by exit looks permanently like 「에이전트 연결 중…」 (connecting to agent…) (the unavailable display never
//   shows up), and conversely if the initial value is true, right after boot every slot is decided unavailable and looks dead.
//   Both malfunctions show only when you look at the screen, so we pin it here.
//
// The boot pull (App.tsx) · reconnect resync (eventBus) · broadcast all pass through setAgents in one place, so only that one place is looked at.

import { beforeEach, describe, expect, it } from 'vitest'

import type { AgentInfo } from '../api/types'
import { useAgentStore } from './agentStore'

function agent(id: string): AgentInfo {
  return {
    id,
    name: id,
    cwd: 'C:/x',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch: 0,
    capabilities: {
      input: { raw: true, message: false, attachment: false },
      output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
      control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: false },
      session: { resume: true, snapshot: false, cwd_env: true },
      model: { select: false, temperature: false, max_tokens: false },
    },
  }
}

beforeEach(() => {
  useAgentStore.setState({ agents: [], agentsLoaded: false })
})

describe('agentStore — agentsLoaded', () => {
  it('초기값은 false(아직 못 받음)', () => {
    expect(useAgentStore.getState().agentsLoaded).toBe(false)
  })

  it('setAgents 가 서면 다시 내려가지 않는다 — 빈 목록으로 받아도 "받았음" 이다', () => {
    // ★The empty array is the key case★: when the last agent exits the registry arrives as []. The flag must stand then too
    //   for it to be decided "truly empty".
    useAgentStore.getState().setAgents([])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)

    useAgentStore.getState().setAgents([agent('a1')])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)

    useAgentStore.getState().setAgents([])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)
    expect(useAgentStore.getState().agents).toEqual([])
  })
})
