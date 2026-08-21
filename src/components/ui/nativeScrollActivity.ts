//! nativeScrollActivity — seam 밖 **네이티브** 스크롤 컨테이너에 ScrollArea 와 같은 스크롤바 가시성
//! 규칙을 입히는 설치기 (ADR-0053 "예외 = xterm").
//!
//! ★왜 있나★: 앱의 스크롤 표면은 전부 `ScrollArea` seam(Radix, `type="scroll"`)을 지나 **스크롤 중에만**
//!   스크롤바를 보여 준다(멈추면 `SCROLL_HIDE_DELAY_MS` 뒤 숨김, hover 에는 무반응). xterm 터미널만은
//!   자체 `.xterm-viewport` 를 소유해 그 컴포넌트로 감쌀 수 없어(ADR-0053) 지금까지 **상시 표시**
//!   네이티브 스크롤바였다 — 두 출력 표면의 룰이 갈렸다(사용자 지적: "터미널은 호버링만 해도 나온다").
//!
//! ★무엇을 하나★: 스크롤이 일어나는 동안만 스크롤 노드에 `data-scroll-active` 를 붙인다. thumb 를
//!   그 표식으로 게이트하는 쪽은 CSS(`src/index.css` 의 `.xterm .xterm-viewport` 규칙)다 — 여기는
//!   "지금 스크롤 중인가" 라는 **상태만** 만들고 그림은 만들지 않는다.
//!   왜 CSS 만으로 안 되나: 네이티브 스크롤바는 DOM 요소가 아니라 Radix 처럼 붙였다 뗄 수 없고, CSS 에는
//!   "지금 스크롤 중" 상태 선택자가 없다(hover·overflow 여부는 있으나 둘 다 다른 룰이다).
//!
//! ★스크롤 이벤트의 출처를 가리지 않는다 — 의도★: 프로그램적 `scrollTop` 대입(터미널의 출력 시 하단
//!   고정)도 사용자 휠과 같게 "스크롤 중" 으로 센다. Radix `type="scroll"` 도 정확히 그렇게 동작하며
//!   (RichSlot 의 하단 고정 auto-scroll 이 seam 의 스크롤바를 띄우는 것을 실측 — 2026-08-21), 여기서만
//!   출처를 가리면 두 표면이 스트리밍 중에 서로 다르게 보인다.
//!
//! ★스크롤 *거동*은 건드리지 않는다★ — 이 모듈은 이벤트를 읽기만 하고 preventDefault·scrollTop 을
//!   쓰지 않는다. 바뀌는 것은 "스크롤바가 언제 보이나" 뿐이다.

import { SCROLL_HIDE_DELAY_MS } from './scroll-area'

/**
 * 표식을 붙일 대상 = seam 밖 네이티브 스크롤러. 현재 xterm viewport 하나뿐이다(ADR-0053 예외 명단).
 * 여기에 선택자를 더하기 전에 그 표면이 `ScrollArea` seam 으로 갈 수 없는지 먼저 확인할 것 —
 * 감쌀 수 있으면 seam 이 정답이고 이 목록은 늘지 않는다.
 */
const NATIVE_SCROLLER_SELECTOR = '.xterm-viewport'

/** CSS(`index.css`)가 thumb 가시성을 게이트하는 표식. 이름을 바꾸면 그 규칙도 함께 바꿔야 한다. */
export const SCROLL_ACTIVE_ATTR = 'data-scroll-active'

/**
 * 문서 전체에 스크롤 감시를 건다. 반환값 = 해제기(설치 지점의 effect cleanup / HMR 에서 중복 누적 방지).
 *
 * `capture: true` 가 필수다 — `scroll` 이벤트는 **버블하지 않는다**. 캡처 단계 리스너만 문서 한 곳에서
 * 모든 스크롤 노드를 볼 수 있고, 그래야 xterm 인스턴스의 생성·폐기 수명에 이 코드가 끼어들지 않는다
 * (터미널 슬롯의 구독·replay·resize 배선을 건드리지 않는다는 뜻 — 그쪽은 다른 이유로 민감하다).
 */
export function installNativeScrollActivity(doc: Document = document): () => void {
  // 노드별 숨김 타이머. 스크롤이 이어지는 동안 계속 미뤄지고, 마지막 스크롤 뒤 한 번만 발화한다.
  const timers = new Map<Element, ReturnType<typeof setTimeout>>()

  const onScroll = (e: Event): void => {
    const el = e.target
    // Document 스크롤(target = document)·비대상 스크롤러는 그냥 흘려보낸다.
    if (!(el instanceof Element) || !el.matches(NATIVE_SCROLLER_SELECTOR)) return

    el.setAttribute(SCROLL_ACTIVE_ATTR, '1')

    const pending = timers.get(el)
    if (pending !== undefined) clearTimeout(pending)
    timers.set(
      el,
      setTimeout(() => {
        timers.delete(el)
        el.removeAttribute(SCROLL_ACTIVE_ATTR)
      }, SCROLL_HIDE_DELAY_MS),
    )
  }

  doc.addEventListener('scroll', onScroll, true)

  return () => {
    doc.removeEventListener('scroll', onScroll, true)
    // 표식은 남기지 않는다 — 해제 후에도 붙어 있으면 스크롤바가 영구히 보이는 상태로 굳는다.
    for (const [el, id] of timers) {
      clearTimeout(id)
      el.removeAttribute(SCROLL_ACTIVE_ATTR)
    }
    timers.clear()
  }
}
