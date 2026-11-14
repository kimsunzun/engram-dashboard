# S37 — multi-daemon: daemon↔daemon agent mail between the user's own PCs

Engram daemons on the user's own PCs (same LAN, or home ↔ office across NAT) link to each other over iroh (QUIC, node-key identity, hole punching with relay fallback) and carry agent-to-agent mail between them. No external account and no separate install. v1 is mail only; the app controlling a remote daemon (shell → N daemons) comes later on the same link.

- Canonical ADRs: none yet (written after the PRD+TRD review, before coding).
- Research: `../../research/multi-daemon-federation-direction-2026-11-11.md` · `../../research/multi-daemon-iroh-link-plan-2026-11-11.md`.
- Tracking: T-85 · T-86 (join the later shell → N daemons stage).

## Documents

- `prd.md` — not written.
- `trd.md` — not written.
