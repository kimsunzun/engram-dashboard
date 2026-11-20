# S37 PRD — daemon↔daemon agent mail between the user's own PCs

- Status: draft 5 · 2026-11-20 (round-2 review fixes) · decisions D1–D7 all taken by the user (D2 amended and D5 revised on 2026-11-20); PRD approval is asked together with the PRD+TRD review. Where each post-review decision lives: TRD §8 「Decided after review」.
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
- A persistent identity per daemon and OS account (a key pair; its public half is the PC's ID), protected so a data folder copied to another PC does not reuse it.
- A list of trusted PCs (ID, display name, optional address) managed from the app and from LLM commands; trust changes an agent asks for wait for a human's approval.
- Links between trusted daemons: same LAN (found automatically), LAN by typed address, and across NAT (helper-server lookup, hole punching, relay fallback).
- Remote agent mail: send to an agent on another PC (by its bare name, or `name@pc`), the reply path back, and a delivery result to the sender.
- A shared agent directory: each trusted PC keeps a read-only copy of the other trusted PCs' agent names and live/asleep state, and agents can list it.
- A backstop limit on inbound remote mail; remote peers may only send mail.
- Settings for the helper servers (relay, address lookup) and a fixed listen port.

Non-goals (v1):
- The app showing or driving agents of another PC's daemon (shell → N daemons, ADR-0179 decision 2) — next stage; v1 lays the permission field and per-relationship protocol names for it.
- Storing mail while the other PC is off and sending it later (outbox / persistence) — the next unit after S37 (T-120).
- Detecting two agents mailing each other without end (loop detection, local and remote) — T-121.
- Mail attachments (images / files) — T-122.
- Approving trust changes from a command line on a machine without a display (it needs a human credential agents never get) — T-118; such requests wait in v1.
- Sending to every PC's agent of one name (`name@*`) — T-117.
- One-time pairing codes, accept prompts, a "my PCs" group that pairs once for all PCs.
- Task delegation (tracked tasks, status, cancel).
- A self-hosting guide, a one-line server profile, server presets.
- Several people sharing one PC's agents (parked: Anthropic Consumer Terms and tool-execution location — plan doc §6 "Later").
- Remote command execution of any kind.
- `@all` / `@here` reaching agents on other PCs (they stay local in v1).
- Sharing anything about remote agents beyond name, id and live/asleep (no folder, repo, branch, backend or activity). An agent's name is often the last segment of its working folder, so that segment is visible to the other PCs through the name.

## 3. Requirements

- R1 — Each daemon has one persistent identity per OS account. It is created on first need, kept across restarts, and its ID can be shown and copied. It is kept per OS account and machine and sealed for that account, so a data folder copied to another PC starts there with a new identity; a key of this account and machine that cannot be opened is reported and a new one offered (re-pairing). A cloned disk or VM carries the identity along; that case is caught by duplicate detection (R20).
- R2 — The user can add a trusted PC with an ID (required), a display name, and an address (optional), list trusted PCs, edit a PC's display name and address, and remove one. The trusted list is kept per OS account, and a list changed outside Engram is not used until a human accepts it. App screen and LLM commands do the same thing (R19 for the ones that widen trust).
- R3 — A daemon keeps links to every trusted PC it can reach and shows each one as online or offline. Links come back on their own after a network drop or a restart of either side.
- R4 — A connection from a PC that is not on the list is refused before any mail is read. Removing a PC drops its live link at once and refuses it from then on.
- R5 — An agent can send mail to an agent on another trusted PC by the agent's bare name, exactly as it names a local agent; `name@pc` names one PC explicitly (D2, decided). A local agent of that name always wins; other PCs are searched only when no local agent has the name. The recipient gets it the way local mail arrives today (delivered now, or held while the recipient is asleep or busy, same as local).
- R6 — The sender receives a result for each remote mail: delivered · held (recipient asleep/busy) · refused (with a reason) · not delivered (PC offline) · unconfirmed (no answer in time). v1 does not resend automatically.
- R7 — The recipient can reply with the existing reply contract, and the reply reaches the original sender on the other PC.
- R8 — Which PC sent a remote mail is decided only from the authenticated link, never from fields inside the message. The sending agent's name is the sending daemon's claim, shown together with that PC's name.
- R9 — Inbound remote mail has one high backstop per PC and the local body cap (D5). Remote peers can only send mail — no agent start/stop, no input outside mail, no commands.
- R10 — Helper servers: n0's public relay and address lookup by default; the user can set their own relay and lookup server addresses without code changes. A typed peer address works with no helper server at all.
- R11 — The daemon's listen port can be fixed in settings so a typed address stays valid across restarts.
- R12 — Same-LAN PCs find each other automatically (D6).
- R13 — With no trusted PC registered, nothing changes: no network listener for this feature and no contact with helper servers (D7).
- R14 — Every new screen action has an LLM command on the existing daemon bus (ADR-0055 · ADR-0307, `docs/reference/llm-command-conventions.md`), except approving a request, which only a human does (R19).
- R15 — Local behaviour stays as it is: local mail, the shell↔daemon loopback link and its rules are untouched.
- R16 — Agent directory sync: each trusted PC keeps a read-only copy of every other trusted PC's agent directory — agent name, id and state (live / asleep) only. Only the PC that owns an agent changes its entry. A copy is refreshed in full when the link comes up and updated when an agent is created, deleted, renamed, or goes live / asleep; there is no periodic polling.
- R17 — Cross-PC name rules: a bare name not found locally and found on exactly one other PC goes to that PC; found on two or more PCs it is refused as ambiguous and the result lists the candidates as `name@pc`; `name@pc` goes to that PC only. A PC whose agent list is not known since this daemon started is not counted (D2); agents are told to write `name@pc` when one name exists on several PCs. When a PC disconnects, its copy is kept and marked disconnected, so mail to one of its names returns "not delivered (PC offline)" instead of "no such agent". A copy is dropped when that PC is removed from the trusted list.
- R18 — Agents (LLM) can list the agents of the other trusted PCs, with each PC's connection state, in one query.
- R19 — Trust changes need a human: an agent may ask to add a PC, change the network settings or make a new identity key, but the change waits until a human approves it; a human acting in the app applies it directly. Removing a PC, editing its address, and listing stay open to agents; renaming a PC needs approval too (its name is part of `name@pc` addresses). The question goes to the app while its PC list is shown, otherwise to a native OS dialog that shows the other PC's full ID and defaults to deny; on a machine without a display it waits. Whether a call comes from a human or an agent is decided by where it comes from, never by what it says.
- R20 — If two machines or accounts run with the same identity at once (a cloned disk or VM), a trusted PC that sees both refuses the newer link as a duplicate, and both sides warn in the app and `peer.list` and offer a new identity. Adding this PC's own ID is simply refused.

## 4. Acceptance criteria

Measured on two PCs (A, B) unless stated.

- AC1 — On A, the app and `peer.list` show the same ID; after a daemon restart it is unchanged.
- AC2 — After adding each other's ID on A and B (same LAN, no address typed), both show the other online within 30 s.
- AC3 — An agent on A sends to `<agent>` (a bare name that exists only on B); the B agent receives it; A's sender gets "delivered"; the B agent's reply reaches the A agent. The same send to `<agent>@<B name>` also works.
- AC4 — Mail to an asleep B agent returns "held" and is delivered when it wakes, as local mail does today.
- AC5 — A third daemon C, not on B's list, connects to B and is refused; B logs the refusal; no mail from C is processed.
- AC6 — With B's daemon stopped, once A shows B offline, A's send returns "not delivered" at once; a send in the short window before A notices (an abrupt stop) returns "unconfirmed". When B starts again the link returns on its own and new mail goes through.
- AC7 — Removing B on A drops the live link within 1 s and later connection attempts from B are refused.
- AC8 — Home ↔ office (different networks) links through hole punching or the relay and AC3 passes.
- AC9 — With helper servers turned off and B's LAN address typed on A, A and B link on the same LAN and AC3 passes.
- AC10 — Mail from one PC past the per-PC backstop (D5) returns "refused" with the limit as the reason; normal sending rates are never refused.
- AC11 — On a PC with no trusted PCs, a packet capture / log shows no helper-server contact and no listener for this feature.
- AC12 — Every action in AC1–AC10 and AC15 that has a screen also works through its LLM command (an agent's add / network change as an approval request, AC14).
- AC13 — Directory, with one agent rule "send to `tester`" on A (three PCs A, B, C where stated):
  - `tester` exists only on B → the send from A reaches B's `tester`.
  - `tester` also created on C → the same send is refused as ambiguous and the result lists `tester@<B name>` and `tester@<C name>`; `tester@<B name>` reaches B.
  - `tester` only on B, B's daemon stopped → the send returns "not delivered (PC offline)", not "no such agent".
  - `tester` deleted on B and created on C → the same rule, unchanged, reaches C (within a few seconds of the change, no restart).
  - A `tester` created on A itself → the send goes to A's local `tester`.
  - On A, `peer.list` shows B's and C's agents with live / asleep and each PC's online state; renaming or putting an agent on B to sleep shows on A within a few seconds.
- AC14 — Approval: an agent on A calls `peer.add` for B → it answers "requested" and nothing is added. With the app's PC list shown the request appears there with the agent and B's full ID, and approving adds B; with the app closed a native OS dialog asks the same, with deny as the default. Denying leaves the list unchanged; an agent cannot approve; a human adding B in the app applies at once.
- AC15 — Renaming B on A (`peer.edit`) keeps the link up and B's agents listed; `tester@<new name>` reaches B and `tester@<old name>` returns "no such agent".
- AC16 — Identity per OS account: two Windows accounts on A using one data folder each have their own ID and trusted list; A's data folder copied to another PC starts there with a new ID; a trusted list file edited outside Engram stops the feature with "trusted list was changed outside Engram" until a human accepts it; a clone of A's identity running at the same time (a copied disk) is refused by B as a duplicate and both sides warn.

## 5. Decisions (user)

### Decided

- D1 — Pairing (decided 2026-11-18): paste the other PC's ID on both PCs (Syncthing device-ID exchange). The user saw the other forms — one paste + accept prompt (Syncthing pending device, KDE Connect), one-time short code (Reason Network, Magic Wormhole), invite link (Reason Network team link) — and noted they all end in registering the ID: 「결국 2,3,4는 1이 되야 가능한거 아냐?」 → 「1먼저 해.」 The others are later conveniences on the same `peer.add`; the accept prompt is the suggested next one.
- D2 — Remote address seen by agents (decided 2026-11-18, quotes in §1): bare names resolve across the user's trusted PCs through a shared directory; `name@pc` stays as the explicit form (R5 · R16 · R17 · R18). Drafted default was `name@pc` only — dropped because rules would have to name the PC 「어디 서버에 어디 에이전트에게」. Conventions: Reason Network keeps a replicated session directory where a bare session name works when it is unique and otherwise must be qualified `…@machine` (source read: `src/daemon/directory.rs` `resolve` / `pick`, research §8); Erlang `global` registered names (one name → one process anywhere in the cluster); service discovery (Consul catalog, Kubernetes Service) — a name maps to its current location through a directory, not a per-message broadcast. Not copied: Reason Network's full presence broadcast (checked every 2 s, re-sent every 60 s, carries repo / branch / activity — `src/net.rs` `presence_loop`); its pick of the live candidate when one name exists on several machines (here that is ambiguous, so the same rule never switches PC silently). Amended 2026-11-20: a PC whose agent list is unknown this run (no copy since the daemon started) is not counted, so a bare name may go to the one PC that is known; to reach the other PC's agent, write `name@pc` — user: 「그냥 놔두면 되지 않아? 유저 잘못이지」 → 「ㅇㅇ 유저가 알아서 하겠지. 겹치면 알아서 이름@pc 조합으로 하라고 해야지」 (Reason Network has the same gap). A bare name on several PCs stays refused (「ㅇㅇ 그렇게 하면 될듯?」); an explicit send-to-all form `name@*` is T-117.
- D3 — Other PC off (decided 2026-11-19): fail at once and tell the sender; keeping and sending later (outbox — Reason Network keeps 7 days, SMTP queues for days) is the next unit — user: 「다음 단위로 해야될것같은데? 이득에 비해서 얻는게 적다」. The user asked that a send cut off by a link drop still gets system feedback: an unconfirmed send gets one later notice with the final outcome, or saying the outcome stayed unknown (TRD §3-4); making such notices ride the next input without a turn is T-115.
- D4 — Helper servers (decided 2026-11-19): one network setting — n0 public relay + address lookup (default) / a server the user hosts (`iroh-relay` + `iroh-dns-server` URLs) / off — plus an optional address per trusted PC. All paths are tried together and a direct path always wins over the relay; there is no user-set priority. Engram runs no server of its own. User: 「유료로되면 못쓴다는거네」 → 「우리서버는 없어. 상대방이 알아서 서버 구축하는건 있고」 → 「다 입력해도 되는거고 우선순위는 어찌된다는거?」 (answer: automatic, direct first).
- D5 — Inbound limits (decided 2026-11-19, revised 2026-11-20): one high backstop per PC at the receiver against a broken peer daemon (about 600 mails/min, refused past it) plus the 64 KB body cap that local mail already has; no per-agent or per-pair limits — remote mail behaves like local mail, which has none. Held as one limits value the daemon passes in, so a future settings system can expose it; v1 has no user-facing setting. Over the backstop v1 refuses; keeping and sending later comes with the outbox (T-120); detecting agents that mail each other without end is T-121. User: 「서로 상호 연결이니 공격하는건 없을테니」 · 「1개 에이전트가 60개 한꺼번에 보낼일은 없을테니 그냥 의미없는 수치」 → 「ㅇㅇ 일단 넘어가고」. The first version took Reason Network's values (60/min per PC, 12 per 10 min per pair — 「resason network 에 맞춰서 두던지. 그리고 설정값으로 빼둘 수 있도록」); a ping-pong loop runs at a few mails a minute, so those caps would not catch it.
- D6 — Same-LAN discovery (decided 2026-11-19): on by default, kept as a setting (Syncthing local discovery is on by default). It only finds where an already-trusted PC is; connections are still limited to trusted PCs. Side effect: other LAN devices can see this PC's ID. User: 「손해 없음 켬으로해」.
- D7 — When the feature runs (decided 2026-11-19): only while at least one trusted PC is registered; it starts on the first `peer.add` and stops when the last PC is removed, so users who never pair contact nothing. User: 「pc 하나 이상 등록한 뒤부터」.

### Main defaults (internal, reported)

iroh 1.x as the link library behind the transport seam; identity = iroh secret key in the daemon state folder, one per OS account and machine, sealed by the OS (Windows DPAPI); trusted list = a JSON file per OS account and machine under the base `file` rules (ADR-0291), signed with a key derived from the identity; the peer link's liveness comes from iroh's QUIC connection (no extra ping); the sending agent waits at most 7 s for a remote result — the wait command delivery already uses, so the `engram` CLI does not cut off first (ADR-0161) — while a late answer is still taken for up to 120 s and reported by one notice (TRD §3-4); delivery results map onto the transport request/response correlation.

## 6. Risks

- Corporate networks may block UDP, multicast or the relay domain; office ↔ home may then not link. Whether running this on a company network is allowed is a company security-policy question, not decided here.
- Default helper servers are a third party's free service (rate-limited, "can change at any time"); mitigated by R10.
- What the default helper servers see: n0's lookup server holds a record from this PC's ID to its home relay (no IP address by default), and the relay sees this PC's public IP address, its ID, the IDs it talks to and the traffic timing. Mail content is end-to-end encrypted. Own servers or "off" (R10) remove n0 from the path.
- Remote mail reaches agents that run with broad permissions; a trusted PC can make an agent act. Trust is limited to the user's own PCs (R4) and to mail (R9), and an agent cannot widen trust without a human (R19).
- An agent running as the same OS user with a shell can still get around the approval (edit the files, script the dialog); the approval stops the common path, and a list edited outside Engram is detected (R2).
- The OS approval dialog may open behind other windows (Windows foreground rules); whether it shows at all for the background daemon is measured on real PCs.
- A Windows forced password reset or reinstall makes the sealed identity unreadable; every PC must then be re-paired.
- The daemon grows by ~170 crates (plus a small native-dialog crate); the exact exe growth is unmeasured.
- iroh is maintained by one vendor (n0); the link sits behind the transport seam so it can be replaced.
- The agent names (and ids, live / asleep) of each PC are visible to every trusted PC and to the agents running there (R16 · R18) — limited to the user's own PCs (R4).
- Local names shadow remote ones: creating an agent on A with the same name as B's makes A's rules reach A's agent, without warning (R5). Mail sent in the moment between a move and the directory update gets "no such agent on <PC>" and is not resent automatically. After a restart, a bare name can go to the one known PC while another PC with the same name has not linked yet (D2 amendment).
