# S18 메시징 v1 — 봉투·타입·그룹·메일박스 spec (PRD+TRD 통합)

> 2026-07-24 사용자 결정 완료. 결정 근거·거부 대안의 정본 = ADR-0103. 이 문서는 구현 계약(무엇을 어떻게)의 정본.
> 배경 리서치(이메일·액터·FIPA·AMQP·LLM 프로토콜 5갈래 서베이 + cross-family 적대 리뷰)는 §9 요약 참조.

## 0. 스코프 한 줄

**데몬이 살아 있는 동안** 에이전트 간 메시지가 확실히 가게 한다 — 봉투 정형화(XML) + 회신 계약(request) + 그룹 발송 + 인메모리 메일박스(파킹·장부·조회). **영속화(디스크) 없음** — 에이전트 시스템 메모리 설계 때(사용자 결정).

## 1. 봉투 — 수신 LLM에게 보이는 것 (4종, XML 단일화)

```xml
<message from="qa-alpha">빌드 끝났음</message>
<message from="qa-alpha" id="m-7f3k" type="request" reply-by="10m">코드 짜고 회신해</message>
<message from="qa-bravo" in-reply-to="m-7f3k">다 짰음, 테스트 통과</message>
<message from="qa-alpha" to="@coders">전원 리베이스 대기</message>
<notice>요청 m-7f3k 기한(10m) 초과 — qa-bravo 회신 없음</notice>
```

- **표기 매핑(고정):** 툴/CLI 인자 = snake_case(`reply_to`·`reply_by`) → XML 속성 = kebab-case(`in-reply-to`·`reply-by`). 발신 인자 `reply_to`가 수신 봉투 속성 `in-reply-to`로 나타난다(요청자가 어느 요청의 답인지 상관하도록 노출).

- **노출 원칙: LLM의 행동을 바꾸는 필드만 보인다.** `id`는 request에만(회신에 필요), `to`는 그룹일 때만(방송임을 알림), 시각·장부 상태는 내부 데이터.
- `<message>` = 동료 발신(from에게 회신 가능) / `<notice>` = 데몬 통지(from 없음 = 회신 대상 아님). 태그 분리로 "system" 가짜 발신자·이름 예약 문제를 구조적으로 제거.
- 포맷은 기존 wrap seam(ADR-0096)의 **Xml 변형을 기본값으로 전환 + 속성 확장**. 콜론 변형은 스위치로 잔존(삭제 아님). 이스케이프는 기존 XML 이스케이프 재사용.
- 전환 검증: `roundtrip-smoke` 재실행(LLM이 XML 봉투를 읽고 회신하는지 — 포맷 결합 부분만 재검증, 배관은 포맷 무관). **미검증 — §7 수용 기준의 구현 QA 항목.**

## 2. 타입 체계 — "데몬이 다르게 행동할 때만 타입"

| 표기 | 데몬 행동 |
|---|---|
| (기본, type 없음) | 통보 — 배달하고 끝 |
| `type=request` | 장부에 "미회신" 오픈(+ `reply_by` 있으면 타이머). **단일 수신자만**(그룹 거부) |
| `in_reply_to=<id>` (필드, 타입 아님) | 회신 판정 → 미회신 닫힘(`replied`) |
| `<notice>` (데몬 전용) | 타임아웃 등 인프라 통지. 에이전트가 못 씀 |

- **refuse/failure는 타입 아님** — 회신 본문 소관("거절함/실패함"). 장부는 `replied`로만 닫음. 필요 시 v2에서 사유 칸 추가(무파괴).
- **회신 매칭 = 엄격**: `in_reply_to` 있는 것만 인정. 프라이밍에 "request 회신엔 원본 id를 달아라" 규칙 보강(관대 매칭=우연 닫힘 오발 거부).
- 응답 계약 스펙트럼 중 채택 = 통보·요청 2종. 구독(→관측 인프라와 중복이라 영구 불채택 방향)·협상·진행보고 타입(스레드 일반 메시지로 무료 지원)·수신거부 ACL(에이전트 시스템 권한 설계 때)은 제외 — 근거 ADR-0103.

