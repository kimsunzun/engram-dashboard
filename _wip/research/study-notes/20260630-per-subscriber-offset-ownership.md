# Study Note: per-subscriber offset/cursor 소유 위치 — 업계 분포 조사

**날짜:** 2026-06-30  
**강도:** deep  
**방법:** Claude(Sonnet) 직접 검색(WebSearch 12회 + WebFetch 7회) + Codex 독립 BLIND 조사 + 교차 대조 + 적대 검증

---

## 조사 주제

중앙에 콘텐츠/스트림이 있고 여러 구독자·디바이스·뷰가 각자 진행도를 갖고 소비하는 broadcast·협업·메시징 계열 시스템에서 "per-subscriber 진행도(offset/cursor) 소유"를 어디에 두는지.

---

## 쟁점과 결론 도달 과정

### 쟁점 1: Kafka — 클라이언트가 커밋을 "호출"한다는 사실이 "클라이언트 소유"로 오해될 수 있음

- **탐색 경로:** Confluent 공식 consumer design 문서 → `__consumer_offsets` 토픽 구조 확인.
- **반증 시도:** Kafka 0.8 이전 ZooKeeper offset 저장 방식이 "클라이언트 소유"처럼 보일 수 있음.
- **해소:** KIP-48 이후 현대 Kafka는 브로커의 `__consumer_offsets` 토픽에 서버 보관. ZK 방식도 클라이언트→중앙 조정자(ZK) 커밋 구조 = 중앙 보관 패턴. "클라이언트가 커밋 API를 호출" ≠ "클라이언트가 보관."
- **결론:** 브로커 서버 소유. 확실.

### 쟁점 2: Yjs awareness — y-websocket 서버가 awareness를 "캐시"하면 서버 소유가 아닌가

- **탐색 경로:** Yjs Awareness 공식 docs + y-websocket 구조 분석.
- **반증 시도:** 서버가 현재 연결 중인 awareness를 메모리에 캐시할 수 있음.
- **해소:** 이는 ephemeral relay cache이지 durable per-client cursor 보관이 아님. 서버 재시작 시 사라짐. awareness는 `yjs` 모듈 외부의 optional ephemeral presence 시스템.
- **결론:** 클라이언트 소유(서버는 중계 역할만). 확실.

### 쟁점 3: Docker daemon — "서버가 로그를 가지고 있으니 per-client cursor도 서버가 관리하지 않나"

- **탐색 경로:** moby/daemon/logs.go 소스코드 직접 확인 + GitHub Issue #11337.
- **반증 시도:** 로그 원본이 서버에 있으니 per-client 진행도도 서버에 있을 것이라는 직관.
- **해소:** `ContainerLogs()`는 매 요청마다 독립 `ReadLogs()` 인스턴스 생성. `ReadConfig{Since, Until, Tail}` = 클라이언트 제공 파라미터. Issue #11337은 offset 기능 부재를 공식 확인 → stateless가 의도된 설계.
- **결론:** per-client cursor 개념 없음 (stateless). 확실.

### 쟁점 4: Matrix — per-user vs per-device 구분

- **탐색 경로:** Patrick Cloke(Matrix core dev)의 read receipts 기술 포스트.
- **발견:** read receipt는 per-user 단위. `m.read.private`(MSC2285)가 기기 간 알림 동기화용이지 per-device storage가 아님. fully_read marker = user account_data.
- **결론:** 서버 보관, per-user(not per-device). 확실.

---

## 교차검증 요약

| 시스템 | Claude | Codex | 결과 |
|---|---|---|---|
| Kafka | 브로커 소유 | 브로커 소유 | 수렴 |
| Redis Streams PEL | 서버 소유 | 서버 소유 | 수렴 |
| Yjs awareness | 클라이언트 소유 | 클라이언트 소유 | 수렴 |
| Automerge sync State | 구현자 선택(p2p) | 구현자 선택(p2p) | 수렴 |
| Matrix read receipt | 서버, per-user | 서버, per-user | 수렴 |
| Slack read cursor | 서버, per-user | 서버, per-user | 수렴 |
| Signal read state | 클라이언트 | 클라이언트 | 수렴 |
| Docker logs cursor | 없음(stateless) | 없음(stateless) | 수렴 |

**전 갈래 수렴 — 불일치 없음.** deep tier 적대 검증 3건 모두 통과.

---

## 업계 분포 — 최종

서버 소유(다수): Kafka, Redis Streams, Matrix, Slack  
클라이언트 소유: Yjs awareness, Signal  
소유 개념 없음(stateless): Docker daemon logs  
구현자 선택(p2p): Automerge sync State  

**핵심 패턴:** 스트림 원본을 중앙에 두는 시스템은 per-subscriber 진행도도 중앙(서버/중계 계층)이 관리하는 것이 표준. 전송 시퀀스와 소비 진행도의 분리는 Kafka/Redis/Matrix/Slack 모두에서 명확.

---

## deep tier 학습 노트 — 이 조사에서 tier가 어떻게 달랐나

- **팬아웃:** 단일 갈래가 아닌 4갈래 병렬(Kafka/Redis · Yjs/Automerge · Matrix/Signal/Slack · Docker).
- **WebFetch 사용:** 검색 결과 URL을 직접 페치해 공식 문서 원문 확인(moby 소스코드 포함).
- **적대 검증 3건:** 각 주요 클레임에 반증 시도 → 모두 통과로 확신도 추가 상승.
- **medium이었다면:** WebFetch 생략, 적대검증 1~2건만, Automerge 소스 확인 생략 가능. Docker stateless 확인이 "가능성 높음"에 머물렀을 것.
- **결과 차이:** Docker의 stateless 설계와 Matrix per-user(not per-device) 구분은 WebFetch/소스 직접 확인으로 "불확실→확실"로 격상됨.
