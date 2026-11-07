# ADR-0103: Messaging v1 — XML envelope · reply contract · groups · in-memory mailbox · MCP as the main entrance

- 상태: 확정 (2026-07-24, basis: a 5-branch design survey + a cross-family adversarial review + in-session measurement — see the basis section below) · 부분 폐기 by ADR-0107 (the notice cap exception channel and the clause that parking is for a single recipient only) · 부분 폐기 by ADR-0111 (decision 2 request limited to single, decision 4 group registration roster and skip, decision 5 absent-recipient parking, decision 6 group tool, the no-retroactive-broadcast invariant)
- 관련: `docs/process/S18-messaging-v1/spec/messaging-v1-spec.md`(canonical implementation contract) · ADR-0086(dual entrance) · ADR-0087(send semantics · name rules) · ADR-0094(send permission — where the MCP server name const `engram` is defined) · ADR-0096(envelope wrap seam) · ADR-0099(mcp-config issuance) · ADR-0101(canonical name) · Amended by ADR-0105 (parking TTL 1h → 24h — in-memory stage only) · Amended by ADR-0107 (the notice cap exception channel and the clause that parking is for a single recipient only) · Amended by ADR-0111 (decision 2 request limited to single, decision 4 group registration roster and skip, decision 5 absent-recipient parking, decision 6 group tool, the no-retroactive-broadcast invariant) · Amends ADR-0086 (replacing the SQLite mailbox-first clause with an in-memory mailbox)

## 맥락

Messaging is only immediate delivery (fire-and-forget), so ① a message is lost (bounced) when the recipient is absent ② there is no reply guarantee or tracking ③ there is no group sending ④ there is no surface to query delivery history. The user's goal = "while the daemon is alive, a message definitely gets through" (persistence is explicitly deferred to the agent system memory design). The orchestration scenario ("delegate to a coder → proceed after receiving the reply") needs a reply contract in peer messaging — in a spawn parent-child relationship the harness guarantees the reply structurally, but for peers the daemon ledger has to play that role.

## 결정

1. **Envelope = XML tags only** (the colon default retired, the switch remains): `<message from id? type? reply-by? to?>` + a daemon notification uses a **separate tag `<notice>` (no from)**. The LLM-exposed fields = only the ones that change behavior (id only on a request, to only on a group, no timestamps exposed).
2. **Types = minimal, by daemon behavior**: default (notification) · `request` (a ledger-open unreplied item, limited to a single recipient) · notice (daemon-only). **A reply is decided by the `in_reply_to` field, not by a type** (strict matching). refuse/failure belong to the reply body.
3. **Reply contract**: when `reply_by` (a duration notation) is exceeded, the daemon sends a notice **to the sender**. Soft on the LLM side + hard in the daemon ledger.
4. **Group = a runtime-registered roster** (in-memory · name-based) + the built-in **`@all`**. Snapshot fan-out at the moment of sending, dead members skipped (no parking), ledger = 1 message : N delivery records.
5. **Mailbox = in-memory**: park for an absent recipient (including a name that does not exist — supports instructing before spawn), TTL 1h · 100 items per recipient · a new one over the limit is bounced, a notice is exempt from the cap. Injected automatically on appearance (spawn/epoch).
6. **Entrance = MCP as the main path (3 tools: send_message · messages · group) + CLI as a spare** (subcommand mirror + `--body-stdin`). The user-global `allowedMcpServers: []` block is bypassed by **the daemon injecting, at spawn, a session-scoped fragment allowing the engram server through `--settings`** (global unchanged).

## 거부한 대안

