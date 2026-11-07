# Client started without its daemon — message and "no selection" convention

- Status: done (2026-11-07). Tier: light (one source-reading collector; main re-read the Docker, kubectl and Tailscale message strings in the original sources via `gh api`). Decision (user, 2026-11-07): the failed-`ensure` text follows the convention — cause + a short question hint ("— is the daemon running?"); applied in 3-2 U4a. The multi-daemon "no daemon chosen" state is left to the multi-daemon PRD.
- Question: when a client's daemon is not running, what does the client say (cause only, or cause + next step), does the client start the daemon itself, and do multi-endpoint clients have a real "nothing selected / not connected" state?
- Why asked: the U4a production switch changed the `ensure` failure text from "daemon down — connect(명시 시작) 필요 (ADR-0021: ensure 는 respawn 안 함)" to "daemon websocket 접속 실패: no daemon endpoint record — no live daemon". The user also noted that multi-daemon will need a "no connection chosen" state.
- Collector notes (raw table with file:line): copied below; scope = Docker CLI, kubectl, Tailscale CLI, tmux only.

## Conclusion (first)

- **Three of four put a short next-step hint after the cause; tmux gives the cause only.** Docker: "Cannot connect to the Docker daemon at %s. Is the docker daemon running?". kubectl: "The connection to the server %s was refused - did you specify the right host or port?". Tailscale: "failed to connect to local tailscaled; it doesn't appear to be running" + " (sudo systemctl start tailscaled ?)" on Linux with systemd, "…; is the Tailscale service running?" on Windows. tmux: "no server running on %s". The hint is phrased as a question, not an order. (likely — strings read in the original source; four tools only)
- **Of these CLIs only tmux starts its own server, and only for some commands** (`new-session`, `attach-session`, bare `tmux`; not `ls`). No start path was found in the Docker, kubectl or Tailscale CLI source; service managers / desktop apps do it (not checked in source). (likely)
- **Docker and kubectl avoid a "nothing selected" state by always resolving to a default endpoint.** Docker's context resolution ends in the built-in "default". kubectl's `current-context` can be unset, but ordinary commands then fall back to `http://localhost:8080`, which surfaces as a confusing "connection refused" instead of "no context chosen" (traced, not run). (likely)

## Findings

| Tool | Message (verbatim) | Cause only / + hint | Starts the daemon | "No selection" | Source | Confidence |
|---|---|---|---|---|---|---|
| Docker CLI | `Cannot connect to the Docker daemon at %s. Is the docker daemon running?` · empty host: `Cannot connect to the Docker daemon. Is the docker daemon running on this host?` | cause + hint (question) | no path in the CLI found | always a context — falls back to `"default"` | moby/moby `client/errors.go` L38-40 ✓ main re-read · docker/cli `cli/command/cli.go` L456-474 | message certain · no auto-start likely |
| kubectl | `The connection to the server %s was refused - did you specify the right host or port?` · otherwise `Unable to connect to the server: %v` · `current-context is not set` | cause + hint | n/a (remote server) | can be unset, but falls back to `localhost:8080` (or `$KUBERNETES_MASTER`) | kubectl `pkg/cmd/util/helpers.go` L272 ✓ main re-read · client-go `tools/clientcmd/client_config.go` L40-56 | strings certain · fallback path likely (traced, not run) |
| Tailscale CLI | Linux: `failed to connect to local tailscaled; it doesn't appear to be running%s` (hint ` (sudo systemctl start tailscaled ?)`) · Windows: `…tailscaled process; is the Tailscale service running?` · macOS: `…Tailscale service; is Tailscale running?` · process found: `…(which appears to be running as %v, pid %v). Got error: %w` | cause + hint; tells "not running" from "running but unreachable" | CLI: no; GUI/service: unknown | single local endpoint | tailscale `cmd/tailscale/cli/diag.go` L55-71 ✓ main re-read | strings certain · auto-start uncertain |
| tmux | `no server running on %s` (stale socket) · `error connecting to %s (%s)` (socket missing) · `no sessions` | cause only | yes for `new-session` / `attach-session` / bare `tmux`; not for `ls` | always a default socket | local clone `client.c` L120-133, L291-299 · `cmd-attach-session.c` L46 · `cmd-list-sessions.c` L48 (clone @5eaf557, shallow) | certain for the code read |

## Implications for us (inference — one confidence step lower)

- A failed `ensure` text in the convention's shape would be cause + a short question hint, e.g. "no live daemon (no endpoint record) — is the daemon running?". The old text (cause + an order to connect) and the new one (cause only, tmux-like) are both outside the majority shape.
- Our app starts the daemon itself on launch (`bootstrapDaemonIfNeeded`); `ensure` (attach without spawning, ADR-0021) is the path that hits this text — popouts and LLM/bus commands while the daemon is down.
- Multi-daemon: Docker/kubectl keep an always-resolved default instead of a "none" state, and kubectl's silent localhost fallback is a known confusion. Whether our multi-daemon model gets a real "not connected / none chosen" state or a default (the local daemon) is for the multi-daemon PRD; if a "none" state exists, its message should say that no daemon is chosen rather than reporting a connection failure.

## Gaps

- Four tools only; no GUI clients (Docker Desktop, Lens, Portainer) and no `tailscale switch` profiles.
- Docker/kubectl/Tailscale were read through `gh api` (no local clones).
