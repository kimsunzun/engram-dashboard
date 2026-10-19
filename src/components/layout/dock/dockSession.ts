// 셸이 이 창 웹뷰에 보내는 끌기 세션 사건의 받는 쪽(TRD S21-slot-docking §3 · §4 · §6-3). 진입점 = `watchDockSession` — 창마다
//   하나를 걸면 셋이 선다:
//   - 창 모드(`dock:spare` + `window_role`) — 대기 창 명부의 창(예비 창 · 끄는 중 비워 남긴 창)은 대기 모드(빈 틀)로, 나머지는
//     보통으로 그린다(§2-5).
//   - 앱 전체 키 감시(`dock:drag`) — 끌기 세션이 `active` 인 동안 이 창의 keydown 하나마다 `tab_drag_end{src: key, at: null}`
//     하나를 보내고(`Escape` = `cancel` · 그 밖 = `drop` · 키 반복 · 수정키도 — Chrome `EventTracker`), 그 키는 어느 슬롯에도
//     닿지 않는다. 그 세션의 `ended` 로 끈다.
//   - 셸이 미는 끌기 입력(`dock:feed`) — 받은 몸을 `subscribeFeed` 구독자에게 넘긴다.
// 세 듣기 다 `target` = 자기 label 로 건다 — 몸에 창 label 이 없고, 기본 타깃 `Any` 로 걸면 남의 창으로 보낸 사건까지 받는다
//   (`src/commands/viewCommandBridge.ts` 의 같은 조항). 셸은 `dock:drag` 를 안내 창 밖 우리 웹뷰 전부에(떼어 들인 예비 창에는
//   그 세션의 `active` 를 한 번 더 — 끌기가 시작된 뒤에 지어졌을 수 있다), `dock:feed` 를 탭을 쥔 창 하나에 보낸다. 듣기 셋을
//   다 건 뒤에 `window_role` 을 묻는다 — 그 사이에 온 사건을 놓치지 않게.
//
// 탭바 끌기 훅(`useTabDrag`)과 맞물리는 법:
//   - 키 — 탭을 쥔 창의 훅은 `window` capture 에서 keydown 을 받아 `stopPropagation` 하고 자기 자리를 실은 끝을
//     `sendDragEnd(keyEnd(session, e.key, at))` 로 보낸다. 이 감시는 `document` capture 라 그 키를 보지 않는다 — 감시는 `at` 을
//     모르는 나머지 창 몫이다 — 그 `drop` 은 탭바 단계면 셸이 놓기 확인 `dock:feed{release}` 로 탭을 쥔 창에 자리를 묻고, 따라오는
//     창 단계(이동 루프)면 루프를 끝내고 그 끝 커서 자리로 놓는다.
//   - 입력 — `subscribeFeed`: `attach`(이 창 탭바에 붙었다) · `move` · `release`(→ `tab_drag_end{drop, release, at}` 로 답한다).
//   - 끝 — `subscribeDrag`: `ended` 를 받으면 그 세션의 탭바 끌기를 거둔다(다른 창의 키 · 셸이 끝낸 끌기).

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { SpareEvent } from '../../../../src-tauri/bindings/SpareEvent'
import type { TabDragEnd } from '../../../../src-tauri/bindings/TabDragEnd'
import type { TabDragEvent } from '../../../../src-tauri/bindings/TabDragEvent'
import type { TabDragFeed } from '../../../../src-tauri/bindings/TabDragFeed'
import type { WindowRoleReply } from '../../../../src-tauri/bindings/WindowRoleReply'

/** `parked` = 대기 모드(탭바 + 빈 캔버스 · 0탭 자가닫힘 끔 · 탭 끌어오기 없음 · 재기 보고는 그대로) · `normal` = 보통. */
export type WindowMode = 'parked' | 'normal'

export const EVT_SPARE = 'dock:spare'
export const EVT_DRAG = 'dock:drag'
export const EVT_FEED = 'dock:feed'
export const CMD_WINDOW_ROLE = 'window_role'
export const CMD_TAB_DRAG_END = 'tab_drag_end'

/** 키 끝 — `Escape` = 되돌리기(`cancel`) · 그 밖의 키 = 놓기(`drop`). `at` = 탭을 쥔 창만 싣는 지금 자리 · 다른 창은 `null`. */
export function keyEnd(session: number, key: string, at: number | null): TabDragEnd {
  return { session, how: key === 'Escape' ? 'cancel' : 'drop', src: 'key', at }
}

/** `tab_drag_end` 를 보낸다 — 답을 기다리지 않는다(실패는 경고 로그만). */
export function sendDragEnd(end: TabDragEnd): void {
  invoke<void>(CMD_TAB_DRAG_END, { args: end }).catch(err => {
    console.warn(`[dockSession] ${CMD_TAB_DRAG_END} 실패:`, err)
  })
}

