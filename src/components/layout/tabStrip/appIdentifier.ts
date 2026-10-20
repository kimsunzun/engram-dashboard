// 다른 판(개발 · 배포)의 탭 끌기를 받지 않는 열쇠 — Tauri `getIdentifier()` 를 웹뷰에 묻고 소문자로 쥔다(Chromium 이 끌기 자료
//   타입을 소문자로 바꾼다). 답 전에 시작한 끌기는 열지 않는다 — 열쇠 없는 끌기는 우리 창도 우리 것으로 못 읽어 놓기를 받지
//   않는다. 출처(`useTabDragSource`)와 받는 쪽(`dock/useWindowDrop`)이 웹뷰마다 하나인 이 값을 읽는다.

import { retryAsync } from '../../../util/retryInvoke'

export interface AppIdentifier {
  /** 소문자 identifier — `null` = 아직 답이 없다(그동안 어떤 끌기도 우리 것이 아니다). */
  current(): string | null
  /**
   * 묻는다 — 묻는 중이거나 답을 받았으면 무동작. 실패는 유계 재시도(ADR-0102)하고, 그것도 다 실패하면 다음 부름에 다시 묻는다.
   * 빈 답은 다시 묻지 않는다(되풀어도 같은 설정이다).
   */
  ask(): void
}

export function createAppIdentifier(get: () => Promise<string>): AppIdentifier {
  let id: string | null = null
  let asked = false
  return {
    current: () => id,
    ask() {
      if (asked) return
      asked = true
      void retryAsync(get, {
        onRetry: (err, attempt) => console.warn(`[appIdentifier] getIdentifier 재시도 #${attempt}:`, err),
      }).then(
        got => {
          if (typeof got === 'string' && got.length > 0) id = got.toLowerCase()
          else console.warn('[appIdentifier] getIdentifier: 빈 답 — 탭 끌기를 열지 않는다:', got)
        },
        err => {
          asked = false
          console.error('[appIdentifier] getIdentifier 최종 실패 — 다음 탭바 · 창 마운트에 다시 묻는다:', err)
        },
      )
    },
  }
}
