// 도구 묶음 펼침 상태 시험(ADR-0239 · TRD S21-chat-ux §4-4).

import { beforeEach, describe, expect, it } from 'vitest'

import { effectiveOpen, useToolGroupStore } from './toolGroupStore'

const store = () => useToolGroupStore.getState()
const openOf = (slotId: string, key: string) => effectiveOpen(store(), slotId, key)

// zustand 싱글톤이라 시험 간 격리.
beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))

describe('toolGroupStore', () => {
  it('고른 값이 없으면 접힘', () => {
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(false)
  })

  it('고른 값이 두 방향 다 그대로 선다', () => {
    store().bind('s1', 'a1')
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
    expect(store().setOpen('s1', 'tool:y', false)).toBe(true)
    expect(openOf('s1', 'tool:x')).toBe(true)
    expect(openOf('s1', 'tool:y')).toBe(false)
  })

  it('슬롯마다 따로다', () => {
    store().bind('s1', 'a1')
    store().bind('s2', 'a1')
    store().setOpen('s1', 'tool:x', true)
    expect(openOf('s2', 'tool:x')).toBe(false)
  })

  it('한 번도 묶이지 않은 슬롯의 setOpen 은 false 를 돌려주고 아무것도 적지 않는다', () => {
    expect(store().setOpen('s1', 'tool:x', true)).toBe(false)
    expect(store().bySlot).toEqual({})
  })

  it('slotId 가 없으면 고른 값을 안 읽고 늘 접힘', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    expect(effectiveOpen(store(), undefined, 'tool:x')).toBe(false)
  })

  it('해제 뒤 setOpen 은 false — 고른 값은 남고, 같은 에이전트로 다시 마운트하면 그대로 붙는다', () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    release()
    expect(store().setOpen('s1', 'tool:y', true)).toBe(false)
    expect(store().bySlot.s1.open).toEqual({ 'tool:x': true })
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(true)
    expect(store().setOpen('s1', 'tool:y', true)).toBe(true)
  })

  it('두 번 해제해도 · 다시 마운트한 뒤의 옛 해제도 새 묶임을 풀지 않는다(StrictMode · 늦은 정리)', () => {
    const first = store().bind('s1', 'a1')
    first()
    const second = store().bind('s1', 'a1')
    first()
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
    second()
    second()
    expect(store().setOpen('s1', 'tool:x', false)).toBe(false)
  })

  it('해제 없이 다시 bind 해도(겹친 마운트) 앞 묶임의 해제는 뒤 묶임을 풀지 않는다', () => {
    const older = store().bind('s1', 'a1')
    store().bind('s1', 'a1')
    older()
    expect(store().setOpen('s1', 'tool:x', true)).toBe(true)
  })

  it('같은 에이전트로 다시 bind 하면 고른 값이 남는다 — 재마운트 · 재구독', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    store().bind('s1', 'a1')
    expect(openOf('s1', 'tool:x')).toBe(true)
  })

  it('다른 에이전트로 bind 하면 그 슬롯 칸을 비운다 — 해제된 칸이어도', () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    release()
    store().bind('s1', 'a2')
    expect(store().bySlot.s1).toMatchObject({ agentId: 'a2', open: {} })
    expect(openOf('s1', 'tool:x')).toBe(false)
  })

  it('clear 는 고른 값만 비우고 묶임은 남긴다 — 뒤 setOpen 이 그대로 먹힌다', () => {
    store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    store().clear('s1')
    expect(store().bySlot.s1).toMatchObject({ agentId: 'a1', open: {} })
    expect(store().setOpen('s1', 'tool:y', true)).toBe(true)
  })

  it('clear · setOpen · 해제가 바꿀 것이 없으면 상태 객체를 새로 만들지 않는다(구독자 재렌더 없음)', () => {
    const release = store().bind('s1', 'a1')
    store().setOpen('s1', 'tool:x', true)
    const before = store().bySlot
    store().setOpen('s1', 'tool:x', true)
    expect(store().bySlot).toBe(before)
    release()
    const released = store().bySlot
    release()
    expect(store().bySlot).toBe(released)
    store().bind('s1', 'a1')
    store().clear('s1')
    const cleared = store().bySlot
    store().clear('s1')
    store().clear('nope')
    expect(store().bySlot).toBe(cleared)
  })

  it('프로토타입 이름(슬롯 id · 키)을 칸으로 읽지 않는다', () => {
    expect(store().setOpen('constructor', 'tool:x', true)).toBe(false)
    expect(openOf('toString', 'tool:x')).toBe(false)
    store().bind('s1', 'a1')
    expect(openOf('s1', 'constructor')).toBe(false)
  })
})
