# ADR-0101: 에이전트 canonical 이름 = 표시 이름(display_name ?? cwd basename) — 라우팅·표시·발신자명 단일화 (ADR-0087 이름주소 step 1)

- 상태: 확정 (2026-07-23, 근거: 사용자 결정 + 실코드 스코프 조사)
- 관련: ADR-0087(send-message 시맨틱·이름 규칙 — 이 ADR이 그 "사람이 읽는 이름 주소"를 구체화하는 step 1) · ADR-0061(display_name 트리 rename) · `crates/engram-dashboard-core/src/agent/manager.rs::agent_info` · `crates/engram-dashboard-daemon/src/control/ingress.rs::resolve_recipient`·`sender_display_name` · `src/util/basename.ts`

## 맥락
에이전트 간 메시지 발신(`send_message`/`engram-send`)에서 수신자 주소가 트리에 보이는 이름으로 안 먹혔다(실사용 관찰: ACB가 "DEF"로 보내니 "라이브 에이전트 없음", 로스터엔 cwd 경로 이름 2개). 원인 = **"보는 이름"과 "라우팅 이름"이 서로 다른 필드**:
- 라우팅(`resolve_recipient`)은 `AgentInfo.name` 정확일치로 찾는데, 이 값이 `agent_info`에서 `profile.name`으로 산출된다(`profile.name`은 CreateProfile 이름 또는 ad-hoc spawn의 cwd 문자열 — "깔끔한 표시명"이 아님).
- 프론트 트리는 `profile.name`을 무시하고 `display_name ?? basename(cwd)`(`AgentList.tsx` `displayNameOf`)로 그린다.

즉 사람이 본 이름과 데몬이 주소로 받는 이름이 갈려 있었고, 같은 cwd에서 뜬 두 에이전트는 둘 다 cwd 경로가 라우팅 이름이라 유일하게 지목조차 안 됐다.

## 결정
**canonical 이름 = 표시 이름(`display_name` 있으면 그것, 없으면 `cwd` basename)** 하나로 통일하고, 이 값을 **라우팅·로스터·트리·봉투 발신자명이 전부 공유**한다.
- 백엔드 `agent_info`가 `AgentInfo.name`을 `profile.name` 대신 `display_name ?? cwd_basename(cwd)`로 산출한다(프론트 `basename.ts`와 동일 규칙을 Rust로 포팅 — 드라이브 루트·UNC·빈 경로 폴백 포함).
- `resolve_recipient`(=`a.name` 매칭)·에러 로스터·`DeliveryObservation.to_name`은 자동으로 이 새 값을 따른다(코드 무변경).
- `sender_display_name`(봉투 발신자명)도 같은 표시-이름 산출로 정렬한다 — 수신자가 보는 발신자 이름도 사람 이름이어야 일관되기 때문(서브결정, 메인 판단·보고).
- **id는 fallback으로 유지**(`resolve_recipient`가 id 정확일치를 이름보다 먼저 시도 — 이름이 남의 UUID인 척하는 가로채기 차단, ADR-0087).
- `profile.name`은 스폰 시드/영속 필드로 강등(라우팅 키에서 물러남). 스키마·프로토콜(`AgentInfo` 형식)·프론트 코드는 무변경.
- **이번 범위 밖(후속 = "②"):** 동명 유일성 강제(자동 suffix `-2`/`-3`·가동 중 번호 재사용 금지·이름을 AgentId에 묶어 epoch 유지). ADR-0087이 이미 "미구현"으로 남긴 부분 — 이 ADR은 그중 "표시 이름으로 주소가 먹히게"만 먼저 잡는다.

## 거부한 대안
- **agent id(UUID)로만 주소** — 버림. 사람/LLM이 로스터를 보고 바로 못 쓰고 id를 먼저 조회해야 함(마찰). ADR-0087도 "OSS에서 UUID 주소 채택 0건"으로 사람 이름 주소를 의도로 박음. id는 없애지 않고 disambiguation fallback으로만 유지.
- **`profile.name`을 라우팅 키로 유지 + 발신자에게 "cwd 경로로 주소해라" 교육** — 버림. 사람이 보는 이름과 달라 직관 위반이고, 같은 cwd 두 에이전트는 라우팅 이름이 동일해 유일 지목 자체가 불가.
- **프론트에서만 고치기(표시를 profile.name에 맞춤)** — 버림. 라우팅은 백엔드라 프론트만 고치면 "보이는 이름=주소" 불성립. 반대로 백엔드 canonical을 표시 이름에 맞추는 게 옳은 방향.

## 근거
- 사용자 결정(2026-07-23): 주소 = 사람이 보는 이름(display), ①(표시 이름 주소) 먼저·②(유일성 기계) 나중.
- 스코프 조사(실코드): 변경점이 `agent_info` 단일 산출 지점 + basename 헬퍼로 국소(S). 라우팅·로스터·트리는 그 값을 따라 자동 수렴, 프로토콜·프론트·기존 테스트 무변경(매칭 로직 불변, 값만 바뀜) 확인.

## 영향 / 불변식
- **WYSIWYA 불변식:** 로스터/트리에 표시되는 에이전트 이름 문자열 = `send_message`/`engram-send`의 `to`가 매칭하는 문자열이어야 한다. 이 둘이 다시 갈리면(예: 프론트가 독자 산출로 회귀, 또는 라우팅이 `profile.name`으로 복귀) 이 버그가 재발한다. 단일 출처 = `agent_info`의 canonical 이름 산출.
- **id-우선 매칭 유지:** `resolve_recipient`는 id 정확일치를 이름보다 먼저(사칭 차단, ADR-0087) — 이 순서를 뒤집지 말 것.
- **유일성 미보장(현 범위):** 동명 라이브 에이전트가 둘이면 여전히 `RECIPIENT_AMBIGUOUS`가 난다(②가 자동 suffix로 해소할 때까지). 이건 의도된 잔여 — 조용히 아무에게나 배달하지 않는 게 안전측.
- **검증 게이트:** 두 에이전트를 서로 다른 표시 이름으로 띄우고 트리에 보이는 이름으로 `send_message` → 실제 배달되는지 GUI/실동작 실측이 완료 기준(코드 테스트만으론 미완).