## 3. 회신 계약 (request)

1. 발송: `request` + 선택 `reply_by`(기간 표기 "10m"/"1h" — 데몬이 절대시각 환산).
2. 장부: 미회신(`awaiting_reply`) 오픈.
3. 회신(`in_reply_to`) 도착 → `replied` 닫힘.
4. `reply_by` 초과 → **발신자에게** `<notice>` 주입(수신자 재촉 아님 — 재촉 여부는 발신 LLM 판단). notice는 메일박스 가득참 예외 통로.
5. LLM측 강제는 soft(프라이밍) — 보장은 데몬 장부 추적이 담당(액터 call/ask + FIPA reply-by의 증류).

## 4. 그룹

- **명단 소스 = 런타임 등록**(데몬 인메모리): `group` 툴/CLI로 생성·증감·삭제. 데몬 재시작 시 소멸(인메모리 단계 정합).
- **멤버십 = 이름 기반**(id 아님) — 주소 체계(WYSIWYA, ADR-0101)와 동일 원칙, 재스폰 생존.
- **`@all` = 내장 그룹**(멤버 = 발송 순간 살아있는 수신 가능 전원, 관리 불요). `@` 네임스페이스는 기예약(GROUPS_NOT_SUPPORTED 자리 대체).
- **발송 = 순간 스냅샷 fan-out**: 살아있는 멤버만 개별 배달, 죽은 멤버 `skipped`(파킹 없음 — 방송 소급 금지). 장부 = 메시지 1 : 배달기록 N.
- 빈/미존재 그룹 발송 = 반려(`GROUP_EMPTY`/`GROUP_NOT_FOUND`). 그룹 request = 반려(`GROUP_REQUEST_UNSUPPORTED`, v1).
- 회신은 항상 발신자 1인에게(전체회신 없음).

## 5. 메일박스 (인메모리)

**단일 수신자 발송의 3분기:**
1. 수신 가능 → 즉시 주입 = `delivered`
2. 부재(미스폰·죽음·unreachable) → **파킹** = `pending` (반려 아님 — "없는 이름"도 파킹, 오타는 TTL이 방어. 스폰 전 선지시 지원)
3. 보관함 초과 → 반려 `MAILBOX_FULL` (오래된 것 조용히 버리기 금지)

**파킹의 운명:** 수신자 등장(스폰/epoch 교체) 시 자동 주입 `delivered` / TTL 초과 `expired`(장부 잔존).

**정책 상수:** TTL **1h** · 수신자당 **100건** · notice는 cap 예외.

**장부(ledger):** 전 메시지 이력 링버퍼 — 발신·수신·상태 전이 + 시각(`pending→delivered→replied` / `expired` / `skipped`). 상태 전이 시각이 곧 회신시각·발신시각 데이터(봉투 미노출).

**flush 훅:** AgentManager의 에이전트 등장/epoch 전이에서 해당 이름의 pending 주입(기존 `write_stdin_observed` 경로 재사용).

**주의:** `RECIPIENT_NOT_FOUND`는 단일 발송에서 소멸(파킹으로 대체). `RECIPIENT_AMBIGUOUS`(동명)·`BODY_TOO_LARGE`는 유지.

## 6. 입구 — MCP 주력 · CLI 예비 (듀얼 입구 유지, ADR-0086)

**MCP 툴 3개 (컨텍스트 비용: 디퍼드 로딩으로 상주 ~0 — 실측 2026-07-24, claude 2.1.170):**

```
send_message { to, body, request?, reply_by?, reply_to? }   # 발송 전부. reply_to↔request 상호배타
messages     { id? }                                        # id=상태 조회 / 무인자=내 미결(pending+awaiting_reply)
group        { group?, add?, remove?, delete? }             # 관리. 무인자=목록(@all 포함)
```

