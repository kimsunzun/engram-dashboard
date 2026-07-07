// ADR-0051 / ADR-0053: rail run-position 순수 함수 단위테스트. StructuredTextView 에서 분리한 pure util
//   (railPositions.ts)의 top/mid/bottom/single 및 skip/boundary 처리를 검증한다(React 무관 — 순수 로직).

import { describe, expect, it } from 'vitest'

import { computeRailRunPositions, type ChatRowKind } from './railPositions'

describe('computeRailRunPositions (ADR-0051 dot-rail clean-ends)', () => {
  it('assistant 한 행만 있으면 single(고립 dot — 연결선 없음)', () => {
    expect(computeRailRunPositions(['assistant'])).toEqual(['single'])
  })

  it('연속 assistant 3행 → top/mid/bottom', () => {
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'mid', 'bottom'])
  })

  it('boundary(user 버블/separator)가 run 을 끊는다', () => {
    // a a | boundary | a  →  두 run: [top,bottom] 과 [single]. boundary 는 null.
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'boundary', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'bottom', null, 'single'])
  })

  it('skip(usage/흡수 tool_result)은 run 을 끊지 않는다(DOM 없음 → 시각적 인접)', () => {
    // a skip a  →  skip 을 무시하면 두 assistant 는 인접 run → top/bottom. skip 은 null.
    const kinds: ChatRowKind[] = ['assistant', 'skip', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', null, 'bottom'])
  })

  it('boundary 로 시작/끝나는 혼합 시퀀스', () => {
    // boundary a a boundary a a a
    const kinds: ChatRowKind[] = [
      'boundary',
      'assistant',
      'assistant',
      'boundary',
      'assistant',
      'assistant',
      'assistant',
    ]
    expect(computeRailRunPositions(kinds)).toEqual([
      null,
      'top',
      'bottom',
      null,
      'top',
      'mid',
      'bottom',
    ])
  })

  it('전부 skip 이면 위치 없음(all null)', () => {
    expect(computeRailRunPositions(['skip', 'skip'])).toEqual([null, null])
  })

  it('선행 skip 은 top 을 무너뜨리지 않는다(맨 앞 assistant 는 여전히 top)', () => {
    const kinds: ChatRowKind[] = ['skip', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual([null, 'top', 'bottom'])
  })
})
