# S18 메시징 v1 — 봉투·타입·그룹·메일박스 spec (PRD+TRD 통합)

> 2026-07-24 사용자 결정 완료. 결정 근거·거부 대안의 정본 = ADR-0103. 이 문서는 구현 계약(무엇을 어떻게)의 정본.
> 2026-07-24(2차) 보완 결정 — 그룹 해석 seam·wake 연기·idle 게이트 일괄 주입. 근거 정본 = ADR-0104.
> 배경 리서치(이메일·액터·FIPA·AMQP·LLM 프로토콜 5갈래 서베이 + cross-family 적대 리뷰)는 §9 요약 참조.

## 0. 스코프 한 줄

**데몬이 살아 있는 동안** 에이전트 간 메시지가 확실히 가게 한다 — 봉투 정형화(XML) + 회신 계약(request) + 그룹 발송 + 인메모리 메일박스(파킹·장부·조회). **영속화(디스크) 없음** — 에이전트 시스템 메모리 설계 때(사용자 결정).

## 1. 봉투 — 수신 LLM에게 보이는 것 (4종, XML 단일화)

```xml
<message from="qa-alpha">빌드 끝났음</message>
<message from="qa-alpha" id="m-7f3k" type="request" reply-by="10m">코드 짜고 회신해</message>
<message from="qa-bravo" in-reply-to="m-7f3k">다 짰음, 테스트 통과</message>
<message from="qa-alpha" to="@coders">전원 리베이스 대기</message>
<notice>[engram] 요청 m-7f3k 기한(10m) 초과 — qa-bravo 회신 없음</notice>
```

- **표기 매핑(고정):** 툴/CLI 인자 = snake_case(`reply_to`·`reply_by`) → XML 속성 = kebab-case(`in-reply-to`·`reply-by`). 발신 인자 `reply_to`가 수신 봉투 속성 `in-reply-to`로 나타난다(요청자가 어느 요청의 답인지 상관하도록 노출).

- **노출 원칙: LLM의 행동을 바꾸는 필드만 보인다.** `id`는 request에만(회신에 필요), `to`는 그룹일 때만(방송임을 알림), 시각·장부 상태는 내부 데이터.
- `<message>` = 동료 발신(from에게 회신 가능) / `<notice>` = 데몬 통지(from 없음 = 회신 대상 아님). 태그 분리로 "system" 가짜 발신자·이름 예약 문제를 구조적으로 제거. **notice 자기식별(사용자 결정 2026-07-26):** 본문 머리표 `[engram]` + 프라이밍에 "`<notice>` = 엔그램 데몬(시스템) 통지, `from` 없음이 표시, 회신 대상 아님" 1절.
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

1. 발송: `request` + 선택 `reply_by`(기간 표기 "10m"/"1h" — 데몬이 절대시각 환산. **하한 1분** = 타임아웃 감지 해상도(60s 스윕)와 정합 · **상한 30일** = 내부 시각 연산 안전 — 구현 결정 2026-07-26). 계약 필드(request/reply_to)는 **XML 봉투 전용** — 콜론/템플릿 포맷 활성 시 반려(속성이 떨어져 회신 불가·오탐 notice가 되므로).
2. 장부: 미회신(`awaiting_reply`) 오픈.
3. 회신(`in_reply_to`) 도착 → `replied` 닫힘.
4. `reply_by` 초과 → **발신자에게** `<notice>` 주입(수신자 재촉 아님 — 재촉 여부는 발신 LLM 판단). notice는 **전용 유계 레인**(반려 없음 — §5 정책 상수, ADR-0107. 구 "가득참 예외 통로" 문구 대체).
5. **기한 초과 ≠ 종결(ADR-0108):** 통지 후에도 계약은 회신까지 오픈 — 수신자의 미결 조회(`messages`)에 `timed_out`으로 계속 보인다. **동시 오픈 상한 512(전역)** 도달 시 **은퇴 가능 계약**(통지 완료분 또는 기한 없는 것 — 통지 의무 미발화분은 절대 불가) 중 최고령을 은퇴시키고 신규 수용(**이력 행은 유지·계약 추적만 제거** + 데몬 로그 잔존, 발신자 통지 없음), 은퇴 불가 계약만 512개면 `REQUEST_CAPACITY` 반려.
6. LLM측 강제는 soft(프라이밍) — 보장은 데몬 장부 추적이 담당(액터 call/ask + FIPA reply-by의 증류).

