# Research: Tauri 2 multi-window output carrier (Channel vs emit) + fan-out routing pattern

**Status:** complete (cross-family deep — Claude Sonnet 3 + Codex 2 BLIND, opus cross-check + adversarial verification)
**Written:** 2026-06-28 (dashboard2/master, opus aggregator)
**Trigger:** before entering module ① T5 (OutputRouter), to resolve the unverified D3 carrier (= Tauri Channel) ("multi-window safety context7+QA full on hold") and to get OSS grounding for fan-out routing/subscription patterns.
**Basis ADRs:** ADR-0036 (unified transport relay) · ADR-0037 (transport semantics = Rust-only guard) · ADR-0006 (lock order)
**Our versions (verification baseline):** `tauri = 2.11.2` (Cargo.lock) · `@tauri-apps/api = ^2.11.0` (package.json)

> Confidence legend: **certain** (primary/official/source directly) · **likely** (secondary convergence or source inference) · **uncertain** (single source/knowledge-based).
> Method: two model families (Claude/Codex) investigated independently and BLIND → opus cross-checked claim by claim + adversarially verified the key/contradicting claims. The caution "unanimity ≠ correct answer" is kept.

---

## 0. One-line conclusion

The D3 carrier = **Tauri `Channel` is right (the official docs explicitly name "child process output" as a Channel use case)** — but it **must be used on the raw byte path (`Channel<tauri::ipc::Response>`/`InvokeResponseBody::Raw`)** to avoid JSON bloat (`Channel<&[u8]>` leaks into JSON). fan-out = **`ArcSwap<RoutingSnapshot>` (hot path with zero locks) + a separate `Mutex<HashMap<AgentId,usize>>` subscription ref-count (0→1 subscribe / 1→0 unsubscribe)** — the two families converged on nearly the same design. This **confirms and adds detail to** the current TRD/spike decisions; it is not a reversal.

---

## 1. Channel carrier — lifecycle · throughput (track A)

### Confirmed facts
- **A Channel is bound by birth to the calling webview** — JS creates `new Channel(onmessage)` and passes it as an `invoke` argument, and Rust receives a `Channel<T>` handle tied to that webview (`JavaScriptChannelId::channel_on(webview)`). **A separate Channel per window solves per-window routing naturally without emit_to.** (certain — triple convergence of Claude A·B·Codex, source `crates/tauri/src/ipc/channel.rs`)
- **3 paths by transfer size:** JSON <8192B = inline `eval` / raw <1024B = `new Uint8Array(...).buffer` (ArrayBuffer) inline eval / larger = stored in `ChannelDataIpcQueue` and then queued through `plugin:__TAURI_CHANNEL__|fetch`. (certain — Codex source + Claude A converge. Code comment: on WebView2 v135 an 8192B JSON eval is ~2× faster than fetch)
- **An ordering guarantee mechanism exists + works correctly in our version:** Rust assigns an increasing index, and JS queues out-of-order messages in an array by `#nextMessageIndex` and drains them in order. (certain — `packages/api/src/core.ts`)
- **Official recommendation:** Channel = "fast, ordered data" / "used internally for streaming such as download progress, **child process output**, websocket messages". The event system is explicitly described as "**not designed for low latency or high throughput**". (certain — `v2.tauri.app/develop/calling-rust` Channels, `/calling-frontend` Event System)

### ★ The raw byte trap (confirmed by adversarial verification — directly tied to implementation)
- `InvokeResponseBody` has both `Json(String)` + `Raw(Vec<u8>)`, so the raw path **really exists**. **However**, the `impl<T: Serialize> IpcResponse` (blanket) serializes to JSON, so **even `Channel<&[u8]>` in the official 4096-byte example goes out as a JSON array (`[104,101,...]`) because `&[u8]: Serialize`** — even bulkier than base64. (certain — Codex adversarial verification, the blanket impl in `crates/tauri/src/ipc/mod.rs` + docs "returning a Serialize = JSON, an optimized array buffer needs `tauri::ipc::Response`")
- **Conclusion:** terminal bytes must be sent with `Channel<tauri::ipc::Response>` (`Response::new(bytes)`) or `InvokeResponseBody::Raw` to take the raw path. **Plain `Channel<Vec<u8>>`/`Channel<&[u8]>` will not do.**
- From the start we build the byte frame with the protocol crate `encode_terminal_frame` → carrying it as Response/Raw is natural.

