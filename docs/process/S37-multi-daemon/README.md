# S37 — multi-daemon: daemon↔daemon agent mail between the user's own PCs

Engram daemons on the user's own PCs (same LAN, or home ↔ office across NAT) link to each other over iroh (QUIC, node-key identity, hole punching with relay fallback) and carry agent-to-agent mail between them. No external account and no separate install. v1 is mail only; the app controlling a remote daemon (shell → N daemons) comes later on the same link.

- Canonical ADRs: ADR-0388 – ADR-0395 (U0, written after the PRD+TRD review, before coding) — ADR-0388 iroh link · ADR-0389 node-key identity per OS account · ADR-0390 mail only and the backstop · ADR-0391 cross-PC addresses · ADR-0392 no external account, no separate install · ADR-0393 remote rows in the ledger · ADR-0394 humans approve trust changes · ADR-0395 the OS account as the local trust boundary.
- Research: `../../research/multi-daemon-federation-direction-2026-11-11.md` · `../../research/multi-daemon-iroh-link-plan-2026-11-11.md`.
- Tracking: T-85 · T-86 (join the later shell → N daemons stage).

## Documents

- `prd.md` — draft 2, D1–D7 decided.
- `trd.md` — draft 2, all decisions taken (D1–D7, X1–X6); before PRD+TRD review.
