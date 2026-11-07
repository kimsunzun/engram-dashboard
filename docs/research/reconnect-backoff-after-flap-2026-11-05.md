# Reconnect backoff after a flapping connection — open-source convention

- Status: done (2026-11-05). Tier: light (one source-reading collector; main re-read reconnecting-websocket `minUptime` (`reconnecting-websocket.ts:42,432,435`), Zulip `server_events.js:196` reset on success and socket.io `manager.ts:562` `backoff.reset()`). Decision: pending with the user — ADR-0346 hold on flap retries at the base delay (review of 3-2 U3a).
- Collector output follows verbatim.

# Flapping-connection retry delay: OSS convention (collect pass 1)

Sources fetched 2026-11-05 from default branches (raw.githubusercontent.com): pladaria/reconnecting-websocket@master, grpc/grpc@master, grpc/grpc-go@master, socketio/socket.io@main (packages/socket.io-client), kubernetes/kubernetes@master + kubernetes/client-go@master, systemd/systemd@main, microsoft/vscode@main. Zulip (zulip/zulip main @7380849) and Rocket.Chat (develop @fb49d89) come from the previous run's clones under `scratchpad/oss/`. Line numbers are for those snapshots.

## Conclusion (first)

**Q1: reset on connect, or only after a stability threshold?**
Most network-client libraries in this sample reset the backoff as soon as the connection or handshake succeeds, with **no stability threshold**. So a peer that keeps connecting and then dropping is retried at the base delay forever. This holds for socket.io-client, gRPC (spec and grpc-go), Rocket.Chat DDP, Zulip and VS Code remote. Confidence: likely. Each library is certain from its source; calling it "the convention" from 7 systems is an inference.
Two systems do use a stability threshold, and these are the values to cite:
- **reconnecting-websocket**: `minUptime` defaults to **5000 ms**. The retry count goes back to 0 only if the socket stays open for 5 s. A drop before that keeps the count, so the delay keeps growing. Confidence: certain.
- **Kubernetes CrashLoopBackOff**: the backoff resets only after **2 × max backoff** with no restart. Max backoff defaults to 300 s, so the window is **600 s (10 min)**. With the `ReduceDefaultCrashLoopBackOffDecay` gate, max is 60 s and the window is 120 s. With the `KubeletCrashLoopBackOffMax` gate, max is configurable. Confidence: certain for the code. That the reset in practice means "the container ran for about 10 min" is my reading of how `Next` and `hasExpired` interact (likely).

**Q2: a separate cap on flapping (N restarts within a window)?**
None of the client libraries examined has a separate flap cap. Their attempt limits (`maxRetries`, `reconnectionAttempts`, the Rocket.Chat retry budget, the VS Code grace time) count failed attempts within one outage, and a successful connect resets them. Confidence: likely, since only the files listed were read. A burst cap does exist in process supervisors: **systemd** `StartLimitBurst`/`StartLimitIntervalSec` default to **5 starts in 10 s**. When the limit is hit, systemd stops restarting the unit (no slow-down) until the interval passes or someone runs `systemctl reset-failed`. Confidence: certain. Kubernetes has no hard cap. Because its backoff is not reset on start, flapping is slowed to the 300 s maximum instead.

## Table

