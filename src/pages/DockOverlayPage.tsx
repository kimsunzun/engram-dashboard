// DockOverlayPage — 안내 창(label `dock-overlay` · 경로 `#/dock-overlay` — TRD S21-slot-docking §2-5)이 그리는 페이지.
//   셸이 대상 창 위에 덮어 도킹 띠 · 미리보기를 보이는 투명 · 클릭 통과 창이라 페이지 바닥도 투명이다. 그릴 내용은 셸이
//   `dock:overlay` 로 민다(`OverlayView` · 치울 때 `null`).

import { useEffect, useLayoutEffect, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { OverlayView } from '../../src-tauri/bindings/OverlayView'
import { DockOverlayView } from '../components/layout/dock/DockOverlay'

const EVT_OVERLAY = 'dock:overlay'

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

  const [guide, setGuide] = useState<OverlayView | null>(null)
  useEffect(() => {
    let disposed = false
    let off: UnlistenFn | null = null
    listen<OverlayView | null>(EVT_OVERLAY, e => setGuide(e.payload), { target: getCurrentWindow().label })
      .then(un => {
        if (disposed) un()
        else off = un
      })
      .catch(err => console.error(`[DockOverlayPage] ${EVT_OVERLAY} 구독 실패 — 안내를 그리지 못한다:`, err))
    return () => {
      disposed = true
      off?.()
    }
  }, [])

  return (
    <div data-testid="dock-overlay-page">
      <DockOverlayView guide={guide} />
    </div>
  )
}