// 구독 하나마다 항목을 따로 둔다 — 같은 함수를 두 번 걸어도 한쪽 해제가 다른 쪽을 지우지 않는다.
const feedListeners = new Set<{ cb: (feed: TabDragFeed) => void }>()
const dragListeners = new Set<{ cb: (event: TabDragEvent) => void }>()

/**
 * 셸이 이 창에 미는 끌기 입력(`dock:feed` 의 몸 그대로 — `session` · 끄는 탭 `view` · `step`)을 받는다. `watchDockSession` 이
 * 걸려 있는 동안만 온다. 반환 = 구독 해제(여러 번 불러도 된다).
 */
export function subscribeFeed(cb: (feed: TabDragFeed) => void): () => void {
  const entry = { cb }
  feedListeners.add(entry)
  return () => {
    feedListeners.delete(entry)
  }
}

/**
 * 끌기 세션이 섰다 · 끝났다(`dock:drag` 의 몸 그대로)를 받는다 — 키 감시를 켜고 끈 뒤에 부른다. 같은 세션의 `active` 가 거듭 올
 * 수 있다(들인 예비 창에 한 번 더). `watchDockSession` 이 걸려 있는 동안만 온다. 반환 = 구독 해제(여러 번 불러도 된다).
 */
export function subscribeDrag(cb: (event: TabDragEvent) => void): () => void {
  const entry = { cb }
  dragListeners.add(entry)
  return () => {
    dragListeners.delete(entry)
  }
}

/**
 * 이 창의 끌기 세션 사건을 받기 시작한다(머리 주석의 셋). 창 모드는 `onMode` 로 알린다 — 첫 값 = `window_role` 답, 그 뒤 =
 * `dock:spare`. 같은 값을 거듭 알릴 수 있다. `window_role` 이 실패하면 `normal` 이다(대기 모드 이전과 같은 그리기).
 * 정리 뒤에는 알리지도 키를 받지도 않는다.
 *
 * @returns 정리 함수 — 등록을 기다리지 않고 곧바로 돌려준다.
 */
// ADR-0295
export function watchDockSession(onMode: (mode: WindowMode) => void): () => void {
  let disposed = false
  const unlistens: UnlistenFn[] = []
  const label = getCurrentWindow().label
  let keys: { session: number; stop: () => void } | null = null

  const watchKeys = (session: number): void => {
    if (keys?.session === session) return
    keys?.stop()
    // ★`document` capture★ — 탭을 쥔 창의 훅이 `window` capture 에서 먼저 받아 끊을 자리를 남긴다(머리 주석).
    const onKey = (e: KeyboardEvent): void => {
      e.preventDefault()
      e.stopPropagation()
      sendDragEnd(keyEnd(session, e.key, null))
    }
    document.addEventListener('keydown', onKey, true)
    keys = { session, stop: () => document.removeEventListener('keydown', onKey, true) }
  }
  const stopKeys = (): void => {
    keys?.stop()
    keys = null
  }

  const onDrag = (event: TabDragEvent): void => {
    if (event.phase === 'active') watchKeys(event.session)
    else if (keys?.session === event.session) stopKeys()
    notify(dragListeners, event, EVT_DRAG)
  }
  const onFeed = (feed: TabDragFeed): void => notify(feedListeners, feed, EVT_FEED)

  const listenHere = async <T>(event: string, handler: (payload: T) => void, lost: string): Promise<void> => {
    try {
      const off = await listen<T>(
        event,
        e => {
          if (!disposed) handler(e.payload)
        },
        { target: label },
      )
      if (disposed) {
        off()
        return
      }
      unlistens.push(off)
    } catch (err) {
      console.error(`[dockSession] ${event} 구독 실패 — 이 창은 ${lost}:`, err)
    }
  }

  void (async () => {
    await Promise.all([
      listenHere<SpareEvent>(
        EVT_SPARE,
        p => onMode(p.step === 'parked' ? 'parked' : 'normal'),
        '대기 모드 바뀜을 못 받는다',
      ),
      listenHere<TabDragEvent>(EVT_DRAG, onDrag, '끄는 중 키로 끌기를 끝내지 못한다'),
      listenHere<TabDragFeed>(EVT_FEED, onFeed, '붙은 탭 끌기 입력을 못 받는다'),
    ])
    if (disposed) return
    let mode: WindowMode = 'normal'
    try {
      const reply = await invoke<WindowRoleReply>(CMD_WINDOW_ROLE)
      if (reply.role === 'parked') mode = 'parked'
    } catch (err) {
      console.warn(`[dockSession] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, err)
    }
    if (!disposed) onMode(mode)
  })()

  return () => {
    disposed = true
    for (const off of unlistens) off()
    unlistens.length = 0
    stopKeys()
  }
}

// 구독자 하나가 던져도 나머지 구독자는 받는다.
function notify<T>(listeners: Set<{ cb: (value: T) => void }>, value: T, event: string): void {
  for (const { cb } of [...listeners]) {
    try {
      cb(value)
    } catch (err) {
      console.error(`[dockSession] ${event} 구독자 오류:`, err)
    }
  }
}
