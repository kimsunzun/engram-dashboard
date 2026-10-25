// TRD S26-window-chrome §7-2: 창 끌 자리 = Tauri `data-tauri-drag-region`(Q5 = ①). 누름 · 두 번 누름(최대화 토글)은 Tauri
//   `drag.js` 가 OS 에 넘긴다 — 버스를 지나지 않는다(D7). 권한 = `core:window:allow-start-dragging`(main · 팝아웃 — §9).
// ★값 없는 속성 = 그 요소 자신을 누를 때만 끈다★ — 누른 요소부터 위로 걸으며 처음 만난 표시가 자기 자신이 아니면 끌지 않는다
//   (`drag.js` `isDragRegion`). 그래서 자식 탭 머리 · `+` · 단추는 끌기를 시작하지 않는다. `"deep"` 으로 바꾸면 탭 머리
//   누름이 창 끌기가 된다.
// ★CSS `app-region: drag` 로 바꾸지 말 것★ — 그 영역의 적중 판정이 최상위 창(HWND)으로 가 파일만 받는 tao 의 놓기 받는 쪽에
//   닿아, 그 위에 놓은 탭이 받는 쪽 없이 끝난다(디버그 세션 실측 2026-10-24).

const DRAG_REGION = { 'data-tauri-drag-region': '' } as const

export function dragProps(): typeof DRAG_REGION {
  return DRAG_REGION
}
