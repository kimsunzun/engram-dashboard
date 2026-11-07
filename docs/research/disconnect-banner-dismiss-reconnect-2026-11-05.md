# Disconnected banner — closable? how to reconnect after closing? does retry stop?

- Status: done (2026-11-05) — cross-family review (codex, effort high) FIX → applied below. Tier: medium (3 collectors: chat/web apps via docs, dev/desktop apps via docs + source, OSS chat clients via source; main re-read the decisive source lines). Decision: pending with the user (ADR-0345 follow-up — what happens after the user closes the reconnect band with ✕).
- Builds on: `reconnect-give-up-ui-2026-11-05.md` (libraries: SignalR stops after 4, socket.io retries forever) and `banner-action-buttons-2026-11-05.md` (one action button on a banner is conventional).
- Why asked: ADR-0345 put 「다시 연결」 on the band. The band must stay closable (ADR-0287 — it covers the canvas top). Our band also remembers a dismissal while the reason is unchanged, and the shell gives up after the retry budget. Together that means: close the band after give-up and there is no visible way back.

## Conclusion (first)

- **In the six apps read from source, closing the notice never removed every way back.** Either the notice comes back on the next failed retry (Zulip; JupyterLab unless "don't show again this session" is ticked), or a reconnect action stays in a persistent place — VS Code's status bar menu ("Reload Window") and JupyterLab's Kernel menu / command palette ("Reconnect to Kernel"), both with bounded retries. (likely within this sample; the review searched Slack, Discord, Teams, Gmail, Docs, Figma, Notion, GitHub Desktop, Postman and found no verified counter-example either — that is not proof none exists)
- **Closable vs not is split.** Closable: VS Code (dialog → notification → hidden), JupyterLab, Zulip. Not closable: Element, Rocket.Chat (stay until reconnected). Mattermost web shows no banner at all. (certain for the code read; not run live)
- **Chat clients never give up; dev tools bound retries.** Mattermost, Element, Zulip, Rocket.Chat retry forever with a capped backoff (caps 5–300 s). VS Code Remote retries transient errors until a configurable grace time (default 3 h) but gives up immediately on permanent protocol, non-temporary resolver and unknown errors, then asks for "Reload Window"; JupyterLab's kernel socket stops after 7 tries while its server poller keeps going. (certain for constants read)
- **A "do it now" action is common while retrying:** Zulip "Try now", Rocket.Chat "Connect", VS Code "Reconnect Now" (skips the current wait). Element and Mattermost have none. (certain)

## Findings

