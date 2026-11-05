# Reconnect give-up UI — open-source / industry convention

- Status: done (2026-11-05). Tier: light (one collector; main re-read the SignalR doc and socket.io `manager.ts` raw sources for every number and quote below). Decision (user, 2026-11-05): the 「다시 연결」 button sits on the disconnected band, not a popup — the app stays operable while disconnected (ADR pending).
- Question: when a client's automatic reconnect gives up, is there a manual "Reconnect" action, where does it live, does auto-retry stop or continue, and what does the manual action do?
- Builds on: `banner-action-buttons-2026-11-05.md` (a dismissible top banner conventionally carries one action button) — not redone here.

## Conclusion (first)

- **Libraries split on giving up; neither ships UI.** SignalR JS stops after four attempts and hands the app a callback; socket.io retries forever by default. Both leave the "connection lost" UI to the app. (likely — official doc and source, read raw)
- **After a give-up, the manual action restarts the connection from scratch.** SignalR's documented manual pattern calls `start` again from `onclose`; socket.io resets its backoff when it gives up, so the next manual connect starts with a fresh budget. (likely)
- **SignalR's own sample UI is "connection lost + try refreshing the page"** — a recovery hint, not a retry button. (likely)
- **Real apps (VS Code Remote, Slack, Jupyter): weak evidence only.** Snippets suggest VS Code shows "Cannot reconnect. Please reload the window.", Slack shows a banner with a "Restart Slack" button, Jupyter shows a dialog. None was confirmed from an original source. (uncertain)
- Banner vs popup for the reconnect action is **not settled** by this survey. The only placement evidence is the earlier banner survey (one action button on a banner is conventional).

## Findings

| System | Auto retry (default) | After give-up | Manual recovery | Source text (raw) | Confidence |
|---|---|---|---|---|---|
| ASP.NET Core SignalR JS | 4 attempts, waits 0, 2, 10, 30 s | `Disconnected` + `onclose` | App code: call `start` again in `onclose`; sample tells the user to refresh | "configures the client to wait 0, 2, 10, and 30 seconds respectively before trying each reconnect attempt. After four failed attempts, it stops trying to reconnect." (`javascript-client.md:172`) · "provides an opportunity to inform users that the connection is permanently lost and to try refreshing the page" (`:227`) · "Call the `start` function in the connection's `onclose` event handler." (`:294`) · "Production implementations typically use an exponential back-off or retry a specified number of times." | likely |
| socket.io client v4 | `reconnectionAttempts` = Infinity, delay 1000 ms → max 5000 ms | Only with a finite limit: backoff reset + `reconnect_failed` | App calls `connect()` again (fresh backoff) | `opts.reconnectionAttempts \|\| Infinity` · `opts.reconnectionDelay \|\| 1000` · `opts.reconnectionDelayMax \|\| 5000` (`manager.ts:172-174`) · `if (this.backoff.attempts >= this._reconnectionAttempts) { … this.backoff.reset(); this.emitReserved("reconnect_failed");` (`:581-584`) | likely |
| VS Code Remote SSH | unknown | "Cannot reconnect. Please reload the window." | Reload Window | search snippet only | uncertain |
| Slack desktop | unknown | banner "Slack cannot connect" | "Restart Slack" button on the banner | search snippet only | uncertain |
| JupyterLab | unknown (snippet: keeps trying) | dialog "A connection to the Jupyter server could not be established" | unknown | search snippet only | uncertain |

Sources: https://raw.githubusercontent.com/dotnet/AspNetCore.Docs/main/aspnetcore/signalr/javascript-client.md · https://raw.githubusercontent.com/socketio/socket.io/main/packages/socket.io-client/lib/manager.ts · https://slack.com/help/articles/205138367 · https://github.com/microsoft/vscode-remote-release/issues/4548 · https://discourse.jupyter.org/t/server-connection-error/15260

## Implication for us (main's inference — one confidence step lower)

- A manual reconnect action after give-up matches both library patterns (the app owns the "start again" entry point). This agrees with ADR-0180 decision 7.
- Pressing it should start a fresh attempt sequence (reset budget), as both libraries do on a manual restart.
- Placement: ADR-0344 already lets the band carry one action button; putting 「다시 연결」 on the band is consistent with that and with the banner survey, but this survey gives no app evidence either way.

## Gaps

- Discord, JetBrains Gateway, Grafana Live not covered (light budget).
- No real-app source read in full; VS Code / Slack / Jupyter rows are snippets.
- Whether apps keep retrying in the background while showing the manual action is unknown.