## 4. 그룹

- **명단 소스 = 런타임 등록**(데몬 인메모리): `group` 툴/CLI로 생성·증감·삭제. 데몬 재시작 시 소멸(인메모리 단계 정합).
- **관리 semantics(D 확정, ADR-0109 — 사용자 결정 2026-07-26~27):** **전원 수정 가능**(ACL·조작 주체 기록 없음 — 고도화 백로그) · **조용히**(변경·삭제 통지 없음) · **암묵 생성**(create 동사 없음 — 없는 그룹에 add하면 생성, remove·조회는 생성 안 함) · **그룹명 `@` 필수**(`INVALID_GROUP_NAME`) · **멤버명에 `@` 금지**(`INVALID_MEMBER_NAME` — 중첩 그룹 미지원) · `@all` 보호(`GROUP_BUILTIN`) · **명단 변경·삭제는 이미 파킹된 방송분에 무영향**(스냅샷 원칙) · 멤버명 정규화(콤마 분해·트림)는 **데몬 단일점**(두 입구 동일 결과) · 배치 변경은 전검증 후 일괄 적용(부분 반영 금지).
- **그룹 해석 = seam**(ADR-0104): "그룹 이름 → 멤버 목록" 해석기(GroupRegistry)를 소스 플러그인 구조로 짠다 — v1 소스 = 런타임 명단 + `@all`. **폴더가 데몬 소유로 생기면 폴더 = 추가 소스**(`@폴더명` — 조직 방송의 정본 단위 예정). 하위 에이전트 계층은 주소 단위 **비채택**(오케스트레이터가 스폰하며 런타임 그룹을 등록하면 동일 효과 + 동적 명단은 스냅샷 원칙과 마찰 — ADR-0104 거부 대안).
- **멤버십 = 이름 기반**(id 아님) — 주소 체계(WYSIWYA, ADR-0101)와 동일 원칙, 재스폰 생존.
- **`@all` = 내장 그룹**(멤버 = 발송 순간 살아있는 수신 가능 전원, 관리 불요). `@` 네임스페이스는 기예약(GROUPS_NOT_SUPPORTED 자리 대체).
- **발송 = 순간 스냅샷 fan-out**(멤버별 결말 — C4 구현, ADR-0107): idle+앞선 큐 없음 → 즉시 주입 `delivered` / **busy 또는 앞선 파킹 있음 → 파킹 `pending`(발송 순간 그 멤버의 `(id,epoch)`에 결박** — 재스폰 동명에게 배달 금지 = 방송 소급 금지의 강제 수단. 구 "파킹 없음" 문구 대체: 불변식은 유지, busy 멤버가 방송을 통째 놓치는 손실 제거) / 부재·죽음·동명 다수·비structured·보관함 cap·write 실패 → `skipped`(파킹 없음 — 단일 발송의 `MAILBOX_FULL` *반려*와 달리 방송 멤버는 *skip 기록*). 장부 = 메시지 1 : 배달기록 N.
- 빈/미존재 그룹 발송 = 반려(`GROUP_EMPTY`/`GROUP_NOT_FOUND`). 그룹 request = 반려(`GROUP_REQUEST_UNSUPPORTED`, v1).
- 회신은 항상 발신자 1인에게(전체회신 없음).

## 5. 메일박스 (인메모리)

**단일 수신자 발송의 3분기:**
1. 수신 가능 → 주입 = `delivered` — 단 턴 진행 중(busy)이면 메일박스 대기 = **`pending`**(부재 파킹과 상태 어휘 공유 — 새 상태 발명 금지, idle 진입 시 일괄 flush: 아래 주입 타이밍, ADR-0104)
2. 부재(미스폰·죽음·unreachable) → **파킹** = `pending` (반려 아님 — "없는 이름"도 파킹, 오타는 TTL이 방어. 스폰 전 선지시 지원. **깨우기(wake) 없음** — 잠든 수신자도 파킹으로 동일 취급, wake-on-request는 v2 후보로 연기: ADR-0104)
3. 보관함 초과 → **압력 회수 후 수용, 회수 불가면 반려 `MAILBOX_FULL`**(ADR-0107): 배달 불가 잔해(죽은 incarnation 결박분 — 동명 생존 목록 비소속만)를 오래된 순 은퇴(장부 `skipped`, TTL 경과분은 `expired`)해 자리를 만들고, 걷어낼 잔해가 없으면 반려. **산 메일 조용히 버리기 금지는 불변** — 모든 제거는 장부 종점을 남긴다(레코드가 이력 링에서 이미 밀려난 경우만 debug 로그로 강등 — best-effort, 제거 사실은 로그 잔존).

