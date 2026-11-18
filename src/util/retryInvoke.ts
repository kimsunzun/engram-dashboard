// ADR-0102: bounded retry (backoff) helper for the boot pull — the frontend-side safety net for the early invoke race.
// ★Why★: the main window has no event recovery path (window:tabs-updated fires only on tab *mutation*, not for the static state right after boot), so a list_tabs/get_view pull failing once at boot (e.g. managed state not ready in a Tauri v2 boot race) would leave the screen stuck on the loading placeholder forever. LayoutState now removes that race structurally with pre-build manage, but the pull must still not be one-shot — other early transients (DaemonClient not ready, IPC init delay) need self-recovery.

/** The defaults are conservative values fit for the boot pull (short and a few tries — do not block the boot UX for long). */
export interface RetryOptions {
  attempts?: number
  baseDelayMs?: number
  /** Default 2 → total wait ~1s (boot perceived upper bound). */
  factor?: number
  onRetry?: (err: unknown, attempt: number) => void
  /** For the unmount guard. */
  isCancelled?: () => boolean
}

/** A sentinel to tell apart from a normal failure — the call site quietly ignores it. */
export class RetryCancelledError extends Error {
  constructor() {
    super('retry cancelled')
    this.name = 'RetryCancelledError'
  }
}

const sleep = (ms: number): Promise<void> => new Promise(res => setTimeout(res, ms))

/**
 * If every attempt fails, throw the last error — it must not be swallowed silently so the call site surfaces the final failure.
 */
export async function retryAsync<T>(fn: () => Promise<T>, opts: RetryOptions = {}): Promise<T> {
  const attempts = opts.attempts ?? 4
  const baseDelayMs = opts.baseDelayMs ?? 150
  const factor = opts.factor ?? 2

  let lastErr: unknown
  for (let i = 0; i < attempts; i++) {
    if (opts.isCancelled?.()) throw new RetryCancelledError()
    try {
      return await fn()
    } catch (err) {
      lastErr = err
      const isLast = i === attempts - 1
      if (isLast) break
      opts.onRetry?.(err, i + 1)
      await sleep(baseDelayMs * factor ** i)
    }
  }
  // ADR-0102 (FIX-4): re-check cancellation right before the exhaustion throw — an unmount *during* the last failing attempt has already passed the loop-top guard, so the backend error would be thrown and the call site would log a pointless final failure (unmount is not a failure); if cancelled here, throw RetryCancelledError instead. ★Limit★: cancellation during an in-flight fn() · a backoff sleep is not caught until the next checkpoint — this only removes the spurious log at the final attempt boundary.
  if (opts.isCancelled?.()) throw new RetryCancelledError()
  throw lastErr
}
