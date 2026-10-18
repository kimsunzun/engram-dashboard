// DockOverlayPage — 안내 창(label `dock-overlay` · 경로 `#/dock-overlay` — TRD S21-slot-docking §2-5)이 그리는 페이지.
//   셸이 대상 창 위에 덮어 도킹 띠 · 미리보기를 보이는 투명 · 클릭 통과 창이라 페이지 바닥도 투명이다. 아직 그리지
//   않는다 — `dock:overlay` 를 받아 그리는 것은 TRD §13 U7b-2.

import { useLayoutEffect } from 'react'

// ADR-0295
export default function DockOverlayPage() {
  // `index.css` 가 html · body · #root 에 바닥색을 깐다 — 이 창만 인라인으로 걷는다(인라인이 시트를 이긴다). 첫 페인트
  //   전에 걷어야 깔린 바닥이 대상 창 위에 비치지 않아 레이아웃 effect 다.
  useLayoutEffect(() => {
    const els = [document.documentElement, document.body, document.getElementById('root')]
    for (const el of els) el?.style.setProperty('background', 'transparent')
    return () => {
      for (const el of els) el?.style.removeProperty('background')
    }
  }, [])

  return <div data-testid="dock-overlay-page" />
}
