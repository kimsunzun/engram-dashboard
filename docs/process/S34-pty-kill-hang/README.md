# S34 — killing a PTY agent mid-burst wedges the daemon connection

Killing a terminal (PTY) agent while it is still printing a burst blocks the daemon's `ClosePseudoConsole` call forever, inside the connection's command task — that connection stops answering and its output freezes while the shell still shows "connected".

- Source: found by the S21 3-2 U4a GUI QA (2026-11-07); root cause by investigation (probes reproduced it on this machine). Tracked on the transport branch as T-79.
- Docs: [`prd.md`](prd.md) · [`trd.md`](trd.md)
- Related ADR: ADR-0001 (its "master drop → reader EOF, no hang" premise needs the pump to keep draining — TRD §3A); ADR-0206 (lanes, unchanged). The wrong "kill is bounded by 5 s" claims live in code comments (TRD §3C).
- Timeline: `docs/process/step-log.md` `## S34`
