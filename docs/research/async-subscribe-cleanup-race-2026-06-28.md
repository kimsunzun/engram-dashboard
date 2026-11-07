# Research — async subscribe cleanup race (leak when dispose arrives during the registration await)

- **Status:** done (cross-family medium: Claude Sonnet fan-out of 3 branches + Codex blind 1 → opus cross-check/adversarial)
- **Date:** 2026-06-28
- **Motivation:** Adversarial verification established that the dispose-during-await guard in `subscribeViewEvents` of the S14 layout slice is a **dead branch**. The design basis for closing the real leak (if cleanup arrives during the registration `await`, the listener leaks permanently) in line with OSS practice.
- **Confidence legend:** (certain) many primary/official sources converge · (likely) agreed on, but only some primary sources · (uncertain) single source / knowledge-based.

## Problem

If cleanup/unmount/dispose is called **before** an async subscription registration (`subscribe`/`listen` returning `Promise<Unlisten>`) finishes its `await`, the caller has not yet received the unlisten handle → a listener whose registration completes late **leaks permanently**.

Our code (`src/store/viewStore.ts` `subscribeViewEvents` + `src/store/eventBus.ts:75` `await subscribeViewEvents()`) has this shape. The existing `if (disposed)` guard is dead, because the disposer is returned *after* the `await`, so there is no path that could set `disposed`.

## Findings (confidence · source)

- **Returning a synchronous teardown handle is the industry standard.** RxJS `Subscription` (unsubscribe is idempotent · "a late unsubscribe is harmless"), zustand `subscribe` (synchronous unsub), Node `EventEmitter` (synchronous on/off), React `useSyncExternalStore` (requires subscribe to return the unsub synchronously) — all synchronous. Reason: it removes the *non-cancellable window* between registration and the first emission. (certain)
  - https://rxjs.dev/guide/subscription · https://react.dev/reference/react/useSyncExternalStore · https://github.com/tc39/proposal-observable/issues/163 (`Promise<Subscription>` rejected as an anti-pattern: unbounded buffer + non-cancellable window)
- **The standard React race fix = an `ignore`/`cancelled` flag (closure-local).** An `isMounted` ref is an anti-pattern (conflicts with concurrent mode). (certain) https://react.dev/learn/synchronizing-with-effects · https://react.dev/reference/react/useEffect
- **But unsubscribing differs from ignoring a fetch result — an unlisten that arrives late must be called even if cancelled.** If it is not called, it leaks. (likely) https://github.com/tauri-apps/tauri/issues/8913
- **Tauri `listen()` does not internally prevent the unmount-before-registration race.** The UnlistenFn is created only after `invoke('plugin:event|listen')` resolves (packages/api/src/event.ts). The official cleanup example is `promise.then(fn => fn())`, but the race window remains. (certain) https://v2.tauri.app/develop/calling-frontend/ · https://github.com/tauri-apps/tauri/issues/8913
- **`@tauri-apps/api` 2.6.0+ (PR #13306): calling unlisten releases the listener immediately and synchronously.** Earlier v2.0.x had unlisten as an async IPC, so a separate race. (This is a *different* layer from our registration-await race, but worth checking the dependency version.) (certain) https://github.com/tauri-apps/tauri/pull/13306 · https://github.com/tauri-apps/tauri/issues/8916
- **AbortController "conveys the intent to cancel"; it does not "guarantee cleanup is complete".** The return of `abort()` ≠ cleanup done. Tauri `listen()` does not accept a signal, so wrapping it would still need the same internal ignore-flag logic in the end → introducing AbortSignal brings no extra benefit. (certain) https://developer.mozilla.org/en-US/docs/Web/API/AbortController
- **`Symbol.asyncDispose`/`await using` (TS 5.2+) is the modern standard for async cleanup** — but if the only requirement is "hold the cancel handle immediately, without an await", a separate adoption is unnecessary. (likely)

## Cross-verification table (Claude ↔ Codex)

| Claim | Claude | Codex | Decision |
|---|---|---|---|
| A synchronous teardown handle is the standard | ✓ | ✓ | converged (certain) |
| Tauri listen does not defend against the race internally | ✓(#8913) | ✓(event.ts implementation) | converged (certain) |
| unlisten must be called even if cancelled | ✓(Claude B) | ✓(`disposed?fn():…`) | converged |
| Is `{dispose, ready}` a standard signature? | no (only a conceptual resemblance) | no | converged |
| Is `{dispose, ready}` appropriate for our case? | conditionally appropriate | "clean for wrapping Tauri listen" | converged |
| Is AbortSignal more standard? | same limits | preferred for a new API, unnecessary for a wrapper | converged |

## Our application (decision basis)

Make `subscribeViewEvents` return `{ dispose: () => void, ready: Promise<void> }` synchronously. eventBus pushes `dispose` into `unlistenFns` immediately without an `await` (zero leaks even if cleanup arrives during registration) + awaits `ready`, then `initFromBackend` (F-listen kept).

**Required contract (avoid the pitfalls, agreed by both families):**
1. `dispose` is idempotent (double-dispose is a noop).
2. If `dispose` is called before `ready`, **call the late-arriving unlisten handle immediately** (regardless of cancelled — if it is not called, it leaks).
3. `ready` must not hang (it ends normally even on dispose/registration failure).
4. On `ready` reject, clean up the partial registrations (zero leaks).

## Gaps / limits

- The **actual trigger probability of our leak is low** (dev HMR + no `import.meta.hot.accept` → usually a full reload · no path in prod). But it is a design defect, so it is worth fixing.
- The current `@tauri-apps/api` version (package.json) was not checked — below 2.6.0 the unlisten IPC race (#8916) exists separately. It is a different layer from our fix scope (the registration-await race).
