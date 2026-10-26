// ADR-0304 — 스냅 오버레이 feature
// TRD S26-window-chrome §8-4: 스냅 레이아웃 오버레이 — 셸이 그것을 켠 빌드(창 사실 `caption.snap_overlay` — 셸 기능
//   `snap-overlay` · 기본 꺼짐 · 사용자 결정 Q1 2026-10-24)일 때만 최대화 단추 사각형을 셸에 보고한다(invoke
//   `report_max_button_rect` — 그 invoke 는 그 빌드에만 등록된다). 셸은 그 자리에 OS 자식 창을 얹어 Windows 11 스냅 메뉴가
//   뜨게 하고(platform `window::snap`), 그 창이 커서를 가져가므로 맴돎 바뀜을 이 창에 `window:snap-hover`(몸 = bool)로
//   돌려준다 — 단추는 그 값으로 맴돎을 그린다(`CaptionButtons`). 꺼진 빌드에서는 아무것도 부르지도 듣지도 않는다.
// 사각형 = 이 창 클라이언트의 물리 px — CSS px 에 `devicePixelRatio` 를 곱해 바깥으로 맞춘다(단추를 다 덮게). 다시 재는 때 = 창
//   크기 바뀜(단추가 오른 끝에 붙어 자리가 옮는다) · 단추 크기 바뀜(최대화로 줄 높이가 바뀐다). ★배율만 바뀌고 CSS 크기가 그대로인
//   때(배율이 다른 모니터로 옮김)는 다시 재지 않는다★ — 켜는 날 잰다(TRD §14 W8).
// invoke/listen 을 직접 부르는 까닭 = 측정 보고라 버스 명령이 아니다(ADR-0227 · `minSizeReport.ts` 와 같다).

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useState, type RefObject } from 'react'

export const CMD_REPORT_MAX_BUTTON_RECT = 'report_max_button_rect'
export const EVT_SNAP_HOVER = 'window:snap-hover'

/** 클라이언트 물리 px 사각형 — 셸 `commands::snap_overlay::MaxButtonRect` 와 같은 모양. */
export interface PhysRect {
  x: number
  y: number
  width: number
  height: number
}

/** Tauri 와의 접점. `listen` 은 이 창으로 지목된 사건만 받고, 등록이 끝나면 푸는 함수로 풀린다. */
export interface SnapOverlayIpc {
  invoke(cmd: string, args: Record<string, unknown>): Promise<unknown>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

const tauriIpc: SnapOverlayIpc = {
  invoke: (cmd, args) => invoke(cmd, args),
  // ★`target` = 자기 label★ — 기본 타깃 `Any` 로 들으면 남의 창 맴돎을 제 것으로 쥔다(`dock/windowFrame.ts` 와 같다).
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
}

/** CSS px 사각형 → 물리 px 사각형(바깥으로 맞춤). 비었으면(너비 · 높이 0 — 단추가 안 보인다) `null` = 걷기. */
export function physicalRect(r: { left: number; top: number; right: number; bottom: number }, dpr: number): PhysRect | null {
  const x = Math.floor(r.left * dpr)
  const y = Math.floor(r.top * dpr)
  const width = Math.ceil(r.right * dpr) - x
  const height = Math.ceil(r.bottom * dpr) - y
  return width >= 1 && height >= 1 ? { x, y, width, height } : null
}

const keyOf = (r: PhysRect | null): string => (r === null ? '' : `${r.x},${r.y},${r.width},${r.height}`)

/**
 * `enabled` 인 동안 `button` 의 사각형을 보고하고(같은 값은 다시 보내지 않는다) 오버레이 맴돎을 돌려준다. 꺼지거나 떼어지면
 * 오버레이를 걷는다(`rect: null`). `enabled` 가 거짓이면 아무것도 부르지 않고 늘 거짓이다.
 */
export function useSnapOverlay(enabled: boolean, button: RefObject<HTMLElement | null>, ipc: SnapOverlayIpc = tauriIpc): boolean {
  const [hover, setHover] = useState(false)
  useEffect(() => {
    const el = button.current
    if (!enabled || el === null) return
    let disposed = false
    let sent: string | null = null
    const report = (rect: PhysRect | null): void => {
      ipc.invoke(CMD_REPORT_MAX_BUTTON_RECT, { rect }).catch(err => console.warn(`[snapOverlay] ${CMD_REPORT_MAX_BUTTON_RECT} 실패:`, err))
    }
    const measure = (): void => {
      const rect = physicalRect(el.getBoundingClientRect(), window.devicePixelRatio || 1)
      const key = keyOf(rect)
      if (key === sent) return
      sent = key
      report(rect)
    }
    measure()
    window.addEventListener('resize', measure)
    // jsdom 처럼 관측기가 없는 환경은 창 크기 바뀜만 듣는다.
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    ro?.observe(el)
    let off: (() => void) | undefined
    ipc.listen(EVT_SNAP_HOVER, payload => {
      if (!disposed) setHover(payload === true)
    }).then(
      unlisten => {
        if (disposed) unlisten()
        else off = unlisten
      },
      err => console.warn(`[snapOverlay] ${EVT_SNAP_HOVER} 구독 실패 — 단추가 맴돎을 못 그린다:`, err),
    )
    return () => {
      disposed = true
      window.removeEventListener('resize', measure)
      ro?.disconnect()
      off?.()
      setHover(false)
      // 걷기 — 이미 빈 사각형(걷기)을 보냈으면 다시 보내지 않는다.
      if (sent !== '') report(null)
    }
  }, [enabled, button, ipc])
  return enabled && hover
}
