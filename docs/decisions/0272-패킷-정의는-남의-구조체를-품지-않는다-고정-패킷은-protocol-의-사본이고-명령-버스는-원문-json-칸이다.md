# ADR-0272: 패킷 정의는 남의 구조체를 품지 않는다 고정 패킷은 protocol 의 사본이고 명령 버스는 원문 JSON 칸이다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 8 — 원칙 · 확정 모양) + 답장 짝 맞추기는 메인 판단 + 조사 `docs/research/wire-shared-types-placement-2026-10-02.md`(medium · cross-family 적대 리뷰 FIX → 반영))
- 관련: Amends ADR-0177 (결정 4의 짝짓기 번호 함수 정본 위치) · ADR-0155(통합 command 버스 — 도장 없음, 아래 영향) · TRD `docs/process/S20-command-bus/trd.md` §3-1 · §5 protocol 행(「protocol → command 의존 0 → 1」의 실제 출처 — 이 결정이 그것을 되돌린다) · ADR-0081 결정 4(단일 `request_id` 왕복 — 그대로) · ADR-0129(wire JSON 모양 보존 전제) · ADR-0270(클라는 agent 를 모른다) · 결정 후보 7(transport TRD 때 박는다)(인증 메시지 → `protocol`) · `crates/engram-dashboard-protocol/src/messages.rs` · `crates/engram-dashboard-protocol/src/ids.rs` · step-log S21

## 맥락

`protocol` = 셸 · 데몬이 주고받는 **패킷 목록**(+ 인코딩 · 프로토콜 버전 · 프론트용 TS 바인딩)이다. 경계 = command 에 기대고 데몬 · 셸 · 프론트가 쓴다.

`protocol` 은 command 에서 명령 버스 양식 넷을 가져다 품는다 — 봉투(`CommandEnvelope`) · 답장(`CommandReply`) · 등록 항목(`CommandDecl`) · 주인 표지(`OwnerToken`)(`crates/engram-dashboard-protocol/src/messages.rs` 머리 `use`). 기능 데이터를 싣는 길은 이미 둘이다 — ① 명령 버스(모듈이 모양을 정하고 JSON 「속」으로) ② `protocol` 사본 타입(에이전트 목록 등 — 데몬 끝자락에서 옮겨 담음).

사용자(2026-10-02): 「패킷이 똥통이 되잖아」 · 「바닥 구조체가 아니라 장기적으론 모든 구조체를 품어야 되는 거잖아」 · 「다른 건 다 패킷 구조체 방식으로 하고 command 만 json 으로」.

## 결정

1. **원칙: `protocol` 은 다른 crate 의 구조체를 품지 않는다 — 바닥 도구(command)도 예외가 아니다**(사용자 2026-10-02). 한 번 허용하면 기능이 늘 때마다 모든 구조체가 흘러든다.
2. **고정 패킷**(에이전트 목록 · 상태 · 구독 · 프로필 등) **= `protocol` 에 타입으로 정의한다**(사용자 2026-10-02 — Zed 식 · 커지면 영역별 모듈로 나눈다). 기능 데이터는 `protocol` 이 통신용 사본을 갖고 셸 · 데몬 끝자락(transport 어댑터)에서 옮겨 담는다. agent 는 `protocol` 을 모른다.
3. **명령 버스 = 패킷의 명령 칸 넷(명령 · 답장 · 등록 · 등록 변경)을 `Box<RawValue>` 로 둔다**(사용자 2026-10-02). 원문 바이트만 나르고 봉투 구조체로 푸는 것은 셸 · 데몬 끝자락이다. JSON-RPC 봉투 · Tauri 명령과 같은 모양이다(명령은 계속 늘어나는 열린 집합). → `protocol` → command 의존이 사라지고 통신선 바이트는 그대로다.
4. **답장 짝 맞추기(메인 판단):** 끝자락 어댑터가 패킷을 받는 즉시 봉투 · 답장을 구조체로 풀고, 짝 맞추기용 요청 번호는 **푼 구조체에서** 읽는다(transport 의 `reply_tag` 가 어댑터 안이므로 자연스럽다). 번호를 칸 밖으로 빼지 않으므로 바이트 · 프로토콜 버전이 그대로이고 추가 파싱도 없다. 지금 `protocol` 안에 있는 짝 맞추기 함수(`command_request_id` · `event_reply_request_id`)는 어댑터로 옮긴다.
5. **비용을 받아들인다** — 바깥 패킷을 풀 때 봉투를 한 번 훑어 검증하고(serde_json 은 `RawValue` 칸도 유효한 JSON 인지 훑는다) 끝자락에서 한 번 더 푼다. 중간 트리는 안 생긴다. 명령 패킷은 드물어 무시할 수준이다.
6. **새로 짤 것** — 「패킷은 풀렸는데 봉투가 깨진」 경우의 처리 · 데몬 중계 경로의 풀기 / 다시 싸기.
7. **착수 = transport 부착 때**(작업 순서 3-1 `protocol` 정리).

