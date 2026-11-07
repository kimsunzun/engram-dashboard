# Survey — ordering guarantees of the registry snapshot broadcast

- **Status:** complete (design-decision mode · tier medium). **No choice has been made yet** — the adversarial review rejected the provisional recommendation, and as a result the requirements grew to three.
- **Method:** 3 parallel collection tracks (direct peers · mechanism precedents · repository constraints) → main-session evidence verification → 1 cross-family adversarial review (effort high, including its own web search).
- **Date:** 2026-08-26.
- **Confidence legend:** certain (evidence verification passed + original text checked) / likely (single source or static reading) / uncertain (not examined).
- **Links:** `../tracking.md` T-32·T-16 · `../todo/replay-subscription.md` 「재생·구독」 (replay · subscription) · `../todo/roster-consistency.md`.

## Problem

**Four points** each read the daemon's session map in full and send the full snapshot to the client (certain — exhaustive proof):

| # | Location | Trigger | Thread |
|---|---|---|---|
| ① | `crates/engram-dashboard-agent/src/manager.rs:1077` | tail of spawn | the caller |
| ② | `crates/engram-dashboard-agent/src/reaper.rs:87` | tail of reap (outside the lock) | the dedicated reaper thread |
| ③ | `crates/engram-dashboard-daemon/src/agent_conn.rs:204` → `connection_core.rs:1719` | greeting on a new connection | that connection's task |
| ④ | `crates/engram-dashboard-daemon/src/connection_core.rs:909` | `ListAgents` query response | that connection's command handling |

**Exhaustive-proof method:** only two wire variants carry the registry to the client (`AgentListUpdated`·`AgentList`), and I counted their **creation points** across the whole repository (zero others excluding tests · bins). As a cross-check I counted the call sites of `core_agents_to_wire` and of `broadcast_text` separately, and there were no new entries. ★③ bypasses `StatusSink` entirely, so a grep along the sink-name axis does not catch it★.

`manager.rs:1774-1780 list_agents()` takes the sessions lock and clones, **releases it**, and then assembles AgentInfo, and the send happens with all locks released. This is not a mistake but the shape ADR-0006 requires. So **reading and sending are not atomic**, and a side that read the older state can send later than a side that read the newer state. The frontend `src/store/agentStore.ts:68 setAgents` assigns the array wholesale, so an older snapshot that arrives late wins.

★There are **two** doors into the frontend★ — the store (`setAgents`) and the attach state machine (`src/api/protocolClient.ts` `observeRoster`, push `:661-666` / pull `:855-861`). ADR-0164 decision 3 fixed it as an invariant that both stay open. **If a freshness check is put only in the store, the attach path still swallows the older snapshot** (certain).

## Candidates

- **A** — serialize the existing senders behind a lock (snapshot read + send in one critical section).
- **B** — attach a monotonic number to the snapshot and have the frontend drop backward steps.
- **C** — do not fix it.
- **D** — fold emission into a single publisher, and the change points only report facts.
- **E** (found in peers, not in the original list) — do not carry state, only announce 「바뀌었다」 (it changed), and the client re-queries the authority.

## Finding 1 — direct peers: no one does this shape

**None of the peers examined has 「여러 변경 지점이 각자 전체 집합을 읽어 독립 브로드캐스트」 (multiple change points each read the full set and broadcast independently)** (likely — a negative claim, so exhaustive proof is impossible). They converge on three:

1. **One bootstrap snapshot + deltas afterwards, a single send point per connection** — wezterm mux (`dispatch.rs` handles notifications · writes · reads sequentially in one loop, a single `item_tx`) · herdr (`session.snapshot` → `events.subscribe`) · Kubernetes list+watch · DAP.
2. **Payload-less invalidation + client re-query** — tmux control mode (`%sessions-changed` **does not carry** the session list; `control-notify.c` emits the same string for both creation and exit) · DAP `invalidated` · JupyterLab.
3. **Fix the party that builds the full snapshot to a single one** — zellij (a single background-job poll) · JupyterLab `SessionManager` (★`startNew()`·`shutdown()` do not produce their own snapshot but call `refreshRunning()`★).

