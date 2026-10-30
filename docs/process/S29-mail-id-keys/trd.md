# S29 TRD — 우편 내부 키를 이름에서 id 로 (ADR-0309) — 1판 (2026-10-29)

- **상태:** 1판 · 워커 초안 → 메인 검토(2026-10-29). `/review trd` 는 생략(진행 강도 = 중간 — 사용자 2026-10-27 · 끝에 `/review code full`). 정본 결정 = ADR-0309. 줄 번호는 `c390e009` 기준 정적 조사다(코드 실행 없음).
- **§7 판정(메인 · 사용자 자율 위임 중):** Q1 = X0~X4 를 받아들인다(ADR-0309 영향 「동명 쌍둥이」 줄에 적었다). Q2 = ADR-0142 대상 축은 이번에 두고 U6 은 하지 않는다 — tracking T-29 에 후속으로 남긴다.
- **범위 밖:** 멀티 데몬(서버 칸 · `이름@서버`) · 우편함 · 장부 영속 · 응답 wire 모양(`results[].to` · `messages` 의 `from`/`to` 는 계속 이름 — `control/ingress.rs:510` `handle_messages` 그대로) · ADR-0142 대상 축 걷기(§7 Q2).
- **CRLF:** 대상 파일이 전부 CRLF 다. 기계 치환은 줄 끝을 보존하는 도구로 한다(Edit · `perl -pi` 처럼 `\r` 을 건드리지 않는 정규식).

## 1. T-29 의 「먼저 결정할 둘」에 대한 답

- **① 아직 없는 에이전트에게 보내기** — 지금도 파킹하지 않는다. 어디에도 없는 이름은 `RECIPIENT_NOT_FOUND` 다(`service.rs:3090` `absent_disposition` · ADR-0116 결정 1). 파킹은 프로필이 있는 잠든 에이전트에게만 일어나고, 그것은 id 가 있다. 별도 장치가 필요 없다(확실).
- **② 새 id 로 재시작하는 에이전트** — 같은 프로필은 같은 id 로 다시 뜬다(`agent/manager.rs:1641` `spawn_session(profile.id, …)` · 세션 id = 프로필 id). 새 id 는 다른 프로필이고, ADR-0309 결정 5 대로 받지 않는다. 이관 장치가 필요 없다(확실). 프로필 없는 산 세션(ADR-0122 의 삭제 구멍)이 죽으면 잠듦이 아니라 없음이라, 그 앞 파킹분은 TTL 로 `expired` 다(지금은 같은 이름의 다음 에이전트가 받았다 — 결정 5 와 같은 방향).

## 2. 자료 모양