**파킹의 운명:** 수신자 등장(스폰/epoch 교체) 시 자동 주입 `delivered`(**일괄·오래된 순** — 아래 주입 타이밍의 flush 규칙과 동일: 경로 2벌 금지, ADR-0104) / TTL 초과 `expired`(장부 잔존). **TTL은 부재 파킹·busy 대기 구분 없이 동일 적용 — 생존 기반 면제 없음**(시계 기반 단일 규칙, ADR-0105).

**주입 타이밍 = idle 게이트 + 일괄 flush (ADR-0104):**
- 수신자가 턴 진행 중(busy)이면 주입하지 않고 메일박스 대기 — `delivered`는 실제 주입 시점에만 찍는다(CLI 내부 stdin 큐로 밀어넣지 않음: 장부 정확성 + 배치 제어권 유지).
- idle 진입(턴 종료)·등장(스폰/epoch) 시 쌓인 메시지 **전부 일괄 주입**(오래된 순, 메시지마다 자기 봉투 — XML이라 배치 내 경계 명확). 1건씩 드리블 금지.
- busy/idle 관측 = 백엔드 capability(stream-json 턴 이벤트 기반) — 관측 불가 백엔드는 즉시 주입 폴백(§2 capability 원칙 정합).
- `reply_by` 시계는 발송 기준 유지(수신 지연과 무관 — 발신자 관점 계약).

**정책 상수(ADR-0107):** TTL **24h**(1h→상향, ADR-0105 — 인메모리 단계 한정, 영속화 때 재설계) · message 레인 수신자당 **100건**(결박 유무 무관 전량 계수, 보관+주입중 합산 — in-flight 회계) · **notice 전용 레인 20건**(반려 없음 — 초과 시 가장 오래된 통지를 은퇴하고 장부 `skipped` 기록. 주입 중 배치 창에서 **+1 일시 초과 허용**(명시 계약·자기수정). 구 "cap 예외" 대체 — 무계 통로 제거).

**장부(ledger):** 전 메시지 이력 링버퍼 **4096건**(1024→상향, 사용자 결정 2026-07-26 — 본문 전문 보관이라 최악(64KiB×4096) ≈ 256MiB 수용, 런타임 설정 노출·본문 절단 저장은 후속 항목) — 발신·수신·상태 전이 + 시각(`pending→delivered→replied` / `expired` / `skipped`). 상태 전이 시각이 곧 회신시각·발신시각 데이터(봉투 미노출). 링에서 밀려난 레코드의 종점 전이는 debug 로그로 강등(best-effort).

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

**발신 응답(두 입구 동일 JSON — 바이트 동일, 파리티 테스트 고정):** `{ id, results: [{to, status, hint?}] }` — 수신자별 `delivered|pending|skipped`. 발송 반려 = `{ status:"error", code, hint }`. 에러 어휘(C3 추가분): `INVALID_SEND_ARGS`(계약 필드 문법·상호배타·비XML 포맷 위반) · `REQUEST_CAPACITY`(**은퇴 불가 계약만으로** 동시 상한 512 도달 — 은퇴 가능분이 있으면 은퇴 후 수용, §3 항목 5·ADR-0108) · `INTERNAL_ID_COLLISION`(id 재생성 1회 후에도 충돌 — 사실상 미발생 가드). **D 추가분(관리·조회, ADR-0109):** `INVALID_GROUP_NAME` · `INVALID_MEMBER_NAME` · `GROUP_BUILTIN` · `INVALID_GROUP_ARGS` · `MESSAGE_NOT_FOUND`. 메시지 id 표기 = `m-` + base36 8자(수신 LLM이 `reply_to`로 옮겨 적는 값이라 단문형 — UUID 폐기). **CLI 신원 = `ENGRAM_TOKEN` 유일**(위장 플래그 없음 — 서버측 파생). `messages{id}` 응답은 수신자별 행 + `may_be_truncated`(발송 시점 기대 행수 대비 — 이력 링 유실 정직 공개), 시각은 상대 초.