**The only counterexample is decisive.** Zed collab pushes the whole room aggregate to participants on every change, but **it holds the room lock until that update finishes.** The comment's original text (certain — the main session checked against the original):

> "room_transaction runs the block in a transaction. It returns a RoomGuard, that keeps the database locked until it is dropped. This ensures that updates sent to clients are properly serialized with respect to database changes."

I confirmed that the implementation matches the comment with `TransactionGuard<T> { data, _guard: OwnedMutexGuard<()>, _not_send }` + `rooms: DashMap<RoomId, Arc<Mutex<()>>>`.

→ **The combination of 「전체 스냅샷 브로드캐스트」 (full snapshot broadcast) + 「보내기 전에 락 놓기」 (release the lock before sending) has no precedent in the surveyed range** (likely).

## Finding 2 — decision rule

| Condition | Right answer |
|---|---|
| A single owner can be established (same process) | serialize at the source |
| It cannot (producers independent · preemptive · network-isolated) | versioned snapshot + receiver rejection |
| The snapshot is large relative to the change rate and **the channel is lossless · order-preserving** | delta |
| **The channel drops** | ★deltas forbidden★ — a dropped snapshot is overwritten by the next one, but a dropped delta silently and permanently corrupts |

**Our channel drops** (`crates/engram-dashboard-net/src/ws.rs:135-145` bounded `try_send`, only a warn on failure — a **designed** path for slow consumers). So full snapshot + last-write-wins is already right with respect to drops, and the defect is on the ordering side.

Three side findings:

- **The actor model does not fix this by itself** (certain). Erlang specifies ordering guarantees **only per pair** — if one entity sends several signals to the same destination, order is preserved, but there is no guarantee between sends from several senders to one process. The effect comes not from 「액터」 (the actor) but from **「단일 발신자」 (a single sender)**. (Review remark: however, Erlang itself explains that a common process can establish that order — and that is D.)
- **The version must be assigned in the same critical section as the read** (certain). Kubernetes #59848 is exactly that failure (a stale read → the same pod on two nodes), KEP-2340 is its fix, and it names the failure "going back in time". **Assigning at send time only moves the race.**
- **Kubernetes reversed `resourceVersion` from 「불투명·동등비교만」 (opaque · equality comparison only)** (certain — the main session checked against the original): *"Resource version strings are orderable as monotonically increasing integers within the same resource type…"* + *"Starting with Kubernetes 1.35, orderability … is included in Certified Kubernetes requirements."* ★But only within the same resource type, and `api-conventions.md` still requires treating it as opaque, so **the two upstream documents are out of step**★. When citing, state which one.

## Finding 3 — repository constraints

- **The non-blocking · non-reentrant contract of `StatusSink` does not cover `agent_list_updated`** (certain). In `crates/engram-dashboard-agent/src/types.rs:711-733` that contract paragraph is attached only to `turn_ended`, and `agent_list_updated` (`:715`) has no comment. The real home of the statement 「블록 금지 경로」 (a no-blocking path) is not the core but a daemon comment (`messaging_host.rs:391-394`).
- **ADR-0028 「단일 push」 (single push) has no binding force here** (certain). `status_fanout.rs:16-18` rules out that misreading in advance — it is a decision that the push **branch** is one, not a guarantee that the transmission happens once.
- ★**The repository has already made the same judgment twice, and both times on the side of 「락을 든 채 보낸다」 (send while holding the lock)**★ (certain). `messaging_host.rs:396-406` stamps it as an **intended departure** from ADR-0006 and states explicitly 「락 놓고 send 로 되돌리지 말 것」 (do not revert to releasing the lock and then sending) (the safety basis = non-reentrant + non-blocking, and it states its own limit that non-blocking holds only on the channel side). ADR-0071 also rejected 「IO 를 락 밖에」 (IO outside the lock) with the same race narrative.
- **That precedent's protection does not reach the frontend broadcast** (certain) — `messaging_host.rs:864` delegates outside the lock. `tracking.md` T-32 recorded this as a gap.
- **D's own conflict** (likely): `MessagingFlushSink::agent_list_updated` (`messaging_host.rs:862-865` → `:445-506`) **diffs every snapshot** to trigger the parked-mail flush. If the publisher coalesces snapshots, intermediate appearances disappear, and mail addressed to that name is stuck until the 24h TTL (`:448-456` warns not to narrow the condition).
- **B's cost** (certain): both wire variants + regenerating the committed bindings + ★the Tauri hop carries only the array and drops sibling fields★ (`src-tauri/src/daemon_client/events.rs:78,110` → `src/api/tauriTransport.ts:196-201` reassembly — 3 places must be fixed together) + frontend types are mirrored by hand and the gate does not catch them (`src/api/types.ts`). Binding sync gate = `.github/workflows/ci.yml:278-286`.
- **B does not resolve the earlier rejection reason** (likely). The root of the rejection of the T-16 attempt by two reviewers was 「필터가 명부를 과신한다」 (the filter over-trusts the registry), and its basis was that the loss in `ws.rs` is a **designed path**. B fixes only the ordering.
- **There is no runtime observation record** (certain). T-32 is not started, and the T-16 survey was also static reading only (`tracking.md` T-16).

