# ADR-0102: Preventing the boot race — register managed state before build + the frontend boot pull retries (eradicating the main window's endless loading)

- 상태: 확정 (2026-07-23, basis: a boot-order survey + user decision)
- 관련: ADR-0057(layout/window state) · ADR-0100(release packaging — release/ is rebuilt to pick up this fix) · `src-tauri/src/lib.rs`(manage location) · `src/components/layout/WindowLayout.tsx`·`src/store/viewStore.ts`(boot pull) · `src/store/eventBus.ts` · ADR-0283(2026-10-08 — ★the premise of the 맥락 and the 영향, 「webview 가 setup 전에 invoke 할 수 있다」, no longer holds for the static windows (main · agent-tree)★: both windows are also left at `"create": false` and are created by boot step ⑧ of the user setup, so the first moment a window exists is inside setup. ★Decision 1 (builder manage) is still right★ — if it is moved into setup, one has to keep by hand that it comes before ⑧, which creates the windows, whereas if it sits on the builder it exists before any window regardless of the order inside setup (the `// ADR-0102` comment in `lib.rs`). Read the 영향 line 「setup() 안으로 되돌리면 이 레이스가 재발한다」 as what happens when that hand-kept order is broken. This is an update of a premise, not an amendment of the decision)

## 맥락
A phenomenon where, when the release exe runs, the main window **sticks permanently** at the frontend loading placeholder ("창 로딩 중… (label: main)"). The diagnosis:
- The backend layout is normal — the main window is always seeded at boot by `ViewManager::new()` and there is no disk persistence. On a rerun `list_tabs('main')` returns OK immediately and the real UI comes up (the original failure is transient). 0 panic/poison in the log → mutex poison and lock deadlock are ruled out.
- The root cause (likely) = a **Tauri v2 boot race**: while `builder.build()` is running, the webview loads the frontend JS/React and may fire `invoke('list_tabs')` early, *before* the `app.manage(LayoutState)` in `setup()` completes. If it lands in that narrow window, Tauri returns a managed-state-does-not-exist error.
- The reason this transient failure becomes a **permanent stick** (certain) = the frontend only does `console.warn` on a boot pull failure and **does not retry**, and the main window afterwards cannot receive `window:tabs-updated` (which fires only on tab changes), so **there is no recovery path**.
- The reason it was mainly observed in release (uncertain · hypothesis): in release the frontend is a minified bundle embedded in the exe (`tauri.localhost`, in-process), so it loads fast → fires the invoke early, while in dev the first HTTP load from the Vite dev server (`localhost:1420`) is slow, so `setup` usually finishes first. The exact error of the original spark could not be captured at that moment (it was an instance without a debug port or logs and it disappeared on rerun).

## 결정
1. **Register deterministic managed state before build.** Move `LayoutState` (no runtime needed · deterministic) from `app.manage(...)` inside `setup()` to **`builder.manage(LayoutState::new())` (before `.build()`)** → the state is guaranteed to exist before the webview comes up, so an early invoke cannot meet "no state" (removing the race class by construction).
2. **The frontend boot pull retries + surfaces.** Wrap the `WindowLayout` mount pull and the `list_tabs` of `initMainWindowFromBackend` (and the `get_view` that follows) in a **bounded retry/backoff**, and do not swallow the final failure but surface it (more than a console warning — user-visible/diagnosable). Transient failures of other early invokes (runtime-dependent state etc.) also self-recover, and if it fails non-transiently the exact error is left behind, serving as instrumentation to catch the root cause.
3. **Confidence:** (1) is valid whether or not the original spark was this race (it structurally removes a race class that really exists). Retry (2) is a safety net + diagnosis for the remaining transient boot failures.

## 거부한 대안
- **`visible:false`, then `show()` after setup finishes** — dropped. Heavier (window display delay · flicker), and it only hides the main window, while other early invokes (other managed state) are still exposed. `manage` before build is more fundamental and general.
- **Frontend retry only (backend unchanged)** — dropped. It masks only the symptom and the race stays. Give priority to removal by construction (1) and run retry in parallel as a defense.
- **Moving even `DaemonClient` before build** — impossible/not a goal. DaemonClient needs the tokio runtime, so it has to be created in `setup()`. Transient failures of an early invoke that touches that state are covered by the frontend retry (2).

## 근거
- Boot-order survey (read-only): `app.manage(LayoutState)` is inside `setup()` (`lib.rs`), and Tauri v2 loads the webview during `build()`, confirming that an early invoke can outrun setup. Lock deadlock and panic-poison are ruled out by code and logs.
- Consistent with every observation: transient · success on rerun · 0 poison/panic logs · capture failure (the very first moment of boot) · release-prone (fast embedded frontend load).

## 영향 / 불변식
- **★Deterministic managed state that an early boot invoke touches is registered before `.build()`★** — moving it back into `setup()` brings this race back. A `// ADR-0102` anchor comment is mandatory at the moved point (preventing rot · regression in the next session).
- **The main window state has no event recovery path** → the boot pull must not be one-shot (retry is mandatory). `window:tabs-updated` fires only on tab changes, so it cannot make up for an initial failure.
- **release/ must be rebuilt:** this fix is a source change, so to reflect it in the distribution `release/` exe it must be rebuilt with `scripts/build-release.ps1` (ADR-0100). Until the rebuild, the release exe is still the old code.
- Verification: the code gate + a frontend retry unit test (success after N failures → win filled in) + a measurement after the release rebuild that the exe boot reaches the real UI.
