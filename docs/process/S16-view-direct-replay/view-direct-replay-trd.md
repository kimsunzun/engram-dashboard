# S16 TRD — Removing the PC mirror buffer: view-direct replay — rev4

> **Why/what = ADR-0046 (supersedes ADR-0040 + partial-supersede list §7)** + research (2026-07-05 medium). This document = **how**. Review history: rev2 = `/review trd deep` (BLOCK 2, FIX 1) → gen fence. rev3 = re-verification (BLOCK) → **single-flight 1:1 coupling** (counting-based coupling is forbidden — both last-value stamping and FIFO counting were shown to lose events or desync). rev4 = third round (FIX) → acked gate, progress-based deadline, no marker emitted on disconnect, full-replay premise made explicit.
> **Confirmed decisions (user, 2026-07-05):** ① per-view direct subscription (each view receives replay directly from the daemon ring; src-tauri = stateless pass-through) · ② gap policy = replay from the retention floor (restoring beyond the floor is a non-goal — the history source of truth = claude session `--resume`) · ③ mobile/remote caching = placeholder only at the transport seam · ④ per-step commits of what passes the gates (no push).
> **Internal decisions (main — to be reported in the morning):** ⓐ wire multiplexing = one wire subscription per agent + full re-replay on every view mount + per-view dedup (extending the frame with a sub_id protocol is rejected — same perceived result, large blast radius). ⓑ **Reconnect = full re-replay accepted** (a regression versus the existing tail-resume, confirmed — accepted because PC loopback and reconnects are rare; remote is solved with after_seq resume at the seam. §4). ⓒ replay boundary = in-Channel marker + gen fence (the three reviewers converged on this).

## 1. Purpose and scope

Remove the daemon buffer mirror in src-tauri (`AgentBufferStore`/`OutputViewStore`/`BoundedSeqLog`/per-view cursor) **entirely**, and replace it with remount/reload/new window = **full re-replay of the daemon ring**. Progress state then lives in one place, "per webview view", and the three-way sync of mirror ↔ daemon ↔ screen disappears. `resync_output` (the symptom fix in 23a8c47) is also absorbed and discarded.

**Out of scope:** a daemon-side screen snapshot (rejected, recorded in an ADR). Mobile caching implementation. Remaining JSON rendering (S15 DEFER — a separate step). Mount-storm coalescing (§4 — recorded only as an optional optimization, not implemented in v1).

## 2. Data flow and protocol (after the redesign)

```
데몬 ring(ReplayBuffer, 유일 보관자) ── R1: Ack→replay→ReplayComplete 단일 FIFO
   │  (리뷰 실측 확인: ws.rs:519 단일 mpsc, text/binary 동일 큐 → 상대 순서 보존.
   │   재구독은 dispatch 직렬 + subscribers lock 안 동기 replay → 동시 replay 절단 없음)
   ▼ WS 단일 연결 (에이전트당 wire 구독 1개)
src-tauri: 무상태 라우터 + replay gen 채번자
   request_replay(agent) command: gen+=1 채번 → wire Subscribe{agent, epoch:known, after_seq:None} → gen 반환
   binary arm: frame 헤더(agentId·epoch)만 읽고 → epoch 필터(★재배선, §3 T5) → targets∩registered 창 Channel로 원본 bytes 통과
   text arm:   SubscribeAck → apply_subscribe_ack(epoch 갱신) + truncated 기억
               ReplayComplete{agent,epoch} → 마커 frame(tag=255) 합성: {agentId, epoch, gen, truncated} → 같은 Channel로 송신
   ▼ Tauri Channel (창당 1개, wire frame 그대로 + 마커) — ★Channel 순서 보존 = 명시 가정, 테스트 고정(§5)
웹뷰: ProtocolClient 뷰(slot) 단위 구독 + gen 펜스
   subscribeOutput(viewId, agentId, cb) → SubState 등록(동기) → requestReplay(agentId) → myGen 회수
   phase: buffering(frame 축적) → 마커(gen ≥ myGen && epoch 일치)에만 sort+dedup flush → live(직행+뷰별 dedup)
```

