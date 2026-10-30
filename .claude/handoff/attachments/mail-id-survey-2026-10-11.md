# 우편 주소 — 이름 키 vs 에이전트 id 현황 조사 (2026-10-11 · 읽기 전용 · cargo 안 돌림)

사용자 요청(2026-10-11): 에이전트에게는 이름으로 부르게 하고 내부는 에이전트 id 로 통일 — 구현 목록에 포함. 앞서 사용자가 정한 것 = 패닉 정책을 끝내고 transport 는 새 세션에서.
순서 = 메인 제안(사용자 확인 전): 패닉 구현 → 우편 id 통일 → transport. 절차 = CLAUDE.md 「개발 스텝」(TRD → 리뷰 → 결정 → 구현).

## 결론
- 에이전트 대면 = 이름(canonical) · 정확한 id · `@here`/`@all`. 내부도 대부분 이름 키 — 우편함 큐 `HashMap<String, VecDeque<ParkedMessage>>`(messaging/src/mailbox.rs:282-284) · 배달 중 `HashMap<String, FlightTicket>`(:286) · flush 연기 `deferred_flush: HashMap<String, Vec<PeerId>>`(service.rs:469) · 장부 `MessageRecord{from,to: String}`(ledger.rs:133-138) · 회신 계약 sender/recipient 이름 + id 곁칸(ledger.rs:176-201, 닫기 = id 먼저 · 이름 폴백 :954-1000).
- id 키인 곳 = 바쁨/턴(busy.rs:159, 248) · 대상 축 in-flight(service.rs:486, 498 — ADR-0142).
- 이름 → id 풀이가 한 번이 아니다: 보낼 때(한 스냅숏) · 배달마다 `unique_reachable_in(roster, name)`(service.rs:1448, 1517-1532) · idle 깨우기(id→name→큐, service.rs:1854-1878) · 명부 diff(이름으로 묶음, daemon messaging_host.rs:486-560) · 회신 닫기(id 먼저 · 이름 폴백).
- 영속 없음(메모리 — ADR-0103) → 데이터 이전 불요.

## 실제 결함
- `agent.rename` 에 메시징 훅 없음(connection_core.rs:1834-1842).
- 잠든 에이전트 앞으로 쌓인 우편은 `hinted_id = None`(mailbox.rs:148-153 · ADR-0116 결정 1) → 이름 바꾸면 옛 이름 큐에 갇혀 24h TTL 만료. 그 사이 다른 에이전트가 옛 이름을 가져가면 배달 시 이름 규칙으로 **그 에이전트에게 배달**(ADR-0120 거부 대안 2 가 막으려던 바로 그것 — 잠듦만 분석, 이름 바꾸기는 분석 안 됨).
- 잠든 동안 받은 요청 계약은 `recipient_id = None` → 이름 바꾼 뒤 회신이 id 로도 옛 이름으로도 안 맞아 계약이 열린 채 마감 알림. (요청자가 이름을 바꾼 경우의 reply_to 라우팅은 추정 — 미추적.)
- 바쁜 동안 쌓인 우편(id 힌트 있음)은 `queues_with_hint` 역방향 훑기(mailbox.rs:612-629)로 살아남는다 — 보조 장치.
- 지우고 같은 이름으로 다시 만들기: 세션이 살아 있으면 정리 건너뜀(service.rs:1187+) → 그 뒤 새 에이전트가 이름 규칙으로 상속.

## 관련 ADR
- 0087 결정 4: 주소 = 사람이 읽는 이름, 유일성 강제, 이름은 AgentId 에 묶임. 기각: UUID 주소(LLM 오기재 · OSS 채택 0).
- 0101: canonical 이름 = 표시 이름; id 는 fallback(정확일치 우선). 기각: id 전용 주소(마찰).
- 0110: 커널 `PeerId = Uuid`(AgentId 와 같은 비트, 의존 없음).
- 0111: 보낼 때 한 스냅숏(결정 2) · 같은 이름 새 화신에 배달 가능(결정 6).
- 0114 · 0115 · 0120: 기각 대안 = 「이름 키 + id 키 이중 체계」 — 사유 = 비용 과다. **단일 id 키 안은 검토된 적 없음.**
- 0116: 잠듦 파킹 = 이름 큐 · id 없음; 프로필 삭제 = id 축 문 + 이름 축 정리.
- 「재스폰 생존」을 이름 키 근거로 인용(groups.rs:6, service.rs:2774, mailbox.rs:153)하지만 AgentId 도 재스폰에 유지됨(ADR-0007 · ledger.rs:196).

## 크기 · 위험
- 중간~큼: 로직은 줄어든다(힌트 장치 · 역방향 훑기 · 동명 건너뛰기 · 이름 폴백 삭제). 시험 약 300개가 이름을 키로 씀(대량 기계 수정).
- 범위: messaging(mailbox · service · ledger · groups · envelope) · daemon(messaging_host · connection_core 삭제 훅 · ingress 는 입구에서만 이름 풀이 · eg_messages 행 렌더) · daemon 시험(control_send · mail_gate · 인라인 ~42).
- wire: MCP `eg_send` 스키마 그대로 가능. 봉투 from/to 는 렌더 때 이름.
- 사용자 결정 거리: `eg_messages` / 열린 항목에 보이는 이름 = 지금 이름 vs 보낼 때 이름.
- 의미 변화: 지우고 같은 이름 재생성은 더는 파킹 우편 · 계약을 상속하지 않음(오배달 수정) → RECIPIENT_DELETED 또는 TTL.
- 순서 안: 우편함을 id 로 먼저(서비스 경계에 이름→id 어댑터) → 계약 → 힌트 장치 삭제. 새 ADR 이 0101/0114/0115/0116/0120 의 내부 이름 키 전제를 대체.