| System | Reset on connect? | Stability threshold | Flap limit | Source line path:line | Confidence |
|---|---|---|---|---|---|
| reconnecting-websocket | No. Reset only after `minUptime` | `minUptime: 5000` ms (default) | None. `maxRetries: Infinity` by default and the count is never reset by a short connection | `reconnecting-websocket.ts:42` `minUptime: 5000,`; `:435` `this._uptimeTimeout = setTimeout(() => this._acceptOpen(), minUptime);`; `:414` `this._retryCount = 0;` (inside `_acceptOpen`); `:511` `clearTimeout(this._uptimeTimeout);` (on close); `:316-318` delay grows with `_retryCount` | certain |
| gRPC spec | Yes, on receipt of the server SETTINGS frame | None | None | `doc/connection-backoff.md` "Reset Backoff": "We choose to reset the Backoff when the SETTINGS frame is received, at that time point, we know for sure that this connection was accepted by the server." Pseudo-code tries immediately and sleeps only after a failure | certain |
| grpc-go | Yes. `backoffIdx = 0` after the transport is created, which blocks until the server preface | None | None | `clientconn.go:1434-1437` `// Success; reset backoff.` ... `ac.backoffIdx = 0`; `internal/transport/http2_client.go:486` `// Block until the server preface is received successfully or an error occurs.`; on transport close the addrConn goes Idle (`clientconn.go:1550`) | certain (reset). Whether the next dial after a drop is immediate was not traced through pick_first: uncertain |
| socket.io-client | Yes. Backoff reset on every close and on reconnect success | None | None. `reconnectionAttempts` defaults to Infinity and counts failures within one outage | `packages/socket.io-client/lib/manager.ts:562` `this.backoff.reset();` (in `onclose`); `:631` `this.backoff.reset();` (in `onreconnect`); `:172` `opts.reconnectionAttempts \|\| Infinity` | certain |
| Kubernetes CrashLoopBackOff | No. Never reset on start; reset only by time | 2 × maxDuration = 600 s (default max 300 s) | No hard stop. Delay capped at max (300 s default) | `client-go/util/flowcontrol/backoff.go:186` `return eventTime.Sub(lastUpdate) > maxDuration*2 // consider stable if it's ok for twice the maxDuration`; `:100-101` reset branch in `Next`; `pkg/kubelet/apis/config/v1beta1/defaults.go:45` `MaxContainerBackOff = 300 * time.Second`; `pkg/kubelet/kubelet.go:171` `reducedMaxCrashLoopBackOff = 60 * time.Second`, `:174` `initialCrashLoopBackOff = time.Second * 10`; `pkg/kubelet/kuberuntime/kuberuntime_manager.go:2123,2137` use the container's `FinishedAt` as eventTime | certain |
| systemd | n/a (process restart) | None (counts starts) | Yes. More than `burst` starts within `interval` stops further starts. Defaults: 5 in 10 s | `man/systemd.unit.xml:1217-1219` "Units which are started more than burst times within an interval time span are not permitted to start any more."; `:1235-1236` units that hit the start limit "are not attempted to be restarted anymore"; `man/systemd-system.conf.xml:735-737` "DefaultStartLimitIntervalSec= defaults to 10s. DefaultStartLimitBurst= defaults to 5."; `src/core/system.conf.in:57-58` | certain |
| Zulip web (long-poll) | Yes. `get_events_failures = 0` on any successful response | None | None. A banner appears at 8 or more consecutive failures | `web/src/server_events.js:196` `get_events_failures = 0;`; `:233` `if (get_events_failures >= 8) {` | certain (long-poll, so this is an analogue, not a socket) |
| Rocket.Chat DDP client | Yes. `retryCount = 0` on the DDP `connected` message | None | None (the retry budget is reset on success, and the comment explains this is deliberate) | `packages/ddp-client/src/Connection.ts:236` `if (payload.msg === 'connected') {`; `:247` `this.retryCount = 0;` | certain (reset). Grep-level only for a threshold: likely none |
| VS Code remote (reconnect) | Yes. Each disconnect starts a fresh loop with `attempt = -1` | None | None across flaps. One outage is bounded by the reconnection grace time. Initial connect: `MAX_ATTEMPTS = 5` | `src/vs/platform/remote/common/remoteAgentConnection.ts:660` `const TIMES = [0, 5, 5, 10, 10, 10, 10, 10, 30];`; `:671` `let attempt = -1;` (local to `_runReconnectingLoop`); `:438` `const MAX_ATTEMPTS = 5;` | certain (structure). Value of `ProtocolConstants.ReconnectionGraceTime` not fetched |

## Gaps

- I did not trace grpc-go's pick_first path after a Ready to Idle transition, so it is unknown whether a flapping gRPC peer is redialed with no delay or after some delay. The spec pseudo-code implies no delay on the first try.
- gRPC core (C++) and grpc-java implementations were not read. Only the spec and grpc-go were.
- I did not open the `engine.io-client`/`backo2` duration formula for socket.io, so the base delay and jitter values are not confirmed here. Only the reset points are.
- `ProtocolConstants.ReconnectionGraceTime` (VS Code) value: unknown, not fetched.
- Kubernetes: the doc wording ("resets after 10 min of running") was not cross-checked against official docs. The 10 min figure is derived from code (2 × 300 s).
- systemd's per-unit restart backoff (`RestartSteps=`/`RestartMaxDelaySec=` in systemd.service) was not read. It may also affect flap slow-down.
- No other client library with a "stable for N s" threshold was searched for beyond the list (e.g. Erlang/OTP supervisor intensity/period, Akka BackoffSupervisor reset, Envoy).

Most expensive step: downloading and grepping the large Kubernetes sources (kubelet.go 157 KB, then locating `MaxContainerBackOff` across three candidate files).