| 무엇 | 지금 | 새 모양 | 단위 |
|---|---|---|---|
| 잠든 수신자 소스 | `AddressingSources.dormant_names: Vec<String>` (`service.rs:399`) | `dormant: Vec<DormantAgent>` · `pub struct DormantAgent { pub id: PeerId, pub name: String }`(`LiveAgent` 옆). 중복을 접지 않는 규칙 · (이름, id) 정렬은 그대로 | U1 |
| 데몬 어댑터 | `messaging_host.rs:181` — `entry.canonical_name` 만 넣는다 | `RosterEntry.id` 와 이름을 함께 넣는다(`agent/manager.rs:176` 에 id 가 이미 있다) | U1 |
| 회신 계약 | `RequestEntry { sender, sender_id, recipient, recipient_id: Option<PeerId> }` (`ledger.rs:175`) · 키 `(request_id, recipient 이름)` | `recipient_id: PeerId`(Option 제거) · 키 `(request_id, recipient_id)`. `sender` · `recipient` 는 **보낼 때 이름 스냅숏**으로 남는다(조회 · 기한 통지 본문) | U1 |
| 계약 파생 타입 | `DueTimeout.recipient`(`:460`) · `OpenRequestView.recipient_id: Option`(`:482`) · `RetiredContract`(`:370`) · `marked_victim: Option<(String, String)>`(`:248`) · `reclaim_abandoned_reservations → Vec<(String, String)>`(`:1150`) · `RequesterCleanup.failed`(`:339`) | `DueTimeout` · `RetiredContract` 에 `recipient_id: PeerId` 추가 · `OpenRequestView.recipient_id: PeerId` · 짝 `(String, PeerId)` | U1 |
| 대기 우편함 | `queues: HashMap<String, VecDeque<…>>` · `in_flight: HashMap<String, …>` (`mailbox.rs:284` · `:286`) · 메서드 인자 `recipient: &str` | 키 `PeerId` · 인자 `recipient: PeerId` | U2a |
| 파킹 항목 | `ParkedMessage.hinted_id: Option<PeerId>` (`mailbox.rs:164`) · `ExpiredParked.recipient: String` (`:225`) | `hinted_id` 삭제 · `recipient: PeerId` | U2a |
| 장부 배달 기록 | `MessageRecord { from: String, to: String }` (`ledger.rs:133`) · `transition` · `fail_pending` 가 `(msg_id, to 이름)` 으로 찾는다 | `from_id: PeerId`(데몬 통지 = nil — `daemon_identity`) · `to_id: Option<PeerId>`(None = 받는 이를 못 찾은 실패 행) 추가 · `from` · `to` 는 보낼 때 이름 스냅숏(조회 표시 — 결정 4) · 찾기 = `(msg_id, to_id)` | U2a |
| 서비스 내부 | `ParkRequest.recipient: &str` + `hinted_id`(`:3321`) · `deferred_flush: HashMap<String, Vec<PeerId>>`(`:469`) · `FlightSettle.recipient: &str`(`:3477`) · `EvictedTransition.recipient: String`(`:3426`) · `Reservation.recipient: String`(`:2634`) | 전부 `PeerId`(`Reservation` 은 U1). `ParkRequest` 는 장부에 적을 `recipient_name` 을 함께 든다 | U2a |
| 입구 해석 결과 | `ResolvedRecipient { display, key: String, target, live_count, dormant_count, dup_of }` (`:2771`) | U1: `dormant_id: Option<PeerId>` 추가 + `fn id()`(산 id 또는 잠든 id). U3: `{ display, to: Resolution }` · `enum Resolution { Live(LiveAgent), Dormant { id, name }, Ambiguous { count, layer }, NotFound }`. 중복 접기 키 = 풀렸으면 id, 아니면 표기 | U1 · U3 |
| 무리 펼침 | `MemberPools { live: &[String], dormant: &[String] }` · `resolve → Vec<String>` (`groups.rs:51` · `:101`) | `GroupMember { id: PeerId, name: String }` · 풀과 반환이 `GroupMember`(결정 3 — 이름 왕복 없음) | U3 |
| 포트 | `DeliveryPort::live_id_for_name`(`:380`) · `canonical_name`(`:383`) | 둘 다 삭제(쓰는 곳이 사라진다) | U2b |
| 조회 | `open_items_for(me: &str, me_id, now)` (`:2353`) | `open_items_for(me_id, now)` — ① 보낸 대기분도 `from_id` 로 대조 | U2b |
| 등장 flush | `FlushMsg::Appear { name, id }`(`messaging_host.rs:357`) · 이름별 diff `prev: HashMap<String, (u32, AgentId)>`(`:490`) | `Appear { id }` · `prev: HashMap<AgentId, u32>`(새 id 또는 다른 화신 표식이면 등장) | U2a(호출) · U4(diff) |

## 3. 걷는 것

