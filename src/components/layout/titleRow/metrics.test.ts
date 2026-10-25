// TRD S26-window-chrome §7-7 의 최소 창 크기 셈 — 왼 여백 + 활성 탭 52 + `+` 상자 26 + 끌 빈 자리 42 + 번호 표식 + 단추 3 × 46 ·
//   높이 = 보통 상태 줄 높이 34 + 칸 최소 30.

import { describe, expect, it } from 'vitest'

import { minWindowSize } from './metrics'

describe('minWindowSize', () => {
  it.each([
    ['한 자리(글자 폭 + 양옆 8 이 최소 28 보다 좁다)', 23, 286],
    ['두 자리', 30.5, 289],
    ['세 자리', 37, 295],
  ])('번호 표식 %s — 표식 폭 %s → 폭 %s · 높이 64', (_name, markerW, width) => {
    expect(minWindowSize({ leadingInset: 0, markerW, buttons: true })).toEqual({ width, height: 64 })
  })

  it('재지 못한 표식(0 폭)도 최소 28 로 친다', () => {
    expect(minWindowSize({ leadingInset: 0, markerW: 0, buttons: true }).width).toBe(286)
  })

  it('대기 모드(표식 · 단추 없음) = 120 · 단추를 OS 가 그리면 단추 몫을 빼고 · 왼 여백은 더한다', () => {
    expect(minWindowSize({ leadingInset: 0, markerW: null, buttons: false })).toEqual({ width: 120, height: 64 })
    expect(minWindowSize({ leadingInset: 0, markerW: 28, buttons: false }).width).toBe(148)
    expect(minWindowSize({ leadingInset: 80, markerW: 28, buttons: true }).width).toBe(366)
  })
})