## 거부한 대안

- **지금처럼 봉투 구조체를 품는다.** 기각 = 사용자 결정(「패킷이 똥통이 되잖아」 — 결정 1 원칙).
- **command 를 정의 전용 crate(봉투 · 답장 · 등록 항목 · 오류 모양)와 도구 crate 로 쪼개고 `protocol` 은 정의 전용 crate 만 품는다(「헤더만 공유」).** 타입 검사가 `protocol` 에 남는다는 이점은 있다. 기각 = 사용자 원칙(결정 1 — 「패킷은 남의 구조체를 품지 않는다」에 어긋난다. 조사 「우리에게」가 사용자 결정 사항으로 올렸다).
- **패킷이 도메인 타입을 직접 품는다(wezterm `codec` 식 — `mux::pane::PaneId` · `portable_pty::CommandBuilder`).** 실재하는 모양이다. 기각 = 클라(셸)가 `protocol` 을 쓰므로 `protocol` 이 기능 crate 를 품으면 클라가 그 crate 를 끌어온다 — 「클라는 agent 를 모른다」(결정 후보 5) 위반(사용자 원칙 + 코드).
- **명령 칸을 `serde_json::Value` 로 둔다.** 가장 단순하다. 기각 = 트리 할당이 생긴다(조사 결론 2 — serde_json 소스 · `RawValue` 문서, 확신도 가능성 높음).
- **제네릭 패킷 `AgentCommand<E>`.** 「헤더만 공유」에 가장 가깝다(모양은 `protocol`, 내용 타입은 끝자락이 꽂음). 기각 = 타입 매개변수가 패킷을 쓰는 모든 곳으로 번지고, 짝 맞추기 함수에 번호를 꺼내는 방법을 넘겨야 하며, TS 바인딩은 구체 타입으로 따로 내야 한다(조사 대안 표). **기각 근거 자평: 약함**(서술).
- **짝 맞추기 번호를 칸 밖 필드로 뺀다.** 기각 = 통신 바이트가 바뀌어 프로토콜 버전을 올려야 한다(메인 판단 — 결정 4).
- **원문에서 번호만 부분 파싱한다.** 기각 = 추가 파싱이 든다(메인 판단 — 결정 4). **기각 근거 자평: 약함**(서술 — 비용을 재지 않았다).

## 근거

