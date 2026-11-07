# Slice B investigation — entry validation and storage of client-made identifiers

- 상태: **medium tier · adversarial review done = MATERIALLY-FLAWED → corrected edition (2nd edition)**
- Three collection tracks (3 lead lightweight workers) · grounding = 7 external checks by main · adversarial review = one other-family run (effort high, web_search ON) → of 11 findings, **8 adopted, each with a refuting source**
- Date: 2026-08-17
- Confidence legend: **certain** (grounding-supported + independently cross-confirmed) · **likely** (supported, single source) · **uncertain/on hold** (unsupported or source silent)

> ★**We keep the spots where the 1st edition was wrong instead of erasing them**★ — so the next session does not re-derive the same conclusion. Each correction is inlined at its spot.

## Our constraints (extracted from the design docs)

- **C1** This value arrives **before authentication passes** (the first hello frame) → validation must be cheap and a rejection must not leak information. (TRD §3-7 clause 6)
- **C2** The only consumer is **the registry** — it is not used for authentication decisions, lookups, events, or the screen. (ADR-0150)
- **C3** A connection without an identifier **derives one from the connection id, fail-open**. (main's decision, not user-approved)
- **C4** There are two kinds of client — the native shell (Rust) and browser/direct (TS). Design doc: "if they diverge, the two clients behave differently, and that is the problem". (TRD §9)
- **C5** ★The app **has no disk persistence surface yet**★ — even the layout is in-memory only.
- **C6** ★The daemon does not notice a half-open connection for **up to 50 seconds**★ — in that window the old owner looks alive.
- **C7** Be careful with new third-party crates. **C8** Repository precedent: name ≤128 B · shape ≤4096 B · 512 per owner · 4096 in total.

### ★Gap in the constraint statement that the review pointed out (acknowledged, unresolved)★

**Nowhere in C1~C8 is there a requirement that "a returning client must be told apart from an impersonator".** The 1st edition substituted pointing at two defects that defer that requirement, but **without a threat model it cannot be derived whether takeover, rejection, authentication binding, or coexistence is needed** (review finding 4, a concern with no refutation but valid). **What the next session does first = write that requirement down.** Picking a method from this investigation alone would be a choice without grounds.

## Finding 1 — there are **at least five** decision methods, not three (the 1st edition's 3 categories are incomplete)

| Method | What it does | Real example | Status |
|---|---|---|---|
| **A. Unconditional takeover** | If the string is the same, it evicts the existing owner without asking. Owning the string = the credential | MQTT ClientID · Kafka `transactional.id` | certain |
| **B. Decide, then fence** | Refuses re-ownership while a live owner is attached, and opens it only after it detaches | **SMB2 Durable Handle v2** | certain (but only 1 real example) |
| **C. Server-issued token** | The client returns the resume token the server gave it | WAMP · Socket.IO · ZooKeeper | certain |
| **★D. Self-certifying identity (new)★** | The client **generates a key pair and derives the identity from the public key** → verified by **proof of ownership** when the channel is established. Knowing the string alone cannot impersonate | libp2p Peer ID | likely (proposed by the reviewer) |
| **★E. Late binding after authentication (new)★** | In pre-auth the value is only parsed cheaply, and **granting ownership is deferred until authentication completes** | XMPP resource binding (after SASL) | likely (proposed by the reviewer) |

**The 1st edition missed D and E entirely, and both fit our constraints well** — D answers the critical objection in Finding 2 below, and E answers C1 (pre-auth) directly. Sources: https://docs.libp2p.io/concepts/fundamentals/peers/ · https://www.rfc-editor.org/info/rfc6120/

### ★★Finding 2 — the 1st edition's central claim collapsed: "wait until the old owner drops" is **exclusivity**, not **identity**★★

Review finding 2 (with a refuting source, adopted). **After** the old connection is decided dead, **any process that knows the string can take it.** The 1st edition wrote that B "meets the requirement at zero C5 cost", which is **false** — both real examples it cited use a **second factor**:

- **SMB2**: `CreateGuid` (generated at open time) + conditional `ClientGuid` + `FileId` + ★**the authenticated `DurableOwner` (OS user)**★. That is, the weight of identity is carried by the **authentication layer**.
- **Kafka static membership**: the broker-issued `member.id`.

**We delete the 1st edition's "B = zero cost" cell.** A second factor is needed, and if it must survive a restart, the C5 cost comes back.

### SMB2 — the **only** model B real example that **survived** this investigation (grounding-supported · confirmed by the adversarial review too)

Re-ownership passes only if **three things hold at the same time**: ① `Open.CreateGuid` == the request's `CreateGuid` ② if there is a lease, `Open.ClientGuid` == that connection's `ClientGuid` ③ ★`Open.Session` must be **NULL** = **rejected if the old owner is still attached**★. In addition, rejected if the OS user differs from `Open.DurableOwner`.
The reviewer's WHAT SURVIVED reconfirmed this and also confirmed that **connection-loss handling turns that field NULL**.

- ★**Correction 1**★ 1st edition: "every failure is `STATUS_OBJECT_NAME_NOT_FOUND`, so it does not distinguish the reason". **False** — a `DurableOwner` mismatch is `STATUS_ACCESS_DENIED`, and some flag failures are `STATUS_INVALID_PARAMETER` (review finding 6; it was right there in the source text I had fetched, but I blurred it in synthesis).
- ★**Correction 2**★ The *reason* for the v1→v2 revision is still unchecked (it is not on the page I saw).
- Source: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/62ba68d0-8806-4aef-a229-eefb5827160f

### ★Classification of Kafka static membership = uncertain/on hold (the 1st edition asserted it was B)★

The reviewer countered: "the newcomer gets a new `member.id`, the mapping is updated, and **the old member** is fenced → not B but takeover + generation fencing". **When I rechecked the KIP source directly, that page does not cover this scenario at all** — neither my reading nor the reviewer's is confirmed.
- Only what the source supports: the client declares `group.instance.id` · a known member that rejoins gets its cached assignment back without a rebalance · reclaiming happens by session timeout or forced removal by an admin · `FENCED_INSTANCE_ID(78)` exists and relates to a `member.id` mismatch.
- ★What the source is silent on: **who is fenced** (newcomer vs old), and whether the mapping update is immediate.★ The *"we shall generate a member.id to reply to the member rejoin request immediately"* that the reviewer quoted leans toward the reviewer's reading but is not conclusive.
- ★**Correction 3**★ `UNRELEASED_INSTANCE_ID`, which the 1st edition also reported, is not in the source → deleted.
- **Consequence: the real examples of model B shrank to SMB2 alone.** Do not rest the conclusion on Kafka.

## Finding 3 — entry validation

**MQTT ClientID (grounding-supported · OASIS v5.0 source text):** *"The Server MUST allow ClientID's which are between 1 and 23 UTF-8 encoded bytes in length"* `[MQTT-3.1.3-5]` + *"MAY allow … more than 23"*. The guaranteed character set is **alphanumerics only**. The empty string is a special case (the server makes one up and announces it with `CONNACK`).

- ★**Correction 4**★ 1st edition: "the spec does not set a maximum". **Inaccurate** — ClientID is the MQTT UTF-8 Encoded String type, and that type's **maximum is 65,535 bytes** (§1.5.4, 2-byte length prefix). What is true is only "**it sets no smaller ClientID-specific cap, and the server has no means to announce its own cap**".
- ★**Correction 5**★ 1st edition: "it lumps the rejection reason codes together so it leaks no information". **False** — MQTT 5.0 has a dedicated code `0x85` **"Client Identifier not valid"** that tells the failure category. That is, **the actual practice of a mature spec is not "hide the reason".** C1's "a rejection must not leak information" cannot be justified by this precedent.
- ★The web summary's "MQTT 5.0 = 256 bytes" was refuted by checking the source directly — do not use it.
- Source: https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html

**Other numbers** (likely, not rechecked): SSH initial identification string 255 characters (RFC 4253 §4.2) · IRC nick grammar 9 characters (RFC 2812) · QUIC connection ID 20 bytes (RFC 9000 — **a different role**, for routing, chosen by the receiving side).

**The real reason for character sets is display/framing safety** (likely): Redis `CLIENT SETNAME` rejects spaces and newlines because the name is embedded in line-oriented replies. The log-injection reason is not from a spec but from general OWASP-family guidance.

- ★**Correction 6**★ 1st edition: "no spec prescribes normalization". **False** — **XMPP normatively applies PRECIS**, the resourcepart uses the OpaqueString profile, and that rule **requires NFC** (RFC 7622 §3.4 · RFC 8265). The 1st edition listed XMPP among its own peers and missed this. Source: https://www.rfc-editor.org/info/rfc7622/ · https://www.rfc-editor.org/rfc/rfc8265.html
- The trap of using normalization **as the key** remains valid (confusable mapping and Unicode normalization disagree on 31 characters, and none of the 12 systems examined got it right — likely, secondary source).

**There is no pre-auth-specific guidance in the specs** (certain — the absence is the finding). CWE-770 + OWASP fill that gap.

## Finding 4 — storage

**Browser storage is best-effort (grounding-supported · MDN source text):** **all-or-nothing** eviction per origin · `navigator.storage.persist()` is a **request, not a guarantee** (Firefox asks the user; Chrome/Safari/Edge decide automatically from interaction history, no prompt) · private mode disappears when the session ends.
- ★**Correction 7**★ 1st edition: wrote "Safari deletes after 7 days" unconditionally. **It is conditional** — *"Safari proactively evicts data **when cross-site tracking prevention is turned on**"*. That condition was in the source text I had fetched, but I dropped it from the report (review finding 9).
- Source: https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria

**Cloning risk is the dominant failure type and almost nobody has built automatic detection** (likely): because the identity material lives inside the directory that gets copied, VM cloning · profile roaming · backup restore share the same root. `systemd machine-id` · Syncthing · Headscale · Fing all have no runtime detection. Headscale **closed** the clone-detection request as **"not planned"** (certain). Only enterprise VDI has dedicated re-personalization tools (there, cloning is the normal path).
- ★**Correction 8**★ The 1st edition, citing the Syncthing docs, said "once per instance · independent of OS user and machine", but **that page describes none of the three** → unsupported. All that is supported is the derivation method (SHA-256 of the certificate DER) and the format (base32, 52 characters, 4-character groups + check digits).

- ★**Correction 9**★ 1st edition: "no product has native and browser present literally the same self-made identifier". **Refuted** — in Eclipse Paho, the browser JS client and the Java client **both accept an application-supplied `clientId`**. The same value can be presented from both sides (the reviewer's point: this does not prove it is desirable, but **the absence claim and the inference drawn from it collapse**). Source: https://eclipse.dev/paho/clients/js/
- What remains: WhatsApp · Signal use **an independent identity per client kind + cross-signing at the account layer**, and GA4 **lets the anonymous layer diverge and unifies only at the authenticated layer** (both likely, secondary sources).

## Constraint fit (corrected edition — the 1st edition's table is discarded)

| Candidate | C1 pre-auth | C4 two clients | C5 persistence surface | C6 half-open | Remaining problem |
|---|---|---|---|---|---|
| **Current A (unconditional takeover)** | fits | unrelated | zero cost | ★cannot close it★ | mismatches the requirement (but that requirement is not yet documented) |
| **B (decide, then fence)** | fits | neutral | ★**not zero cost**★ | closes it | **needs a second factor** — SMB2 gets it from the authentication layer |
| **C (server-issued token)** | fits | same storage burden | same | closes it | conflicts with the wording of ADR-0150 (client-made) |
| **★D (self-certifying identity)★** | fits | ★libp2p is implemented on both native and browser★ | needs key storage | closes it | **cannot be impersonated by knowing the string alone** — the only one that answers Finding 2's objection. Cost · crate impact not investigated |
| **★E (late binding after authentication)★** | ★answers directly★ | neutral | can be zero cost | partial | unconfirmed what our authentication knows as identity |
| Storage P1 process lifetime | — | simple on both | zero cost | — | gives up recognizing restarts |
| Storage P2 persistence | — | browser is the weak link (but C3 fail-open exists, so it is **not a ground for elimination** — review finding 5) | creates a new surface | — | cloning risk |
| Storage P3 independent identity per client | — | has mature precedent | shell only | — | ★C4 said "divergence is the problem", but P3 makes divergence by design — the 1st edition blurred this contradiction★ |

## Rejected candidates (to hand over as ADR rejected alternatives)

- **Server-issued resume token (WAMP · Socket.IO · ZooKeeper)** — the direction is opposite (the server gives the identity). Conflicts with the wording of ADR-0150.
- **Kubernetes leader election `holderIdentity`** — the identity is random per process, and anyone can take it after expiry. There is no "the same one vs someone else" decision.
- **tmux session ownership** — delegated to OS socket file permissions (uid). Not a protocol-layer declaration.
- **SIP `+sip.instance`** — for flow management after already passing strong authentication. (★XMPP must be removed from the rejected candidates — it is the real example of model E and the source of the PRECIS normalization basis.)
- **Identity decisions based on confusables** — the algorithms diverge, giving the 31-character mismatch. Nothing to gain in C2.

## Contested points · limits (kept contested — main does not rule)

1. ★**The requirement is undocumented**★ — "tell a returning client from an impersonator" is not in the constraints. **If this is not written first, picking a method has no grounds.**
2. ★**Kafka static membership classification = on hold**★ — the source is silent. Do not rest the conclusion on it.
3. ★**D and E not investigated**★ — the reviewer proposed them and they look like a good fit for our constraints, but **they were not in this investigation's budget.** Cost · crate impact · browser feasibility are unconfirmed. **Priority 1 for the next round.**
4. The **P3 vs C4 contradiction** is unresolved — whether to treat divergence as a problem or as design is the user's decision.
5. **Not investigated:** the reason for the SMB2 v1→v2 revision (Samba proposal document) · STOMP · etcd lease · length caps of `transactional.id`/AMQP `container-id` · identity handling when one app runs two at once or stable/dev builds side by side.
6. **Not proofs of absence:** "no one uses a process-lifetime identity" · "no product has two clients presenting the same value" — the latter was actually refuted by Paho. Treat the former the same way.

## What the next session does (in order)

1. **Write down the threat model and the requirement** (contested point 1). Do not pick a method before that.
2. **Investigate D (self-certifying identity) and E (late binding)** (contested point 3) — the candidate space is currently incomplete.
3. Only then bring the options to the user. **Do not decide from this 2nd edition alone.**
