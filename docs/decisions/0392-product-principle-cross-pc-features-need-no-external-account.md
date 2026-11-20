# ADR-0392: Product principle — cross-PC features need no external account and no separate install

- 상태: 확정 (2026-11-20, basis: user decisions 2026-11-11 · 2026-11-15 (PRD §1) + D4 2026-11-19 + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20)
- 관련: ADR-0388 (iroh chosen under this principle) · ADR-0390 (mail only, pairing by pasted IDs) · ADR-0179 (decision 2: shell → N daemons) · `docs/process/S37-multi-daemon/prd.md` (§1 · D4 · D7 · R10 · R13 · §6) · `docs/process/S37-multi-daemon/trd.md` (§3-1 network settings · T10 · T13) · `docs/research/multi-daemon-iroh-link-plan-2026-11-11.md` (user criteria · §1 · §3) · `docs/research/multi-daemon-federation-direction-2026-11-11.md` (§7)

## 맥락
The user runs agent teams on more than one PC and wants agents to mail each other across PCs (S37). The common ways to connect machines either require an account with a service (an overlay VPN, a vendor relay) or a separate program to install and run. When a login requirement came up, the user rejected it: 「로그인을 해야된다고? 그럼 우리게 아니잖아」 (2026-11-11). The shape was fixed across 2026-11-11 and 2026-11-15 (PRD §1).

## 결정
1. **A cross-PC feature of Engram needs no external account and no separate install.** Connectivity, encryption and peer authentication come from a library inside the daemon; Engram only defines who is trusted and what is sent.
2. **Engram runs no server of its own.** The default helper servers (relay, address lookup) may be a third party's free public servers (n0), but the user can point them at servers they host (`iroh-relay` + `iroh-dns-server` URLs) or turn them off, without code changes; a typed peer address works with no helper server at all (D4, R10). Network settings `relay` / `address_lookup` = `default` | `none` | a URL.
3. **Daemon↔daemon mail first; the app controlling a remote daemon comes later on the same link without restructuring** — 「그 순서로 하면 된다는거지? 괜히 구조 바꾸고 하는건 없는거고」.
4. **Nothing ships beside the daemon**: the link library links statically into the daemon executable.

## 거부한 대안
- **Reachability through Tailscale (embedded `tailscale-rs` / `tsnet`) or another account-based overlay** — needs a Tailscale account or a Headscale server; user rejected directly (「로그인을 해야된다고? 그럼 우리게 아니잖아」); Rust embedding is also experimental (plan §1, direction §7).
- **Separate network daemons (Nebula / Yggdrasil / NetBird)** — each is a separate Go program to install and run; not embeddable (plan §1).
- **A central relay server run by Engram** (Happier / Claude Code Remote Control style) — needs hosted infrastructure for a personal tool and contradicts "no mandatory master" (direction §7); user: 「우리서버는 없어. 상대방이 알아서 서버 구축하는건 있고」.
- **n0's public servers only, with no setting** — user rejected directly: 「유료로되면 못쓴다는거네」 → 「중계 서버는 반드시 필요하고 나중에 내걸로 바꾸는 장치는 있다는거잖아」; n0's public relays are rate-limited and "can change at any time".
- **Helper servers off only** — a typed address would become required, and home ↔ office would need the user's own relay (TRD T10).
- **A message broker hub (MQTT, hcom style)** — requires a hub process (direction §7).

## 근거
- User decisions 2026-11-11 / 2026-11-15 (PRD §1) and D4 2026-11-19: 「유료로되면 못쓴다는거네」 → 「우리서버는 없어. 상대방이 알아서 서버 구축하는건 있고」 → 「다 입력해도 되는거고 우선순위는 어찌된다는거?」 (answer: automatic, direct first).
- Plan candidate survey (2026-11-11): iroh is account- and install-free with hole punching, a relay fallback and self-hostable `iroh-relay`; measured build: everything links statically, `dumpbin /dependents` shows only Windows system DLLs — no extra DLL or file to ship.
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- A later cross-PC feature (outbox T-120, shell → N daemons, pairing codes) that needs a login, a hosted Engram service or a second installed program breaks this principle and needs a new decision.
- The helper-server setting must keep `default` | `none` | URL; swapping servers never needs a code change.
- With no trusted PC registered nothing runs and nothing is contacted (D7, R13).
- Accepted cost: the default helper servers are a third party's free service (rate-limited); n0's lookup server and relay see this PC's ID, its public IP address on the relay, the IDs it talks to and traffic timing — mail content is end-to-end encrypted, and own servers or `none` remove n0 from the path (PRD §6).
