// A key that does not accept tab drags of another build (dev · release) — asks the webview for Tauri `getIdentifier()` and holds it in lowercase (Chromium lowercases the drag data
//   type). A drag started before the answer is not opened — a drag without a key cannot be read as ours even by our window, so it does not accept
//   the drop. The source (`useTabDragSource`) and the receiving side (`dock/useWindowDrop`) read this value, one per webview.

import { retryAsync } from '../../../util/retryInvoke'

export interface AppIdentifier {
  /** Lowercase identifier — `null` = no answer yet (meanwhile no drag is ours). */
  current(): string | null
  /**
   * Ask — no-op if asking or an answer has been received. Failure gets a bounded retry (ADR-0102), and if all of those fail too, it asks again on the next call.
   * An empty answer is not asked again (asking again gives the same setting).
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
