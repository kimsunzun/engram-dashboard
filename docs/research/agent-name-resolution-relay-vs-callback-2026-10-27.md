# 에이전트 이름 → id 해석 위치 — 중계자(데몬)에서 vs 처리기(셸)가 되묻기 — 2026-10-27

- **상태:** light 요약 · 결정 대기(사용자) — S28 T-65 의 이름 풀기 경로.
- **방법:** `/research` light · 서브클래스 engram · 외부 수집자 1 명 + 우리 코드 손익 조사 1 명 · 메인 grounding(Fowler · Kubernetes 원문 확인). 적대 리뷰 없음(light).
- **두 안:**
  - **(A) 되묻기(지금 구현)** — 셸이 명령을 처리하다 데몬에 `agent.resolve` 를 한 번 묻고 받은 id 로 적용한다. 데몬은 셸 명령 인자를 안 연다.
  - **(B) 중계자 변환(사용자 제안)** — 셸이 명령을 등록할 때 「이 인자는 에이전트 이름」이라고 표시하고, 데몬이 넘기기 전에 그 인자만 기계적으로 id 로 바꿔 넘긴다. 셸은 id 만 받는다.

## 외부 선례

| # | 클레임 | 출처 | grounding | 확신도 |
|---|---|---|---|---|
| 1 | 「smart endpoints and dumb pipes」 — 중계(파이프)에 로직을 두지 말라(A 쪽). 같은 글이 잘게 쪼갠 원격 호출을 굵게 바꾸라고 한다 — 「Remote calls are more expensive than in-process calls, and thus remote APIs need to be coarser-grained」(A 의 추가 왕복에 반대). | [Fowler, Microservices](https://martinfowler.com/articles/microservices.html) | 지지(메인 원문 확인) | 가능성 높음 |
| 2 | Kubernetes 는 저장 전에 API 서버가 변형(mutating admission)을 해 컨트롤러가 정규화된 객체를 받는다(B 쪽). 변형 웹훅 권고 = 멱등 · 부작용 피하기 · 호출 순서에 기대지 않기 · 의존 순환 피하기. | [Kubernetes admission webhook good practices](https://kubernetes.io/docs/concepts/cluster-administration/admission-webhooks-good-practices/) | 권고 문구 지지(메인 확인) · 「B 가 낫다」는 해석 | 가능성 높음(사실) · 추론(적용) |
| 3 | API 게이트웨이 · BFF 는 경계에서 인증 문맥을 풀어 사용자 id 를 주입하고 하위는 그것만 믿는다(B 쪽 정황 — 이름 해석이 아니라 인증 문맥이라 거리가 있다). | [Zuplo — API gateway patterns](https://zuplo.com/learning-center/api-gateway-patterns)(검색 요약) | 수집자 요약 | 불확실 |
| 4 | GraphQL federation 의 router 가 엔티티 참조를 풀어 하위에 보낸다(하위가 되묻지 않는다). | 수집자 사전지식 · 출처 미확인 | — | 불확실 |

**경향(추론):** 경계에서 한 번 풀고 id 만 내려보내는 쪽(B)이 더 관례적이다 — 순환 의존(데몬 → 셸 → 데몬)과 추가 왕복이 없고, 해석 규칙이 명부 주인 한 곳에 모인다. 단 **중계자가 그 인자의 뜻을 몰라야** 「dumb pipe」 원칙을 덜 어긴다 — 범용 표시(「에이전트 이름 인자」)만 보고 치환할 때.

## 우리 코드에서의 손익(코드 실측)

- **기존 결정과 정면으로 부딪힌다** — ADR-0081(「데몬은 payload 를 파싱하지 않는다」 · 「UI payload 를 파싱하지 않는다」) · ADR-0156(「데몬은 저장·중계만 하고 파싱·검증·분기를 하지 않는다」 · 「데몬 불투명 유지 = 하드 제약」). (B) 는 새 ADR 로 그 조항을 개정해야 한다.
- **중계 지점은 한 곳** — `command_delivery::deliver`(CLI · MCP · WS 모두 지남). 거기서 인자는 JSON 값으로 있다. 우회 경로 없음(사람 UI 는 id 를 쓰는 invoke).
- **표시를 실을 길이 지금 없다** — 선언 매크로는 필드에 `#[doc]` 만 받고 스키마에 싣지 않는다. 데몬은 스키마를 불투명 문자열로만 쥔다. 필요 = 새 인자 타입(예: `AgentRef`) + 등록 패킷에 「에이전트 인자 이름 목록」 칸 + 바인딩 · 골든 · 판 올림.
- **옛 데몬 위험** — 새 셸 + 옛 데몬이면 이름이 그대로 넘어온다. 셸이 「이 칸은 반드시 UUID」를 검사해 막지 않으면 이름이 에이전트 id 로 말없이 적힌다 — (B) 에서도 셸 쪽 UUID 검사(실패 시 「데몬을 다시 띄우라」)가 필요하다.
- **명부 읽기가 길게 막힐 수 있다** — 잠든 프로필의 경로 정규화가 수십 초 걸릴 수 있다는 문서(`manager.rs`) → 중계 경로에서 `spawn_blocking` 필요.
- **오류 문구가 데몬으로 옮겨 간다** — `tab_of 'x': … candidates` 같은 인자 이름 머리를 데몬이 붙인다(의미 결합이 조금 생긴다).
- **규모(추정)** — 8~10 파일 · 더하기 ≈ 500 · 빼기 ≈ 900(대부분 셸 시험). 셸의 되묻기 · 시한 · 대기 슬롯 정리 코드가 빠진다(정리 코드 자체는 범용 위생이라 남겨도 된다). `agent.resolve` 명령은 CLI 편의로 남겨도 된다.
- **TOCTOU** — 두 안 모두 이름 → id 는 셸 적용 잠금 밖, id → 칸은 안. (B) 가 한 홉 더 넓을 뿐 실질 차이 없음(추론).

## 공백

- 「중계자가 하위 명령의 특정 인자를 치환해 넘긴다」의 정확한 선례(예: Envoy 필터 인자 치환)는 못 찾았다.
- tmux · GraphQL federation 1차 문서 미확인.
