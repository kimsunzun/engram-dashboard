# ADR-0394: Humans approve trust changes — agents only request, human vs agent is a typed caller, one shared ask-the-user seam with a native OS dialog

- 상태: 확정 (2026-11-20, basis: user decision P-C 2026-11-20 (「사람이 승인 하는게 맞다고 생각하는데」) + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20) · 부분 폐기 by ADR-0410 (decision 5 app route rule)
- 관련: ADR-0389 (the trusted-list accept question, regenerate) · ADR-0390 (what a trusted PC may do) · ADR-0395 (the OS account as the trust boundary) · ADR-0266 (OS code in platform) · ADR-0157 (`call_daemon_command` checkpoint) · ADR-0310 (decision 8: named policy entrances) · ADR-0055 · ADR-0307 (LLM command surface) · `docs/process/S37-multi-daemon/trd.md` (§4-8 · T18 · T32 · T38 · T41 · T42 · T43) · PRD R14 · R19 · AC14 · Amended by ADR-0410 (decision 5 app route rule)

## 맥락
S37's `peer.*` commands change who this PC trusts. Agents can call commands, and remote mail now reaches agents, so mail from another PC could talk an agent into widening trust (adding a PC, changing network settings, a new identity key, renaming a PC — labels drive `name@pc` routing, and three renames could swap two PCs). The user decided that trust changes need a human (P-C). The daemon also needs one place to ask the user that works with the app open, with the app closed, and on a machine without a display.

## 결정
1. **Who may call what.** Open to agents: `peer.list`, `peer.remove` (it narrows trust), `peer.edit` of an address. Gated: `peer.add`, `peer.setNetwork`, `peer.regenerateKey`, `peer.edit` of a name — an agent's call creates a pending request; a human's call applies at once. Human-only: `peer.approve` / `peer.deny` and the trusted-list accept question.
2. **Human vs agent = a typed caller**, never an argument and never the `entrance` string: a socket-delivered call from a live shell connection (`caller: Some(ConnId)` from `ConnRegistry`) = human; a control-route call carrying the agent token's `BoundIdentity` (`/control/call`, `/control/agent`, `ENGRAM_TOKEN`) = agent; the synthetic callers `CommandBus::invoke` allocates for relays are never human. `call_daemon_command` takes that typed caller; a request names the agent.
3. **The app must not turn an agent call into a socket call.** Every frontend slot action that issues a gated `peer.*` (add, approve, deny, accept-list, setNetwork, regenerateKey, rename) is `humanOnly` in `src/commands/registry.ts`, has no `help`, runs only through the human path (`runAsHuman` or a direct UI handler), and is never declared in the shell command table — so `window.__engramCmd.run`, the bus bridge and `/control/call` relays cannot reach it.
4. **Pending requests**: kind + arguments + requesting agent + time + origin; the agent is answered at once: "requested — waiting for the user's approval (request `<id>`)". Origin marking (best-effort, a hint, not a boundary): "after mail from `<pc>`" when remote mail reached that agent this run. Expiry 24 h; cap 8 (a 9th is refused with a hint to ask the user); an identical request is not added twice; memory only; listed by `peer.list`. Every answer consumes the request atomically before applying it; late or repeated answers are ignored. After a deny, the same (agent, kind) is refused for 10 min without asking; repeats while one waits are coalesced into "N requests waiting" and an answer applies only to the request on screen; at most one new OS dialog per request kind per minute; a dialog left open after its request was answered elsewhere or expired is dismissed, or marked stale where the platform cannot close it; "deny and stop asking for this agent this run" refuses that agent's gated calls until the daemon restarts.
5. **One shared "ask the user" seam** (daemon `ask_user`, no peer knowledge) for every question kind (trust requests, the trusted-list accept question, later kinds). Route, in order: the app while it shows a PC list slot (visible = in the active tab of a non-minimized window) → platform's native approval dialog (a Rust native-dialog crate, no custom window; topmost without stealing focus where the OS allows) → otherwise (no display) the request waits until a human answers in the app or it expires. A question is rerouted to the dialog when the app detaches or the slot closes. One dialog at a time, off the daemon's async tasks.
6. **Question content**: system-written text saying what is asked and what it allows, labelled "an agent asks" with the agent's id prefix; agent-supplied strings (agent name, PC name, URLs) quoted, on one line, length-capped; the other PC's full ID grouped for reading; choices Approve / Deny / Deny and stop asking this agent; default Deny.
7. **Agents never resolve a trusted-list mismatch or answer an approval**: both go through human paths only.

## 거부한 대안
- **Agents apply trust changes directly** — mail from another PC could talk an agent into widening trust; user decision P-C (「사람이 승인 하는게 맞다고 생각하는데」).
- **Human vs agent from an argument or the `entrance` string** — `entrance` is a log label (tests pass `"test"`), not an identity.
- **A relayable frontend command for approve / gated actions** — it would arrive as a socket call, i.e. as the human; `run` refuses `humanOnly`, while a missing `help` only hides an entry and does not block it.
- **A per-agent cooldown alone** — evaded by `agent.new`; hence the global per-kind dialog rate.
- **A custom approval window** — user rejected directly: 「공용창을 따로 개발하지는 말고 디폴트 쓰자고」.
- **A daemon-opened terminal for yes / no** — no precedent, it opens behind other windows, and a same-user agent can type into it.
- **A short fingerprint of the other PC's ID in the question** — a short one can be ground.
- **A human approve CLI for headless machines in v1** — it needs a credential agents never get, and today `engram` authenticates only with `ENGRAM_TOKEN`; later (T-118). Such requests wait in v1.

## 근거
- User decision P-C (2026-11-20): 「사람이 승인 하는게 맞다고 생각하는데」 · 「UI가 열리면 그거 기준으로 하는데 UI가 없으면 다른방법을 고민」 · 「공용창을 따로 개발하지는 말고 디폴트 쓰자고」 · 「플랫폼으로 분리되서 실행되는 공용이 있어야될것같네」.
- Precedents: Syncthing pending devices; Tailscale device approval; OpenClaw ("humans approve — not the agent itself"); MCP spec human-in-the-loop SHOULD; gpg-agent pinentry and the polkit agent (the daemon delegates the prompt); Reason Network `ask_user` (native Approve / Deny panel, topmost without activation).
- The 24 h / 8 values bound memory without timing out a user who is away briefly.
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- A new command that widens trust is gated the same way, and its app action is `humanOnly`, has no `help` and stays out of the shell command table.
- `peer.*` handlers finish on the first poll (daemon-table handlers are blocking); the approval wait never holds the CLI past its 10 s silence window.
- The gate stops the common path — an agent talked into a trust change through our commands — not a same-account process (ADR-0395).
- The native dialog lives in platform (ADR-0266); the dialog's text lives on the daemon side.
- Risk carried: whether the dialog shows for the WMI-launched daemon, and where (the foreground lock can put it behind other windows), is measured in U5; if it cannot show, questions wait for the app.

- Correction (2026-11-20, U2 platform piece): the native dialog is the Windows Task Dialog called directly (loaded at run time from comctl32 v6 through an activation context), not a Rust dialog crate — comctl32 v6 needs a manifest, and a static import stops manifest-less executables (the daemon, test binaries) at start with 0xC0000139 (`src-tauri/build.rs` records that failure); `rfd` also lacks a Deny default, topmost and external close.