## Adversarial review result — ★provisional recommendation rejected★

Review verdict = **REJECT.** "D alone is not justified. The defensible design is a single authoritative publisher + an end-to-end generation barrier + explicit loss recovery, and the messaging flush transition must be separated from the GUI registry state."

Key findings (in order of severity):

1. ★**The door splits again in the frontend — the main session's argument 「wire 변경 불필요」 (no wire change needed) is broken**★ (fatal). Even if the daemon-side per-connection channel preserves order, at the Tauri boundary the broadcast goes as an event and the query response goes as a command return value (`src/api/tauriTransport.ts:191-201` vs `:429-445`). Tauri documents the order of a Channel but **does not specify the order between an event and a command return.** → serializing only the producer is not enough.
2. **Ordering and loss are separate requirements, but the candidates were ranked on a single axis** (fatal). A and D also leave 「마지막 브로드캐스트가 드롭되면 복구 계기가 없다」 (if the last broadcast is dropped there is no recovery trigger). Whichever candidate is chosen, one of periodic resync · acknowledgement · latest-state guaranteed delivery is needed **separately**.
3. **「D 는 합치지만 않으면 된다」 (D only needs not to coalesce) is inaccurate** (fatal). If facts pile up in front of the publisher, even 「변경마다 한 번 읽기」 (read once per change) can miss an intermediate state. Conversely, a separation is possible in which the messaging flush is fed from a lossless internal transition stream and the GUI is pushed through a latest-value slot.
4. **An unexamined deadlock risk in A** (high). ③ sends with `send(...).await`, but that connection's writer comes up **after** `on_connect` returns, and the code itself writes that if the queue is full that await may never return (`crates/engram-dashboard-net/src/ws.rs:359-383`). If the global registry mutex is held across that await, every publisher stops.
5. ★**There is one more competitor — snapshot vs `status_changed`**★ (high). `AgentInfo.status` is read after the sessions lock is released (`manager.rs:1774-1780` → `:1883-1895`). After a newer `status_changed` is applied, an older full snapshot can overwrite it. Moreover the frontend callback discards the incarnation marker that was passed in and updates the store (`src/store/eventBus.ts:151-160`). **A, B, and D must all define the rule not only between snapshots but between a snapshot and a status event.**
6. **The premise 「전역 카운터라 gap 감지가 함정 → 그래서 B 가 약하다」 (it is a global counter so gap detection is a trap → so B is weak) is right but the inference is wrong** (high). B only needs to drop `version < last` and does not require consecutive values. Kubernetes also does not issue 410 because values are non-consecutive but when **it leaves the retention window**.
7. **「채널이 드롭하면 델타 금지」 (if the channel drops, no deltas) is an exaggeration** (high). etcd · Kubernetes use snapshot+delta over a stream that breaks — deletion is carried as an explicit event, reconnection resumes by revision, and when that fails it falls back to relist. The claim 「키별 상태로는 제거를 표현 못 한다」 (per-key state cannot express removal) is also refuted by tombstones.
8. ★**The Kleppmann citation was over-extended**★ (high). That argument presupposes **expiring distributed leases** and delayed network requests — a structure where a stalled client's lease expires, another client gets the lock, and then the old request reaches the storage late. **If a same-process non-expiring mutex is held until enqueue, other publishers cannot get through.** So 「생산자가 복수인 한 수신자 거부는 필수」 (as long as there are multiple producers, receiver rejection is mandatory) does not hold — but it becomes valid again if point 1 above (the Tauri branch) cannot be eliminated.
9. The wording 「액터 모델은 안 고친다」 (the actor model does not fix it) is too strong (low) — it distinguished several senders from a single owning actor, and is not a denial of actors in general.
10. The statement that Kubernetes "reversed" the convention is inaccurate (low) — it is upstream document skew + a narrow scope limit, not a general approval of client parsing.

