# ADR-0356: Version-mismatch band only informs and carries 「다시 연결」, no stop button

- 상태: 확정 (2026-11-10, 근거: 사용자 결정 2026-11-10 「높고 낮은거 빼고 안내만해. 데몬은 알아서 끄라고하고. 나중에 업데이트 시스템에서 자세히 다루면 될듯」 · 「ㅇㅇ 그냥 다시연결로해. 끄기 말고. 그게 좋아보이네」)
- 관련: Amends ADR-0344 (결정 1에서 4의 띠 끄기 버튼) · Amends ADR-0350 (결정 1의 버전 다름 띠 끄기 버튼) · ADR-0345 (「다시 연결」 on the disconnected band — the same button now also sits on the version band) · ADR-0342 (version mismatch shown on the connection-failure surface) · `docs/tracking.md` T-84 (update system)

## 맥락

ADR-0344 gave the version-mismatch band (②) one action, 「데몬 끄기」, shown only when the running daemon is older than the app, with a confirm step; ADR-0350 decision 1 carried it into the two-band design. While planning the frontend topic (S36) the user asked what happens when the daemon is newer than the app, and concluded that the right action for a version mismatch is an update, not a stop from the band — and that an older app must not push a newer daemon down. No update system exists yet.

On a version mismatch the app and the daemon do not talk at all: the shell's attach is refused, the link gives up and does not redial. The only exception is the one-shot stop connection (ADR-0343 decision 2). So once the user stops the daemon by some other means (task manager, the newer app), nothing reattaches and ② would stay until the app restarts. The tray stop is different: after a `User` stop the shell fires one attach-only re-add (ADR-0350 decision 3), which turns ② into ①.

## 결정

1. **② only informs, for an older and a newer daemon alike.** Text: 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요」.
2. **② carries one button, 「다시 연결」** — the same action as on ① (ADR-0350 decision 2: start the local daemon if none runs, then attach). If a mismatched daemon still runs, the attach is refused again and ② stays; nothing else happens.
3. **No stop on the band** — no 「데몬 끄기」 button, no confirm step, no 「끄는 중…」 state. The user stops the daemon themselves: the tray 「데몬 끄기」 still stops an older daemon (ADR-0344 decision 6); a newer daemon is stopped from the newer app or the OS.
4. **Unchanged:** the shell's one stop path, the tray stop, app exit, and the LLM `daemon.stop` (ADR-0344 decisions 6–7, ADR-0350 decision 3). `connection.reconnect` acts on ② the same as the button.
5. **Replacing a mismatched daemon (stop old, start new, never downgrade) belongs to a later update system** (T-84), not to the band.

## 거부한 대안

- **Keep 「데몬 끄기」 on ② for an older daemon (ADR-0344)** — the user ruled that the fix for a mismatch is an update and that the band should only inform; the confirm and 「끄는 중…」 states also leave the frontend topic. (약함 · 사용자 직접 기각)
- **② with text only, no button** — after an external stop the refused link does not redial, so ② would stay until the app restarts (the given-up link and the re-add that only the shell's own stop fires — ADR-0350 decision 3).
- **Stop and restart (or update) from the band now** — no update system exists, and from an older app it would downgrade a newer daemon. Deferred to T-84 by the user.

## 근거

- User decisions 2026-11-10 (status line).
- The given-up link on a version refusal and the re-add after a `User` stop: S21 3-2 U5 mechanism inventory (items A21 and B2, code-traced) and ADR-0350 decision 3.
- Newer daemons are already never stop targets (`StopReason::NewerDaemon` → `no_target`).

## 영향 / 불변식

- Frontend (S36): `ConnectionNotice` shows 「다시 연결」 on both bands; no stop UI on the band. The band model reads only the band kind and the refusal / cause text.
- `Band::Version { can_stop }` stays on the wire but the band no longer reads it; dropping the field is a separate change.
- `daemon_shutdown` stays — it serves the LLM `daemon.stop`.
- Breaking it: a stop button reappearing on ② reopens ADR-0344's confirm and busy states; removing 「다시 연결」 from ② leaves ② stuck after an external stop.
