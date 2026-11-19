# Shell ↔ daemon link (daemon side)

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0129 (daemon layers: network row · agent system · assembly) · ADR-0177 (transport crate) · ADR-0310 (stage-3 transport attachment) · ADR-0346 (shell retry and refusal policy) · ADR-0021 (a reconnect never starts a daemon) · ADR-0347 (`AUTH_TIMEOUT`, pre-auth window) · ADR-0206 (read loop hands off; read / dispatch / bypass lane split).

**Whole picture** = `docs/reference/architecture.md` · **Transport crate** = `docs/reference/structure/transport.md`

## What it is

The daemon accepts shell connections on one loopback WS listener built from the transport crate. `crates/engram-dashboard-daemon/src/link/` is the daemon's network row: it binds the listener, decides the first-frame auth, attaches each accepted link to an accept-only transport `Registry<DaemonWire>`, and runs per-connection read / dispatch tasks. It meets the upper layers only through the connection port `crates/engram-dashboard-daemon/src/conn_port.rs` (`FrameSink` · `FrameFanout` · `ConnectionHandler` · `ConnectionHandlerFactory`); it never decodes a command. The shell is the dialing half: `src-tauri/src/daemon_client/link/` (`ShellWire`, `ShellHandshake`, `LocalDaemonDialer`) driven by `src-tauri/src/daemon_client/connection.rs`. The packet types are `AgentCommand` (shell → daemon) and `AgentEvent` (daemon → shell) from `engram-dashboard-protocol`; the two `Wire` impls are not in protocol — `DaemonWire` is in the daemon, `ShellWire` in the shell.

## Modules

Daemon (`crates/engram-dashboard-daemon/src/`):

- `link/mod.rs` — module root; the vocabulary wall (agent vocabulary only through `conn_port`).
- `link/wire.rs` — `DaemonWire`: raw pass-through of text and binary frames; no pairing, no stream layer; never fails to decode.
- `link/auth.rs` — `AuthCheck`: first-frame decision (constant-time token, protocol version); `Refusal` carries the `RejectKind`. Token and version are injected.
- `link/handshake.rs` — `DaemonHandshake`: transport `Handshake` per accepted link; `start` = `Await`, `on_frame` = `AuthCheck` → `Done` or `Reject` with the upper layer's `Error` text (`ConnectionHandlerFactory::handshake_error_frame`).
- `link/server.rs` — `Listening::bind_loopback` (WS listener on `127.0.0.1`, kernel-picked port, `accept_policy`), `Server` (`new` · `run` · `serve` · `run_connection`), `ConnRegistry` (connection ids + all-connection fan-out), `ConnFrameSink` (per-connection `FrameSink`), `KeepaliveConfig`, `NoDialer`; read task, arrival-order lane, bypass lane, dispatch task, teardown.
- `link/server/tests.rs` — adapter tests over synthetic inbox items and a real loopback listener.
- `conn_port.rs` — the contract between the network row and the upper layers: `ConnId`, `Frame`, `FrameSink`, `FrameFanout`, `ConnFlow`, `Saturated`, `ConnectionHandler`, `ConnectionHandlerFactory`.
- Upper layers (not part of the link, implement or use the port): `agent_conn.rs` (`AgentConnections` = the factory, `AgentConnection` = the handler; all wire encoding), `connection_core.rs` (`ConnectionCore`, transport-neutral command handling), `status_fanout.rs` (`DaemonStatusSink` → `FrameFanout`).
- Assembly: `lib.rs` (`DaemonWiring`, `build_daemon_wiring_with_store`, `run_accept_loop`, `run`, `start_test_server*`); `portfile.rs` (`daemon.json`).

Shell (`src-tauri/src/daemon_client/`), only as far as the pair needs:

- `link/wire.rs` — `ShellWire`: decodes JSON text frames to `AgentEvent`, passes binary output frames through; pairing via `command_request_id` / `event_reply_request_id`; no stream layer.
- `link/handshake.rs` — `ShellHandshake`: `start` sends `Auth`; `Hello` → `Done`; an `Error` before `Hello` → terminal refusal.
- `link/dialer.rs` — `LocalDaemonDialer`: resolves the logical address `engram-local:<key>#<n>` through `EndpointSource::current` (never `ensure`) on every dial; version mismatch → `DialFailure::Refused`.
- `link/endpoint.rs` — `EndpointSource` (`ensure` may start a daemon, `current` never does), reads `daemon.json`.
- `link/mod.rs` (`DaemonKey`, `HandleToken`), `link/pump.rs`, `link/route.rs`, `link/errors.rs`, `link/stop.rs` — inbox pump, routing / key filter / bands, caller-facing failure texts, the one-shot stop client.
- `connection.rs` — the connection driver actor that owns the shell's `Registry<ShellWire>` and every `Registry::add`.

## Wiring and flow

