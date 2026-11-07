# ADR-0039: Frontend event subscription teardown — subscribeViewEvents returns dispose + ready synchronously

- 상태: 확정 (2026-06-28, basis: cross-family OSS research + opus/Codex adversarial review)
- 관련: `src/store/viewStore.ts` (subscribeViewEvents) · `src/store/eventBus.ts` (initEventBus) · `docs/research/async-subscribe-cleanup-race-2026-06-28.md` · ADR-0035 (layout authority = src-tauri) · CLAUDE.md 「프론트 구조」

## 맥락
In the S14 layout slice, `subscribeViewEvents` was `async (): Promise<() => void>` — it returned the disposer only *after* finishing the registration of the two emits (layout:updated/view:list-updated) with `await Promise.all([listen(...), listen(...)])`. Its caller `eventBus.initEventBus` received that disposer with `unlistenFns.push(await subscribeViewEvents())`.

The problem: if cleanup (HMR `import.meta.hot.dispose` / re-init) runs `unlistenFns.forEach(fn=>fn())` while the registration `await` is pending (or before the HMR dispose callback is registered), the viewStore disposer is not yet in `unlistenFns` at that point (before the push), so it is left out of the cleanup, and the listener whose registration completes when the await finishes late **leaks permanently**. The `if (disposed)` guard we had was a **dead branch** — the only code that sets `disposed` to true is the returned disposer, and the disposer reaches the caller only after the await, so `disposed` can never become true during the await. In other words, the race the guard was meant to block could structurally never touch the guard. (Confirmed by cross-family adversarial verification: opus + Codex.)

The runtime risk is low (the trigger is limited to dev HMR — without `import.meta.hot.accept` it is usually a full reload, and there is no path in a prod build). But a false comment ("this leak is blocked") is a design defect that makes the next session mistake it as *solved*, so we close it.

## 결정
Change `subscribeViewEvents` into a **synchronous function** `(): { dispose: () => void; ready: Promise<void> }`.
- It returns `dispose` immediately on the call → the caller pushes it to `unlistenFns` at once without a registration `await` (the core of blocking the leak). The registration (the two `listen()` calls) starts in the background.
- `ready` (Promise<void>) resolves after both registrations settle → the caller awaits it and then does the follow-up init (F-listen preserved: preventing a loss of emits that arrive before registration completes).
- The internal `adopt(fn)` receives a freshly registered unlisten handle — if `disposed`, it *calls it at once* (releasing a late-arriving handle), otherwise it keeps it in `handles`. `dispose` only empties `handles` by calling them (idempotent).

The caller `eventBus` changes too: ① push `dispose` to `unlistenFns` *before* awaiting ready ② **register the `import.meta.hot.dispose` callback also before awaiting ready** (guaranteeing a dispose call path even for HMR while ready is pending — without this, even pushing dispose early is void because nobody is there to call it) ③ wrap `await ready` in try/catch and, on reject, call `viewSub.dispose()` to clean up the partial success + isolate from the agentClient subscription (the ADR-0011 domain).

## 거부한 대안
- **Async return + the `if (disposed)` guard (existing)** — a dead branch. Since the disposer is returned after the await, there is no path to set `disposed` during the await, so the guard never fires. It cannot block the leak while falsely claiming "blocked" in a comment → cannot be adopted.
- **AbortController/AbortSignal** — Tauri `listen()` takes no signal argument, so we would have to put our own ignore-flag logic inside, with no extra gain. Moreover the return of `abort()` is "delivery of the intent to cancel", not "a guarantee that cleanup is complete" (cross-family agreement). It would be preferred if we were designing a new subscription API, but it is unsuitable in our case of *wrapping* a non-abortable Promise API (listen).
- **Async dispose (`dispose(): Promise<void>`, no ready)** — it can cancel and can wait for completion, but it does not satisfy "the cancel handle must be in hand *immediately*, without await" (the core requirement for blocking a leak during the registration await). If we await to receive dispose, the gap in between is itself the leak window.

## 근거
- **OSS standard (converged, cross-family):** RxJS `Subscription` · zustand `subscribe` · Node `EventEmitter` · React `useSyncExternalStore` all return the teardown handle *synchronously* — to remove the uncancellable window between registration and the first emission. The TC39 Observable proposal rejected `Promise<Subscription>` (an async return) for its unbounded buffer + uncancellable window. Details and sources = `docs/research/async-subscribe-cleanup-race-2026-06-28.md`.
- **Codex verdict:** "When wrapping a non-abortable Promise API (Tauri listen) and both a synchronous cancel handle + awaiting registration completion are needed, `{dispose, ready}` is the clean solution."
- **Found by adversarial review:** round 1 — the existing guard confirmed a dead branch. Round 3 (FIX) — Codex found that "the HMR dispose callback is registered *after* the ready await, so with HMR while ready is pending the leak remains" (opus did not see it, the value of cross-family). The fix moved the callback before ready and closed it.
- **Measured:** at boot, the 1 empty slot of the default View renders normally as `ViewLayoutRenderer` (confirmed `Slot … — empty` in `list_views` + the main canvas via cdp `eval`) → no regression in the subscribe → ready → init flow.

## 영향 / 불변식
- The `subscribeViewEvents` signature = `{ dispose: () => void; ready: Promise<void> }` (synchronous). The caller must **secure dispose immediately without await** and register it in the cleanup path.
- **Four contracts (violating them causes a leak/hang):** ① `dispose` is idempotent (double-dispose is a noop) ② if `dispose` is called before `ready`, `adopt` calls a late-arriving handle at once (regardless of cancelled — not calling = a leak) ③ no `ready` hang (it ends normally even when dispose comes first or a registration rejects) ④ when `ready` rejects, cleaning up the partial success is the **caller's responsibility to call dispose**.
- **eventBus integration invariants:** both the dispose push and the `import.meta.hot.dispose` callback registration finish **before `await ready`**. `await ready` is isolated with try/catch (a layout subscription failure does not block the agentClient subscription). The F-listen order (`initFromBackend` after registration completes) is kept.
- If violated: delaying dispose until after ready, or registering the HMR callback after ready, brings back the leak of HMR while ready is pending. Initializing without awaiting ready loses emits before registration (violating F-listen).
