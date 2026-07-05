# ADR-0049: JSON 에이전트 thinking 기본 활성화 — MAX_THINKING_TOKENS 백엔드 주입

- 상태: 확정 (2026-07-06, 근거: CLI 스파이크 실측 + 프로세스 env 실측 + 라이브 ThinkingRow 실측)
- 관련: ADR-0004(백엔드 지식 격리) · ADR-0045(디코더 — thinking→Structured 매핑; stream-json 기원은 ADR-0044) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(`// ADR-0049` 앵커) · step-log 2026-07-06 섹션(Cline 시각 충실도 + thinking 활성화)

## 맥락
thinking 표시 파이프는 이미 완성돼 있었다 — 디코더가 `"type":"thinking"` 블록을 `Structured{kind:"thinking"}`으로 매핑하고(ADR-0045 — ADR-0044는 텍스트 MVP, thinking 매핑은 0045의 디코더 백엔드 이동분), 프론트 ThinkingRow가 접힘 행으로 렌더한다(ADR-0048 dispatch). 그런데 실사용에서 thinking이 한 번도 표시되지 않았다. 원인: claude CLI는 headless(stream-json) 모드에서 **extended thinking을 기본으로 켜지 않는다** — 스폰 시 env `MAX_THINKING_TOKENS`가 있어야 thinking 블록을 방출한다(실측: claude 2.1.170, 2026-07-06). 대시보드는 스폰 시 이 env를 주지 않아 블록 자체가 안 왔다.

## 결정
`backend/claude.rs`의 `build_spec`이 **json(stream-json) 모드에 한해** env `MAX_THINKING_TOKENS=8000`을 기본 주입한다.
- **프로필 우선(explicit-skip):** 프로필 env에 같은 키가 이미 있으면 주입하지 않는다 — 병합 순서(last-wins)에 기대지 않는 결정적 방식.
- **대소문자 무시:** Windows 환경변수는 대소문자 무구분 — `eq_ignore_ascii_case`로 비교해 소문자 프로필 키도 중복 주입을 막는다(cross-family 리뷰 지적 반영).
- **터미널/대화형 모드는 주입하지 않는다** — 맨 CLI와 동작 동일(parity) 유지.
- 프로필 env 내부의 중복 키 정규화는 범위 밖 — 모든 키 공통의 표준 last-wins 의미론을 따른다.

## 거부한 대안
- **프로필 데이터(agents.json)에만 env 추가** — 코드 무변경이지만 에이전트마다 수동 반복해야 하고 데몬 재시작이 필요하며, 새 JSON 에이전트가 기본으로 thinking을 못 받는다. 표시 파이프가 이미 있는데 기본이 꺼진 채 출고되는 불일치가 지속된다.
- **CLI 플래그 방식** — claude 2.1.170에 thinking 활성화 전용 플래그가 없다(실측 시점). env가 유일하게 확인된 메커니즘.
- **터미널 모드까지 주입** — 대화형 claude는 자체 thinking 토글(Tab)·설정을 가진다. 대시보드가 끼어들면 맨 CLI와 동작이 갈려 혼란(parity 위반).

## 근거
- **스파이크(2026-07-06, claude 2.1.170):** env 없으면 thinking 블록 0건, `MAX_THINKING_TOKENS=8000`이면 `"type":"thinking"` 블록 방출.
- **프로세스 실측:** 데몬이 스폰한 claude 자식 프로세스 env에 키 존재 확인(QA full).
- **라이브 실측:** sonnet-4-6 에이전트로 ThinkingRow 접힘/펼침·평문 추론 렌더 확인(`qa-thinking-sonnet.png`).
- **알려진 한계(업스트림):** opus-4-8은 headless stream-json에서 thinking을 **암호화(signature)로만** 방출 — 평문이 비어 UI가 설계대로 빈 행을 억제하므로 화면에 안 보인다. `--include-partial-messages`의 thinking_delta도 평문이 빈 것을 실측 확인 — 우리 쪽 코드로 해결 불가. 평문을 주는 모델(sonnet 등)에서만 가시화된다.

## 영향 / 불변식
- claude 지식(env 키·주입 조건)은 `backend/claude.rs`에만 존재한다(ADR-0004 격리 유지 — manager/transport로 새지 않는다).
- 회귀 가드 테스트 4개: 기본 주입 / 프로필 우선(정확히 1개 유효값) / 소문자 프로필 키 무중복 / 터미널 모드 미주입.
- 기본값 8000은 예산 상한이지 강제 사용량이 아니다 — 모델이 필요 시까지만 쓴다. 변경 시 이 ADR과 테스트를 함께 갱신한다.
