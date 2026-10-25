// TRD S26-window-chrome §7-2: 창 끌 자리 = Tauri `data-tauri-drag-region`(Q5 = ①). 누름 · 두 번 누름(최대화 토글)은 Tauri
//   `drag.js` 가 OS 에 넘긴다 — 버스를 지나지 않는다(D7). 권한 = `core:window:allow-start-dragging`(main · 팝아웃 — §9).
// ★값 없는 속성 = 그 요소 자신을 누를 때만 끈다★ — 누른 요소부터 위로 걸으며 처음 만난 표시가 자기 자신이 아니면 끌지 않는다
//   (`drag.js` `isDragRegion`). 그래서 자식 탭 머리 · `+` · 단추는 끌기를 시작하지 않는다. `"deep"` 으로 바꾸면 탭 머리
//   누름이 창 끌기가 된다.
// ★CSS `app-region: drag` 로 바꾸지 말 것★ — 그 영역의 적중 판정이 최상위 창(HWND)으로 가 파일만 받는 tao 의 놓기 받는 쪽에
//   닿아, 그 위에 놓은 탭이 받는 쪽 없이 끝난다(디버그 세션 실측 2026-10-24).
// ★끌 자리 누름은 document 의 거품 단계 `mousedown` 에 닿지 않는다★ — `drag.js`(tauri 2.11.3 `src/window/scripts/drag.js`)가
//   초기화 스크립트로 가장 먼저 document 에 거품 단계 `mousedown` 을 걸고, 끌 자리의 왼쪽 누름이면 `preventDefault` 와
//   `stopImmediatePropagation` 을 부른다. 그래서 그 뒤에 건 document · window 의 거품 단계 `mousedown` 이 불리지 않고, 기본
//   동작(포커스 옮김)도 없어 입력 칸이 포커스를 잃지 않는다. 바깥 누름으로 닫는 쪽은 캡처 단계(`addEventListener(…, true)`)로
//   듣는다 — `pointerdown` 은 막지 않는다.

import { useEffect, type RefObject } from 'react'

const DRAG_REGION_ATTR = 'data-tauri-drag-region'
const DRAG_REGION = { [DRAG_REGION_ATTR]: '' } as const

export function dragProps(): typeof DRAG_REGION {
  return DRAG_REGION
}

/** 누른 요소가 끌 자리인가 — 값 없는 표시만 쓰므로 그 요소 자신이 표시를 단 경우다(자식 위 누름은 끌 자리가 아니다). */
function isDragRegionTarget(target: EventTarget | null): boolean {
  return target instanceof Element && target.hasAttribute(DRAG_REGION_ATTR)
}

/**
 * `editing` 인 동안 창 끌 자리를 누르면 입력 칸 `input` 의 포커스를 뺀다 — 그 누름은 포커스를 옮기지 않으므로(머리) 편집이
 * 포커스 잃음으로 확정되는 칸은 이것 없이는 끌 자리 누름에 끝나지 않는다. 확정 자체는 그 칸의 blur 처리가 한다. 캡처 단계라
 * `drag.js` 보다 먼저 듣는다.
 */
export function useBlurOnDragRegionPress(editing: boolean, input: RefObject<HTMLInputElement | null>): void {
  useEffect(() => {
    if (!editing) return
    const onDown = (e: MouseEvent): void => {
      if (isDragRegionTarget(e.target)) input.current?.blur()
    }
    document.addEventListener('mousedown', onDown, true)
    return () => document.removeEventListener('mousedown', onDown, true)
  }, [editing, input])
}
