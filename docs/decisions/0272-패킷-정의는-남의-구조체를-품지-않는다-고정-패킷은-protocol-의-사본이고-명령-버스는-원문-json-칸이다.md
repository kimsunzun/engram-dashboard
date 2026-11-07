# ADR-0272: Packet definitions do not embed other crates' structs; fixed packets are copies in protocol and the command bus is a raw JSON field

- 상태: 확정 (2026-10-02, basis: user decision 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` decision candidate 8 — principle · confirmed shape) + the reply pairing is the main session's judgment + research `docs/research/wire-shared-types-placement-2026-10-02.md` (medium · cross-family adversarial review FIX → applied)) · 부분 폐기 by ADR-0310 (of the impact: disposition of the open OwnerToken and making the TS bindings JSON)
- 관련: Amends ADR-0177 (decision 4: canonical location of the pairing-number function) · ADR-0155 (unified command bus — no stamp, see the impact below) · TRD `docs/process/S20-command-bus/trd.md` §3-1 · §5 protocol row (the actual source of 「protocol → command 의존 0 → 1」 — this decision reverses it) · ADR-0081 decision 4 (single `request_id` round trip — as is) · ADR-0129 (premise of preserving the wire JSON shape) · ADR-0270 (the client does not know agent) · decision candidate 7 (to be fixed at the transport TRD) (auth message → `protocol`) · `crates/engram-dashboard-protocol/src/messages.rs` · `crates/engram-dashboard-protocol/src/ids.rs` · step-log S21 · Amends ADR-0175 (of the dependency graph in the impact: the line where protocol holds command) · Amended by ADR-0310 (of the impact: disposition of the open OwnerToken and making the TS bindings JSON)

## 맥락

`protocol` = the **packet list** that the shell and daemon exchange (+ encoding · protocol version · TS bindings for the frontend). Boundary = it depends on command, and the daemon, shell and frontend use it.

`protocol` takes in four command-bus forms from command — the envelope (`CommandEnvelope`) · the reply (`CommandReply`) · the registration entry (`CommandDecl`) · the owner marker (`OwnerToken`) (the head `use` of `crates/engram-dashboard-protocol/src/messages.rs`). There are already two ways to carry feature data — ① the command bus (the module decides the shape, and it goes inside the JSON 「속」) ② `protocol` copy types (the agent list and so on — moved over at the daemon's edge).

The user (2026-10-02): 「패킷이 똥통이 되잖아」 · 「바닥 구조체가 아니라 장기적으론 모든 구조체를 품어야 되는 거잖아」 · 「다른 건 다 패킷 구조체 방식으로 하고 command 만 json 으로」.

## 결정

1. **Principle: `protocol` does not embed structs of other crates — the bottom-level tool (command) is no exception either** (user 2026-10-02). Allow it once and every struct flows in each time a feature grows.
2. **Fixed packets** (agent list · state · subscription · profile and so on) **= defined as types in `protocol`** (user 2026-10-02 — Zed style · when it grows, split into per-area modules). For feature data, `protocol` holds a copy for communication and it is moved over at the shell and daemon edges (the transport adapter). agent does not know `protocol`.
3. **Command bus = the four command fields of the packet (command · reply · registration · registration change) are held as `Box<RawValue>`** (user 2026-10-02). Only raw bytes are carried, and unpacking them into envelope structs is done at the shell and daemon edges. It has the same shape as a JSON-RPC envelope · a Tauri command (commands are an open set that keeps growing). → the `protocol` → command dependency disappears and the bytes on the wire stay the same.
4. **Reply pairing (the main session's judgment):** the edge adapter unpacks the envelope and reply into structs as soon as it receives a packet, and reads the request number used for pairing **from the unpacked struct** (natural, since transport's `reply_tag` is inside the adapter). Because the number is not taken outside the field, the bytes and protocol version stay the same and there is no extra parsing. The pairing functions now inside `protocol` (`command_request_id` · `event_reply_request_id`) are moved to the adapter.
5. **We accept the cost** — when unpacking the outer packet we scan the envelope once to validate it (serde_json also scans a `RawValue` field to check that it is valid JSON) and unpack it once more at the edge. No intermediate tree is created. Command packets are rare, so the cost is negligible.
6. **To be newly written** — handling of the case of 「패킷은 풀렸는데 봉투가 깨진」 · unpacking / repacking on the daemon relay path.
7. **Start = at the time of attaching transport** (work order 3-1, tidying `protocol`).

## 거부한 대안

- **Embed the envelope struct as now.** Rejected = user decision (「패킷이 똥통이 되잖아」 — the principle of decision 1).
- **Split command into a definition-only crate (envelope · reply · registration entry · error shape) and a tool crate, and have `protocol` embed only the definition-only crate (「헤더만 공유」).** It has the advantage that type checking stays in `protocol`. Rejected = user principle (decision 1 — it goes against 「패킷은 남의 구조체를 품지 않는다」. Research 「우리에게」 raised it as a user-decision item).
- **The packet embeds domain types directly (wezterm `codec` style — `mux::pane::PaneId` · `portable_pty::CommandBuilder`).** It is a shape that really exists. Rejected = the client (shell) uses `protocol`, so if `protocol` embeds a feature crate the client pulls in that crate — a violation of 「클라는 agent 를 모른다」 (decision candidate 5) (user principle + code).
- **Hold the command field as `serde_json::Value`.** It is the simplest. Rejected = a tree allocation arises (research conclusion 2 — serde_json source · `RawValue` docs, confidence likely).
- **A generic packet `AgentCommand<E>`.** It is the closest to 「헤더만 공유」 (the shape is `protocol`, the content type is plugged in by the edge). Rejected = the type parameter spreads to every place that uses a packet, the pairing function has to be handed a way to extract the number, and the TS bindings have to be emitted separately for concrete types (research alternatives table). **Self-assessment of the rejection basis: weak** (descriptive).
- **Move the pairing number out into a field outside the packet field.** Rejected = the bytes on the wire change and the protocol version has to be bumped (main session's judgment — decision 4).
- **Partially parse only the number from the raw text.** Rejected = extra parsing is needed (main session's judgment — decision 4). **Self-assessment of the rejection basis: weak** (descriptive — the cost was not measured).

## 근거

- **User decision 2026-10-02** — the three 「맥락」 quotes above.
- **Research (`docs/research/wire-shared-types-placement-2026-10-02.md`)** — rust-analyzer (copy + `to_proto` / `from_proto`) · Cargo `cargo-util-schemas` (a shared schema with no behavior) · the JSON-RPC envelope (an opaque field) have this shape, and there is no primary source that recommends a shape where a packet crate embeds other people's types. Zed `crates/proto` is a central packet crate + per-area files (the basis of decision 2 — not the basis of the opaque field, adversarial review 4).
- **Control messages are already JSON** — commands and events are JSON text frames and only terminal output is binary (the head comment of `protocol/src/codec.rs`). Changing this does not increase JSON. ★Not a performance argument★ — there is no trustworthy source that gave a numeric criterion at which JSON becomes a bottleneck (research conclusion 1).
- **「두 번 파싱 안 한다」 was wrong** — `RawValue` also costs one outer validation scan + one at the edge. All it saves is the intermediate tree (adversarial review 1 → correction of conclusion 2 · decision 5 is the corrected version).

## 영향 / 불변식

- **Invariant: there is no workspace crate in the manifest of `protocol`** (the 「의존 0 게이트」 of work order 3-1). The fixed packet types are `protocol`'s own, and the command field is `RawValue`. So the `protocol → command` line in the dependency graph of the ADR-0175 impact closes (`protocol` becomes a leaf).
- ★**Open — decided by the transport TRD (work order 3): the owner marker `OwnerToken`**★ — it is not among the four of decision 3. `RegisterCommands` · `UpdateCommands` carry `owner: OwnerToken` as a command type. If this remains, the `protocol` → command dependency does not disappear (decision 1 · the invariant above), so that TRD chooses whether to hold it as a `protocol` copy or put it in the raw field.
- **The `From<command::RequestId> for RequestId` of `protocol/src/ids.rs` loses its home** — the reason in that comment is 「화살표가 `protocol → command` 한 방향」. Move it to the adapter. The value carries the same UUID as is (ADR-0081 decision 4 — as is).
- **The 「짝짓기 번호를 꺼내는 함수 — 신규가 아니다, `protocol` 에 `command_request_id` · `event_reply_request_id` 로 이미 정본이 있다」 of ADR-0177 decision 4 becomes stale** — the canonical source moves to the edge adapter (decision 4). 「밖(소비자)에 남는다」 stays as is.
- **No stamp is put on ADR-0155** — memo §10 wrote 「ADR-0155 결정 3(봉투를 `protocol` 에 싣기)」 but that citation is wrong: decision 3 of that ADR is 「배달은 홉마다 같은 3단계다」, and 「`protocol` 이 봉투 · 등록 단위 타입을 실어 도구 crate 하나를 의존한다(0 → 1)」 is in the TRD S20 §3-1 · §5 protocol row (the outcome of its 2nd edition) — what this decision reverses is that TRD's outcome, and it is listed only under related. ADR-0155 decision 6 (「`name` 은 겉봉에 노출한다 … `args` 는 여전히 데몬 불투명」) keeps its effect as is, because the daemon unpacks the envelope at the edge and reads `name`.
- **In the TS bindings the four command fields become 「JSON」** (research alternatives table). The frontend source does not currently use that envelope TS type (`src/` non-test imports 0 — measured 2026-10-02).
- **`Box<RawValue>` needs the serde_json `raw_value` feature flag.** There are cases of failure in untagged enum · buffering paths (serde-rs/json#497). Our packet enum uses the externally tagged form so the risk is low, but we confirm it by test — the wire golden carries the byte invariance.
- **To be looked at together in 3-1 (memo §11)** — the packet `CommandListEntry` is almost the same as the registry entry `RosterEntry` (`available` is always true).
- **Code anchor = `// ADR-0272`** — the `protocol` command field · pairing in the edge adapter.