- **힌트 장치(결정 7):** `ParkedMessage.hinted_id` · `Mailbox::queues_with_hint`(`mailbox.rs:623`) · `drain_queue` 의 항목별 대상 해석(힌트 → 이름 유일 도달, `service.rs:1520`) · `flush_for_agent` 의 이름 큐 + 힌트 큐 훑기(`:1855`) · `deliver_notice` 의 `by_name` 추가 도어벨(`:2005`) · `unique_reachable_in`(`:3888`) · `sweep` 의 `live_agents()` 호출(`:1941` — `by_name` 만 썼다).
- **계약 이름 폴백(결정 7 · ADR-0120 목록 3):** `match_contract_for_replier` 의 이름 갈래(`ledger.rs:1010`) · `close_on_reply` · `fail_on_undeliverable_reply` 의 `replier_name` · `allow_name_fallback` 인자 · `handle_send` 의 동명 셈(`service.rs:701`) · `ReplyClosing` 의 두 칸 · `rebind_request_recipient`(`ledger.rs:897` — 받는 이 id 가 처음부터 있고 큐 = 그 id 라 착지 id 가 늘 같다) · `matches_contract_party`(`service.rs:2427`).
- **삭제 정리 이름 축 가드(결정 6 · 목록 5):** `handle_profile_deleted` 의 `live_id_for_name` 갈래(`:1204`) 와 그 포트 메서드.
- **`canonical_name` 포트:** `flush_for_agent` 가 「큐가 비었나」로 먼저 거르면 쓰는 곳이 없다.
- **loser 키 · A8/M3(ADR-0120 목록 4 · 6):** `loser_key`(`:3009`) · `dup_of` · `push_recipient` 의 해석 갈아끼우기 · 「다른 실체가 같은 이름 키」 실패 행. 키가 id 면 서로 다른 실체가 키를 다툴 수 없어 구조적으로 닿지 않는다(U3).
- **남기는 것(ADR-0309 영향 — TRD 판정):** 목록 1(잠듦 층 동명 차단) · 2(산 층이 잠듦 층을 이김)는 **남긴다**. 이름으로 부를 때의 해석 규칙이고, 동명이 운영에서 아직 생긴다 — 이름을 바꾼 산 프로필을 지우면 세션이 살아 남아 이름이 basename 으로 내려가 다른 에이전트와 겹친다(ADR-0122 결정 6 의 알려진 구멍 — 가능성 높음). 잠든 쌍둥이는 옛 `agents.json` · 시험 seam 에서만 생긴다.
- **죽지 않지만 쓸모가 사라지는 것(걷지 않는다 — Q2):** ADR-0142 대상 축(`in_flight_targets` · `deferred_by_target` · `FlightSettle.targets`)은 그 ADR 이 적은 유일한 계기(두 이름 큐가 한 세션으로 수렴)가 id 키에서 생길 수 없다. `deferred_flush` 의 id 집합도 늘 큐 id 하나다. 정확성엔 해가 없어 이번엔 둔다.

## 4. 동작 변화

| # | 변화 | 근거 | 단위 |
|---|---|---|---|
| B1 | 잠든 사이 이름이 바뀐 에이전트가 깨면 대기 우편을 받는다(지금 24h 뒤 `expired`) | 결정 5 | U2a |
| B2 | 옛 이름을 새로 가진 **다른** 에이전트는 대기 우편을 받지 않는다(잠듦 · 바쁨 · 주입 실패 파킹 모두) | 결정 5 | U2a |
| B3 | 이름을 바꿔도 자기가 보낸 대기 우편이 `messages` 목록에 남는다(행의 `from` 은 보낼 때 이름) | 결정 5 · 4 | U2b |
| B4 | 삭제 정리가 id 로 쓸어 간다 — 개명 뒤 삭제해도 옛 이름 앞 파킹분이 `RECIPIENT_DELETED` 가 되고, 산 동명이 있어도 지운 프로필 몫은 정리된다(지금은 정리 전체를 건너뛴다) | 결정 6 | U2b |
| B5 | 잠든 수신자 앞 계약이 처음부터 id 를 든다 — 개명 뒤 회신도 그 계약을 닫고, 받는 이가 개명해도 「내가 답할 의무」가 목록에 남는다 | 결정 1 · 7 (결정 5 목록엔 없다 — 아래 X0) | U1 |
| B6 | 잠든 에이전트를 정확한 id 로 부르면 파킹된다(지금 `RECIPIENT_NOT_FOUND`) | 결정 3 | U3 |
| B7 | 대문자 · 중괄호 · 하이픈 없음 · `urn:uuid:` 철자도 id 로 읽는다(`resolve_in` 과 같게 — `PeerId::parse_str` 은 기능 추가 없이 된다) | ADR-0309 영향(TRD 판정) — 다른 사유 없이 남아 있던 차이라 맞춘다 | U3 |

