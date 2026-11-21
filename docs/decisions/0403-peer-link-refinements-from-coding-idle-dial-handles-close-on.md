# ADR-0403: Peer link refinements from coding: idle dial handles close on an accepted link, same-direction flaps, run-long duplicate evidence, one refusal-log budget, revocation abort

- 상태: 확정 (2026-11-22, 근거: S37 U2 piece C2 code review deep, 3 rounds + QA)
- 관련: Amends ADR-0388 (decision 7 dialing handle removal) · Amends ADR-0389 (decision 8 flap rule and evidence lifetime) · ADR-0390 (decision 3, a redialing stranger must not flood the log) · TRD `docs/process/S37-multi-daemon/trd.md` §4-2 · AC7

## 맥락
Building the peer link (`link/peer/book.rs`, `gate.rs`, `runtime.rs`; transport `Peer::abort`) showed that the rules of ADR-0388 decision 7 and ADR-0389 decision 8 (c), read literally, either churn or raise false alarms on ordinary reconnects, and that three edges were left open: what the duplicate evidence survives, how refused strangers are logged, and how fast a removed PC is cut. The review found concrete event sequences for each (a backoff dialing handle connecting late after an accepted link; 3 such reconnects in 10 min → a false "suspected duplicate" on both PCs; `set_network` wiping the evidence; a key-rotating stranger producing one warn line per attempt; a revoked PC that stops reading staying connected for the 2 s flush).

## 결정
1. **A dialing handle that is between attempts (idle, backoff or gave up) is closed the moment the other side's link to that PC comes up; only a handle that is dialing or handshaking right then is left to the tie-break.** Narrows ADR-0388 decision 7 ("removed while an accepted link is live") to what can be closed without dropping both links.
2. **A flap counts only when a link replaces one in the same direction** (accepted replaces accepted, dialed replaces dialed). A replacement across directions is one PC's dial meeting the other's — an ordinary reconnect — and is not counted. Narrows ADR-0389 decision 8 (c). Residual, accepted: when the other PC notices a loss first and redials before ours idles out, its new link replaces the half-open one in the same direction and counts; three such long drops within the flap window still warn.
3. **The duplicate evidence lives for the daemon run**: replaced and refused runs, the flap history and the warnings survive an endpoint restart (identity or network settings change); a restart resets only links and dialing (`LinkBook::restart`). Warnings are not cleared within a run.
4. **One global refusal-log budget per window** (`gate.rs` `LOG_WINDOW` 60 s): the first refusal of a window logs at once, the rest fold into one line with the count and the number of distinct PCs; the change signal ticks on the first refusal and once more at the end of a window that folded anything. The per-PC list (32) stays for state only.
5. **Revocation and the not-trusted cut close at once through a transport abort** (`Peer::abort` → iroh `Connection::close`): no flush, no wait for the tail to be acknowledged, and it also cuts a graceful close already waiting. Graceful closes (daemon stop, going away) keep the flush.

## 거부한 대안
- **Keep the dialing handle until its next failure (the first coding)** — its late connect replaced the working link and counted as a flap; ordinary reconnects then raised a false duplicate warning (review: event sequence through `settle_dial` / `link_up`; test `replacements_across_directions_are_ordinary_reconnects_and_raise_no_suspicion`).
- **Close every dialing handle at once, even mid-attempt** — closing an in-flight dial on one side while the other side keeps that same connection by the tie-break drops both links.
- **Count every replacement as a flap** — cross-direction replacements are the normal shape of a reconnect after an outage, sleep or restart.
- **Reset the evidence with the endpoint** — after `set_network` a replaced copy could reconnect within the duplicate memory without the immediate refusal (review; test `the_duplicate_evidence_survives_an_endpoint_restart`).
- **Per-PC refusal throttling only** — a stranger rotating keys, or more than 32 dialers, gets one warn line per attempt with no bound (review; test `a_hundred_distinct_keys_in_one_window_give_one_line`).
- **Revoke with the graceful close** — a PC that stops answering keeps the connection up to the 2 s flush, past AC7's 1 s (transport test with a frozen peer: 2.0 s on the old path, under 1 s with the abort).

## 근거
Code review deep (lead + risk-axis lens + codex), 3 rounds, findings with event sequences; daemon tests 948 → 1000 and transport `iroh_link` 29 → 32, each rule pinned by a test that fails when the rule is removed (checked by the coder and the reviewers).

## 영향 / 불변식
- `link/peer/book.rs` (`settle_dial`, `link_up`, `restart`, `claim_probe_answer`), `gate.rs` (`Knocks`), `runtime.rs` (`on_link` dial-idle read, `close_node`), transport `peer.rs` (`Peer::abort`, `AbortSignal`, `Supervisor::drop_link`), `link.rs` (`LinkTx::abort`), `iroh.rs` (`IrohTx::abort`).
- Breaking 1 or 2 brings back false duplicate warnings on ordinary reconnects; breaking 3 lets a replaced copy back in after a settings change; breaking 4 lets the network fill the log; breaking 5 breaks AC7 (removal bites within 1 s).
- The shell↔daemon WS link never calls `Peer::abort`; its close path is unchanged.
