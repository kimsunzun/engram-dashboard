# S37 PRD — daemon↔daemon agent mail between the user's own PCs

- Status: draft 1 · 2026-11-15 · awaiting user approval (decisions D1–D7 open).
- Research: `../../research/multi-daemon-federation-direction-2026-11-11.md` (GPT draft vs code, peers, Syncthing / hcom / Reason Network source reads) · `../../research/multi-daemon-iroh-link-plan-2026-11-11.md` (iroh vs alternatives, seam fit, weight, build plan).

## 1. What · why

An agent on one of the user's PCs can send mail to an agent on another of the user's PCs and get the reply back, the same way local agent mail works today. Example: the home PC's Claude asks the office PC's Codex to review a branch that lives on the office PC; the review comes back as mail.

Why: the user runs agent teams on more than one PC. Today mail stops at the daemon boundary (`RECIPIENT_NOT_FOUND` for anything not local), so cooperation across PCs needs a human to copy text between machines.

Shape the user chose (2026-11-11 · 2026-11-15):
- No external account and no separate install — 「로그인을 해야된다고? 그럼 우리게 아니잖아」. Connectivity, encryption and peer authentication come from a library inside the daemon (iroh); we only define who is trusted and what is sent.
- Daemon↔daemon mail first; the app controlling a remote daemon comes later on the same link without restructuring — 「그 순서로 하면 된다는거지? 괜히 구조 바꾸고 하는건 없는거고」.
- Default helper servers may be n0's free public servers, but swapping to the user's own servers must be possible without code changes — 「유료로되면 못쓴다는거네」 → 「중계 서버는 반드시 필요하고 나중에 내걸로 바꾸는 장치는 있다는거잖아」.
- Pairing record = ID required + address optional, one form — 「ip 입력하는 모드도 있고 아니면 id로 해주는 모드도 있고」.

## 2. Scope / non-goals