- Boot (`lib.rs` `run`): `Listening::bind_loopback` → MCP server → `build_daemon_wiring` (one `ConnRegistry` whose `FrameFanout` face feeds the manager's status sink chain) → `daemon.json` written with host `127.0.0.1`, port, token, protocol version (`portfile::DaemonInfo`) → `run_accept_loop` builds `AgentConnections` and `link::server::Server::new(listening.clock(), registry, handlers, token, PROTOCOL_VERSION, keepalive)` → `Server::run(listening, stop)`. The registry inside `Server` must use the listener's clock (`Listening::clock`): the listener stamps `Accepted::upgraded_at` with it and the auth deadline is measured from that stamp (`transport.md`, clock pairing on accept).
- Accept (`Server::run` → `serve`): every `WsListener::accept` result is spawned as one connection; label = `accepted-<n>`, formed in `Server::serve` from the counter field `Server::labels`; `Registry::attach(label, accepted, DaemonHandshake)`. The registry never dials (`NoDialer`). Refusal reports from `AcceptRefusals` are logged (`log_refusal`).
- Accept rules: listener bound to loopback only (the transport refuses any other bind); Origin must be in `ALLOWED_ORIGINS` or absent; Host must be one of `ALLOWED_HOST_NAMES` with the bound port or absent; upgrade deadline and auth deadline = `AUTH_TIMEOUT`, auth deadline counted from the end of the upgrade; first frame must be `Auth` with the daemon.json token and `PROTOCOL_VERSION`. Token mismatch → `RejectKind::Retryable`; version mismatch, non-`Auth` text or a binary first frame → `Terminal`.
- Live (`run_connection`): `wait_live` sees `Connected` → `ConnRegistry::alloc_id` (ids start at 1, only go up) → `register` in the fan-out map → `ConnectionHandlerFactory::handler_for(conn_id)` → `on_connect` (puts `Hello` and the initial list) → spawn read task, dispatch task, bypass task. `on_text` / `on_binary` run one at a time in arrival order on the dispatch task; under saturation the handler picks `Saturated::Bypass` / `Refused` / `Unanswered` (`feed_lanes`; `Unanswered` ends the connection).
- Out: per-connection frames go through `ConnFrameSink` (`try_send` from sync threads, `send` only for `on_connect` and `Close`) to `Peer::notify_in` on the generation the connection went live on; `conn_port::Frame::Close` maps to `Peer::close_after_flush`. All-connection frames go through `ConnRegistry::broadcast_text` (skips saturated connections).
- End: three ends (dispatch finished, write side ended, read side ended — `link/server.rs` header). In every branch `on_disconnect` runs before the fan-out drops the connection, and the read task drains the inbox to `None`. Shutdown: `stop` resolves (StopDaemon watch or Ctrl-C) → listener dropped → `Registry::close_all(policy.flush_timeout)`.
- Shell half: `connection.rs` actor → `Registry::add(DaemonKey, engram-local address, ShellHandshake)` with a fresh `LocalDaemonDialer` registration per add → dial reads the current `daemon.json` endpoint and token → `Auth` → daemon `Hello` → `LinkUp` (the actor queues `UsageSubscribe` and `RegisterCommands`, then opens the connection to callers). Retries on network failure never run out; no refusal is retried (ADR-0346).

## Promises no single file shows

- One connection registry per daemon: the `ConnRegistry` given to `Server::new` must be the one whose fan-out sits inside the manager's status sink (`DaemonWiring` keeps them as one value). A mismatched pair compiles and silently loses every broadcast.
- Connection ids are never reused; the command bus relies on it (`command_delivery` `Pending::origin`, `CommandBus::open_caller` counts down from `u64::MAX` so the two never meet).
- `on_connect` runs before any task is spawned, so command dispatch never overtakes the greeting; but fan-out frames may already be queued before `Hello` (registration precedes `on_connect`).
- Middle handler calls never overlap except a frame the handler chose to bypass; `on_disconnect` can overlap a cancelled `on_text` (`ConnectionHandler` doc).
- Saturation asymmetry: per-connection `FrameSink::try_send` / `send` close that connection on a full queue; `FrameFanout::broadcast_text` skips it and closes nothing.
- The daemon link never awaits a transport request (`DaemonWire` pairs nothing), so transport receiver invariant 1 holds by construction; invariant 5 (drain to `None`) is kept by the read task.
- The network row reads exactly one frame itself (the `Auth` frame); everything else is opaque bytes handed to the handler. Logs never quote the auth frame (it may carry the token).
- The listener is loopback-only and the auth is a shared token from `daemon.json`; nothing here identifies a peer by key (`LinkInfo::peer_key` is unused).
- Shell side: a reconnect never starts a daemon (`EndpointSource::current` only); the driver actor never awaits anything that waits on its inbox.

## Where to look

- Daemon accept: `crates/engram-dashboard-daemon/src/link/server.rs` (`Listening::bind_loopback`, `accept_policy`, `accepted_policy`, `Server::run`, `Server::serve`, `Server::run_connection`, `ConnRegistry`, `ConnFrameSink`).
- Auth and handshake: `link/auth.rs` (`AuthCheck::check_text`), `link/handshake.rs` (`DaemonHandshake`).
- Port contract: `crates/engram-dashboard-daemon/src/conn_port.rs`; its implementers `agent_conn.rs` (`AgentConnections`, `AgentConnection`), `connection_core.rs`.
- Assembly: `crates/engram-dashboard-daemon/src/lib.rs` (`DaemonWiring`, `run_accept_loop`, `run`).
- Shell pair: `src-tauri/src/daemon_client/link/` (`wire.rs` `ShellWire`, `handshake.rs` `ShellHandshake`, `dialer.rs` `LocalDaemonDialer`, `endpoint.rs` `EndpointSource`), `src-tauri/src/daemon_client/connection.rs`, `src-tauri/src/daemon_client/mod.rs` (`WsDialer` construction).
- Tests: `crates/engram-dashboard-daemon/src/link/server/tests.rs`, `crates/engram-dashboard-daemon/tests/ws_e2e.rs`.