**ADR 밖 — 메인 판단 필요(Q1).** 전부 동명 쌍둥이 경우다(ADR-0120 이 막지만 ADR-0122 구멍 · 옛 데이터로 생긴다).
- **X0(낮음)** — B5 의 「의무가 목록에 남는다」는 결정 1 · 7 의 귀결이지만 결정 5 목록에 없다.
- **X1** — 산 쌍둥이가 `@all` · `@here` 로 펼쳐지면 지금 `RECIPIENT_AMBIGUOUS` 한 행 → **각자 행 · 둘 다 배달**. 잠든 쌍둥이도 `@all` 에서 지금 실패 행 → 둘 다 파킹. 결정 3 둘째 줄(명부 id 로 바로 펼친다)이 함의하지만 결정 5 에 적히지 않았다.
- **X2** — 쌍둥이 둘을 정확한 id 로 각각 지목하면 지금 둘째가 실패 행(M3) → 둘 다 배달. `["dup", "<dup1 id>"]` 는 지금 한 행 배달(A8) → 이름 행 `RECIPIENT_AMBIGUOUS` + id 행 배달 두 행.
- **X3** — `@all` 이 발신자와 같은 이름의 잠든 프로필을 포함한다(지금 이름으로 뺀다 — `service.rs:2861`). 발신자 제외는 id 로만 한다.
- **X4** — 산 쌍둥이의 등장 flush 를 지금은 건너뛴다(유일해질 때까지) → 각자 자기 큐를 flush.

**내부 결정(보고만):** 배달 관측 레코드 `DeliveryObservation.to_name` 은 지금 큐 키(보낼 때 이름) → 주입 순간 로스터 이름. 소비처는 관측 bin 하나(`bin/roundtrip_smoke.rs:883`)라 화면 변화가 없다.

## 5. 단위 — 순차(파일이 겹친다) · 단위 끝마다 초록

