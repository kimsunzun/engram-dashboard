// ADR-0237 (U6 「칸 안 어디든」): 칸 안 버튼이 눌린 뒤 사라지거나 잠기면 포커스가 문서 body 로 떨어져, 그 칸의 Esc 가
//   칸 루트의 capture 에 닿지 않는다(끊기가 무반응이 된다). 그 버튼이 쥔 포커스를 가장 가까운 「클릭으로 포커스를
//   받는 칸 컨테이너」(tabindex -1 — 스크롤 뷰포트 · RichSlot 루트)로 옮긴다.

/**
 * `button` 이 지금 포커스를 쥐었을 때만 옮긴다 — 안 쥐었으면 잃을 것이 없고, 다른 칸의 포커스를 빼앗으면 그 칸 대신
 * 이 칸의 턴이 Esc 에 끊긴다. 받을 조상이 없으면 아무것도 하지 않는다. 버튼의 동작 뒤(같은 처리기 안)에 부른다.
 */
export function keepFocusInSlot(button: HTMLElement): void {
  if (button.ownerDocument.activeElement !== button) return
  button.parentElement?.closest<HTMLElement>('[tabindex="-1"]')?.focus({ preventScroll: true })
}
