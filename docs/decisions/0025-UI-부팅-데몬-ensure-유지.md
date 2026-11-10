# ADR-0025: Keep the one UI-boot daemon ensure — discard ADR-0024 C3 ("no UI ensure")

- 상태: 확정 (2026-06-18, dashboard7 session — user decision) · 부분 폐기 by ADR-0350 (부팅 1회 ensure 범위와 죽은 데몬 되살리기 수단)
- 관련: ADR-0024 (C3 is the discard target) · ADR-0023 (topology) · ADR-0021 (on-demand · no restart) · CLAUDE.md §5 · `src/api/clientFactory.ts` (bootstrapDaemonIfNeeded) · `src/App.tsx` · Amends ADR-0024 (C3 UI ensure ban) · Amended by ADR-0350 (부팅 1회 ensure 범위와 죽은 데몬 되살리기 수단)
- Scope: the decision on "may the UI start the daemon" in daemon mode. It reverses only C3 of ADR-0024 (C1 · C2 · C4 · data location remain valid).

## 맥락
ADR-0024 C3 decided "the daemon desired-state is owned by the owner (tray-host); the UI must not ensure directly, only connect" and said to remove the current `bootstrapDaemonIfNeeded` (the daemon ensure at UI boot). User review showed that this is the opposite of the intent — **the state "the UI is on but there is no daemon" is a contradiction in the user's mental model** (UI = monitor, daemon = workshop; if you turn on the monitor, the workshop must of course exist).

## 결정
- **The UI ensures the daemon once at boot (keep `bootstrapDaemonIfNeeded`).** "Opening the UI = the daemon comes along automatically" is the user's intent.
- **Automatic means "once at boot," not "revive on death."** If the daemon dies (by an external kill, etc.), the UI does not restart it automatically — the command path (`ensureReady`) is attach-only, so it cannot wake the daemon (ADR-0021). The tray icon shows daemon life and death as active/inactive, and revival is only by an explicit "데몬 켜기" / rerunning the UI. ← This is the point where the "restarts even when I block it" the user suffered does not recur.
- **There are two daemon ensure actors (the tray-host "데몬 켜기" + the UI boot).** The daemon is a singleton (lockfile, ADR-0024 C2), so whoever ensures, it is a no-op if it is alive → multiple ensures are safe. The "single desired-state owner" was meant to block the multiple-ensure race that ADR-0024 C3 worried about, but the singleton lockfile already neutralizes that race, so there is no need to *remove* the UI ensure.
- **The respawn-during-exit race is blocked by C4 (not by C3).** On receiving the full-exit signal (reason=full_shutdown), the UI blocks reconnect/ensure (ADR-0024 C4 kept). In phase 1, full exit is a taskkill (both end at the same time), so there is no race at all; the precise guard is phase 3.

## 거부한 대안
- **The original ADR-0024 C3 (remove the UI ensure entirely, only the tray-host ensures):** running the UI alone would leave the state "daemon off," which sits head-on against the user's intent (UI = the daemon comes along automatically). The "multiple-ensure race" that justified C3 is neutralized by the daemon singleton lockfile (C2), so there is no reason to pay even the cost of removing the ensure wholesale. → Discarded.
- **The UI auto-restarting the daemon when it dies (supervision):** the user explicitly rejected it ("only when I bring the UI up once"). It is also consistent with discarding auto-restart (ADR-0021).

## 근거
- The user's direct decision (dashboard7 session). The current code (`bootstrapDaemonIfNeeded` + the attach-only `ensureReady`) already works as intended → the code change concludes as "do not remove."
- The singleton guarantee depends on ADR-0024 C2 (the lockfile{PID · port · generation} stale check). If that guarantee breaks, the multiple-ensure race comes back, so C2 must be implemented in phase 3.

## 영향 / 불변식
- **ADR-0024 C3 is discarded.** The C3 line in the ADR-0024 body is marked as discarded.
- Delete the item "remove the daemon ensure in the UI bootstrap" from the phase-1 plan (the boot ensure in `bootstrapDaemonIfNeeded` · `App.tsx` stays). The `clientFactory` default-mode flip (embedded→daemon) proceeds as is.
- The "does not restart even if I block it" invariant is load-bearing on **"the UI ensure is once at boot + the command path is attach-only"**. If the command path is later changed to wake the daemon, this invariant breaks.
- Blocking respawn during exit (C4) is still mandatory — implement precisely in phase 3.