In scope (v1):
- A persistent identity per daemon (a key pair; its public half is the PC's ID).
- A list of trusted PCs (ID, display name, optional address) managed from the app and from LLM commands.
- Links between trusted daemons: same LAN (found automatically), LAN by typed address, and across NAT (helper-server lookup, hole punching, relay fallback).
- Remote agent mail: send to an agent on another PC, the reply path back, and a delivery result to the sender.
- Limits on inbound remote mail; remote peers may only send mail.
- Settings for the helper servers (relay, address lookup) and a fixed listen port.

Non-goals (v1):
- The app showing or driving agents of another PC's daemon (shell → N daemons, ADR-0179 decision 2) — next stage; v1 lays the permission field and per-relationship protocol names for it.
- Storing mail while the other PC is off and sending it later (outbox / persistence).
- One-time pairing codes, accept prompts, a "my PCs" group that pairs once for all PCs.
- Task delegation (tracked tasks, status, cancel).
- A self-hosting guide, a one-line server profile, server presets.
- Several people sharing one PC's agents (parked: Anthropic Consumer Terms and tool-execution location — plan doc §6 "Later").
- Remote command execution of any kind.

## 3. Requirements

- R1 — Each daemon has one persistent identity. It is created on first need, kept across restarts, and its ID can be shown and copied.
- R2 — The user can add a trusted PC with an ID (required), a display name, and an address (optional), list trusted PCs, and remove one. App screen and LLM commands do the same thing.
- R3 — A daemon keeps links to every trusted PC it can reach and shows each one as online or offline. Links come back on their own after a network drop or a restart of either side.
- R4 — A connection from a PC that is not on the list is refused before any mail is read. Removing a PC drops its live link at once and refuses it from then on.
- R5 — An agent can send mail to an agent on another trusted PC with a remote address (format = D2). The recipient gets it the way local mail arrives today (delivered now, or held while the recipient is asleep or busy, same as local).
- R6 — The sender receives a result for each remote mail: delivered · held (recipient asleep/busy) · refused (with a reason) · not delivered (PC offline) · unconfirmed (no answer in time). v1 does not resend automatically.
- R7 — The recipient can reply with the existing reply contract, and the reply reaches the original sender on the other PC.
- R8 — Who sent a remote mail is decided only from the authenticated link, never from fields inside the message.
- R9 — Inbound remote mail is limited (values = D5). Remote peers can only send mail — no agent start/stop, no input outside mail, no commands.
- R10 — Helper servers: n0's public relay and address lookup by default; the user can set their own relay and lookup server addresses without code changes. A typed peer address works with no helper server at all.
- R11 — The daemon's listen port can be fixed in settings so a typed address stays valid across restarts.
- R12 — Same-LAN PCs find each other automatically (D6).
- R13 — With no trusted PC registered, nothing changes: no network listener for this feature and no contact with helper servers (D7).
- R14 — Every new screen action has an LLM command on the existing daemon bus (ADR-0055 · ADR-0307, `docs/reference/llm-command-conventions.md`).
- R15 — Local behaviour stays as it is: local mail, the shell↔daemon loopback link and its rules are untouched.

## 4. Acceptance criteria

Measured on two PCs (A, B) unless stated.

- AC1 — On A, the app and `peer.list` show the same ID; after a daemon restart it is unchanged.
- AC2 — After adding each other's ID on A and B (same LAN, no address typed), both show the other online within 30 s.
- AC3 — An agent on A sends to `<agent>@<B name>`; the B agent receives it; A's sender gets "delivered"; the B agent's reply reaches the A agent.
- AC4 — Mail to an asleep B agent returns "held" and is delivered when it wakes, as local mail does today.
- AC5 — A third daemon C, not on B's list, connects to B and is refused; B logs the refusal; no mail from C is processed.
- AC6 — With B's daemon stopped, A's send returns "not delivered" within the request deadline; when B starts again the link returns on its own and new mail goes through.
- AC7 — Removing B on A drops the live link within 1 s and later connection attempts from B are refused.
- AC8 — Home ↔ office (different networks) links through hole punching or the relay and AC3 passes.
- AC9 — With helper servers turned off and B's LAN address typed on A, A and B link on the same LAN and AC3 passes.
- AC10 — Sending past the D5 limits returns "refused" with the limit as the reason.
- AC11 — On a PC with no trusted PCs, a packet capture / log shows no helper-server contact and no listener for this feature.
- AC12 — Every action in AC1–AC10 that has a screen also works through its LLM command.

## 5. Open decisions (user)

Conventions are named in each line; ★ = main default.

- D1 — Pairing: paste the other PC's ID on both PCs ★ (Syncthing device-ID exchange; hcom token paste) / one-time short code (Reason Network four-word code, Magic Wormhole) — needs a separate join protocol and a lookup service for the code.
- D2 — Remote address seen by agents: `name@pc-name` ★ (Reason Network `person/session@machine`; hcom `name:DEVICE`) / bare names that also search other PCs — bare names can collide across PCs and need a merged directory.
- D3 — Other PC off: fail at once and tell the sender ★ / keep and send later. Convention is to keep: Reason Network keeps an outbox for 7 days, SMTP queues for days. ★ departs from it for v1 size — keeping needs persistent storage, a resend loop and dedup; it is listed as the next unit.
- D4 — Helper servers: n0 public relay + address lookup by default, plus settings for own servers and the optional typed address ★ (Syncthing: public relays/discovery with `default` + custom; Reason Network: own relays + custom) / n0 only / none (LAN or typed address only).
- D5 — Inbound limits: per PC 60 mails/min, per agent pair 12 per 10 min (loop guard), body 64 KB ★ (Reason Network `daemon/mail.rs` values) / other values / none — "none" lets two agents on different PCs answer each other forever and spend tokens.
- D6 — Same-LAN automatic discovery (mDNS): on ★ (Syncthing local discovery on by default) / off.
- D7 — When the feature runs: only after at least one trusted PC is registered ★ (no helper-server contact for users who never pair; convention not found — Syncthing and Reason Network run their networking whenever installed) / always.

Main defaults (internal, reported): iroh 1.x as the link library behind the transport seam; identity = iroh secret key in the daemon state folder; trusted list = a JSON file under the base `file` rules (ADR-0291); request deadline = the existing shared deadline; delivery results map onto the transport request/response correlation.

## 6. Risks

- Corporate networks may block UDP, multicast or the relay domain; office ↔ home may then not link. Whether running this on a company network is allowed is a company security-policy question, not decided here.
- Default helper servers are a third party's free service (rate-limited, "can change at any time"); mitigated by R10.
- Remote mail reaches agents that run with broad permissions; a trusted PC can make an agent act. Trust is limited to the user's own PCs (R4) and to mail (R9).
- The daemon grows by ~170 crates; the exact exe growth is unmeasured.
- iroh is maintained by one vendor (n0); the link sits behind the transport seam so it can be replaced.