각 단위 끝 = `cargo build` · `cargo test -p engram-dashboard-messaging` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` 초록(데몬은 `test-harness` bin `drain_latency` 까지 컴파일된다). 단위 시작 전 메인이 로컬 커밋으로 되돌릴 지점을 만든다. 크기 = 손으로 쓰는 로직 줄 / 손대는 시험 줄(추정).

- **U1 — 계약 키 = id + 데몬이 잠든 (id, 이름) 쌍을 넘긴다** · 로직 ~250 / 시험 ~300 · 한 워커 가능(추정 80~110 호출).
  - 파일: messaging `service.rs`(포트 타입 · `push_recipient` 가 잠든 id 를 잡는다 · `handle_send` 의 `open_request` · `Reservation` · `ReplyClosing` · 회신 닫기 둘 · `deliver_notice` 의 `is_request_closed` · 드레인의 rebind 호출 삭제 · `open_items_for` ②③ id 대조 · 계약 시험 접근자) · `ledger.rs`(§2 계약 줄 전부 · 시험) · 데몬 `messaging_host.rs`(어댑터 + 그 시험 둘 · `LaneProbe`) · `tests/control_agent.rs`(`FixturePort`) · `connection_core.rs` 시험(계약 접근자).
  - 해석 규칙은 그대로(이름으로만 잠든 쪽을 찾는다). 시험용 결정적 id(§6-1)도 여기서 들인다.
  - 넘기는 접점: `AddressingSources { roster, dormant }` · `ResolvedRecipient::id()` · 계약 키 `(request_id, recipient_id)` · 시험 도우미 `tid` · `live` · `live_distinct` · `set_dormant`.
- **U2a — 우편함 · 장부 배달 기록 키 = id · 힌트 장치 걷기** · 로직 ~350(순감) / 시험 ~600(대부분 기계 치환) · 한 워커 경계(추정 110~150). 넘칠 조짐이면 §6-2 의 U2a 의미 시험 목록만 같은 단위 안에서 둘째 워커에 넘긴다.
  - 파일: `mailbox.rs`(키 · 힌트 삭제 · 시험) · `ledger.rs`(`MessageRecord` id 칸 · `record*` 가 id 와 이름을 함께 받는다 · `transition` · `fail_pending` 을 id 로) · `service.rs`(`park_into` · `park_notice(id)` · `flush_for(id)` · `drain_queue(id, snapshot)` — 대상 = 로스터에서 그 id 하나 · `flush_for_agent` = 큐가 비었으면 반환 후 `flush_for` · `deliver_notice` = 요청자 id 로 파킹 · `sweep` · 삭제 정리의 큐 쓸기와 장부 실패를 id 로(이름 축 가드는 아직 남는다) · 시험 접근자 id 화 + `parked_total_for_test` 신설) · 데몬 `messaging_host.rs:809`(`svc.flush_for(id)` — diff 는 아직 이름별) · `bin/drain_latency.rs`(`parked_len(id)`) · `connection_core.rs` 시험 · `tests/control_send.rs`(`parked_len(id)` · ` @all` / 공백 이름 「유령 큐 없음」 단언 → `parked_total_for_test() == 0`).
  - 넘기는 접점: 큐 · 기록이 id. 포트 두 메서드 · 이름 축 가드 · `open_items_for(me, …)` 의 이름 ① 대조는 아직 남는다(U2b 몫).
- **U2b — 삭제 정리 id 하나로 · 보낸 대기분 조회 id 로 · 포트 다듬기** · 로직 ~120 / 시험 ~200 · 한 워커 가능.
  - 파일: `service.rs`(`handle_profile_deleted` 가드 = `is_agent_live` 하나 · `fail_open_requests_from(sender_id)` · `name` 인자는 로그 전용 · 포트 두 메서드 삭제 · `open_items_for(me_id, now)`) · `ledger.rs`(`fail_open_requests_from` 을 `sender_id` 로) · 데몬 `messaging_host.rs`(구현 둘 + `LaneProbe`) · `control/ingress.rs:572` · `connection_core.rs:1711`(로그용 이름 그대로) · `tests/control_agent.rs`.
- **U3 — 입구: 무리 id 펼침 · 잠든 id 지목 · UUID 철자 · loser/A8/M3 걷기** · 로직 ~300(순감) / 시험 ~350 · 한 워커 가능(추정 100~130).
  - 파일: `groups.rs`(`GroupMember` · 계약 시험 묶음 · 음성 소스 넷 `:340`–`:401`) · `service.rs`(`resolve_addressing` · 새 해석기 — ① `PeerId::parse_str` 성공이면 산 id → 잠든 id 순서로 찾고 없으면 이름으로 ② 이름: 산 1 = 산 · 산 2+ = 모호(산) · 잠든 1 = 잠듦 · 잠든 2+ = 모호(저장) · 0 = 없음 ③ 같은 표기는 한 행 ④ 접기 키 = id/표기 · `absent_disposition` 을 `Resolution` 으로 흡수 · `build_to_attr` 는 정규 이름을 쓴다) · `tests/control_agent.rs:2016` 대조 시험(대문자 · 중괄호를 일치 목록으로 옮기고 「알고 남긴 불일치 둘째」 단언 삭제) · agent `commands.rs:1774` 부근 `resolve_in` 주석(「알고 남긴 불일치 — id 철자」 삭제 — 주석만).
- **U4 — 데몬 등장 diff = id** · 로직 ~60 / 시험 ~100 · 한 워커 가능(U2b 와 묶어도 된다).
  - 파일: `messaging_host.rs`(`FlushMsg::Appear { id }` · diff `prev: HashMap<AgentId, u32>` · 동명 건너뜀 삭제 · `flush_sink_*` 시험 여섯 `:1375`–`:1469` · 레인 시험 `:1855`).
- **U5 — 문서** · 메인 또는 문서 워커 + `/review doc`. `docs/process/S18-messaging-v1/spec/messaging-v1-spec.md`(§5 125 · 126 · 159 · 161 · 272줄 — 파킹 키 · 삭제 정리 두 축) · `docs/reference/architecture-overview.md:433` · `:500` · `docs/tracking.md` T-29 닫기 · step-log `## S29` · 코드 주석 쓸기(`rg "이름-키|이름 키|힌트|hinted|canonical 이름" crates/engram-dashboard-messaging/src` — 주석 갱신은 각 단위 몫이고 여기선 남은 것만).
- **U6(선택 — Q2)** — ADR-0142 대상 축 · `deferred_flush` id 집합 단순화. ADR-0142 개정이 먼저다.

## 6. 시험 전략

