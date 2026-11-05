# ADR-0345: Reconnect action sits on the disconnected band, not a popup, so the app stays operable while disconnected

- 상태: 확정 (2026-11-05, 근거: 사용자 결정 2026-11-05 「재연결 버튼이 있어야지」 · 「일단 음.. 띠에두자. 연결안되도 클라는 조작되게.」 + 조사 `docs/research/reconnect-give-up-ui-2026-11-05.md` (light) · `docs/research/banner-action-buttons-2026-11-05.md` (light))
- 관련: Amends ADR-0180 (결정 2 예산 소진 팝업 자격과 결정 7 다시 연결 팝업) · ADR-0344 (same shape — one action button on the band) · ADR-0180 결정 4 (count 3 rests on this button) · ADR-0180 결정 8 (per-connection reconnect on the band — kept) · ADR-0342 (version mismatch on the band) · `src/components/layout/ConnectionNotice.tsx` · `src-tauri/src/daemon_client/connection.rs` (`MAX_RECONNECT_ATTEMPTS`) · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` D14 · §6-2 U5 · step-log S21

## 맥락

Today the shell retries the daemon connection 5 times (about 15.5 s) and then stays Down for good: the band shows the disconnected state with no action, and restarting the app is the only recovery. ADR-0180 planned the fix as a popup — decision 2 made budget exhaustion popup-eligible and decision 7 put a 「다시 연결」 button on that popup — and decision 4 lowered the retry count to 3 on the strength of that button. The popup was never built, so TRD D14 ① keeps injecting 5 until the button lands.

The transport rewrite (stage 3-2) now produces `GaveUp(BudgetExhausted)` from the new driver, so what the user sees after it had to be settled. Asked whether to keep today's behaviour for the refactor, the user answered that a reconnect button must exist (「재연결 버튼이 있어야지」) and asked for the open-source convention. After the survey the user chose the band over a popup so the app keeps working while disconnected (「일단 음.. 띠에두자. 연결안되도 클라는 조작되게.」).

## 결정

1. **When the reconnect budget is exhausted, the disconnected band carries one action button 「다시 연결」.** No popup. The rest of the app (layout, tabs, panes) stays operable while disconnected. Budget exhaustion leaves ADR-0180 decision 2's popup-eligible list.
2. **Pressing it starts a fresh attempt sequence with a reset budget** — not a single extra try. (Main's call from the survey: both surveyed libraries restart from scratch on a manual reconnect after give-up.) The mechanism (re-add vs `reconnect_now`) is the TRD's.
3. **Per connection.** With several daemons (stage 3-3), each lost connection's band entry gets its own button — ADR-0180 decision 8 already put per-connection reconnect on the band.
4. **The retry count drops to 3 in the same change that lands the button** (ADR-0180 decision 4 · TRD D14 ①). Until then the shell keeps injecting 5.
5. **LLM path:** the same action is reachable through the existing command surface (`__engramCmd`), per CLAUDE.md 「LLM-우선 제어」.
6. **Not decided here:** whether a "retry now" button also shows during the retrying phase (before give-up).

## 거부한 대안

- **Popup with the button (ADR-0180 decision 7 as written)** — a popup covers the app while disconnected; the user wants the client operable without a connection. It would also need a new reusable popup component (none exists — ADR-0180 맥락). 강도: 약함 — 사용자 직접 기각.
- **No button; keep today's permanent Down band (restart the app)** — the user rejected it outright (「재연결 버튼이 있어야지」). It also leaves ADR-0180 decision 4's count of 3 meaning only "give up sooner" (ADR-0180 decision 7). 강도: 약함 — 사용자 직접 기각.

## 근거

- Survey `reconnect-give-up-ui-2026-11-05.md` (light, raw sources re-read by main): SignalR JS stops after four attempts (0, 2, 10, 30 s) and leaves recovery to the app — its manual pattern calls `start` again from `onclose`; socket.io retries forever by default and resets its backoff when a finite budget gives up. Neither ships UI. App examples (VS Code Remote, Slack, Jupyter) are snippet-only and do not settle banner vs popup.
- Survey `banner-action-buttons-2026-11-05.md`: one action button on a dismissible top banner is conventional (WinUI `InfoBar` sample "Reconnect to continue working."). ADR-0344 already gives the band this shape for the version-mismatch stop action.

## 영향 / 불변식

- **The band shows at most one action at a time** (ADR-0344): stop (version mismatch, daemon older than the app) or reconnect (budget exhausted) — the two states do not overlap.
- **Change the count everywhere it lives, together with the button:** `connection.rs` `MAX_RECONNECT_ATTEMPTS` and the 「5회」 prose in `src/api/agentClient.ts` and `src/api/daemonControl.ts` (ADR-0180 영향; `src/api/wsTransport.ts` is the non-production direct path).
- **Landing unit:** proposed for stage 3-2 U5 alongside ADR-0344's button (same band, same files); the TRD records the final unit.
- Main's inference, not a decision: with this ADR and ADR-0342, no connection event remains popup-eligible under ADR-0180 decision 2, so decision 8's "popup only when every daemon is lost" has nothing to judge until a new popup case appears.
