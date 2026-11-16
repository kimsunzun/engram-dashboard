# ADR-0344: Version-mismatch band carries one confirmed stop action, no automatic restart

- 상태: 확정 (2026-11-05, 근거: 사용자 결정 2026-11-05 「행동 버튼하나 둬 그럼」 + 조사 `docs/research/banner-action-buttons-2026-11-05.md` (light)) · 부분 폐기 by ADR-0350 (결정 4와 5의 안내 문구) · 부분 폐기 by ADR-0356 (결정 1에서 4의 띠 끄기 버튼)
- 관련: Amends ADR-0180 (결정 1 띠는 알리기만 한다) · Amends ADR-0343 (결정 1 재시작 행동과 결정 4 새 데몬 자동 시작) · ADR-0287 (band layer and ✕ close — unchanged) · ADR-0342 (version mismatch on the existing connection-failure surface) · ADR-0158 결정 3 (no automatic kill — kept) · `src/components/layout/ConnectionNotice.tsx` · `src-tauri/src/daemon_client/stop.rs` · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2 U5 · step-log S21 · Amended by ADR-0350 (결정 4와 5의 안내 문구) · Amended by ADR-0356 (결정 1에서 4의 띠 끄기 버튼)

## 맥락

ADR-0343 gave the version-mismatch case a user-pressed 「데몬 재시작」 action that stops the old daemon and starts the new one. In discussion (2026-11-05) the user narrowed it: the app must only inform, and the user decides — 「앱버전이 안맞는다고 데몬을 끄는게 맞아? 안내만 해야되는거 아님?」. Main then moved the action to the tray, reading ADR-0180 decision 1 (band = inform, popup = act) as the band convention; the user pointed out that "convention" means open-source/industry convention, not the project's own rule (「프로젝트 관례 말고 … 오픈소스 관례 말하는거잖아」), and asked for that.

The survey (light) found an action button on a dismissible top banner is conventional: WinUI `InfoBar` has one action-button slot and its own samples are "Update available — restart the application" and "Reconnect to continue working" (certain); libadwaita `AdwBanner` has one button, KDE `KMessageWidget` N actions, GitHub Primer `Banner` primary + secondary (likely). No source addresses a destructive action in a banner; WinUI says to use a dialog "for scenarios where the application is confirming a user action".

## 결정

1. **The version-mismatch band carries exactly one action button: 「데몬 끄기」**, next to the cause text and the ✕ close. The band stays dismissible (ADR-0287).
2. **Pressing it asks for confirmation first** — the old daemon's running agents stop (they can be resumed later; the running turn is lost). Main's judgement, not a sourced rule.
3. **On confirm, the old daemon is stopped as ADR-0343 decisions 2–3 describe** (graceful stop with the daemon's own advertised version, fallback identity-checked kill). **Nothing starts automatically** — the user starts the new daemon by restarting the app (or the tray 「데몬 켜기」).
4. **The button appears only when the running daemon is OLDER than the app.** If the daemon is newer, the band shows the cause and "use the newer app" with no button.
5. **If the version still differs after the restart** (the daemon exe was not replaced because the zip overwrite was blocked while it ran), the band says to quit and re-extract — no button.
6. **The tray 「데몬 끄기」 uses the same stop path**, so it also works on a version mismatch (today it returns `NoTarget` and does nothing). Same older-than-app rule.
7. **LLM path:** the same stop is reachable through `__engramCmd` (CLAUDE.md 「LLM-우선 제어」).
8. Only the "band only informs" part of ADR-0180 decision 1 changes, and only to allow one action button where the user must act to recover; popups and their qualification (decision 2) are untouched.

## 거부한 대안

- **Band notice-only; the action lives in the tray menu** — follows the project's ADR-0180 decision 1 but not open-source/industry convention, and the user must find the tray item from the text. Rejected by the user (「행동 버튼하나 둬 그럼」). 근거 = 조사(WinUI InfoBar 원문).
- **Restart action that also starts the new daemon (ADR-0343 decision 4)** — the user wants the app to inform and the user to decide; stopping is the one step the user cannot do from the app today. Rejected by the user. 약함 — 사용자 직접 기각.
- **No confirmation step** — the press ends the old daemon's agents; nothing in the survey supports a destructive banner action without confirmation. 약함 (main judgement).

## 근거

- `docs/research/banner-action-buttons-2026-11-05.md` (WinUI InfoBar quotes read in full; others summarized).
- `docs/research/daemon-version-mismatch-restart-2026-11-05.md` (graceful stop on shipped daemons; cost of a stop).

## 영향 / 불변식

- 3-2 U5: band button + confirmation + tray item share one stop path (ADR-0343 decisions 2–3); the button shows only when daemon version < app version; post-restart re-check text; LLM command; GUI check of both paths.
- Until U5 the band shows the cause only (`link::route::refusal_message`).
- Other band notices keep ✕ only unless a later decision gives them an action.
