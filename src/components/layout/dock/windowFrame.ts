// TRD S21-slot-docking §3-8: 이 창의 창 사실(복원 안쪽 크기 · 최대화 · 틀 inset · 앞 창 · 크롬 모양 — 뒤 둘은 TRD
//   S26-window-chrome §6-5) — 마운트 때 `window_frame` 으로 한 번 당기고,
//   그 뒤는 셸이 값이 바뀔 때만 미는 `window:frame` 을 받아 마지막 값 하나만 쥔다. 출처 웹뷰가 dragstart 에 원래 크기 · 새 창
//   크기를 셈하는 재료다(R7 · R22). 끌기 밖의 통신이라 끄는 동안에도 막지 않고 받는다(ADR-0297) — 끌기는 dragstart 에 그때
//   값을 한 번 읽을 뿐이다.
// ★틀 inset 을 웹에서 `outer* − inner*` 로 셈하지 않는다★ — WebView2 에서 그 값이 창 틀과 맞는지 미검이라 셸이 셈한 값만 쓴다.
// invoke/listen 을 직접 부르는 까닭 · 끊는 법은 `tabDrop.ts` 와 같다(셸이 주인 · 버스 명령 아님 · ADR-0012).

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { CaptionChrome } from '../../../../src-tauri/bindings/CaptionChrome'
import type { Insets } from '../../../../src-tauri/bindings/Insets'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import type { WindowFrame } from '../../../../src-tauri/bindings/WindowFrame'
import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export type { WindowFrame }

export const CMD_WINDOW_FRAME = 'window_frame'
export const EVT_WINDOW_FRAME = 'window:frame'

/** Tauri 와의 접점. `listen` 은 이 창으로 지목된 사건만 받고, 등록이 끝나면 푸는 함수로 풀린다. */
export interface WindowFrameIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

export interface WindowFrameSource {
  /** 마지막으로 받은 값 — 아직 못 받았으면 `null`(그동안 대상은 크기 없음으로 친다 · TRD §4-2). */
  current(): WindowFrame | null
  /** 사건 듣기를 단 뒤 한 번 당긴다. 반환 = 떼기. */
  install(): () => void
}

function finite(x: unknown): x is number {
  return typeof x === 'number' && Number.isFinite(x)
}

function sizeOf(raw: unknown): SizeCss | null | undefined {
  if (raw === null) return null
  if (typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return finite(v.w) && finite(v.h) ? { w: v.w, h: v.h } : undefined
}

function insetsOf(raw: unknown): Insets | undefined {
  if (raw === null || typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return finite(v.t) && finite(v.r) && finite(v.b) && finite(v.l) ? { t: v.t, r: v.r, b: v.b, l: v.l } : undefined
}

function captionOf(raw: unknown): CaptionChrome | undefined {
  if (raw === null || typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return typeof v.buttons === 'boolean' && finite(v.leading_inset) && typeof v.snap_overlay === 'boolean'
    ? { buttons: v.buttons, leading_inset: v.leading_inset, snap_overlay: v.snap_overlay }
    : undefined
}

/** 모양이 계약과 다르면 `null`. */
function frameOf(raw: unknown): WindowFrame | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  const restored = sizeOf(v.restored)
  const insets = insetsOf(v.insets)
  const caption = captionOf(v.caption)
  if (
    restored === undefined ||
    insets === undefined ||
    caption === undefined ||
    typeof v.maximized !== 'boolean' ||
    typeof v.focused !== 'boolean'
  )
    return null
  return { restored, maximized: v.maximized, insets, focused: v.focused, caption }
}

export function createWindowFrame(ipc: WindowFrameIpc): WindowFrameSource {
  let frame: WindowFrame | null = null
  // 받은 `window:frame` 의 수 — 당기기 답은 그것을 낸 뒤 사건이 하나라도 왔으면 버린다.
  let pushes = 0

  return {
    current: () => frame,

    install() {
      let disposed = false
      let off: (() => void) | undefined
      const isCancelled = () => disposed

      void (async () => {
        try {
          const unlisten = await retryAsync(
            () =>
              ipc.listen(EVT_WINDOW_FRAME, raw => {
                if (disposed) return
                const next = frameOf(raw)
                if (next === null) {
                  console.warn(`[windowFrame] ${EVT_WINDOW_FRAME}: 모양이 계약과 다르다 — 버린다:`, raw)
                  return
                }
                pushes += 1
                frame = next
              }),
            {
              isCancelled,
              onRetry: (err, attempt) => console.warn(`[windowFrame] ${EVT_WINDOW_FRAME} 구독 재시도 #${attempt}:`, err),
            },
          )
          if (disposed) {
            unlisten()
            return
          }
          off = unlisten
        } catch (err) {
          if (err instanceof RetryCancelledError) return
          // 당기기는 그대로 낸다 — 사건을 못 받아도 마운트 때 값은 있어야 한다.
          console.error(`[windowFrame] ${EVT_WINDOW_FRAME} 구독 최종 실패 — 이 창은 창 사실 바뀜을 못 받는다:`, err)
        }
        if (disposed) return
        // ★구독을 단 **뒤에** 당긴다★ — 나란히 띄우면 당기기와 등록 사이의 바뀜을 놓쳐 옛 값에 머문다
        //   (`api/restoreClient.ts` 의 같은 조항).
        const seen = pushes
        try {
          const raw = await retryAsync(() => ipc.invoke<unknown>(CMD_WINDOW_FRAME), {
            isCancelled,
            onRetry: (err, attempt) => console.warn(`[windowFrame] ${CMD_WINDOW_FRAME} 재시도 #${attempt}:`, err),
          })
          // 당기기를 낸 뒤 사건이 왔으면 마지막 값은 사건이 나른다(셸은 바뀔 때마다 민다) — 답이 사건보다 늦게 닿아 옛 값으로
          //   덮지 않게 버린다.
          if (disposed || pushes !== seen) return
          const next = frameOf(raw)
          if (next === null) {
            console.warn(`[windowFrame] ${CMD_WINDOW_FRAME}: 모양이 계약과 다르다 — 버린다:`, raw)
            return
          }
          frame = next
        } catch (err) {
          if (err instanceof RetryCancelledError) return
          console.error(`[windowFrame] ${CMD_WINDOW_FRAME} 최종 실패 — 다음 ${EVT_WINDOW_FRAME} 까지 창 사실이 없다:`, err)
        }
      })()

      return () => {
        disposed = true
        off?.()
        off = undefined
      }
    },
  }
}

const tauriIpc: WindowFrameIpc = {
  // ★`target` = 자기 label★ — 몸에 창 label 이 없어, 기본 타깃 `Any` 로 들으면 남의 창 값을 제 것으로 쥔다(TRD §2-2).
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** 웹뷰마다 하나. */
export const windowFrame: WindowFrameSource = createWindowFrame(tauriIpc)