- **사용자 결정 2026-10-02** — 위 「맥락」 인용 셋.
- **조사(`docs/research/wire-shared-types-placement-2026-10-02.md`)** — rust-analyzer(사본 + `to_proto` / `from_proto`) · Cargo `cargo-util-schemas`(행동 없는 공용 스키마) · JSON-RPC 봉투(불투명 칸)가 이 모양이고, 패킷 crate 가 남의 타입을 품는 모양을 권하는 1차 자료는 없다. Zed `crates/proto` 는 중앙 패킷 crate + 영역별 파일(결정 2 의 근거 — 불투명 칸의 근거는 아니다, 적대 리뷰 4).
- **제어 메시지는 이미 JSON 이다** — 명령 · 이벤트는 JSON 텍스트 프레임이고 터미널 출력만 바이너리다(`protocol/src/codec.rs` 머리 주석). 바꿔도 JSON 이 늘지 않는다. ★성능 근거는 아니다★ — JSON 이 병목이 되는 수치 기준을 낸 신뢰할 출처는 없다(조사 결론 1).
- **「두 번 파싱 안 한다」는 틀렸다** — `RawValue` 도 바깥 검증 훑기 1회 + 끝자락 1회다. 아끼는 것은 중간 트리뿐이다(적대 리뷰 1 → 결론 2 정정 · 결정 5 가 그 정정판이다).

## 영향 / 불변식

- **불변식: `protocol` 의 매니페스트에 워크스페이스 crate 가 없다**(작업 순서 3-1 의 「의존 0 게이트」). 고정 패킷 타입은 `protocol` 자기 것이고, 명령 칸은 `RawValue` 다.
- ★**열린 것 — transport TRD(작업 순서 3) 가 정한다: 주인 표지 `OwnerToken`**★ — 결정 3 의 넷에 없다. `RegisterCommands` · `UpdateCommands` 가 `owner: OwnerToken` 을 command 타입으로 싣는다. 이것이 남으면 `protocol` → command 의존이 안 사라지므로(결정 1 · 위 불변식), `protocol` 사본으로 둘지 원문 칸에 넣을지를 그 TRD 가 고른다.
- **`protocol/src/ids.rs` 의 `From<command::RequestId> for RequestId` 가 거처를 잃는다** — 그 주석의 사유가 「화살표가 `protocol → command` 한 방향」이다. 어댑터로 옮긴다. 값은 같은 UUID 를 그대로 나른다(ADR-0081 결정 4 — 그대로).
- **ADR-0177 결정 4 의 「짝짓기 번호를 꺼내는 함수 — 신규가 아니다, `protocol` 에 `command_request_id` · `event_reply_request_id` 로 이미 정본이 있다」가 낡는다** — 정본이 끝자락 어댑터로 옮긴다(결정 4). 「밖(소비자)에 남는다」는 그대로다.
- **ADR-0155 에는 도장을 박지 않는다** — 메모 §10 은 「ADR-0155 결정 3(봉투를 `protocol` 에 싣기)」라 적었으나 그 인용이 틀렸다: 그 ADR 결정 3 은 「배달은 홉마다 같은 3단계다」이고, 「`protocol` 이 봉투 · 등록 단위 타입을 실어 도구 crate 하나를 의존한다(0 → 1)」는 TRD S20 §3-1 · §5 protocol 행(2판 귀결)에 있다 — 이 결정이 되돌리는 것은 그 TRD 의 귀결이고 관련으로만 든다. ADR-0155 결정 6(「`name` 은 겉봉에 노출한다 … `args` 는 여전히 데몬 불투명」)은 데몬이 끝자락에서 봉투를 풀어 `name` 을 읽으므로 효과가 그대로다.
- **TS 바인딩에서 명령 칸 넷이 「JSON」이 된다**(조사 대안 표). 지금 프론트 소스는 그 봉투 TS 타입을 가져다 쓰지 않는다(`src/` 비테스트 import 0 — 2026-10-02 실측).
- **`Box<RawValue>` 는 serde_json `raw_value` 기능 플래그가 필요하다.** untagged enum · 버퍼링 경로에서 실패한 사례가 있다(serde-rs/json#497). 우리 패킷 enum 은 외부 태그 방식이라 위험은 낮지만 시험으로 확인한다 — wire golden 이 바이트 불변을 진다.
- **3-1 에서 함께 볼 것(메모 §11)** — 패킷 `CommandListEntry` 가 명부 항목 `RosterEntry` 와 거의 같다(`available` 은 늘 참).
- **코드 앵커 = `// ADR-0272`** — `protocol` 명령 칸 · 끝자락 어댑터의 짝 맞추기.