**allowedMcpServers 대책 (실측 2026-07-24 — 유저 전역 `[]`=전면 차단이 에이전트에도 적용됨):**
데몬이 스폰 시 `--settings`로 `{ "allowedMcpServers": [{"serverName":"engram"}] }` 조각을 **세션 한정 주입**(backend/claude.rs — 백엔드 지식 격리 ADR-0004 자리). **구현 확정(D, ADR-0109): 인라인 JSON이 아니라 파일 경로 주입**(Windows `cmd /c` 인용 붕괴 = 조용한 실패 모드 회피) — 조각 파일은 mcp-config 디렉토리·스윕·revoke 수명주기 동승(ADR-0099), 단일 키 유지(회귀 테스트 고정), 쓰기 실패는 warn+계속. 전역 파일 무변경·허용 범위 = 엔그램 스폰 에이전트뿐. merge 동작 라이브 실증 = 인수 런 항목(미검증).

## 7. 수용 기준

- 파킹→스폰→자동배달 시나리오 하네스(roundtrip-smoke 확장) green
- **idle 게이트·배치(중점 검증 — 2026-07-24 사용자 지시로 강화):** busy 중 도착 → 미주입·턴 종료 시 flush 관측 / 다건 누적 순서 보존(오래된 순)·일괄 주입 / 배치 내 request 포함 케이스 / 잠든 수신자 wake 미발동(파킹 유지) 회귀 — 단위 + 실 claude 하네스 양쪽
- 턴 진행 중 stdin 주입 시 CLI(stream-json) 큐잉 동작 실측(즉시 주입 폴백 설계의 근거 데이터)
- request→회신→`replied` 전이 + `reply_by` 초과→notice 주입 관측
- XML 봉투로 실 claude 왕복(포맷 전환 재검증) green
- 그룹 스냅샷 fan-out + skipped 기록 단위테스트
- cap/TTL/`MAILBOX_FULL` 단위테스트 · 기존 전 테스트 무회귀(`cargo test` 워크스페이스)
- `--settings` 주입 후 스폰 에이전트에서 send_message 툴 가시 + 호출이 데몬 도달(실측)

## 8. v2+ 확장 경로 (봉투 = XML 속성이라 전부 무파괴)

cc(수신자 역할 — 역할 명시 주입 필수, 리뷰 지적) · 그룹 request(수신자별 계약 + any/all 집계) · 장부 사유 칸(refused/failed 구분) · thread_id(대화 묶음 — v1은 in_reply_to로 충분, 장부 TTL 확장 시 재검토) · ACL/수신거부 · 영속화(SQLite 예상) · 시스템 조작 버스 `engram_control`(S17) · **wake-on-request**(잠든 수신자를 request 한정 깨워 배달 — 2026-07-24 연기, ADR-0104) · **폴더 그룹 소스**(트리/폴더 데몬 소유화 시 `@폴더명` — GroupRegistry seam에 소스 추가).

## 9. 리서치 근거 요약 (2026-07-24, medium tier + cross-family 적대 리뷰)

- **수렴 코어(확실 — 3계보 교차확증):** 메시지 id·발신자·회신 상관(이메일 In-Reply-To ≡ FIPA reply-with/in-reply-to ≡ AMQP correlation-id). 회신 기한 = FIPA `reply-by` + 액터 call/ask 타임아웃의 선례.
- **타입 어휘:** FIPA 22 performative(산업 채택 실패) ↔ MCP 4종·A2A 무타입(본문이 의도 전달)의 스펙트럼에서 "데몬 행동 기준 최소형" 채택.
- **전원 생략 확인(LLM 프로토콜 2024-26):** priority·cc·TTL 봉투 필드 — 우리도 skip(priority는 AMQP 실무 경고도 근거).
- **선례:** Claude Code 팀 = 파일 수신함+idle 주입(우리 파킹·주입과 동형) · AMQP DLX = 만료를 사유 딱지로 장부화 · validated user-id = 발신자 데몬 각인(기구현).
- **적대 리뷰 교정 반영:** 그룹×회신계약 충돌(→v1 그룹 request 금지) · cc는 서술적 관례일 뿐(→v2로, 역할 명시 주입 전제) · in_reply_to 체인은 스레드 대체 불가(→thread_id v2 항목) · "MDN 사실상 사망"은 과장(교정됨).
- 상세 출처·확신도는 세션 리서치 산출(ADR-0103 참조 절)에 요약.
