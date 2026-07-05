// fixture 파서 단위테스트 — 실측 fixture 로 stream-json → RichMessage[] 검증.
// 순수 함수라 jsdom 불필요. vitest 가 src/ 하위에서 자동 수집(tsconfig include:["src"]).
// (구 parse.test.ts 의 parseStreamJson 케이스를 승계 — parseStreamLine 은 S15 F5 에서 제거됨.)

import { describe, it, expect } from 'vitest'
import { parseStreamJson } from './fixtureParse'
import toolFixture from './fixtures/tool.jsonl?raw'

describe('parseStreamJson', () => {
  it('실측 tool fixture 에서 ContentBlock 4종(text/thinking/tool_use/tool_result) 추출', () => {
    const msgs = parseStreamJson(toolFixture)
    expect(msgs.length).toBeGreaterThan(0)
    const types = new Set(msgs.flatMap((m) => m.blocks).map((b) => b.type))
    expect(types.has('tool_use')).toBe(true)
    expect(types.has('tool_result')).toBe(true)
    expect(types.has('text')).toBe(true)
    expect(types.has('thinking')).toBe(true)
  })

  it('tool_use ↔ tool_result 가 id 로 페어링된다(리버트 버튼 근거)', () => {
    const blocks = parseStreamJson(toolFixture).flatMap((m) => m.blocks)
    const use = blocks.find((b) => b.type === 'tool_use')
    const res = blocks.find((b) => b.type === 'tool_result')
    expect(use && res && use.type === 'tool_use' && res.type === 'tool_result').toBeTruthy()
    if (use?.type === 'tool_use' && res?.type === 'tool_result') {
      expect(res.tool_use_id).toBe(use.id)
    }
  })

  it('비-JSON 라인(stderr 경고)을 스킵', () => {
    const input =
      'Warning: no stdin data received\n' +
      '{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}'
    const msgs = parseStreamJson(input)
    expect(msgs).toHaveLength(1)
    expect(msgs[0]).toEqual({ role: 'assistant', blocks: [{ type: 'text', text: 'hi' }] })
  })

  it('메타 라인(system/result/rate_limit_event)은 메시지로 잡지 않음', () => {
    const input =
      '{"type":"system","subtype":"init"}\n' +
      '{"type":"result","subtype":"success"}\n' +
      '{"type":"rate_limit_event"}'
    expect(parseStreamJson(input)).toHaveLength(0)
  })
})
