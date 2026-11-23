# ADR-0410: Approval questions go to the app while it is attached with a PC list slot open, else the OS dialog; no visibility or focus test

- 상태: 확정 (2026-11-23, 근거: user decision 2026-11-23 (「그냥 앱이 있냐없냐만 구분하면 되지 않음?」 · 「묻히는건 알아서 유저가 해야지」 · 「일단 작업표시줄 깜박임으로 한다고 치고 당장은 하지마」))
- 관련: Amends ADR-0394 (decision 5 app route rule) · ADR-0406 (dialog text) · `docs/process/S37-multi-daemon/trd.md` (U4) · `docs/tracking.md` T-126 (taskbar flash, tray launcher)

## 맥락
ADR-0394 decision 5 routes an "ask the user" question to the app while it shows a PC list slot, defined as visible = in the active tab of a non-minimized window. The U4 brief made it stricter (active tab of the focused window, a 30 s fallback to the OS dialog while the app shows the question, a 1 s delay before reporting "no longer seen"). Both need the app to keep reporting visibility changes to the daemon. The user asked why the route does not just tell whether the app is there.

## 결정
1. **Route** = the app while it is attached and has at least one PC list slot mounted in any window — tab active or not, window focused or not, behind other windows or minimized; otherwise the OS approval dialog; with no display the request waits as before. The app's report changes only on slot mount / unmount and on detach.
2. **No visibility or focus test, no timed fallback** — a question shown in an app the user is not looking at is the user's to find. The reroute to the dialog when the app detaches or the last slot closes (ADR-0394 decision 5) stays.
3. **Attention** — when the app gets a new question it flashes its taskbar button rather than restoring the window. Agreed as the direction and deferred to T-126; not part of S37.
4. The rest of ADR-0394 decision 5 (one shared seam, one dialog at a time, off the async tasks) is unchanged.

## 거부한 대안
- **Visible = active tab of a non-minimized window** (ADR-0394 decision 5 as written) — the app must report every tab switch and minimize to the daemon; user rejected directly (「최소화까지 감지하려면 별도로 데몬에 보내야되잖아」).
- **Focused window + 30 s fallback to the OS dialog + 1 s "no longer seen" delay** (the U4 round-2 code) — more reporting and two timers for the same goal; user rejected directly (「묻히는건 알아서 유저가 해야지」).
- **The app attached at all, slot or not** — with no PC list slot open the question has no place to be answered and would wait up to 24 h (the answer UI lives in that slot — code fact); the main narrowed the user's "app present or not" to "a slot is open". The user may still choose otherwise.
- **Restore / un-minimize the window on a new question** — Windows' foreground lock often leaves a restored window behind others, and it pops a window over the user's work; messengers such as Slack and Discord flash the taskbar instead (named from memory, not re-checked — 약함, 사용자 직접 기각: the user chose the flash).

## 근거
- User decision 2026-11-23 (quotes in the status line).
- Simpler reporting: mount / unmount only, no focus or visibility tracking, no timers on the daemon side.

## 영향 / 불변식
- U4 drops the focus / visibility tracking, the 30 s fallback and the 1 s delay; the frontend reports PC list slot presence only.
- A question can sit unseen in a hidden or minimized app; that is accepted. The taskbar flash (T-126) is the planned remedy.