**6-1 기계 치환(단위마다 같은 법).**
- 메시징 시험의 결정적 id: `#[cfg(test)]` 도우미 `tid(name) -> PeerId`(std 만 쓰는 FNV-1a → `PeerId::from_u128` — uuid `v5` 기능을 들이지 않는다). `live(name)` 은 `tid(name)` 을 쓰게 바꾼다 — 같은 이름으로 다시 띄운 시험(잠듦 → 복원 151곳 중 다수)이 「같은 에이전트」로 그대로 읽힌다. **다른 실체여야 하는 쌍둥이 · 동명은 `live_distinct(name)`(무작위 id)로 바꾼다** — 같은 함수 안에서 같은 이름으로 `live` 를 두 번 부르는 시험을 먼저 grep 으로 뽑는다. `FakeDeliveryPort::set_dormant(&["x"])` 는 `(tid("x"), "x")` 를 싣고, 쌍둥이용 `set_dormant_agents(&[(id, name)])` 를 더한다.
- 접근자 치환(서비스 시험 기준 개수): `parked_len("x")` 96 → `parked_len(tid("x"))` · `flush_for("x", id)` 53 → `flush_for(id)` · `parked_msg_ids` 11 · `in_flight_len` 8 · `contract_*_for_test(m, "x")` 39 → `(m, tid("x"))` · `open_items_for("x", id, now)` 20 → `(id, now)` · 장부 시험 `open_request(…, Some(id)|None, …)` 54 → `PeerId`. 데몬 시험은 실제 id(`boss.id` · `to_id` 등)로 바꾼다.

**6-2 의미가 바뀌는 시험(이름 = 서비스 시험 모듈 안 함수 이름 · 판정은 워커가 다시 확인).**
- U1: `a_dormant_contract_binds_the_real_id_when_the_parked_request_is_injected` → 계약이 열 때부터 잠든 id 를 든다로 재작성 · `rebind_request_recipient_moves_the_obligation_to_the_actual_deliveree`(장부) 삭제 · 이름 폴백 · `allow_name_fallback` 장부 시험 삭제 · `a_namesake_cannot_close_a_dormant_contract_it_never_received` 는 남긴다(단언 그대로).
- U2a: `hinted_id_survives_park_drain_and_restore_ordered` · `queues_with_hint_finds_only_queues_holding_that_id`(우편함) 삭제 · `two_name_queues_resolving_to_one_session_do_not_inject_at_the_same_time` · `a_busy_recipients_hint_*` 둘 · `a_stale_id_deferral_does_not_erase_the_live_one_and_the_mail_still_drains` · `rename_flush_ignores_queues_without_a_hint_for_that_id` 삭제(만들 수 없는 상황) · `dead_id_hint_falls_back_to_unique_name_rule_for_respawn` → **B2 회귀로 뒤집기** · `flush_reinjects_to_current_id_when_supplied_id_is_stale` → 같은 id 새 화신으로 재작성 · `flush_skips_when_name_ambiguous_at_execution_time` · `rename_mid_turn_still_flushes_old_name_queue_via_id_hint` · `exact_id_send_to_busy_agent_is_delivered_via_id_hint_despite_duplicate_names` · `timeout_notice_reaches_a_renamed_sender_via_id_hint` · `flush_for_agent_resolves_name_and_is_noop_without_parked` → 힌트 없이 같은 결말로 재작성.
- U2b: `the_name_axis_gate_sees_a_live_session_that_is_not_in_the_mail_roster` 삭제 · `deleting_a_dormant_profile_spares_a_live_namesakes_mail_and_contracts` → 지운 쪽은 정리되고 산 동명 몫은 남는다로 재작성(B4).
- U3: `the_loser_key_prefix_keeps_it_outside_the_agent_name_space` · `a_loser_row_never_collides_with_an_agent_whose_name_is_another_agents_id` 삭제 · `two_exact_ids_of_same_named_twins_both_get_a_row` · `an_exact_id_token_rescues_an_ambiguous_name_in_the_same_to_list` · `at_all_reports_a_duplicate_live_name_as_an_ambiguous_row_and_still_delivers_the_rest` · `two_dormant_profiles_sharing_a_name_block_the_park_as_ambiguous`(`@all` 갈래) → X1 · X2 결말로(Q1 이 받아들인 경우) · groups 계약 시험 묶음 · 데몬 `two_dormant_profiles_sharing_a_name_are_both_reported` · `a_live_namesake_does_not_hide_…`(`messaging_host.rs:1220` · `:1243`) → `dormant` 쌍 단언.

