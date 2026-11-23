# ADR-0391: Edge addresses across PCs — bare names resolve through replicated agent directories, name@pc is explicit, a PC with an unknown directory is not counted

- 상태: 확정 (2026-11-20, basis: user decision D2 2026-11-18, amended 2026-11-20 (P-B0 · P-B) + X6 2026-11-19 + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20) · 부분 폐기 by ADR-0411 (decision 4 multi-part snapshots)
- 관련: Amends ADR-0101 (routing and sender name of the canonical name, extended across PCs with name@pc) · ADR-0309 (decision 3: one resolution point, 「여기 없으면 다른 데몬으로」 · ids unique across servers · `이름@서버` as an amendment of ADR-0101) · ADR-0003 (the agent crate knows no peers) · ADR-0110 · ADR-0116 · ADR-0390 (no forwarding, the remote name rule) · `docs/process/S37-multi-daemon/trd.md` (§3-3 · §3-4 · §4-3 · §4-7 · T8 · T21 · T22) · PRD §1 · D2 · R5 · R16 · R17 · R18 · Amended by ADR-0411 (decision 4 multi-part snapshots)

## 맥락
ADR-0101 made the canonical name (display name ?? cwd basename) the one name for routing, roster, tree and envelope sender, within one daemon. ADR-0309 kept the edge address as the name, put name → id conversion at one resolution point where 「여기 없으면 다른 데몬으로」 is to be inserted, and recorded `이름@서버` addresses as a later amendment of ADR-0101 by user decision. S37 links the user's PCs. The user's rules name agents, not PCs: 「아예 픽스로 고정시켜서 어디 서버에 어디 에이전트에게 메세지를 보내라라고 하기에는 엄청 불편하고」 → 「한마디로 에이전트 목록 동기화가 되야된다는거네 모든 pc에」 → 「반영해봐」 (2026-11-18) — the tester may run on PC A today and on PC B tomorrow.

## 결정
1. **Resolution order at the single resolution point** (`resolve_token`, one `AddressingSources` snapshot = local roster + dormant + listed PCs + directory copies):
   1. **`name@pc`** — the text after the last `@` matches a listed PC name, case-insensitively → that PC only. It wins over a local agent whose literal name is the whole token. The copy is not consulted: the receiver answers authoritatively (`RECIPIENT_NOT_FOUND`), and a disconnected PC gives `PEER_OFFLINE`.
   2. **Local, unchanged** — id (live → asleep), then exact name (live, then asleep), with today's local `RECIPIENT_AMBIGUOUS`. A local agent always wins over a remote one of the same name.
   3. **id** (UUID-shaped, not local) → the copy that lists it; listed by 2+ copies → ambiguous with `name@<pc>` candidates.
   4. **Bare name** (not local) → exact, case-sensitive match against every copy, connected or disconnected, live or asleep, counted per PC: one PC → that PC; two or more → `RECIPIENT_AMBIGUOUS` with sorted `name@<pc>` candidates and "resend with one"; none → not found. ★A PC whose directory is unknown this run (no copy since this daemon started) is not counted★ (D2 amendment, P-B).
   5. A chosen PC with no live link → `PEER_OFFLINE` at once.
