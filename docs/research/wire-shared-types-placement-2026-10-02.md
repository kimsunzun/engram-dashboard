# Where to put shared structs — wire packet crate · command envelope · JSON performance

- **Status:** medium (4 collection tracks · main-session grounding · cross-family adversarial review FIX → applied) · 2026-10-02
- **Decision link:** `../refactoring/architecture-discussion-2026-09-26.md` decision candidate 8
- **Question:** ① May the packet-definition crate (`protocol`) directly hold another crate's struct (currently command's command envelope)? ② If the envelope is carried in a JSON field and unpacked at the edge, is there a double-parse · performance loss? ③ Is there a shape-sharing method like the C++ 「헤더만 공유」 (share only the header)? ④ How do real projects lay out a packet crate?
- **Confidence legend:** certain = the main session checked the primary source directly · likely = a primary source quoted (collector) · uncertain = only secondary summaries · practice observed

## Conclusion

1. **Control messages are already JSON.** Our commands · events are JSON text frames, and only the heavy-traffic terminal output is a separate binary frame (the head comment of `crates/engram-dashboard-protocol/src/codec.rs` · `crates/engram-dashboard-net/src/ws.rs:569` — certain). LSP · DAP · JSON-RPC also keep control messages as JSON (DAP overview — likely). But this is **only a precedent for the format choice, not evidence for performance** — no document gives a numeric criterion for our command traffic (adversarial review point). Changing it does not increase JSON. No credible source gave a numeric threshold at which JSON becomes a bottleneck (unknown).
2. **Carrying the envelope in a `RawValue` field avoids an 「중간 트리」 (intermediate tree) but scans the text twice** — ★an item the adversarial review corrected★. When unpacking the outer packet, serde_json does not interpret the inside but **scans it once to validate that it is valid JSON** (`ignore_value`), and when unpacking into the struct at the edge it scans **once more**. All that is saved is building an intermediate tree such as `serde_json::Value` (serde_json `src/de.rs` · `RawValue` API docs — likely · the main session checked as far as 「파싱을 미루는 용도」 (a purpose of deferring parsing) in the doc comment of the local `serde_json-1.0.151/src/raw.rs`). Carrying it in owned form needs `Box<RawValue>` + the `raw_value` feature flag, and there are cases that failed on untagged enum · buffering paths (serde-rs/json#497) — our packet enum is externally tagged (main-session checked: no `#[serde(tag)]`) so the risk is low, but it must be confirmed with a test. protobuf's `Any`/`bytes` field is the same 「봉투 감싸기」 (envelope wrapping) pattern (likely).
3. **「헤더만 공유」 = a definition-only crate with no behavior, and in the same workspace and the same language that is the convention.** `lsp-types` · `cargo-util-schemas` ("Low-level Cargo format schemas" — certain). IDL + code generation (protobuf · FlatBuffers · Cap'n Proto · OpenAPI) pays off **when there are other languages or separately shipped services** — in the same Rust workspace it only adds a build step (likely). **We found no convention of sharing the shape through traits** — serde serializes concrete types, so a concrete shared type is needed again in the end (likely · evidence of absence).
4. **Real projects 「중앙에 두되 영역별로 파일을 나눈다」 (place it centrally but split the files by area) for the packet crate.**
   - **Zed** `crates/proto`: one packet crate + about 16 `.proto` files by area (`buffer` · `git` · `lsp` · `task` …), all gathered into a single `Envelope { oneof payload }` (400+ kinds). Feature crates do not define their own packet types and use the generated types (likely — the collector checked the original files).
   - **wezterm** `codec`: a central `Pdu` enum + most of the packet structs. But field types such as `mux::pane::PaneId` · `portable_pty::CommandBuilder` **directly hold domain crate types** — the shape 「패킷이 남의 구조체를 품는」 (a packet holds someone else's struct) really exists (likely).
   - **nushell** `nu-plugin-protocol`: a central packet crate, serde only with no I/O · the encoding (JSON/MessagePack) is swapped from outside (uncertain — doc summary).
   - **Tauri IPC**: no shared packet crate. The feature side (`#[tauri::command]`) defines its own argument · result types and the core provides only a generic envelope (JSON) — **the same shape as our command bus** (likely · a weak comparison because the problem shapes differ).
   - rust-analyzer: does not serialize its internal types; makes a separate LSP counterpart and converts by hand with `to_proto`/`from_proto` (likely — original text not checked).

## For us (main-session synthesis)

- **There are two paths.** (a) `protocol` fixed packets = the Zed shape — place centrally but split into modules by area so it stays easy to find as it grows. Feature data is a `protocol` copy + conversion at the edge (rust-analyzer). (b) The command bus = the Tauri · JSON-RPC shape — the feature crate decides the shape and `protocol` carries only the raw text in a `RawValue` field. If new features go by (b), `protocol` does not grow.
- **The wezterm style (a packet holding domain types) really exists but does not fit us** — the client (shell) uses `protocol`, so if `protocol` held a feature crate, the client would pull in that crate (a violation of the principle 「클라는 agent 를 모른다」 (the client does not know agent)).
- **An alternative for the command envelope (share only the header):** split command into a 「정의 전용 crate(봉투·답장·등록 항목·오류 모양)」 (definition-only crate (envelope · reply · registration entry · error shape)) and a 「도구 crate(표·명부·배달)」 (tool crate (table · registry · delivery)), and have `protocol` hold only the definition-only crate. Type checking remains, but it conflicts with 「패킷은 남의 구조체를 품지 않는다」 (a packet does not hold someone else's struct) (the user's principle) — a user decision.

- **How to carry the command envelope — alternatives compared (adversarial review point 6 applied):**

  | Method | Does `protocol` know command | Parsing | Type checking | Cost · risk |
  |---|---|---|---|---|
  | Now (hold the envelope struct) | knows | once | in `protocol` | violates the user's principle (「패킷이 똥통」 (the packet as a dumping ground)) |
  | `Box<RawValue>` field | does not know | validation scan + once at the edge | at the edge | the `request_id` for reply matching must be pulled out separately · in TS it is 「JSON」 |
  | `serde_json::Value` field | does not know | tree creation + conversion at the edge | at the edge | the simplest, but a tree allocation occurs |
  | Generic packet `AgentCommand<E>` | does not know (type parameter) | once | the edge decides `E` | the closest to 「헤더만 공유」 — the shape is in `protocol`, and the edge plugs in the content type. In exchange the type parameter spreads to every place that uses the packet, and the reply-matching function must be handed a way to pull out the number · TS bindings must be emitted separately as concrete types |
  | Definition-only crate (split command into shape/tools) | only the shape crate knows | once | in `protocol` | conflicts with the user's principle |

- **Open question:** with an opaque field, where does `protocol`'s reply matching (`messages.rs:1048`) get the number — ① move the number to a field outside the opaque field (the wire bytes change → bump the protocol version) ② partially parse only the number from the raw text ③ move the matching to the edge (the transport adapter).

## Adversarial review (cross-family · GPT · level 2 · verdict FIX — applied)

| # | Point | Verdict | Applied |
|---|---|---|---|
| 1 | 「두 번 파싱 안 한다」 (it does not parse twice) is wrong — `RawValue` also has one validation scan on the outside + one parse at the edge | with a counter-proof · accepted | conclusion 2 corrected |
| 2 | Reply matching inside `protocol` reads the `request_id` inside the envelope (`messages.rs:1048`) — missing how to pull out the number with an opaque field | accepted | as an open question in "For us" |
| 3 | `RawValue` owned-form · buffering · untagged enum constraints omitted | accepted | conclusion 2 |
| 4 | Zed is evidence for a central typed packet, not for an opaque field | accepted (the report already said so) | — |
| 5 | Using the JSON precedent as performance evidence is an exaggeration | accepted | conclusion 1 |
| 6 | Alternatives comparison omitted — `Value` · `Box<RawValue>` · generic packet `Packet<T>` · definition-only crate · evolution · TS impact | accepted | the alternatives table in "For us" |
| 7 | Citation corrections — #355 is an adoption-request issue · the wezterm path is `mux::pane::PaneId` | accepted | source · body corrected |

## What was not seen

- Deno · Neovim msgpack-rpc · the packet layout of Lapce/Helix — not surveyed.
- Checking the rust-analyzer architecture doc against the original text — the main session's check attempt was interrupted during the session.
- Zed's 「400+ 종」 (400+ kinds) figure · wezterm's design rationale — collector check only.
- A measured comparison of FlatBuffers/Cap'n Proto — documentation basis only.

## Sources

- rust-analyzer architecture: https://rust-analyzer.github.io/book/contributing/architecture.html
- cargo-util-schemas: https://docs.rs/cargo-util-schemas · https://github.com/rust-lang/cargo/pull/13178
- serde_json RawValue: https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html · https://github.com/serde-rs/json/blob/master/src/de.rs · https://github.com/serde-rs/json/issues/497 · local `serde_json-1.0.151/src/raw.rs`
- protobuf encoding/Any: https://protobuf.dev/programming-guides/encoding/
- DAP overview: https://github.com/microsoft/debug-adapter-protocol/blob/main/overview.md
- Zed proto: https://github.com/zed-industries/zed/tree/main/crates/proto
- wezterm codec: https://github.com/wezterm/wezterm/blob/main/codec/src/lib.rs
- prost: https://github.com/tokio-rs/prost
