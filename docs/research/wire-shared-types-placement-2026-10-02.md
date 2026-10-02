# 공용 구조체를 어디에 두나 — wire 패킷 crate · 명령 봉투 · JSON 성능

- **상태:** medium(수집 갈래 4 · 메인 grounding · cross-family 적대 리뷰 FIX → 반영) · 2026-10-02
- **결정 연결:** `../refactoring/architecture-discussion-2026-09-26.md` 결정 후보 8
- **질문:** ① 패킷 정의 crate(`protocol`)가 다른 crate 의 구조체(지금은 command 의 명령 봉투)를 직접 품어도 되나 ② 봉투를 JSON 칸으로 싣고 끝자락에서 풀면 두 번 파싱·성능 손해가 생기나 ③ C++ 「헤더만 공유」 같은 모양 공유 방식이 있나 ④ 실제 프로젝트는 패킷 crate 를 어떻게 꾸리나
- **확신도 범례:** 확실 = 1차 자료를 메인이 직접 확인 · 가능성 높음 = 1차 자료 인용(수집자) · 불확실 = 2차 요약·관행 관찰뿐

## 결론

1. **제어 메시지는 이미 JSON 이다.** 우리 명령·이벤트는 JSON 텍스트 프레임이고 많이 흐르는 터미널 출력만 별도 바이너리 프레임이다(`crates/engram-dashboard-protocol/src/codec.rs` 머리 주석 · `crates/engram-dashboard-net/src/ws.rs:569` — 확실). LSP · DAP · JSON-RPC 도 제어 메시지를 JSON 으로 둔다(DAP 개요 — 가능성 높음). 단 이건 **형식 선택의 선례일 뿐 성능 근거가 아니다** — 어느 문서도 우리 명령 트래픽에 대한 수치 기준을 주지 않는다(적대 리뷰 지적). 바꿔도 JSON 이 늘지 않는다. JSON 이 병목이 되는 수치 기준을 낸 신뢰할 출처는 없다(모름).
2. **봉투를 `RawValue` 칸으로 실으면 「중간 트리」는 안 생기지만 텍스트는 두 번 훑는다** — ★적대 리뷰가 고친 항목★. 바깥 패킷을 풀 때 serde_json 은 안쪽을 해석하지 않되 **유효한 JSON 인지 한 번 훑어 검증**하고(`ignore_value`), 끝자락에서 구조체로 풀 때 **한 번 더** 훑는다. 아끼는 것은 `serde_json::Value` 같은 중간 트리 생성뿐이다(serde_json `src/de.rs` · `RawValue` API 문서 — 가능성 높음 · 로컬 `serde_json-1.0.151/src/raw.rs` 문서 주석으로 「파싱을 미루는 용도」까지는 메인 확인). 소유형으로 들고 다니려면 `Box<RawValue>` + `raw_value` 기능 플래그가 필요하고, untagged enum·버퍼링 경로에서 실패한 사례가 있다(serde-rs/json#497) — 우리 패킷 enum 은 외부 태그 방식이라(메인 확인: `#[serde(tag)]` 없음) 위험은 낮지만 시험으로 확인해야 한다. protobuf 의 `Any`/`bytes` 칸이 같은 「봉투 감싸기」 패턴이다(가능성 높음).
3. **「헤더만 공유」 = 행동 없는 정의 전용 crate 이고, 같은 워크스페이스·같은 언어에선 그것이 관례다.** `lsp-types` · `cargo-util-schemas`("Low-level Cargo format schemas" — 확실). IDL + 코드 생성(protobuf · FlatBuffers · Cap'n Proto · OpenAPI)은 **다른 언어나 따로 배포되는 서비스**가 있을 때 값을 한다 — 같은 Rust 워크스페이스에선 빌드 단계만 는다(가능성 높음). **트레이트로 모양을 공유하는 관례는 못 찾았다** — serde 는 구체 타입을 직렬화하므로 결국 구체 공용 타입이 다시 필요하다(가능성 높음 · 부재 증거).
4. **실제 프로젝트는 패킷 crate 를 「중앙에 두되 영역별로 파일을 나눈다」.**
   - **Zed** `crates/proto`: 패킷 crate 하나 + 영역별 `.proto` 파일 ~16개(`buffer` · `git` · `lsp` · `task` …), 전부 하나의 `Envelope { oneof payload }` 로 모인다(400+ 종). 기능 crate 는 자기 패킷 타입을 정의하지 않고 생성된 타입을 가져다 쓴다(가능성 높음 — 수집자가 원본 파일 확인).
   - **wezterm** `codec`: 중앙 `Pdu` enum + 대부분의 패킷 구조체. 단 필드 타입은 `mux::pane::PaneId` · `portable_pty::CommandBuilder` 처럼 **도메인 crate 타입을 직접 품는다** — 「패킷이 남의 구조체를 품는」 모양이 실재한다(가능성 높음).
   - **nushell** `nu-plugin-protocol`: 중앙 패킷 crate, I/O 없는 serde 전용 · 인코딩(JSON/MessagePack)은 바깥에서 갈아끼움(불확실 — 문서 요약).
   - **Tauri IPC**: 공용 패킷 crate 가 없다. 기능 쪽(`#[tauri::command]`)이 자기 인자·결과 타입을 정의하고 코어는 범용 봉투(JSON)만 준다 — **우리 명령 버스와 같은 모양**(가능성 높음 · 문제 모양이 달라 약한 비교).
   - rust-analyzer: 내부 타입은 직렬화하지 않고 LSP 쪽 짝을 따로 만들어 `to_proto`/`from_proto` 로 손변환(가능성 높음 — 원문 미대조).

## 우리에게 (메인 합성)

- **길이 둘이다.** (가) `protocol` 고정 패킷 = Zed 모양 — 중앙에 두되 영역별 모듈로 나눠 커져도 찾기 쉽게. 기능 데이터는 `protocol` 사본 + 끝자락 변환(rust-analyzer). (나) 명령 버스 = Tauri·JSON-RPC 모양 — 기능 crate 가 모양을 정하고 `protocol` 은 `RawValue` 칸으로 원문만 나른다. 새 기능은 (나)로 가면 `protocol` 이 안 자란다.
- **wezterm 식(패킷이 도메인 타입을 품기)은 실재하지만 우리에겐 맞지 않다** — 클라(셸)가 `protocol` 을 쓰므로 `protocol` 이 기능 crate 를 품으면 클라가 그 crate 를 끌어온다(「클라는 agent 를 모른다」 원칙 위반).
- **명령 봉투의 대안(헤더만 공유):** command 를 「정의 전용 crate(봉투·답장·등록 항목·오류 모양)」와 「도구 crate(표·명부·배달)」로 쪼개고 `protocol` 이 정의 전용 crate 만 품는 길도 있다. 타입 검사는 남지만 「패킷은 남의 구조체를 품지 않는다」(사용자 원칙)에는 어긋난다 — 사용자 결정 사항.

- **명령 봉투를 싣는 방법 — 대안 비교(적대 리뷰 6 반영):**

  | 방법 | `protocol` 이 command 를 아나 | 파싱 | 타입 검사 | 비용·위험 |
  |---|---|---|---|---|
  | 지금(봉투 구조체를 품기) | 안다 | 1회 | `protocol` 에서 | 사용자 원칙 위반(「패킷이 똥통」) |
  | `Box<RawValue>` 칸 | 모른다 | 검증 훑기 + 끝자락 1회 | 끝자락에서 | 짝 맞추기용 `request_id` 를 따로 꺼내야 함 · TS 에선 「JSON」 |
  | `serde_json::Value` 칸 | 모른다 | 트리 생성 + 끝자락 변환 | 끝자락에서 | 가장 단순하나 트리 할당이 생김 |
  | 제네릭 패킷 `AgentCommand<E>` | 모른다(타입 매개변수) | 1회 | 끝자락이 `E` 를 정함 | 「헤더만 공유」에 가장 가깝다 — 모양은 `protocol`, 내용 타입은 끝자락이 꽂음. 대신 타입 매개변수가 패킷을 쓰는 모든 곳으로 번지고, 짝 맞추기 함수에 번호를 꺼내는 방법을 넘겨야 함 · TS 바인딩은 구체 타입으로 따로 내야 함 |
  | 정의 전용 crate(command 를 모양/도구로 쪼갬) | 모양 crate 만 안다 | 1회 | `protocol` 에서 | 사용자 원칙에 어긋남 |

- **열린 질문:** 불투명 칸이면 `protocol` 의 답장 짝 맞추기(`messages.rs:1048`)가 번호를 어디서 얻나 — ① 번호를 칸 밖 필드로 뺀다(통신 바이트가 바뀜 → 프로토콜 버전 올림) ② 원문에서 번호만 부분 파싱 ③ 짝 맞추기를 끝자락(transport 어댑터)으로 옮긴다.

## 적대 리뷰 (cross-family · GPT · 레벨 2 · 판정 FIX — 반영 완료)

| # | 지적 | 판정 | 반영 |
|---|---|---|---|
| 1 | 「두 번 파싱 안 한다」는 틀렸다 — `RawValue` 도 바깥에서 검증 훑기 1회 + 끝자락 파싱 1회 | 반증 동반 · 수용 | 결론 2 정정 |
| 2 | `protocol` 안의 답장 짝 맞추기가 봉투 안 `request_id` 를 읽는다(`messages.rs:1048`) — 불투명 칸이면 번호를 어떻게 꺼낼지 빠졌다 | 수용 | 「우리에게」에 열린 질문으로 |
| 3 | `RawValue` 소유형·버퍼링·untagged enum 제약 누락 | 수용 | 결론 2 |
| 4 | Zed 는 불투명 칸의 근거가 아니라 중앙 타입 패킷의 근거다 | 수용(보고서는 이미 그렇게 썼다) | — |
| 5 | JSON 선례를 성능 근거로 쓴 것은 과장 | 수용 | 결론 1 |
| 6 | 대안 비교 누락 — `Value` · `Box<RawValue>` · 제네릭 패킷 `Packet<T>` · 정의 전용 crate · 진화·TS 영향 | 수용 | 「우리에게」 대안 표 |
| 7 | 인용 정정 — #355 는 도입 요청 이슈 · wezterm 경로는 `mux::pane::PaneId` | 수용 | 출처·본문 정정 |

## 못 본 것

- Deno · Neovim msgpack-rpc · Lapce/Helix 의 패킷 구성 — 미조사.
- rust-analyzer 아키텍처 문서 원문 대조 — 메인 확인 시도가 세션 중 중단됐다.
- Zed 의 「400+ 종」 수치 · wezterm 의 설계 사유 — 수집자 확인만.
- FlatBuffers/Cap'n Proto 의 실측 비교 — 문서 근거뿐.

## 출처

- rust-analyzer 아키텍처: https://rust-analyzer.github.io/book/contributing/architecture.html
- cargo-util-schemas: https://docs.rs/cargo-util-schemas · https://github.com/rust-lang/cargo/pull/13178
- serde_json RawValue: https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html · https://github.com/serde-rs/json/blob/master/src/de.rs · https://github.com/serde-rs/json/issues/497 · 로컬 `serde_json-1.0.151/src/raw.rs`
- protobuf 인코딩/Any: https://protobuf.dev/programming-guides/encoding/
- DAP 개요: https://github.com/microsoft/debug-adapter-protocol/blob/main/overview.md
- Zed proto: https://github.com/zed-industries/zed/tree/main/crates/proto
- wezterm codec: https://github.com/wezterm/wezterm/blob/main/codec/src/lib.rs
- prost: https://github.com/tokio-rs/prost