**CLI (예비 미러 + 사람 테스트):**
```bash
engram-send --to <이름|@그룹> --body "..." [--request [--reply-by 10m]] [--reply-to m-xxxx]
engram-send --to <이름> --body-stdin <<'EOF' ... EOF        # 인용 지옥 회피(신규)
engram-send status <id> | pending
engram-send group list | group update @g --add a,b [--remove c] [--delete]
```

**발신 응답(두 입구 동일 JSON):** `{ id, results: [{to, status, hint?}] }` — 수신자별 `delivered|pending|skipped`. 발송 반려 = `{ status:"error", code, hint }`.

**allowedMcpServers 대책 (실측 2026-07-24 — 유저 전역 `[]`=전면 차단이 에이전트에도 적용됨):**
데몬이 스폰 시 `--settings`로 `{ "allowedMcpServers": [{"serverName":"engram"}] }` 조각을 **세션 한정 주입**(backend/claude.rs — 백엔드 지식 격리 ADR-0004 자리). 전역 파일 무변경·허용 범위 = 엔그램 스폰 에이전트뿐. merge 동작은 구현 QA에서 실증(미검증 항목).

## 7. 수용 기준

- 파킹→스폰→자동배달 시나리오 하네스(roundtrip-smoke 확장) green
- request→회신→`replied` 전이 + `reply_by` 초과→notice 주입 관측
- XML 봉투로 실 claude 왕복(포맷 전환 재검증) green
- 그룹 스냅샷 fan-out + skipped 기록 단위테스트
- cap/TTL/`MAILBOX_FULL` 단위테스트 · 기존 전 테스트 무회귀(`cargo test` 워크스페이스)
- `--settings` 주입 후 스폰 에이전트에서 send_message 툴 가시 + 호출이 데몬 도달(실측)

## 8. v2+ 확장 경로 (봉투 = XML 속성이라 전부 무파괴)

cc(수신자 역할 — 역할 명시 주입 필수, 리뷰 지적) · 그룹 request(수신자별 계약 + any/all 집계) · 장부 사유 칸(refused/failed 구분) · thread_id(대화 묶음 — v1은 in_reply_to로 충분, 장부 TTL 확장 시 재검토) · ACL/수신거부 · 영속화(SQLite 예상) · 시스템 조작 버스 `engram_control`(S17).

## 9. 리서치 근거 요약 (2026-07-24, medium tier + cross-family 적대 리뷰)

- **수렴 코어(확실 — 3계보 교차확증):** 메시지 id·발신자·회신 상관(이메일 In-Reply-To ≡ FIPA reply-with/in-reply-to ≡ AMQP correlation-id). 회신 기한 = FIPA `reply-by` + 액터 call/ask 타임아웃의 선례.
- **타입 어휘:** FIPA 22 performative(산업 채택 실패) ↔ MCP 4종·A2A 무타입(본문이 의도 전달)의 스펙트럼에서 "데몬 행동 기준 최소형" 채택.
- **전원 생략 확인(LLM 프로토콜 2024-26):** priority·cc·TTL 봉투 필드 — 우리도 skip(priority는 AMQP 실무 경고도 근거).
- **선례:** Claude Code 팀 = 파일 수신함+idle 주입(우리 파킹·주입과 동형) · AMQP DLX = 만료를 사유 딱지로 장부화 · validated user-id = 발신자 데몬 각인(기구현).
- **적대 리뷰 교정 반영:** 그룹×회신계약 충돌(→v1 그룹 request 금지) · cc는 서술적 관례일 뿐(→v2로, 역할 명시 주입 전제) · in_reply_to 체인은 스레드 대체 불가(→thread_id v2 항목) · "MDN 사실상 사망"은 과장(교정됨).
- 상세 출처·확신도는 세션 리서치 산출(ADR-0103 참조 절)에 요약.