**gen fence + single-flight coupling (rev3 — coupling method settled):** a view flushes only on a successful marker **at or above** the `myGen` returned by its own `requestReplay` — this blocks early flush on someone else's (earlier) replay marker → loss of the head of its own replay through dedup (the path by which bug B re-enters). The buffer of a late-mounting view = tail of the earlier replay + its own whole replay → sort+dedup flush = complete. gen is an **in-Channel contract** (daemon protocol unchanged).

**The gen↔ReplayComplete coupling is not inferred by counting** — re-verification broke both naive implementations: ⓐ last-value stamping = a new gen is stamped on someone else's Complete, so early-flush loss recurs; ⓑ FIFO counting = on a failure path where no Complete arrives (when the agent vanishes, send_error skips Ack/Complete themselves — connection_core.rs:1041-1051·1080-1088) it desyncs by one slot permanently → no later view can flush → endless re-request storm. Instead, **single-flight per agent** enforces the 1:1 correspondence:

- src-tauri per-agent state: `{gen_counter(단조, 리셋 안 함), in_flight: Option<{gen, deadline}>, waiters(대기열)}`.
- `request_replay(agent)`: no in_flight → assign gen, send Subscribe(None), set in_flight, return gen. in_flight present → **merge into the next single Subscribe** (every request in the queue = returns the same next gen — a request always arrives after the view's SubState is registered, so merging into "the Sub sent after that" is safe. The coalescing safety condition of rev2 is met by structure).
- **in_flight lifetime = sent → (SubscribeAck) → acked → (ReplayComplete) → marker.** Complete is stamped **only onto an in_flight in the acked state** — because the wire order is [Ack_k]…[Complete_k][Ack_k+1], a Complete that arrives before the Ack is provably an orphan of the previous generation → ignored (misattribution blocked at the source, rev4). On stamping, send a **success marker (gen)**, release in_flight → if the queue is non-empty, send the next Subscribe.
- **deadline = progress-based (rev4):** not absolute time but **reset on every frame/Ack received for that agent** — a healthy slow replay (debug build, contention) never trips it while it is streaming. Deadline exceeded without progress (agent vanished or subscribe failed so no Ack/Complete arrives at all): send a **failure marker (gen, flags.failed)**, release in_flight, move on. (An absolute deadline gives false positives on a healthy-slow replay → a later Complete is mis-stamped onto the next gen, creating a desync — rejected by re-verification NEW-A.)
- **On connection disconnect (rev4):** **internal clear only** of in_flight and the queue — no failure marker is sent to the webview (the only driver of re-requests is the connected transition — avoids double driving and exhausting the ladder during an outage). gen_counter stays monotonic (to avoid mistaking an old-generation marker).

**Replay trigger = view-driven, unified:** `request_replay` is the only place that sends a wire Subscribe(None). ① view mount/remount → requestReplay ② window reload → each view calls requestReplay as it mounts (subscribe_output only registers the Channel) ③ WS reconnect → on the `daemon-connection-state` connected transition the frontend resets every mounted view to buffering + requestReplay (= re-arms the marker, self-recovery from a stuck buffering). The eager resubscribe and seq read in the src-tauri reconnect hook are deleted. The router sends **only Unsubscribe (cleanup of agents no longer visible)** on the wire.

**View state transition table (F1 — fixed before coding):**

| State | Event | Transition / action |
|---|---|---|
| buffering | frame(epoch = awaited epoch or undetermined) | push to buffer (cap §4, drop-oldest when exceeded) |
| buffering | frame(epoch higher) | discard buffer → buffering with the new epoch + **reissue requestReplay (new myGen)** — the existing myGen marker, aimed at the old epoch, is invalid (re-verification NEW-5) |
| buffering | marker(gen<myGen or epoch mismatch) | ignore (someone else's / old-generation replay) |
| buffering | marker(gen≥myGen, epoch match, flags.failed) | no flush — **keep the buffer** (sort+dedup absorbs duplicates, no need to discard) and go to the re-request ladder (bounded rule below) |
| buffering | marker(gen≥myGen, epoch match, success, **token current**) | sort+seq dedup flush → live (adopt epoch, lastDeliveredSeq = tail) · if truncated, surface a warning |
| buffering | marker(token mismatch — StrictMode dead subscription) | ignore (only markers for the surviving subscription are valid) |
| buffering | **marker arrives while myGen is unset** (pipe crossing where the invoke response is later than the Channel — re-verification NEW-3) | do not drop the marker; **keep the one highest-gen marker** → re-evaluate when myGen is set |
| buffering | watchdog expiry (e.g. 10s) | **no flush** — reissue requestReplay (new myGen). No partial flush |
| buffering | buffer cap exceeded | discard buffer + reissue requestReplay (no partial flush) |
| buffering | connected transition (reconnect) | discard buffer → reissue requestReplay |
| live | frame(epoch match, seq>lastDeliveredSeq) | deliver + update progress |
| live | frame(epoch match, seq≤) | dedup drop (absorbs duplicate replay) |
| live | frame(epoch higher) | drop — the epoch transition is handled by the existing `[agentId, epoch]` remount flow (remount → new subscription → replay redelivers everything) |
| live | marker(any gen) | ignore (the boundary of someone else's replay, reaching us by fan-out — for a live view, dedup alone is enough) |
| live | connected transition (reconnect) | reset to buffering + requestReplay |
| any | unsubscribe(viewId, token match) | remove SubState |

**Re-request ladder (bounded — re-verification NEW-4):** reissuing requestReplay because of watchdog / failure marker / cap exceeded has a **limit on attempts (3) + exponential backoff**. When exhausted, move the view to an explicit error state (no unbounded storm) and surface it in the slot — this error state must also be queryable from the §5 LLM control surface. `[agentId, epoch]` remount and the connected transition reset the ladder.

## 3. Design per seam

### Daemon·protocol (unchanged)
`connection_core.rs` handle_subscribe (sink swap + atomic snapshot + R1), frame format, and codec all stay as they are. The daemon invariant (a single lossless stream per agent, viewer-agnostic) is preserved.

### src-tauri
| # | File | Change |
|---|---|---|
| T1 | `output_channel.rs` | Remove `AgentBufferStore`, `build_deliverable`, `flush_snapshot`. `WindowChannelRegistry` (window→Channel) stays; a dead Channel is still removed on send failure |
| T2 | `daemon_client/connection.rs` | binary arm: remove buffer and cursor → pass the original frame to targets∩registered windows **only for what passes the epoch filter (T5)**. text arm: SubscribeAck→update epoch + remember truncated; ReplayComplete→synthesize the **gen-stamped marker** and send it **through the same Channel::send path as the binary frame** (★no app.emit route — it breaks ordering, review finding). Delete the `ReplaySlots`/`DropSlots` arms and the seq read and eager resubscribe in `resubscribe_and_sweep` |
| T3 | `commands/agent.rs` | `subscribe_output` = Channel registration only. `resync_output` → **`request_replay(agent_id) -> gen`** (§2 single-flight: send immediately or merge into the next Sub, return gen). The Subscribe/Unsubscribe block in `forward_daemon_command` stays (BLOCK-1) |
| T4 | `output_router.rs` | Remove `SubscriptionDelta` axis B. On the wire the router emits **only Unsubscribe** (prune). targets() routing stays |
| T5 | `daemon_client/protocol_state.rs` | epoch tracking stays, but it is **a rewiring, not "staying"**: the current binary hot path does not call decide_epoch (the epoch filter was folded into the mirror's on_frame — review measurement). The new binary arm calls `decide_epoch` (SubState.epoch, updated by Ack) directly and drops stale-epoch frames before passing. Add per-agent **single-flight state** (`gen_counter`, `in_flight{gen,deadline}`, queue, `truncated`) — **on disconnect, in_flight and queue are failed and then cleared; gen_counter stays monotonic** (re-verification NEW-2). (§7 allowed state scope) |
| T6 | `crates/engram-dashboard-core` | Delete `output_view_store.rs` and `output_view_buffer.rs` (+tests) — mirror-only, no other users (wiring map + reviewer grep re-check) |

### Frontend (TS)
| # | File | Change |
|---|---|---|
| F1 | `api/protocolClient.ts` | `subs` keyed by viewId. `SubState{agentId, phase, buffer[], myGen, lastDeliveredSeq, epoch, token}` + implement the §2 transition table. **Re-evaluate the token guard when the marker arrives** (not at registration time — review finding). frame → fan-out to views of the same agent. Delete `pendingBuffers` and `resubscribeAll` (the wire Subscribe send path) — the reviewer confirmed its only caller is onConnectionStateChange. The connected transition = replaced by view buffering reset + re-request |
| F2 | `api/transport.ts`, `tauriTransport.ts` | `requestReplay(agentId) -> Promise<gen>`. The marker (tag=255) is **normalized by the transport into a control event** and delivered to ProtocolClient (do not expose the marker concept in the public agentClient API — Designer finding). Add a guard to frame decode that **tolerantly skips unknown tags** (M0) |
| F3 | `components/slot/*` subscribe effect | `subscribeOutput(viewId, agentId, ...)` signature. The `[agentId, epoch]` effect, `terminal.reset()` before subscribe, and existing micro-rules stay — the epoch-transition re-arm hangs on this effect (race lens #4: it is the only re-arm for restart recovery, so pin it with vitest) |
| F4 | `ViewLayoutRenderer.tsx` | Remove the lines 61-63 dedup workaround |

**Marker frame spec (in-Channel contract):** `[tag=255][agentId:16][epoch:4][gen:8][flags:1(bit0=truncated, bit1=failed)]`. Not defined in the daemon codec — documented at the src-tauri↔webview layer. The webview silently skips an unknown tag (forward compatibility). Every `request_replay` is **closed by at least one marker (success or failure)** — Complete received (success) / deadline exceeded or disconnect (failure). It is not "exactly one": with a zombie late-Complete, a failed→success pair is possible, where a success marker (late Complete) follows a failure marker (deadline) of the same gen, and the gen fence absorbs it (the view passes the failure marker to the ladder and then flushes on the following success marker). This determinism is the premise of the frontend state machine.

## 4. Volume, trade-offs, risks

- **Net reduction:** mirror + tests −600 LOC class (core/src-tauri), frontend ±0 to +150 (adding the transition-table machine vs deleting pendingBuffers/resubscribeAll).
- **Reconnect regression (explicitly accepted — morning report):** before = tail-resume (after_seq), now = full re-replay per view. Accepted because PC loopback and reconnects are rare. Remote/mobile carriers are solved with after_seq resume + caching at the transport seam (outside the scope of this TRD). **The compromise of keeping seq in src-tauri is rejected because it violates §7.**
- **Mount storm:** single-flight coalesces structurally (every request that arrives during in-flight → merged into the next single Sub) — N views remounting at once ≤ 2 replays.
- **View buffering cap:** twice the ring cap (4MB/8192) per view — because the buffer must be able to hold "tail of the earlier replay + its own whole replay" (Codex re-review #5). When exceeded, **no partial keep (drop-oldest) is allowed** → discard buffer + re-request (ladder applies). It defends against pathological cases and is unreachable in normal operation.
- **mid-replay drop (part of the replay lost because the daemon outbound is saturated — connection_core.rs:1105 Error) = a limit equal to the current one:** the current code does not consume this Error either (emit_broadcast drops it — review measurement). The new design is the same in v1 (a gap is possible after a success marker, very low rate). Attributing the daemon Error (request correlation) and then promoting it to a failure marker is recorded as backlog.
- **High-risk seams:** T2 main_loop (lock discipline — the direction is fewer locks) · forcing the marker onto the same Channel path · F1 transition table (StrictMode, epoch rotation, reconnect crossings) → coder high · `/review code deep` · `/qa full`.

## 5. Tests (TDD, ADR-0012)

- **rust (src-tauri):** pure routing part (targets∩registered) · **single-flight state machine** (exactly one in-flight enforced, queue merge, **acked gate: a Complete arriving before Ack = orphan, ignored**, Complete→success marker 1:1, **progress-based deadline** (reset on every frame/Ack, no false positives on a healthy-slow replay)→failure marker, **no desync even when Complete is missing (agent vanished)**, internal clear on disconnect (no marker emitted) + gen stays monotonic) · marker comes after the replay tail + **on the same Channel path as binary** (R1 relative-order fixture) · epoch filter rewiring (stale-epoch frame dropped) · truncated/failed flag propagation.
- **vitest (F1) — pin every scenario the review raised:** ★**staggered mount (a view that received only the tail of an in-progress replay ignores someone else's marker → flushes completely on its own gen marker)** · fan-out to 2 views of the same agent (bug B regression) · live frame arriving before replay is still restored by sort+dedup · ignore marker with token mismatch (StrictMode) · **marker arrives before myGen is set (keep→re-evaluate)** · buffering during epoch rotation (discard + re-request, old-epoch marker ignored) · buffering during reconnect (discard + re-request) · failure marker→ladder (error state when the cap is reached) · watchdog = re-request, not flush · per-view dedup independent · unsubscribe cleanup.
- **Channel ordering assumption:** pinned twice, by a tauriTransport unit test (mock Channel ordering) and by a cdp measurement.
- **Gates:** `cargo build` · `cargo test --workspace --exclude engram-dashboard` · `cargo test -p engram-dashboard --no-run` · `cargo fmt --check` · core-isolation rg 0 lines · `npx tsc --noEmit` · `npm test`.
- **cdp measurement (completion criteria):** 3 kinds of single-slot remount (the 23a8c47 case) · **restore with the same agent shared by 2 slots + reload (reproduces bug B)** · immediate re-split right after a split (actually triggers the staggered mount) · tag1 reload restore · reconnect (client restart) restore · agent restart (epoch transition) restore.

## 6. Implementation order (commit per module)

0. **M0 forward-compat guard:** the frontend frame decode tolerantly skips unknown tags (+test) — so M1 can land before the frontend swap.
1. **M1 src-tauri switch (T1~T5):** remove the mirror + request_replay/gen + marker + epoch filter rewiring + rust tests.
2. **M2 frontend per-view (F1~F4):** transition table + vitest.
3. **M3 cleanup of leftovers (T6):** delete the core mirror files · update old anchors (the protocolClient.ts:479 epoch-authority comment, the connection.rs ADR-0037 dedup comment, etc. — paired with the §7 supersede) · attach `// ADR-0046`.
4. **M4 cdp measurement** → step-log and document cleanup.

## 7. Invariants and ADR impact

- **BLOCK-1 applied across the board:** the frontend never sends a wire Subscribe/Unsubscribe by any path (the resubscribeAll exception is deleted). Wire subscription formation = request_replay alone, cleanup = router Unsubscribe alone. **However, BLOCK-1 (no frontend wire Subscribe) is limited to the src-tauri hub path (TauriTransport)** — the direct-daemon carrier (WsTransport, legacy/test only) has 1 connection = 1 subscription, so the storm premise does not exist and it forms replay with its own wire Subscribe (not the production carrier, clientFactory.ts:24).
- **Allowed state scope in src-tauri (the line that prevents re-introduction):** per-agent `epoch` (Ack authority) + single-flight bookkeeping (`gen_counter`, `in_flight`, queue, `truncated`) are allowed as **request bookkeeping** — all of it lives only as long as the "request in progress" and is cleared on disconnect (only gen_counter stays monotonic). **Output progress state (seq/cursor/buffer) is forbidden** — the only home of progress = the webview per-view `lastDeliveredSeq`.
- **R1 (daemon) unchanged.**
- **★The consistency premise of the gen fence = "every replay is in full from the retention floor" (full-from-oldest).** "Flush on a gen≥myGen marker" is safe because a later replay of the same agent is always a cumulative superset of the earlier one. **The moment after_seq partial resume is introduced at the remote seam, this premise breaks** — at that point per-view marker correlation (exact gen match, etc.) must be strengthened first (re-verification NEW-C, stated explicitly in the ADR).
- **ADR impact (to be stated in the new ADR — review finding):** ADR-0040 **superseded entirely** · ADR-0037 clause "dedup/transport semantics: Rust is the single source of truth" **partially superseded** (the home of dedup moves to the webview per-view level) · ADR-0007 clause "frontend epoch authority = SubscribeAck alone" **partially superseded** (src-tauri decide_epoch as the first-stage filter + the frontend adopts the epoch of the filtered frames/markers) · ADR-0043 deliverable gate mechanism **discarded, reflected** · ADR-0041/0042 axis A inherited (form changed: eager→view-driven).
- Mobile/remote caching = allowed only inside the transport implementation (ProtocolClient interface unchanged) — placeholder only.