**6-3 새 회귀 시험(최소).**
| 시험 | 층 | 단위 |
|---|---|---|
| 잠든 X(이름 a) 앞 파킹 → X 를 b 로 개명(잠든 채) → X 가 b 로 깨어 flush → 배달 | 메시징 단위 | U2a |
| 위에서 X 개명 뒤 다른 프로필 Y 가 a 로 깨어 flush → **주입 없음** · 그 뒤 X 가 깨면 X 에게 배달 | 메시징 단위 | U2a |
| 바쁜 산 X 앞 파킹 → X 개명 → 턴 종료 flush → 배달(힌트 없이) | 메시징 단위 | U2a |
| alice(X) 가 잠든 bob 에게 보냄 → alice 개명 → `open_items_for(X)` 에 보낸 대기분이 `from = "alice"`(보낼 때 이름)로 남는다 | 메시징 단위 | U2b |
| 잠든 X(a) 에 파킹 + X 가 요청자인 계약 → X 를 b 로 개명 → 삭제 정리 → 파킹분 `RECIPIENT_DELETED` · 계약 `reply_failed` · 산 동명 Y(a) 의 파킹 · 계약은 그대로 | 메시징 단위 + 데몬 `connection_core` 삭제 훅 시험 확장 | U2b |
| 데몬 어댑터: `RenameProfile` 뒤 `addressing_sources().dormant` 가 같은 id · 새 이름 | 데몬 단위 | U1 |
| 대문자 · 중괄호 · 하이픈 없는 UUID 로 산/잠든 수신자 지목 → 배달/파킹 · 대조 시험에서 제어 = 우편 | 메시징 단위 + `tests/control_agent.rs` | U3 |
| 잠든 X 를 정확한 id 로 지목 → 파킹(B6) | 메시징 단위 | U3 |
| 등장 diff: 개명(같은 id · 같은 화신)은 등장 아님 · 새 화신 · 새 id 는 등장 · 쌍둥이도 각자 등장 | 데몬 단위 | U4 |

**6-4 게이트.** 단위 끝마다 §5 머리의 세 명령 + `cargo fmt --check`. U3 끝에 메시징 격리 게이트(CLAUDE.md 의 `rg "engram_dashboard_(…)"` · `cargo tree -p engram-dashboard-messaging` 상한 — 새 의존이 없어야 한다) · `cargo test -p engram-dashboard-agent -- --test-threads=4`(주석만 바뀌지만 빌드 확인). 마지막에 `/qa` · `/review code`.

## 7. 위험 · 열린 질문

- **Q1(사용자 결정 필요 — ADR 밖):** X0~X4 를 받아들이나. 기본안 = 받아들인다(동명은 미지원 — ADR-0116 결정 5 · ADR-0120 · 결정 3 둘째 줄의 귀결). 거부하면 U3 에서 무리 펼침을 이름 왕복으로 남겨야 하고, 그건 결정 3 둘째 줄과 어긋난다.
- **Q2(메인):** ADR-0142 대상 축을 걷나. 계기가 사라져 그 축을 덮던 시험(수렴 · 힌트)이 U2a 에서 사라지고, 남은 경로(같은 큐의 마지막 정산 ~ 대상 해제 사이)는 시험으로 만들기 어렵다 — 시험 없는 코드가 남는다. 기본안 = 이번엔 두고 U6 으로 따로(ADR-0142 개정).
- **위험 — 결정적 `live(name)`:** 같은 이름을 두 실체로 쓰던 시험이 조용히 한 실체가 돼 빈 통과할 수 있다. 완화 = 치환 전에 쌍둥이 시험을 grep 으로 뽑아 `live_distinct` 로 먼저 바꾼다.
- **위험 — U2a 크기:** 시험 모듈이 ~7300줄이다. 기계 치환을 한 번에 하지 않으면 호출이 넘친다. 완화 = §6-1 정규식 일괄 + §6-2 목록 분리.
- **위험 — 계약 `recipient_id` 를 Option 에서 뺀다(U1):** 계약을 여는 길이 `handle_send` 하나이고 실패 행은 계약을 열기 전에 빠지므로 id 가 늘 있다(가능성 높음 — `service.rs` 의 `Failed` → `continue` 가 `open_request` 앞). 다른 입구가 생기면 이 가정을 다시 본다.
- **남는 잔여(바뀌지 않음):** 회신은 가장자리에서 이름으로 간다(결정 2). 요청자가 개명하면 옛 이름 회신은 `RECIPIENT_NOT_FOUND` → 계약 `reply_failed`, 옛 이름을 다른 에이전트가 가져갔으면 그에게 배달된다 — 지금과 같다.
- **확인 안 한 것:** 프로필 복사 기능 유무(ADR-0309 영향) · CI 에 clippy 가 있는지.