| App | Notice | Closable | After closing / way back | Auto retry | Manual action | Source (main re-read ✓) | Confidence |
|---|---|---|---|---|---|---|---|
| VS Code Remote | "Connection Lost" progress dialog after 40 s; then "Cannot reconnect. Please reload the window." | Yes — closing the dialog re-shows it as a notification; closing that hides it | Status bar stays "$(alert) Disconnected from {0}"; its menu adds "Reload Window" when disconnected | `TIMES = [0, 5, 5, 10, 10, 10, 10, 10, 30]` s for transient errors until grace time (default `3 * 60 * 60 * 1000`, configurable); permanent / unknown errors give up at once | "Reconnect Now" (skip wait) · "Reload Window" | `remote.ts:785,830-832,851-853,921,1011` ✓ · `remoteAgentConnection.ts:660,703` ✓ · `remoteIndicator.ts:518,781` ✓ | certain (code); grace actually used by Remote-SSH unknown |
| JupyterLab | "Server Connection Error … JupyterLab will continue trying to reconnect." | Yes — "Close" + "Do not show this message again in this session." | Re-shown on a later connection failure unless suppressed; "Reconnect to Kernel" stays in the Kernel menu and command palette | Kernel socket `_reconnectLimit = 7`; server poller backoff max 300 s | "Reconnect to Kernel" (`kernel.reconnect()`) — not in the dialog | `connectionlost.ts:23,41,46` ✓ · `notebook-extension/src/index.ts:4986-4997` ✓ · `mainmenu-extension/schema/plugin.json:173` ✓ · `kernel/default.ts:1862` (collector) | certain (strings) / likely (re-show trigger) |
| Zulip web | "Unable to connect to Zulip. Trying to reconnect in {n} seconds…" | Yes — `close_button: true` | Next qualifying failed retry re-adds the banner (backoff cap 90 s, but a server `retry-after` can be longer — `retry_backoff.ts:35-41`); fades out on success | Forever — "Retry indefinitely on timeout."; `min(2 ** ((n+1)/2), 90)` s × jitter | "Try now" | `popup_banners.ts:39-47` ✓ · `server_events.js:226` ✓ | certain |
| Element Web | "Connectivity to the server has been lost." | No — critical `Banner` rendered without `onClose` | Disappears on reconnect; window title gets `[Offline]` | Forever — keepalive every 5–10 s | none (SDK `retryImmediately()` unused) | `RoomStatusBarView.tsx:162` ✓ | certain |
| Rocket.Chat | "You're offline" · "Trying to reconnect in {{count}} seconds" | No — fixed `role='alert'` bar, no close | Disappears on reconnect | Forever — `retryCount: Infinity, retryTime: 1000`, cap 30 s, jitter | "Connect" | `ddp-client/src/DDPSDK.ts:201` ✓ · CHANGELOG (#42953: changed from "giving up after one retry" to infinite) ✓ | certain |
| Mattermost web | none rendered (error logged without display flag) | n/a | n/a — automatic | Forever — 3 s ×7, then `3000*n²` capped 300 s | none | `webapp/platform/client/src/websocket.ts:28-31` (collector) | likely (traced, not run) |

Snippet-only (original not confirmed, not used for conclusions): Slack ("Restart Slack" under the cannot-connect error), Figma ("Figma will automatically try to reconnect." with "Try again" / "Close tab"), Teams, Spotify, Docker Desktop, JetBrains Gateway, Live Share (~60 s give-up), Codespaces.

## Implication for us (main's inference — one confidence step lower)

- Our current combination — band closable + dismissal remembered + give-up after a fixed budget — produces exactly the state no surveyed app has: closed and no way back.
- Convention-shaped ways out, each seen in at least one app:
  1. **Keep retrying in the background after the band is closed, and show the band again on the next failure** (Zulip; JupyterLab's server poller). In the apps read this goes with unbounded retries; re-showing on a terminal failure with bounded retries is possible but not observed here.
  2. **Bounded retries plus a reconnect action in a persistent place that closing cannot remove** (VS Code status bar menu; JupyterLab Kernel menu + command palette). For us the persistent places that exist today are the tray menu and the LLM command surface.
  3. **Not closable** (Element, Rocket.Chat) — conflicts with ADR-0287 (the band covers the canvas top; ✕ is its way out).
- Giving up after 3 tries (~3.5 s) on a transient error is far shorter than any app read here (VS Code: up to its grace time for transient errors; chat clients: never).

## Gaps

- Closed-source chat apps (Slack, Discord, Teams, Gmail, Google Docs, Notion) — help centers do not state closability; snippets only.
- No app was run live; dialog → notification behaviour (VS Code) and re-show triggers (JupyterLab) are read from code.
- Mattermost Desktop wrapper, mobile clients, JetBrains not covered.

## Review (codex, effort high — verdict FIX, all applied)

- refuted → fixed: VS Code "gives up only after grace time" (permanent / unknown errors give up at once — `remoteAgentConnection.ts:645-685`); JupyterLab "no manual action" (Reconnect to Kernel — main re-read ✓); Zulip "≤ ~90 s" (`retry_backoff.ts:35-41` — main re-read ✓).
- not-found → headline narrowed to the sample: no verified dismiss-without-recovery example found in closed-source apps either.
- minority (no counter-evidence) → applied: re-showing the notice does not by itself imply unbounded retries.
- Reviewer re-derived: VS Code dialog → notification → hidden, status bar + Reload Window, `TIMES`, 3 h default (Remote-SSH's actual value unverified); Zulip, Element, Rocket.Chat, Mattermost, JupyterLab rows confirmed.