## Open issues · limits

- ★**The choice has not been decided.** The review grew the requirements to three (ordering · loss recovery · status-event race), and whether to bind the three into one design or split them is the next decision.★
- **There is one prerequisite** — what `../todo/replay-subscription.md` noted: *「명부 조회가 원래 읽기였는데 부착 권위로 승격됐다 — 정합 작업 때 이 구조부터 정할 것」* (the registry query was originally a read but was promoted to the attach authority — settle this structure first during the consistency work).
- **There has never been a runtime reproduction.** Everything is static reading.
- **Collection gaps (abstentions):** herdr's license · source · loss recovery · orca's registry sync mechanism · paseo · the Docker events convention · systemd D-Bus · wezterm's recovery after a subscription break · whether the Zed `RoomUpdated` payload is really the full set · the unknown-field policy of `AgentEvent` serde · whether a `PROTOCOL_VERSION` bump is needed · the ordering-related passage in the original Saltzer.
- **The adversarial review was one round (medium).** It caught what did not pass, and is not a proof that nothing remains.

## Sources

- [Kubernetes API Concepts](https://kubernetes.io/docs/reference/using-api/api-concepts/) · [KEP-2340](https://github.com/kubernetes/enhancements/blob/master/keps/sig-api-machinery/2340-Consistent-reads-from-cache/README.md) · [k8s#59848](https://github.com/kubernetes/kubernetes/issues/59848) · [API conventions(불투명 쪽)](https://github.com/kubernetes/community/blob/main/contributors/devel/sig-architecture/api-conventions.md)
- [Envoy xDS protocol(ADS)](https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol) · [etcd data model](https://etcd.io/docs/v3.5/learning/data_model/) · [etcd Watch API](https://etcd.io/docs/v3.7/learning/api/)
- [Erlang — processes / signal ordering](https://www.erlang.org/doc/system/ref_man_processes.html) · [Erlang message passing](https://www.erlang.org/blog/message-passing/)
- [Kleppmann — distributed locking](https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html) · [Thompson — single writer principle](https://mechanical-sympathy.blogspot.com/2011/09/single-writer-principle.html)
- [Zed collab db.rs](https://raw.githubusercontent.com/zed-industries/zed/main/crates/collab/src/db.rs) · [wezterm mux](https://github.com/wezterm/wezterm/blob/main/mux/src/lib.rs) · [tmux control-notify.c](https://raw.githubusercontent.com/tmux/tmux/master/control-notify.c) · [zellij background_jobs.rs](https://raw.githubusercontent.com/zellij-org/zellij/main/zellij-server/src/background_jobs.rs) · [JupyterLab session manager](https://github.com/jupyterlab/jupyterlab/blob/main/packages/services/src/session/manager.ts) · [herdr socket API](https://herdr.dev/docs/socket-api/)
- [DAP specification](https://microsoft.github.io/debug-adapter-protocol/specification) · [Tauri — calling frontend](https://v2.tauri.app/develop/calling-frontend/) · [tokio watch](https://docs.rs/tokio/latest/tokio/sync/watch/index.html) · [Phoenix.Tracker](https://hexdocs.pm/phoenix_pubsub/Phoenix.Tracker.html)
- [LSP#1706](https://github.com/microsoft/language-server-protocol/issues/1706) · [ZOOKEEPER-1277](https://issues.apache.org/jira/browse/ZOOKEEPER-1277)
