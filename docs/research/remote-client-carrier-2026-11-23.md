# Remote client → daemon carrier — how local-daemon products let a phone or another PC in (2026-11-23)

- Status: light survey · no decision (direction recorded as T-127 in `docs/tracking.md`)
- Method: `/engram-research` light — one collector (Claude, worker-scout-sonnet) + main grounding spot check (zedra README and Happy doc re-opened by the main session). No cross-family review (light).
- Starts from: `docs/research/transport-security-build-vs-adopt-2026-10-12.md` (general programs leave the remote leg to an existing secure channel — SSH · mTLS · E2E relay · Tailscale; iroh lists Android / iOS support). Only the gaps were collected here.
- Trigger: user question — local app ↔ daemon stays WS; should a remote / mobile client reach the daemon over iroh, and is our "local daemon + outside client" shape common at all?
- Confidence legend: certain (source re-opened by the main session) · likely (collector cited the source; not re-opened) · uncertain (search summary only)

## Conclusion first
1. **The shape is common** — a resident local service with a local control channel, reached from outside through some other carrier: Docker (unix socket locally; tcp+TLS or `ssh://` remotely — prior survey, uncertain there), code-server, Home Assistant, VS Code Remote Tunnels, t3code (prior survey). likely
2. **For coding-agent phone apps the remote leg splits three ways:** own relay server with E2E encryption and QR pairing (Happy — certain), vendor server (Omnara — uncertain), **iroh P2P with relay fallback (zedra — certain)**. None of the ones seen use Tailscale.
3. **iroh for client → daemon exists, only in small projects:** zedra (phone app ↔ desktop daemon, "P2P tunnel over QUIC/UDP by Iroh", QR pairing, relay fallback under symmetric NAT / CGNAT — certain); Codux (uncertain); dumbpipe / iroh-ssh tunnels (likely). "No large product does this" is a hypothesis — the awesome-iroh list was not scanned in full.

## Findings
| Product | Local leg | Remote leg | Confidence · source |
|---|---|---|---|
| zedra | desktop daemon (`zedra start --detach`) | phone app over iroh QUIC; direct P2P, relay fallback; E2E TLS 1.3; QR scan to pair | certain — https://github.com/tanlethanh/zedra (README re-opened) |
| Happy (happy-coder) | `happy-cli` on the PC | phone and PC both dial out to Happy's relay server; E2E encrypted ("The server only sees encrypted blobs"); QR with a one-time public key | certain — https://happy.engineering/docs/how-it-works/ (re-opened) |
| Omnara | wrapper on the user's machine | proxied and persisted through Omnara's server; later pivoted to a control plane | uncertain — search summaries (HN 46991591) |
| Codux | terminal for coding agents | "encrypted peer-to-peer control across desktop, mobile, and headless hosts" using iroh | uncertain — awesome-iroh entry via search summary |
| dumbpipe · iroh-ssh | a TCP service / sshd on the home machine | iroh tunnel to it (tunnel only; exposes the service to anyone holding the endpoint id) | likely — https://github.com/n0-computer/dumbpipe · https://clan.lol/docs/unstable/services/official/p2p-ssh-iroh |
| Tailscale LocalAPI | GUI ↔ tailscaled over a unix socket / named pipe | none — local only (not a same-protocol-two-carriers example) | likely — deepwiki summary |

## Implications for us (main session inference — one notch lower confidence)
- T-127's direction (local stays WS; a remote / mobile client reaches the daemon over iroh) has a direct precedent in zedra, which has the same shape as ours (desktop daemon for coding agents + phone, iroh, QR pairing). The common alternative is Happy's own E2E relay server, which needs a server we would run.
- Pairing by QR with a one-time key (Happy, zedra) is the convention for adding a phone; our peer link pairs by pasted ids (ADR-0390) — a phone stage would likely add QR.

## Gaps
- awesome-iroh not scanned in full; Codux and Omnara not opened; Docker remote modes not re-opened this pass.
- Not checked: Plex, Zed remote, Sunshine / Moonlight, Syncthing mobile (dropped to fit the light budget).
