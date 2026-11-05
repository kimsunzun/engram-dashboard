# First auth frame timeout after connect — open-source defaults

- Status: done (2026-11-06). Tier: light (one source-reading collector; main re-read graphql-ws `connectionInitWaitTimeout = 3_000` (`src/server.ts:613`)). Why: our daemon's `AUTH_TIMEOUT = 1s` (`crates/engram-dashboard-net/src/ws.rs:124`) came from the S12 design draft (`b6145f30`, 2026-06-14) with no recorded reason; it decides whether a slow shell's handshake reads as a terminal refusal (ADR-0346, U3a review hold). Decision (user, 2026-11-06): raise the daemon's `AUTH_TIMEOUT` to 15 s (ADR-0347).
- Collector output follows verbatim.

# First auth/init message timeout — OSS server defaults (collection)

## Conclusion (first)
Servers that wait for an in-band first auth/init frame after the connection opens use defaults between **3 s and 60 s**. The common case is **5–15 s** (Mattermost 5 s, SignalR 15 s, EMQX 15 s). graphql-ws is the strict end at 3 s. socket.io (45 s) and PostgreSQL (60 s) are the loose end. Confidence: **certain** for each value below (read from source on the default branch, 2026-11-06). Calling 5–15 s "typical" is **likely**: it is my reading of 7 systems, not a measured survey.
Phoenix has no first-frame auth window. Auth runs in `connect/3` during the HTTP upgrade, using query params or connect_info. Its only timeout is a 60 s idle timeout.

## Table
| System | What it waits for | Default | Source line | Confidence |
|---|---|---|---|---|
| graphql-ws (enisdenjo) | `ConnectionInit` message after socket open; closes with 4408 "Connection initialisation timeout" | 3 s | `src/server.ts:613` (master): `connectionInitWaitTimeout = 3_000, // 3 seconds`; timer at :650-659 | certain |
| ASP.NET Core SignalR | Hub handshake request message (protocol negotiation) after transport connect | 15 s | `src/SignalR/server/Core/src/HubOptionsSetup.cs:15` (main): `internal static TimeSpan DefaultHandshakeTimeout => TimeSpan.FromSeconds(15);` used in `HubConnectionHandler.cs:124,145` | certain |
| socket.io server | Client joins any namespace (CONNECT packet, which carries the `auth` payload) | 45 s | `packages/socket.io/lib/index.ts:324` (main): `this.connectTimeout(opts.connectTimeout \|\| 45000);` doc :103 `how many ms before a client without namespace is closed`; enforced `lib/client.ts:111-118` (`if (this.nsps.size === 0) ... this.close()`) | certain |
| Phoenix Socket (Elixir) | No first-frame wait. Auth is `connect/3` at HTTP upgrade. `:timeout` is idle time since last received data | n/a (idle 60 s) | `lib/phoenix/transports/websocket.ex:31` (main): `timeout: 60_000,`; `lib/phoenix/endpoint.ex:1051-1052`: "`:timeout` - the timeout for keeping websocket connections open after it last received data, defaults to 60_000ms" | certain |
| EMQX | MQTT CONNECT packet after TCP open (`mqtt.idle_timeout`) | 15 s | `apps/emqx/src/emqx_schema.erl:4273-4277` (master): `{"idle_timeout", ... default => <<"15s">>`; `rel/i18n/emqx_schema.hocon:1420`: "The time that a new connection can stay without a CONNECT packet. When no CONNECT packet arrives in this time, the broker closes the connection." | certain |
| Mosquitto | No dedicated option. A new context starts with keepalive 20 and is added to the keepalive list on accept, so it expires at keepalive*1.5 with no CONNECT | ~30 s (derived; checked every ~5 s) | `src/context.c:86` (master): `context->keepalive = 20; /* Default to 20s */`; `src/net.c:274` `keepalive__add(new_context);`; `src/keepalive.c:71` `context->last_msg_in + context->keepalive*3/2` | likely (derived by reading the code, not run) |
| Mattermost | Session token, from the `authentication_challenge` frame or the upgrade-time cookie/header. Checked once by a ticker | 5 s | `server/channels/app/platform/web_conn.go:40` (master): `authCheckInterval = 5 * time.Second`; :710-714 `case <-authTicker.C: if wc.GetSessionToken() == "" { ... "did not authenticate" ... return }` | certain |
| PostgreSQL (non-WS) | Complete client authentication | 60 s | `src/backend/utils/misc/guc_parameters.dat:108-112` (master): `name => 'authentication_timeout' ... boot_val => '60'` (`GUC_UNIT_S`) | certain |

Note: Mattermost's 5 s is one ticker tick after writePump starts, so the real window is about 5 s. If the token arrived with the upgrade request, the check passes immediately.

## Gaps
- Jupyter Server and Zulip were not checked (budget). Zulip uses an HTTP event-queue/long-poll model, so a first-frame timeout may not apply. Not verified.
- Nginx was not checked. Nginx has no WS first-frame auth concept. `client_header_timeout` is HTTP only and was not read from source.
- Mosquitto's ~30 s was not run, and I did not check whether the `WITH_OLD_KEEPALIVE` build path behaves differently.
- SignalR: I did not confirm whether the handshake timeout also covers transport-level auth. Auth normally happens at HTTP negotiate/upgrade, before the handshake.

Most expensive step: tracing the Mosquitto accept → keepalive path (3 files) to derive the implicit pre-CONNECT window.