### Lifecycle bugs (against our version 2.11.2)
| Issue | Symptom | Impact on us |
|---|---|---|
| #13133 | Channel `onmessage` stays on `window` permanently → memory leak | **Resolved** (v2.5.0 fix, PR #13136). But frontend cleanup is `delete channel.onmessage` (not null) — matches the existing CLAUDE.md micro-rule (certain) |
| #12065 | `pendingMessageIds` **sorted as strings** → order reversal, `onmessage` does not fire | **Resolved** (@tauri-apps/api 2.2.0 + tauri 2.2.0 fix, PR #12069) (likely→certain, CHANGELOG checked) |
| #15583 | **backend `js_event_listeners` not cleaned up when a webview is destroyed** → stale accumulation | **Unresolved (including 2.11.2)** — an explicit `unlisten` in `onCloseRequested` is required when closing a window (certain) |
| 2.11.3 deadlock | deadlock in `send_channel_data_handler`/`send_channel_data` | **We are on 2.11.2 = one patch below.** **Consider upgrading to 2.11.3+** before using Channel in earnest (likely — release notes, Codex) |
| large-queue residue | after `ChannelDataIpcQueue` insert, if the window closes before fetch, a pending payload may remain | operational cleanup needed (queue cleanup on close/unsubscribe is not automatic) (likely — Codex source inference) |

- **dead window send:** `Channel::send` returns `Result<()>`, and its final internal `webview.eval(...)?` → `Err` if the webview is destroyed. **The exact error type is not officially documented → the realistic approach is to detect it by the `send` `Err` and remove the channel from the registry.** Never `unwrap()`. (likely — two families converge, source inference)

---

## 2. Channel vs emit/emit_to (track B)

- **An event payload is always a JSON string** → unsuitable for bytes (JSON array bloat). `emit` = broadcast to all windows, `emit_to(label)` = targeted, `emit_filter` = selection by closure. (certain — triple convergence + official)
- **A JS filter after `emit` breaks not only performance but also correctness/security** — it does not fit the requirement "only to the window that is watching" (every window receives the payload). (certain — emphasized by Codex)
- Event routing bugs (low impact on us — we adopted Channel as the carrier, and emit is used only for layout control):
  - #8916 unlisten malfunction (early v2, beta.3, PR #8930 closed) — *the handoff's "below 2.6.0" notation is inaccurate* (a beta.3 issue). Unrelated to our 2.11.2. (certain)
  - #10182 `emit_to` ignores the label and sends to everyone (beta.23, closed as "not planned") — impact on stable 2.x uncertain, and we do not use emit_to for output (likely)
  - #11561 AnyLabel matching failure (2.0.6, PR #11581) — avoided by specifying the receiving target type explicitly (certain)
- **Implication:** emit is enough for layout control (`layout:updated`/`view:list-updated`) (low frequency · JSON is fine, and the current module ② already does so). **Only high-bandwidth output goes through Channel.** Splitting the carrier in two is right.

---

## 3. fan-out routing + subscription ref-count (track C) — the two families converged almost identically

### Routing table = `arc-swap`
- **arc-swap is an officially stated use case for a "read often, update rarely" routing table** — it removes lock + refcount contention compared with `RwLock<Arc<T>>`. Under contention, reads are ~2.5× better than the std RwLock. (certain — docs.rs + vorner blog, two families converge)
- **Recommended shape (presented almost identically by the two families):**
  ```rust
  struct RoutingSnapshot { by_agent: HashMap<AgentId, Arc<[WindowId]>> }
  // 핫패스(프레임마다): 락0
  let routes = routing.load();
  if let Some(windows) = routes.by_agent.get(&frame.agent_id) {
      for w in windows.iter() { send_to_window(*w, &frame); }
  }
  // 레이아웃 변경(드물게): routing.store(Arc::new(new_snapshot));
  ```
- **Pitfalls (certain, two families converge):** ① do not carry the `load()` Guard across an `.await` (slot exhaustion) → use `load_full()` when going through async. ② If entries are updated often one by one, the full-clone cost is high → irrelevant for us, since we update only on layout changes. ③ Sized only — a trait object must be boxed.
- Alternatives rejected: `left-right`/`evmap` (eventual consistency · `publish()` is complex), `watch` (fine for snapshot distribution · unsuitable for hot-path lookup), `broadcast` (a slow receiver gets `Lagged` and **messages are lost** → dangerous for lossless terminal output). (certain — two families converge)

### Subscription union ref-count
- **Separate from the hot path**: the ref-count map changes only on layout/window lifetime events, so `Mutex<HashMap<AgentId,usize>>` is enough. **0→1 transition = backend subscribe / 1→0 = unsubscribe.** (likely — two families converge)
- Tie it to the window/slot lifetime with `SubscriptionGuard { agent_id, owner: Arc<...> }` + `Drop`.
- **★ Beware of async Drop (certain):** `Drop` cannot `.await` → if unsubscribe is async, **enqueue the unsubscribe command to the manager task from Drop** (do not await directly). This fits naturally with our DaemonClient actor model (cmd_tx).

