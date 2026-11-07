// fileDropGuard unit test — blocks only file · URL drags and does not stop propagation (PRD AC22). jsdom has no DragEvent · DataTransfer, so
//   it issues a MouseEvent with a drag data field attached — whether the real WebView2 navigates is not covered by this test.

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { installFileDropGuard } from './fileDropGuard'

interface FakeDt {
  types: string[]
  dropEffect: string
}

function drag(type: string, types: string[]): { e: MouseEvent; dt: FakeDt } {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true })
  const dt: FakeDt = { types, dropEffect: 'unset' }
  Object.defineProperty(e, 'dataTransfer', { value: dt })
  document.body.dispatchEvent(e)
  return { e, dt }
}

let remove: (() => void) | null

beforeEach(() => {
  remove = installFileDropGuard()
})
afterEach(() => {
  remove?.()
})

describe('파일 막기 — 탐색을 부르는 끌기(파일 · URL)만', () => {
  it.each(['Files', 'text/uri-list'])(
    '%s 끌기 = dragenter · dragover preventDefault + dropEffect none · drop preventDefault · 끊지 않는다',
    type => {
      const winSeen: string[] = []
      const spy = (e: Event): void => {
        winSeen.push(e.type)
      }
      window.addEventListener('dragenter', spy)
      window.addEventListener('dragover', spy)
      window.addEventListener('drop', spy)
      try {
        // On the step where the element under the cursor changed, only dragenter arrives — so that releasing on that step does not navigate either.
        const enter = drag('dragenter', [type, 'text/plain'])
        expect(enter.e.defaultPrevented).toBe(true)
        expect(enter.dt.dropEffect).toBe('none')
        const over = drag('dragover', [type, 'text/plain'])
        const drop = drag('drop', [type, 'text/plain'])
        expect(over.e.defaultPrevented).toBe(true)
        expect(over.dt.dropEffect).toBe('none')
        expect(drop.e.defaultPrevented).toBe(true)
        expect(winSeen).toEqual(['dragenter', 'dragover', 'drop'])
      } finally {
        window.removeEventListener('dragenter', spy)
        window.removeEventListener('dragover', spy)
        window.removeEventListener('drop', spy)
      }
    },
  )

  it('글자 끌기는 건드리지 않는다 — 입력창에 글자를 끌어 놓는 기본 동작이 선다', () => {
    const enter = drag('dragenter', ['text/plain', 'text/html'])
    const over = drag('dragover', ['text/plain', 'text/html'])
    const drop = drag('drop', ['text/plain', 'text/html'])
    expect(enter.e.defaultPrevented).toBe(false)
    expect(enter.dt.dropEffect).toBe('unset')
    expect(over.e.defaultPrevented).toBe(false)
    expect(over.dt.dropEffect).toBe('unset')
    expect(drop.e.defaultPrevented).toBe(false)
  })

  it('떼면 더 막지 않는다', () => {
    remove?.()
    remove = null
    const over = drag('dragover', ['Files'])
    const drop = drag('drop', ['Files'])
    expect(over.e.defaultPrevented).toBe(false)
    expect(over.dt.dropEffect).toBe('unset')
    expect(drop.e.defaultPrevented).toBe(false)
  })
})
