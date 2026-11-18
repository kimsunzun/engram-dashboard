# S37 PRD — daemon↔daemon agent mail between the user's own PCs

- Status: draft 2 · 2026-11-18 · awaiting user approval (D2 decided 2026-11-18; D1, D3–D7 open).
- Research: `../../research/multi-daemon-federation-direction-2026-11-11.md` (GPT draft vs code, peers, Syncthing / hcom / Reason Network source reads) · `../../research/multi-daemon-iroh-link-plan-2026-11-11.md` (iroh vs alternatives, seam fit, weight, build plan).

## 1. What · why

An agent on one of the user's PCs can send mail to an agent on another of the user's PCs and get the reply back, the same way local agent mail works today. Example: the home PC's Claude asks the office PC's Codex to review a branch that lives on the office PC; the review comes back as mail.

Why: the user runs agent teams on more than one PC. Today mail stops at the daemon boundary (`RECIPIENT_NOT_FOUND` for anything not local), so cooperation across PCs needs a human to copy text between machines.

Shape the user chose (2026-11-11 · 2026-11-15):
- No external account and no separate install — 「로그인을 해야된다고? 그럼 우리게 아니잖아」. Connectivity, encryption and peer authentication come from a library inside the daemon (iroh); we only define who is trusted and what is sent.
- Daemon↔daemon mail first; the app controlling a remote daemon comes later on the same link without restructuring — 「그 순서로 하면 된다는거지? 괜히 구조 바꾸고 하는건 없는거고」.
- Default helper servers may be n0's free public servers, but swapping to the user's own servers must be possible without code changes — 「유료로되면 못쓴다는거네」 → 「중계 서버는 반드시 필요하고 나중에 내걸로 바꾸는 장치는 있다는거잖아」.
- Pairing record = ID required + address optional, one form — 「ip 입력하는 모드도 있고 아니면 id로 해주는 모드도 있고」.
- Agents are called by name, wherever they run (2026-11-18). Agent rules say "send the test request to the tester agent"; the tester may run on PC A today and on PC B tomorrow, so a rule must not name the PC — 「아예 픽스로 고정시켜서 어디 서버에 어디 에이전트에게 메세지를 보내라라고 하기에는 엄청 불편하고」 → 「한마디로 에이전트 목록 동기화가 되야된다는거네 모든 pc에」 → 「반영해봐」. Agents may also look the list up themselves — 「토큰 좀 쓰고 목록 조회하는 방향으로 해도 되긴하겠네」. So every trusted PC keeps a copy of the other PCs' agent lists, and a bare name reaches the PC that has that agent.

## 2. Scope / non-goals