- **A FIPA-style multi-performative type vocabulary (22 kinds)** — a precedent of industry adoption failure + an LLM recipient receives the intent through the body, so a type that does not change daemon behavior only adds cost.
- **`type=reply` · splitting refuse/failure into types** — the daemon behavior is the same, "close the unreplied item". A field/body is enough, and a reason column in the ledger can be added non-destructively in v2.
- **A subscribe contract** — session state and output are already exposed by the existing observation infrastructure (events · replay). Making it mail would duplicate the surface for the same information.
- **Group definition = profile tag / tree structure** — tags cannot be dynamically reorganized (can follow later as a convenience on top of A), and the tree is a frontend display concept so a daemon migration job would have to come first.
- **A separate special case for `@all`** — absorbing it into a built-in group gives one fan-out code path.
- **Group request (v1)** — adding it with the completion decision (any/all/quorum) semantics undecided causes timeout misfires (the adversarial review's CRITICAL finding). In v2, per-recipient contracts + aggregation.
- **cc/bcc (v1)** — not adopted by any LLM protocol + even in email cc is only a descriptive convention (the risk that the cc'd LLM acts on an imperative — a design that injects the role explicitly has to come first). A v2 item.
- **A priority field** — not adopted by any actor or LLM framework + AMQP practitioner warnings (starvation · TTL cross bugs · "splitting the queue is better").
- **Lenient reply matching** (treating the next message from an unreplying party as the reply) — an accidental-close misfire. Adopt strict + priming reinforcement.
- **Silently dropping old parked items (drop-head)** — no silent loss. Over the limit = a new one is bounced (MAILBOX_FULL), made immediately visible to the sender.
- **CLI (Bash) as the main path** — rejected by measurement and survey: unresolved Windows Bash tool pathologies (#19585 30~90s, #31437 snapshot ~8s, #38882 console window spam) + shell quoting fragility. The concern about MCP context cost was resolved by measuring deferred loading (only names stay resident). The CLI remains as a spare and for human testing (keeping the ADR-0086 duality).
- **Splitting the query/group MCP tools individually (5+)** — tool sprawl. Compressed to 3, with system operations (spawn, layout, etc.) on a separate bus, a single `engram_control` (the S17 direction — consistent with the §5 command bus).
- **Adding engram to the global allowedMcpServers** — violates the user's principle (global = block everything, override locally). A project-local setting is also rejected (the agent's cwd is an arbitrary folder, so it is powerless) → adopt injecting `--settings` at spawn.
- **An output marker channel** (the daemon parses and executes tags in the response body — 0 tool calls) — in theory the fastest, but it carries a quoted-injection risk (the Copilot CVE precedent) + there is no real-world case of the nonce mitigation. Revisit when a burst-send bottleneck is measured.
- **A file outbox channel** (the Write tool + a daemon watch) — kept as the first-ranked valid alternative (the Claude Code team feature precedent). Adopt it if MCP deferred loading collapses in measurement.

## 근거

- **Design survey (medium, 2026-07-24)**: 5 branches collected in parallel — email RFCs · actors (gen_server call/cast, Akka ask) · FIPA-ACL · AMQP · LLM protocols (A2A/AutoGen/LangGraph/Claude Code team) — + one codex (cross-family) adversarial review (FIX — 13 corrections applied: group × reply-contract conflict, cc being descriptive, in_reply_to ≠ thread, MDN overstatement, etc.). The converged core (id · sender · reply correlation · deadline) is cross-confirmed by 3 lineages = certain.
- **Measurement (in-session)**: daemon HTTP round trip ~3ms · engram-send.exe spawn ~105ms · bash spawn ~120ms → speed is not disqualifying, and LLM inference is the dominant term. MCP deferred = confirmed live in this session (only the names of one connected MCP server (~70 tools) stay resident). `allowedMcpServers: []` (line 42 of the user-global settings) is the cause of the spawned agent lacking the send_message tool (consistent with that day's QA observation — the ADR-0099 wiring is normal).
- **Unverified (implementation QA items)**: the allowedMcpServers merge behavior of the `--settings` injection · the XML envelope in a real claude round trip (of what was verified with colon, only the format-coupled part is re-verified — the plumbing is format-independent, so it stays valid).

## 영향 / 불변식

- **The party that guarantees the reply contract = the daemon ledger** (LLM compliance is soft) — implementing request without the timeout notice makes the contract half. The notice must have a mailbox cap exception channel.
- **No retroactive broadcast**: a group/@all is a snapshot at the moment of sending — delivery to agents spawned afterwards is forbidden (parking is for a single recipient only).
- **The `<notice>` tag is daemon-only** — agents cannot send it. The distinction from a `<message>` with from is the basis for judging whether a reply is possible, so mixing them makes the LLM try to reply to infrastructure.
- **The 3-tool cap discipline**: the MCP surface for agents grows only by argument extension of send_message · messages · group. System operations go on the `engram_control` bus (separate).
- The implementation contract details (fields · response JSON · error codes · policy constants · acceptance criteria) = the spec document is canonical (related section link).