### OSS prior art (borrow the pattern — not copy-paste)
| System | Language/license | Mechanism | Implication for us |
|---|---|---|---|
| **wezterm** | Rust/MIT | `Mux::notify`: broadcasts PaneOutput to a `RwLock<HashMap<id, Box<dyn Fn(Notif)->bool>>>`, and when a subscriber returns false it is removed lazily with `retain`. Client-side filter | With few subscribers, write-lock notify+retain is simple. But ours is high-frequency, so arc-swap routing fits better |
| **zellij** | Rust/MIT | central Screen state machine: PtyBytes→owning pane→**per-client render push**. `Arc<Mutex<HashMap<ClientId,ClientSender>>>` | "Parse once, push a view per client" — we delegate parsing to the webview xterm, so raw frame routing is simpler |
| **tmux** | C/ISC | the server owns the PTY, clients in a `TAILQ`, diff push to **only the viewing pane**, detach cleanup through `server_client_lost` | "Do not send to a window that is not watching" = the same as our OutputRouter goal. Acceptance criterion 5 |
| **ra-multiplex/lspmux** | Rust/MIT | rewrites ids with `HashMap<RequestId,(ClientId,OriginalId)>` → reverse-routes responses. Notifications that cannot be routed are dropped | the classic per-agent routing table. Isomorphic to our request_id pending map |

**Common lesson:** mature multiplexers do not retransmit output N times; they **process it once at the source → send only to the viewing clients**. Our OutputRouter ("fan-out only to the viewing windows" with an arc-swap snapshot) is consistent with this practice — but since each webview xterm does the parsing, we only have to carry raw frames.

---

## 4. Cross-verification table (Claude ↔ Codex)

| Claim | Claude | Codex | Verdict |
|---|---|---|---|
| Channel is bound to the calling webview → naturally per-window | ✓ | ✓ | **converged · certain** |
| Events = JSON, unsuitable for high bandwidth / Channel = recommended for streaming | ✓ | ✓ | **converged · certain (official)** |
| Channel supports raw bytes | A: **not supported (error)** / B: supported | supported (but blanket = JSON) | **contradiction → resolved:** supported. A confused it with #13405 (an event FR). Raw requires an explicit `Response`/`Raw` |
| Channel ordering guarantee (our version) | B: patch version of #12065 uncertain | 2.2.0 fix (adversarial verification) | **resolved · certain:** fixed in our 2.11 |
| #15583 listener not cleaned up on webview destruction is unresolved | ✓ | ✓ | **converged · certain (affects us)** |
| arc-swap = suitable for a routing table | ✓ | ✓ | **converged · certain** |
| Subscription ref-count (0→1/1→0) + async Drop caution | ✓ | ✓ | **converged · likely** |
| broadcast is dangerous for output because of Lagged loss | ✓ | ✓ | **converged · certain** |

---

## 5. Gaps / limits
- **No measurement done:** the real PTY MB/s throughput of the Channel raw path and the overhead of one extra hop are estimates from docs/source. The ADR-0036 assumption "local IPC is negligible" still has to be closed with a `cdp.mjs eval` throughput measurement (QA full).
- **The dead-window send error type** is not officially documented — worked around by detecting the `send` `Err` (do not depend on a type assertion).
- **The 2.11.3 deadlock** fix is not yet applied to us — an upgrade needs to be considered before using Channel in earnest (open).
- **Third-party binary IPC benchmarks such as tauri-conduit (~11×)** depend on environment and version, and the methodology is unverified (uncertain) — not a first-choice candidate, to be considered only after measurement.
- The exact wezterm client fanout data structure was not retrieved in the Codex pass (Claude C confirmed Mux notify) — no effect on our design.

## 6. Output → reflected in the design (summary)
1. **Splitting the carrier in two is confirmed:** output = `Channel<Response>` (raw) per window / layout control = emit. (D3 confirmed + raw detail added)
2. **OutputRouter:** `ArcSwap<RoutingSnapshot{by_agent}>` with zero locks on the hot path, store on layout change. (resolves the spike open item "snapshot update mechanism")
3. **Subscription ref-count:** `Mutex<HashMap<AgentId,usize>>` 0→1/1→0, `SubscriptionGuard` Drop→manager enqueue (avoids async Drop). (resolves spike §6 risk 3 "new concurrency surface")
4. **Cleanup convention:** send `Err`→remove the channel · window close `onCloseRequested`→unsubscribe+unlisten (#15583) · clean up large-queue residue.
5. **Version:** consider upgrading to tauri **2.11.3+** before wiring up Channel (deadlock fix).