In scope (v1):
- A persistent identity per daemon (a key pair; its public half is the PC's ID).
- A list of trusted PCs (ID, display name, optional address) managed from the app and from LLM commands.
- Links between trusted daemons: same LAN (found automatically), LAN by typed address, and across NAT (helper-server lookup, hole punching, relay fallback).
- Remote agent mail: send to an agent on another PC (by its bare name, or `name@pc`), the reply path back, and a delivery result to the sender.
- A shared agent directory: each trusted PC keeps a read-only copy of the other trusted PCs' agent names and live/asleep state, and agents can list it.
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
- `@all` / `@here` reaching agents on other PCs (they stay local in v1).
- Sharing anything about remote agents beyond name, id and live/asleep (no folder, repo, branch, backend or activity).

## 3. Requirements

- R1 — Each daemon has one persistent identity. It is created on first need, kept across restarts, and its ID can be shown and copied.
- R2 — The user can add a trusted PC with an ID (required), a display name, and an address (optional), list trusted PCs, and remove one. App screen and LLM commands do the same thing.
- R3 — A daemon keeps links to every trusted PC it can reach and shows each one as online or offline. Links come back on their own after a network drop or a restart of either side.
- R4 — A connection from a PC that is not on the list is refused before any mail is read. Removing a PC drops its live link at once and refuses it from then on.
- R5 — An agent can send mail to an agent on another trusted PC by the agent's bare name, exactly as it names a local agent; `name@pc` names one PC explicitly (D2, decided). A local agent of that name always wins; other PCs are searched only when no local agent has the name. The recipient gets it the way local mail arrives today (delivered now, or held while the recipient is asleep or busy, same as local).
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
- R16 — Agent directory sync: each trusted PC keeps a read-only copy of every other trusted PC's agent directory — agent name, id and state (live / asleep) only. Only the PC that owns an agent changes its entry. A copy is refreshed in full when the link comes up and updated when an agent is created, deleted, renamed, or goes live / asleep; there is no periodic polling.
- R17 — Cross-PC name rules: a bare name not found locally and found on exactly one other PC goes to that PC; found on two or more PCs it is refused as ambiguous and the result lists the candidates as `name@pc`; `name@pc` goes to that PC only. When a PC disconnects, its copy is kept and marked disconnected, so mail to one of its names returns "not delivered (PC offline)" instead of "no such agent". A copy is dropped when that PC is removed from the trusted list.
- R18 — Agents (LLM) can list the agents of the other trusted PCs, with each PC's connection state, in one query.

## 4. Acceptance criteria

Measured on two PCs (A, B) unless stated.

- AC1 — On A, the app and `peer.list` show the same ID; after a daemon restart it is unchanged.
- AC2 — After adding each other's ID on A and B (same LAN, no address typed), both show the other online within 30 s.
- AC3 — An agent on A sends to `<agent>` (a bare name that exists only on B); the B agent receives it; A's sender gets "delivered"; the B agent's reply reaches the A agent. The same send to `<agent>@<B name>` also works.
- AC4 — Mail to an asleep B agent returns "held" and is delivered when it wakes, as local mail does today.
- AC5 — A third daemon C, not on B's list, connects to B and is refused; B logs the refusal; no mail from C is processed.
- AC6 — With B's daemon stopped, A's send returns "not delivered" within the request deadline; when B starts again the link returns on its own and new mail goes through.
- AC7 — Removing B on A drops the live link within 1 s and later connection attempts from B are refused.
- AC8 — Home ↔ office (different networks) links through hole punching or the relay and AC3 passes.
- AC9 — With helper servers turned off and B's LAN address typed on A, A and B link on the same LAN and AC3 passes.
- AC10 — Sending past the D5 limits returns "refused" with the limit as the reason.
- AC11 — On a PC with no trusted PCs, a packet capture / log shows no helper-server contact and no listener for this feature.
- AC12 — Every action in AC1–AC10 that has a screen also works through its LLM command.
- AC13 — Directory, with one agent rule "send to `tester`" on A (three PCs A, B, C where stated):
  - `tester` exists only on B → the send from A reaches B's `tester`.
  - `tester` also created on C → the same send is refused as ambiguous and the result lists `tester@<B name>` and `tester@<C name>`; `tester@<B name>` reaches B.
  - `tester` only on B, B's daemon stopped → the send returns "not delivered (PC offline)", not "no such agent".
  - `tester` deleted on B and created on C → the same rule, unchanged, reaches C (within a few seconds of the change, no restart).
  - A `tester` created on A itself → the send goes to A's local `tester`.
  - On A, `peer.list` shows B's and C's agents with live / asleep and each PC's online state; renaming or putting an agent on B to sleep shows on A within a few seconds.

## 5. Decisions (user)

### Decided

- D1 — Pairing (decided 2026-11-18): paste the other PC's ID on both PCs (Syncthing device-ID exchange). The user saw the other forms — one paste + accept prompt (Syncthing pending device, KDE Connect), one-time short code (Reason Network, Magic Wormhole), invite link (Reason Network team link) — and noted they all end in registering the ID: 「결국 2,3,4는 1이 되야 가능한거 아냐?」 → 「1먼저 해.」 The others are later conveniences on the same `peer.add`; the accept prompt is the suggested next one.
- D2 — Remote address seen by agents (decided 2026-11-18, quotes in §1): bare names resolve across the user's trusted PCs through a shared directory; `name@pc` stays as the explicit form (R5 · R16 · R17 · R18). Drafted default was `name@pc` only — dropped because rules would have to name the PC 「어디 서버에 어디 에이전트에게」. Conventions: Reason Network keeps a replicated session directory where a bare session name works when it is unique and otherwise must be qualified `…@machine` (source read: `src/daemon/directory.rs` `resolve` / `pick`, research §8); Erlang `global` registered names (one name → one process anywhere in the cluster); service discovery (Consul catalog, Kubernetes Service) — a name maps to its current location through a directory, not a per-message broadcast. Not copied: Reason Network's full presence broadcast (checked every 2 s, re-sent every 60 s, carries repo / branch / activity — `src/net.rs` `presence_loop`); its pick of the live candidate when one name exists on several machines (here that is ambiguous, so the same rule never switches PC silently).
- D3 — Other PC off (decided 2026-11-19): fail at once and tell the sender; keeping and sending later (outbox — Reason Network keeps 7 days, SMTP queues for days) is the next unit — user: 「다음 단위로 해야될것같은데? 이득에 비해서 얻는게 적다」. The user asked that a send cut off by a link drop still gets system feedback: an unconfirmed send gets one later notice with the final outcome (TRD §3-4); making such notices ride the next input without a turn is T-115.
- D4 — Helper servers (decided 2026-11-19): one network setting — n0 public relay + address lookup (default) / a server the user hosts (`iroh-relay` + `iroh-dns-server` URLs) / off — plus an optional address per trusted PC. All paths are tried together and a direct path always wins over the relay; there is no user-set priority. Engram runs no server of its own. User: 「유료로되면 못쓴다는거네」 → 「우리서버는 없어. 상대방이 알아서 서버 구축하는건 있고」 → 「다 입력해도 되는거고 우선순위는 어찌된다는거?」 (answer: automatic, direct first).

### Open

Conventions are named in each line; ★ = main default.

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
- The agent names (and ids, live / asleep) of each PC are visible to every trusted PC and to the agents running there (R16 · R18) — limited to the user's own PCs (R4).
- Local names shadow remote ones: creating an agent on A with the same name as B's makes A's rules reach A's agent, without warning (R5). Mail sent in the moment between a move and the directory update gets "no such agent on <PC>" and is not resent automatically.
