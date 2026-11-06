# ADR-0272: A packet definition does not embed another crate's structs — fixed packets are copies in protocol, and the command bus is a raw JSON field

- 상태: 확정 (2026-10-02, basis: user decision 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` decision candidate 8 — principle · confirmed shape) + the reply-pairing is the main session's judgment + research `docs/research/wire-shared-types-placement-2026-10-02.md` (medium · cross-family adversarial review FIX → applied))
- 관련: Amends ADR-0177 (the home of the canonical pairing-number function in decision 4) · ADR-0155 (unified command bus — no stamp, see Impact below) · TRD `docs/process/S20-command-bus/trd.md` §3-1 · §5 protocol row (the real source of 「protocol → command 의존 0 → 1」 — this decision reverses it) · ADR-0081 decision 4 (single `request_id` round trip — unchanged) · ADR-0129 (premise of preserving the wire JSON shape) · ADR-0270 (the client does not know agent) · decision candidate 7 (to be fixed at the transport TRD) (authentication message → `protocol`) · `crates/engram-dashboard-protocol/src/messages.rs` · `crates/engram-dashboard-protocol/src/ids.rs` · step-log S21 · Amends ADR-0175 (the dependency-graph line under Impact where protocol holds command)

## 맥락

`protocol` = the **list of packets** the shell and the daemon exchange (+ encoding · protocol version · TS bindings for the frontend). Its boundary = it depends on command, and the daemon, the shell and the frontend use it.

`protocol` takes four command-bus forms from command and embeds them — the envelope (`CommandEnvelope`) · the reply (`CommandReply`) · the registration entry (`CommandDecl`) · the owner marker (`OwnerToken`) (the `use` at the head of `crates/engram-dashboard-protocol/src/messages.rs`). There are already two paths that carry feature data — ① the command bus (the module decides the shape, and it goes 「속」 the JSON) ② `protocol` copy types (the agent list etc. — copied over at the daemon's edge).

The user (2026-10-02): 「패킷이 똥통이 되잖아」 · 「바닥 구조체가 아니라 장기적으론 모든 구조체를 품어야 되는 거잖아」 · 「다른 건 다 패킷 구조체 방식으로 하고 command 만 json 으로」.

## 결정

1. **Principle: `protocol` does not embed another crate's structs — the bottom-layer tool (command) is no exception** (user 2026-10-02). Allow it once and every struct flows in each time a feature is added.
2. **Fixed packets** (agent list · status · subscription · profile etc.) **= defined in `protocol` as types** (user 2026-10-02 — Zed style · when it grows, split into per-area modules). For feature data, `protocol` keeps a copy for communication and the shell · daemon edge (the transport adapter) copies the data over. agent does not know `protocol`.
3. **Command bus = the four command fields of the packet (command · reply · registration · registration change) are `Box<RawValue>`** (user 2026-10-02). It only carries raw bytes, and unpacking into envelope structs is done at the shell · daemon edge. It is the same shape as the JSON-RPC envelope · Tauri commands (commands are an open set that keeps growing). → The `protocol` → command dependency disappears and the wire bytes stay the same.
4. **Reply pairing (main session's judgment):** the edge adapter unpacks the envelope · reply into structs the moment it receives the packet, and reads the request number for pairing **from the unpacked struct** (natural, since transport's `reply_tag` is inside the adapter). The number is not pulled out of the field, so the bytes · protocol version stay the same and there is no extra parsing. The pairing functions now inside `protocol` (`command_request_id` · `event_reply_request_id`) move to the adapter.
5. **We accept the cost** — when unpacking the outer packet, the envelope is scanned once for validation (serde_json also scans a `RawValue` field to check it is valid JSON), and unpacked once more at the edge. No intermediate tree is created. Command packets are rare, so it is negligible.
6. **To be newly written** — the handling of the case 「패킷은 풀렸는데 봉투가 깨진」 · unpacking / re-wrapping on the daemon relay path.
7. **Start = at transport attachment** (work order 3-1 `protocol` cleanup).

## 거부한 대안

- **Keep embedding the envelope structs as now.** Rejected = user decision (「패킷이 똥통이 되잖아」 — the principle of decision 1).
- **Split command into a definition-only crate (envelope · reply · registration entry · error shape) and a tool crate, and `protocol` embeds only the definition-only crate (「헤더만 공유」).** It has the advantage that type checking stays in `protocol`. Rejected = user principle (decision 1 — it goes against 「패킷은 남의 구조체를 품지 않는다」. The research 「우리에게」 raised it as a user-decision item).
- **The packet embeds domain types directly (wezterm `codec` style — `mux::pane::PaneId` · `portable_pty::CommandBuilder`).** It is a shape that really exists. Rejected = the client (shell) uses `protocol`, so if `protocol` embeds feature crates the client pulls those crates in — a violation of 「클라는 agent 를 모른다」 (decision candidate 5) (user principle + code).
- **Make the command field a `serde_json::Value`.** The simplest. Rejected = a tree allocation appears (research conclusion 2 — serde_json source · `RawValue` docs, confidence: likely).
- **Generic packet `AgentCommand<E>`.** The closest to 「헤더만 공유」 (the shape is in `protocol`, the content type is plugged in by the edge). Rejected = the type parameter spreads to every place that uses the packet, the pairing function has to be handed a way to extract the number, and the TS bindings have to be emitted separately for the concrete types (research alternatives table). **Self-assessment of the rejection basis: weak** (descriptive).
- **Pull the pairing number out into a field outside the command field.** Rejected = the wire bytes change and the protocol version has to be raised (main session's judgment — decision 4).
- **Partially parse only the number out of the raw text.** Rejected = it costs extra parsing (main session's judgment — decision 4). **Self-assessment of the rejection basis: weak** (descriptive — the cost was not measured).

## 근거

- **User decision 2026-10-02** — the three quotations in 「맥락」 above.
- **Research (`docs/research/wire-shared-types-placement-2026-10-02.md`)** — rust-analyzer (copy + `to_proto` / `from_proto`) · Cargo's `cargo-util-schemas` (a shared schema with no behavior) · the JSON-RPC envelope (opaque field) have this shape, and there is no primary source that recommends a packet crate embedding other crates' types. Zed `crates/proto` is a central packet crate + per-area files (the basis for decision 2 — not the basis for the opaque field, adversarial review 4).
- **Control messages are already JSON** — commands · events are JSON text frames and only the terminal output is binary (head comment of `protocol/src/codec.rs`). Changing it does not increase JSON. ★It is not a performance basis★ — there is no trustworthy source that gave a numeric threshold at which JSON becomes a bottleneck (research conclusion 1).
- **「두 번 파싱 안 한다」 was wrong** — `RawValue` too costs 1 outer validation scan + 1 at the edge. What it saves is only the intermediate tree (adversarial review 1 → correction of conclusion 2 · decision 5 is the corrected version).

## 영향 / 불변식

- **Invariant: the manifest of `protocol` has no workspace crate** (the 「의존 0 게이트」 of work order 3-1). The fixed packet types are `protocol`'s own, and the command fields are `RawValue`. So the `protocol → command` line in the dependency graph under ADR-0175's Impact closes (`protocol` becomes a leaf).
- ★**Open — the transport TRD (work order 3) decides: the owner marker `OwnerToken`**★ — it is not among the four of decision 3. `RegisterCommands` · `UpdateCommands` carry `owner: OwnerToken` as a command type. If this stays, the `protocol` → command dependency does not disappear (decision 1 · the invariant above), so that TRD chooses between a `protocol` copy and putting it in the raw field.
- **`From<command::RequestId> for RequestId` in `protocol/src/ids.rs` loses its home** — the reason in its comment is 「화살표가 `protocol → command` 한 방향」. It moves to the adapter. It carries the same UUID as it is (ADR-0081 decision 4 — unchanged).
- **ADR-0177 decision 4's 「짝짓기 번호를 꺼내는 함수 — 신규가 아니다, `protocol` 에 `command_request_id` · `event_reply_request_id` 로 이미 정본이 있다」 goes stale** — the canonical copy moves to the edge adapter (decision 4). 「밖(소비자)에 남는다」 stays as it is.
- **No stamp is put on ADR-0155** — memo §10 wrote 「ADR-0155 결정 3(봉투를 `protocol` 에 싣기)」 but that citation is wrong: that ADR's decision 3 is 「배달은 홉마다 같은 3단계다」, and 「`protocol` 이 봉투 · 등록 단위 타입을 실어 도구 crate 하나를 의존한다(0 → 1)」 is in TRD S20 §3-1 · §5 protocol row (the outcome of edition 2) — what this decision reverses is that TRD's outcome, and it is listed only as related. ADR-0155 decision 6 (「`name` 은 겉봉에 노출한다 … `args` 는 여전히 데몬 불투명」) keeps its effect because the daemon unpacks the envelope at the edge and reads `name`.
- **The four command fields become 「JSON」 in the TS bindings** (research alternatives table). The frontend source does not currently import those envelope TS types (`src/` non-test imports 0 — measured 2026-10-02).
- **`Box<RawValue>` needs the serde_json `raw_value` feature flag.** There are cases of failure on untagged enums · buffering paths (serde-rs/json#497). Our packet enum uses external tagging, so the risk is low, but we confirm it with a test — the wire golden carries the byte-invariance.
- **To look at together in 3-1 (memo §11)** — the packet `CommandListEntry` is almost the same as the registry entry `RosterEntry` (`available` is always true).
- **Code anchor = `// ADR-0272`** — the `protocol` command fields · the pairing in the edge adapter.
