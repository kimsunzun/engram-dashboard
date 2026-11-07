// TRD S26-window-chrome §7-2 · §9: draggable area attribute and its permission. If the permission is missing, a press on the draggable area silently does nothing (W4) —
//   type checks · render tests cannot see it, so it is measured by reading the capability source. The forbidden-permission side is `src/__tests__/noWindowControl.test.ts`.

import { describe, expect, it } from 'vitest'

import mainCapability from '../../../../src-tauri/capabilities/default.json?raw'
import popupCapability from '../../../../src-tauri/capabilities/popup.json?raw'
import { dragProps } from './dragRegion'

describe('dragRegion', () => {
  it('속성은 값 없는 data-tauri-drag-region 하나 — `drag.js` 의 「자기 자신만」 꼴', () => {
    expect(dragProps()).toEqual({ 'data-tauri-drag-region': '' })
  })

  it('끌 자리를 그리는 두 창(main · 팝아웃)의 capability 가 core:window:allow-start-dragging 을 준다', () => {
    for (const [name, raw, windows] of [
      ['default.json', mainCapability, ['main']],
      ['popup.json', popupCapability, ['slot-popup-*']],
    ] as const) {
      const cap = JSON.parse(raw) as { windows: string[]; permissions: unknown[] }
      expect(cap.windows, name).toEqual(windows)
      expect(cap.permissions, name).toContain('core:window:allow-start-dragging')
    }
  })
})
