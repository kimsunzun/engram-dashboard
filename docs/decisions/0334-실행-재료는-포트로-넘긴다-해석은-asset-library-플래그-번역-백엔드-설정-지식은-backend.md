# ADR-0334: 실행 재료는 포트로 넘긴다 — 해석은 asset-library, 플래그 번역 · 백엔드 설정 지식은 backend

- 상태: 제안 (2026-10-31, 근거: 메인 결정 M2 · M16 · M18(보고 — 사용자 이의 가능) · M63 · M72(재리뷰 r7 · r8 — 보고) · TRD 리뷰 #2 · #3 · CLAUDE.md 「백엔드 확장」)
- 관련: ADR-0004(백엔드 지식 격리) · ADR-0311 · ADR-0327 · ADR-0316 · S25-K(초안) · TRD §1-4 포트 표 · §1-5 · §7 3

## 맥락
에이전트를 띄울 때 붙은 프리셋을 펼쳐 지침 · 스킬 · 설정을 CLI 플래그로 바꿔 싣는다. 펼치기 · 플래그 · 백엔드마다 다른 설정 형식을 누가 아나.

## 결정
1. 실행 재료 포트 `LaunchKitSource`(`check` · `prepare` · `release`)는 agent 가 정의하고 asset-library 서비스가 직접 구현한다. manager 가 부른다.
2. 플래그 번역은 backend `materialize`(`backend/claude/launch.rs` · `backend/codex/launch.rs` — 플래그 지식은 이 둘뿐).
3. 백엔드마다 다른 설정 지식(스키마 출처 · CLI 판 확인 · 원문 → JSON · claude 합침 · 모르는 키 처리 · codex 스킬 형식 · 명령줄 가드)은 agent `backend` 의 설정 검사 포트 하나에 모은다(M18). 내려받기는 데몬이 꽂는 fetch 포트로.
4. `check` 에 실행 출처 인자가 없다(M16 — 다음 주제가 더한다) · `prepare` 에 priming 인자가 없다(ADR-0327).
5. **codex 스킬 앞머리의 YAML 은 설정 검사 포트가 읽지 않는다** — YAML 을 아는 자리는 asset-library `format.rs` 하나이고, 부르는 쪽(`validate.rs`)이 그 파싱 함수를 포트에 클로저로 넘긴다. 수선(글 수준 · codex 지식)은 codex backend 에 있고 수선한 글로 같은 파싱 함수를 다시 부른다. agent 는 YAML 의존을 갖지 않는다(메인 2026-10-31 — M63).
6. 그 파싱 함수는 앞머리를 **agent 가 정의한 serde 타입**으로 역직렬화한다 — 형이 안 맞는 값도 파싱 실패로 셈해져 codex 처럼 수선을 부른다. 형은 agent 가 갖고 `format.rs` 는 serde-saphyr 를 돌릴 뿐이다(메인 2026-10-31 — M72).

## 거부한 대안
- **manager 가 프리셋을 해석** — 근거(메인 2026-10-31 — 보고): 편집 칸의 충돌 줄 · 문제 목록 · 트리 ⛔ · 실행 막힘 사유 · 실행 재료를 asset-library 의 해석 함수 하나가 모두 만든다 — 화면과 실행이 다른 규칙으로 갈리지 않게(TRD §1-3 「같은 함수가 … 모두를 만든다」 문단). manager 가 실행 몫을 따로 해석하면 해석이 둘이 된다. 또 agent 는 asset-library 를 모르고 포트만 정의한다(ADR-0311 결정 2 — 사용자 G16 의 crate 분리) — manager 가 프리셋 형식을 알면 그 지식이 agent 로 새어 crate 판정의 조건이 무너진다(판정표 — ADR-0311).
- **backend 가 라이브러리를 읽음** — 근거(메인 2026-10-31 — 보고): 라이브러리를 읽고 쓰는 것은 데몬 안 asset-library 하나다(ADR-0314 결정 2 · TRD §7 1) · backend 는 agent crate 안이라 asset-library 를 모른다(ADR-0311 결정 2). CLAUDE.md 「백엔드 확장」 이 backend 에 모으는 것은 백엔드마다 다른 지식이고 라이브러리 읽기는 백엔드와 무관하다 — backend 에 두면 같은 읽기가 claude · codex 에 둘로 선다.
- **`ControlEndpoint` 에 칸 덧붙이기** — 근거(메인 2026-10-31 — 보고): `ControlEndpoint` 는 제어 채널과 함께만 실리는 것의 descriptor 다 — `Option<ControlEndpoint>` 가 「제어 채널이 있나」 를 타입으로 답하고(ADR-0099 · `crates/engram-dashboard-agent/src/types.rs` 의 그 구조체 주석) 엔드포인트가 없으면 거기 실린 것도 실리지 않는다(ADR-0217 결정 5). 실행 재료는 제어 채널 유무와 무관하게 실리고 수명도 따로다 — `LaunchKitGuard` 는 `ProvisionGuard` 와 독립이고 재료 descriptor 는 `ControlEndpoint` 와 같은 방식의 따로 된 인자다(TRD §1-5 (나) 5 · 7).
- **설정 검사 지식을 라이브러리(`validate.rs`)에** — TRD 리뷰 #2 · #3 이 걷어 backend 포트로 옮겼다(M2). 그대로면 crate 분리 판정의 조건이 서지 않는다(판정표 결론 — ADR-0311).
- **앞머리를 일반 YAML 나무로 받기** — 형 어긋남이 파싱 실패로 셈해지지 않아 수선이 불리지 않고, codex 가 싣는 스킬을 ⛔ 로 막는다(메인 2026-10-31 — M72 · TRD §1-5 설정 검사 포트).

## 근거
CLAUDE.md 「백엔드 확장」(백엔드 전용 코드는 한 곳 `backend` 에 모은다) · M18 · M63 · M72(재리뷰 r7 · r8 — 보고). 거부한 대안 셋의 근거 = 위(메인 2026-10-31 — TRD §1-3 · ADR-0311 결정 2 · ADR-0314 결정 2 · ADR-0099 · ADR-0217 결정 5).

## 영향 / 불변식
- 플래그 · 백엔드 설정 형식 지식이 asset-library 나 manager 로 새면 위반이다. 새 백엔드는 `materialize` 와 설정 검사 포트 구현을 늘린다.
