// ADR-0102: bounded retry (backoff) helper for the boot pull — the frontend-side safety net for the early invoke race.
//
// ★Why it is needed★: the main window has no event recovery path (window:tabs-updated fires only on tab *mutation* —
//   it does not come for the static state right after boot). So if the list_tabs/get_view pull at boot fails once (e.g. the moment managed state is
//   not yet ready due to a Tauri v2 boot race), that is the end and the screen is permanently stuck on the loading placeholder. LayoutState now
//   removes the race structurally with pre-build manage (the root fix), but the pull must still not be
//   one-shot — it must recover by itself from other early transients too (a momentary not-ready state of runtime-dependent state such as DaemonClient,
//   IPC initialization delay).

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
  // ADR-0102 (FIX-4): re-check cancellation right before the exhaustion throw — if unmounted *during* the last attempt failing,
  //   the guard at the top of the loop above has already been passed, so the backend error is thrown as is and the call site logs a pointless
  //   final failure (unmount is not a failure). If cancelled here, turn it into RetryCancelledError so the call site quietly
  //   ignores it. ★Limit★: cancellation in the middle of an in-flight fn() · a backoff sleep is not caught here (the isCancelled
  //   transition at that moment is not reflected until the next checkpoint) — it only removes the spurious log at the final attempt boundary.
  if (opts.isCancelled?.()) throw new RetryCancelledError()
  throw lastErr
}