2. **`@all` / `@here` stay local.** PC names are unique case-insensitively, without `@` or whitespace. An explicit fan-out `name@*` is T-117.
3. **The sender display of remote mail is `"<name>@<receiver's name for that PC>"`** — the address the recipient replies to (R7).
4. **Agent directory sync**: each PC publishes its own directory — the set local name resolution sees (live agents that read mail plus every dormant profile), each row `id` · `name` · `state` (`live` | `sleeping`) only; rows whose name breaks the remote name rule are omitted. Each PC keeps one read-only copy per trusted PC.
   - Owner-only writes: directory frames carry no node; whose directory it is = the link key, on the chosen link only. Copies are never re-published (no transitive gossip).
   - `DirSnapshot` after every handshake (multi-part above the frame cap; it replaces the copy when its `last` part arrives) and `DirDelta` on create / delete / rename / live ↔ asleep, diffed and coalesced (~250 ms); no timer, no periodic re-send; idle traffic zero. An oversized delta goes as a multi-part snapshot; no delta between parts.
   - Ordering: every frame carries `run` (the daemon's `instance`) and `rev`; a delta applies only on the same `run` with `rev` = copy `rev` + 1, otherwise the holder sends `DirResync`; an older snapshot of the same run is ignored, a snapshot of another run replaces the copy.
   - Row checks on arrival: a row whose id is a local agent id, or whose name breaks the remote name rule, is dropped and logged; an id listed by two copies is kept in both (a send to it is ambiguous).
   - Lifecycle: on the last link's end the copy is kept and marked `disconnected` (its names give `PEER_OFFLINE`); dropped on `peer.remove` and when the endpoint stops; in memory only (X6), so after a restart a PC is `unknown` until it links. No entry cap.
5. **Listing**: `peer.list` gains per PC `directory` = `connected` | `disconnected` | `unknown` and `agents[{name, id, state}]`; `agent.list` stays local.

## 거부한 대안
- **`name@pc` only** (the drafted default of D2) — user rejected directly: rules would have to name the PC 「어디 서버에 어디 에이전트에게」.
- **Sending a bare name found on several PCs to all of them, implicitly** (P-B0) — user rejected directly (「ㅇㅇ 그렇게 하면 될듯?」); no precedent fans out implicitly; the explicit `name@*` is T-117.
- **Persisting directory copies so an unknown PC still counts** (Claude's first proposal for P-B) — user rejected directly: 「그냥 놔두면 되지 않아? 유저 잘못이지」 → 「ㅇㅇ 유저가 알아서 하겠지. 겹치면 알아서 이름@pc 조합으로 하라고 해야지」; copies stay in memory (X6, 「메모리에만」).
- **Picking the live candidate among same-named agents on several PCs** (Reason Network) — the same rule could then switch PC silently.
- **Full presence broadcast with a periodic check and re-send** (Reason Network `presence_loop`: 2 s check, 60 s re-send, with repo / branch / activity) — the link is one ordered stream, so a snapshot per link plus deltas cannot drift; the user asked for names and state only.
- **`rev` alone for ordering** — it cannot order two runs of one publisher.
- **An entry cap on directories** — a copy must stay complete (R16); multi-part snapshots remove the need.
- **Dropping one of two copies that list the same id** — the winner would depend on which link came up first.
- **Listing remote agents in `agent.list`** — `agent.list` lives in the agent crate, which knows no peers (ADR-0003), and its rows carry cwd / parent that are not replicated.
- **A local agent's literal name shadowing `name@pc`** — an explicit PC choice must not be shadowed by a local name that happens to contain it.
- **Disconnected copies left out of the ambiguity count** — a PC going off would make the same rule switch to another PC silently.

## 근거
- User decision D2 (2026-11-18) and its amendment (2026-11-20), quoted above; agents may also look the list up themselves — 「토큰 좀 쓰고 목록 조회하는 방향으로 해도 되긴하겠네」.
- Conventions: Reason Network `src/daemon/directory.rs` `resolve` / `pick` (a unique bare name works, otherwise `…@machine`, the ambiguity error lists candidates; `candidates()` counts only machines heard from — the same gap as the unknown-directory exception); Erlang `global` registered names; Consul catalog / Kubernetes Service.
- ADR-0309 decision 3 placed the remote branch at the single resolution point and kept ids unique across servers (fresh UUID v4 per profile).
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- ADR-0101's single canonical name stands; what changes is its scope: the address string may now reach agents on other PCs, may carry an explicit `@pc`, and a remote sender shows as `name@pc`. Everything else in ADR-0101 (WYSIWYA inside one daemon, id-first matching locally) is unchanged.
- Resolution stays at one point (ADR-0309 decision 3); the messaging kernel reads copies from the snapshot but does not own or sync them, and keeps zero workspace deps (ADR-0110).
- Known edges: a new local agent with a remote agent's name takes over that bare name on this PC without warning; a local agent whose literal name ends in `@<listed PC name>` is reachable only by id; after a restart a bare name can go to the one known PC while another PC with the same name is still unknown; mail between a move and the delta gets `RECIPIENT_NOT_FOUND` from the old PC and is not re-routed.
- Every trusted PC (and its agents, through `peer.list`) sees this PC's agent names, ids and live / asleep state — nothing else.
- `prompts/agent-priming.md` tells agents to write `name@pc` when one name exists on several PCs.
