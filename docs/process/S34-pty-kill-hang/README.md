# S34 — killing a PTY agent mid-burst wedges the daemon connection

Killing a terminal (PTY) agent while it is still printing a burst blocks the daemon's `ClosePseudoConsole` call forever, inside the connection's command task — that connection stops answering and its output freezes while the shell still shows "connected".

- Source: found by the S21 3-2 U4a GUI QA (2026-11-07); root cause by investigation (probes reproduced it on this machine). Tracked on the transport branch as T-79.
- Docs: [`prd.md`](prd.md) · [`trd.md`](trd.md)
- Related ADR: ADR-0206 (its "kill is bounded by 5 s" claim is wrong for `shutdown()` — see TRD §5)
- Timeline: `docs/process/step-log.md` `## S34`
