# ADR-0187: Settle the codex Phase 2 channel on app-server — exec does not stream characters

- 상태: 확정 (2026-09-09, basis: user decision + measurement of three modes)
- 관련: ADR-0004 (backend knowledge isolation) · ADR-0044 (input encoding · output cleaning) · ADR-0185 (session-restore invariant) · `.claude/handoff/attachments/codex-measurements-2026-09-09.md` (canonical measurements) · `docs/process/S21-codex-backend/trd-phase2a.md` §10-8 · step-log S21

## 맥락

Phase 1 started codex in a PTY as a **conversation screen for humans** (`codex` with no arguments) and drew it along the existing xterm path. The goal of Phase 2 is to draw that output on our chat surface **in the neutral vocabulary**, and for that codex has to be driven in a way other than the human screen. There were three candidate ways to drive it — `codex app-server` (another program drives it over JSON-RPC) · `codex exec --json` (give it one job, receive JSON lines, and it exits) · the official SDK.

★**The cost of postponing this choice actually occurred**★ — even though the adversarial review of the survey report pointed out 「표본이 app-server 쪽으로 편향됐다」 (the sample is biased toward app-server) (HIGH 5), the Phase 2a design document was revised up to a third edition without measuring. §10 of that document had marked this item **at the top of the question order**, and fifteen other items hung from it.

## 결정

**The Phase 2 channel is `codex app-server`.** Our client sends requests to that process and receives notifications over line-delimited JSON-RPC.

- The requests to send are **three kinds** by measurement — `thread/start` · `turn/start` · `turn/interrupt`.
- **Requests where the server asks us first are not handled in the 2a scope** — under the read-only + no-approval condition the measurement was 0 (11 kinds are declared). If that condition changes, this exemption dies.
- The conversation id is issued by codex and we receive it from the response (ADR-0185 decision 1 as is).

## 거부한 대안

- **`codex exec --json`** — ★**characters do not stream (measured)**★. The events that arrived were **only four**: `thread.started` · `turn.started` · `item.completed` · `turn.completed`, and the message body comes whole, all at once. When we killed 「1부터 30까지 세라」 at 4.5 seconds, only the first two lines had come out, and at the same moment app-server had already sent 10 fragments. Put on the chat surface, **the screen stays empty until the turn ends.** Two side facts: there is no means of cutting in mid-turn, so cancel is only a forced process kill, and a turn cut that way leaves **only the user message** in the record (no assistant output, no end marker). And the approval-policy flag (`-a`) **does not exist at all on `codex exec`.**
- **The official SDK** — ★**does not exist (measured)**★. Searching `codex --help` for `sdk|library|npm|typescript|python` gave 0 hits, and the installed `@openai/codex@0.153.4` also gave 0. The two closest are not libraries but code generation and a protocol (`app-server generate-ts` · `mcp-server`).
- **Use the Phase 1 PTY conversation screen as it is** — the screen is a terminal, so there are no structured events to move into the chat surface's neutral vocabulary. Phase 1 is already that road, and the goal of Phase 2 lies beyond it.

## 근거

Canonical source = `.claude/handoff/attachments/codex-measurements-2026-09-09.md` (codex-cli 0.153.4 · Windows 11 · 2026-09-09). In brief:

- **app-server streams fragments** — during a turn `item/agentMessage/delta` arrives in succession as `{threadId, turnId, itemId, delta}`, and 11 kinds of notifications were observed.
- ★**`turn/interrupt` succeeds in 18ms and the conversation survives**★ — the fragments stop at once, `turn/completed` closes with `status:"interrupted"`, and **the next turn with the same `threadId` worked normally.** That is, cancel does not require process termination.
- **The conversation id parses as a `Uuid`** — `thread.id == thread.sessionId`, UUIDv7 confirmed. It goes as is into the profile's existing `Option<Uuid>` field.
- **0 non-JSON lines on stdout**, 0 bytes on stderr. Teardown = close stdin, exit code 0 in 46ms.
- **The schema can be generated locally** (`generate-json-schema`, no network or model call) — there is a path to fill the translation table without guessing.

Both rejections are caught by measurement (no rejection rests on description alone).

## 영향 / 불변식

- ★**`codex` on PATH is a shim, so launching it directly from a program gives `ENOENT`**★ — our code already wrapping it in `cmd.exe /c` is handling for the same reason, and removing that wrapping kills this channel (`console_command` in `crates/engram-dashboard-agent/src/backend/mod.rs`).
- **Server responses omit the `jsonrpc` field.** The axis that separates responses from notifications is direction and shape (presence of `method` · `result`/`error`) and **not the id value** — the value space of server request ids and our request ids can overlap.
- **The turn id is `result.turn.id`** (not `result.turnId`). `turn/interrupt` requires that value as a string and rejects `null` with `-32600`.
- **The `item/completed` of an interrupted message does not come** — its `item/started` stays unclosed, so a translator that assumes 「시작한 아이템은 반드시 닫힌다」 (an item that starts is always closed) breaks.
- **Declared ≠ observed.** The schema declares 155 client request kinds · 81 server notification kinds · 11 server request kinds, but what was observed is only 11 notification kinds. ★**Do not write 「온다」 (it comes) just because it is in the schema**★ — that is why this repository's source grades separate `[스키마]` from `[실측]`.
- **The values this decision opens** — five items of §10 of the Phase 2a design document (request/response contract · telling killed from not · normal exit · inbound path · the driver's access to the profile) hold only with app-server as the premise. Had `exec` been chosen, those five would have vanished, and that difference in size is the substance of this ADR.
- **Triggers to reopen this decision** — ① if approvals are actually to be received (the value design of ADR-0188), the 11 server request kinds come alive and the 「0 건」 (0 hits) exemption dies ② if the cost of one resident process per conversation, or an incident where the server dies and truncates the record because of an expired credential (reported as an upstream issue), actually occurs, one-shot `exec` becomes a candidate again.
