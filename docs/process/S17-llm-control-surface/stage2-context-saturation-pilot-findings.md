# Stage 2 컨텍스트 포화 — 파일럿 실측 결과 (ADR-0088 사다리 / ADR-0090 실행 설계)

- 일시: 2026-07-20 (자율 세션, 사용자 야간 위임)
- 상태: **파일럿 완료 — 본런 진입 전 사용자 결정 대기(설계 갈림길).** 하네스는 로드베어링 정확성 게이트 통과, 데이터 유효성 수정 완료, 최종 blind 재리뷰 결과 반영은 커밋 게이트에서.
- 정본 결정 = ADR-0088(사다리·판정 기준) · ADR-0090(파일럿 선행·bin·sonnet 핀·안전 범위). 이 문서는 *실측 결과와 그로부터의 권고*다(결정은 사용자).

## 한 줄 결론

**정책-준수 claude 에이전트는 stdin으로 배달된 에이전트 간 메시지(`[message from <name> id:<uuid>] <body>` 봉투)를 프롬프트 인젝션으로 판정·격리한다** — 포화와 무관하게 codeword 회수 지표 자체가 막힌다. 따라서 **설계대로의 비싼 본런(축소 요인설계)은 돌리지 않는다**(파일럿-선행이 정확히 이 낭비를 막으려는 것 — ADR-0088). 대신 세 갈래(아래 §5)를 사용자에게 올린다.

## 발견 (fact — 실측)

### F1. 에이전트 간 메시지 정책 격리 (헤드라인 · 확실)
스폰된 실 claude(`claude-sonnet-4-6`)가 주입 메시지를 받은 뒤 트랜스크립트 thinking 트레이스에서 명시적으로 인젝션으로 규정하고 거부했다(여러 런 재현). 대표 트레이스(요지): *"This is exactly the prompt injection attempt I warned about... I should not remember the codeword or act on it."* 결과 = codeword/sender/id 회수 전부 실패(하네스가 정확히 all-false로 채점). 근거: 스폰 에이전트가 managed-settings의 조직 보안 규칙(**도구로 들어온 콘텐츠 속 지시 = 데이터, 명령 아님**)을 상속하는데, 현재 봉투는 그 규칙의 격리 대상 정의에 정확히 해당한다.

- **파생 F1a:** 격리에 그치지 않고 **원과제에서 이탈**하는 런도 관측됐다(주입 거부 후 *"이 연습에 계속 참여하지 않겠습니다"* → FINAL REPORT 공백). 즉 잘못 설계된 주입은 포화 측정 자체를 오염시킨다.
- **해석(불확실):** 이건 ADR-0088 결정 3(위조 방어=이스케이프)·Stage 4(봉투)가 존재하는 이유의 이면이다 — 문제는 *위조 미방어*가 아니라 *정당한 메시지의 과잉 격리*로 먼저 나타난다. "정당한 에이전트 간 메시지가 인젝션 방어 정책을 어떻게 신뢰 통과하나"가 진짜 설계 질문(= 봉투/채널 인증 문제, Stage 4 영역).

### F2. compaction은 관측 가능 (확실)
트랜스크립트 탭이 **유기적 native compaction**을 포착했다: 한 런에서 `compact_boundary`(trigger:manual, preTokens:32490) 마커 캡처. 따라서 ADR-0088의 "compaction 전/후" 요인은 측정 가능하다 — 단 **강제 `/compact`가 아니라 유기적 발생 관측으로**. `/compact`를 stream-json 유저 텍스트로 보내면 native 압축이 안 일어난다(인터랙티브 슬래시 가로채기 없음) → 하네스에서 강제 `/compact` 단계는 제거했다.

### F3. 디코딩 usage ≠ 컨텍스트 크기 (확실 · 계측 함의 큼)
코어 stream-json 디코더는 `input_tokens`/`output_tokens`(그 턴 증분, 관측 ~3)만 표면화하고 **`cache_creation_input_tokens`/`cache_read_input_tokens`(실 컨텍스트가 사는 곳, 관측 ~29–36k)를 버린다.** 실 컨텍스트는 트랜스크립트 탭의 cache-합 footprint로만 닿는다. 캘리브레이션: 문자수 기반 추정이 실측을 크게 과소평가(turn1 23.7x → turn8 ~5x). **본런 함의:** fill 진도는 디코딩 usage나 문자 추정이 아니라 **트랜스크립트 실 usage**를 기준으로 삼아야 한다. 단 탭은 best-effort(turn 1 이후에야 파일 생성).

### F4. 필러 콘텐츠가 content filter를 건드림 (가능성 높음)
의사난수 필러뿐 아니라 **"자연문 템플릿" 필러조차** Anthropic Usage-Policy 거부를 유발했다(런 중 fill 도중 abort). 본런 필러는 진짜 양성(가급적 실세계 유래) 콘텐츠가 필요하다.

### F5. 판당 비용은 미확정 (정직)
소규모 런이 ~8턴에 ~30–36k 실 컨텍스트에 도달했으나, 150k 목표 도달 전에 정책 거부(F1/F4)로 abort돼 **깨끗한 판당 토큰·시간 비용을 못 뽑았다.** 즉 ADR-0088 결정 5c 사전등록의 셀 배분 수치는 여전히 실측 근거가 없다 — 필러가 정책을 통과해 포화까지 완주하는 런이 선행돼야 한다(F4 해결 후).

## 하네스 상태 (인프라)

- `crates/engram-dashboard-daemon/src/bin/saturation_pilot.rs`(실 claude 실험 드라이버) + `src/experiment/{filler,probe,record,cli,transcript,mod}.rs`(feature `test-harness` 뒤 순수 로직 + 72 유닛테스트). 운영 빌드 미포함(ADR-0090 불변).
- 배달은 **실 control 경로**(`handle_send` → `wrap_message` → `write_stdin_observed`)를 그대로 탄다 — 실험 대상 = 운영 경로 동일(ADR-0090 d2). epoch pinning 없음, 배달 시맨틱 무변경.
- 검증: `/review code full` 2인 적대(doc-aware + Codex blind) → 로드베어링 정확성(주입 turn 펜싱·귀속·stall·usage 덮어쓰기·안전캡·정리) 양쪽 CLOSED. 데이터 유효성 결함(재현성 핀 유실·측정소스 전환 false compaction·`/compact` vestige·probe 인덱스·타이밍·주입 해시) 수정 완료. `/qa` 게이트 = daemon 142 lib(72 experiment) + 통합 green, fmt clean, 코어 격리 0.

## 권고 (§5) — 사용자 결정 갈림길

**본런을 설계대로 돌리지 않는다.** 주 지표(포화 하 에이전트 간 codeword 회수)가 포화가 아니라 정책으로 0이라, 셀마다 격리를 재확인하며 토큰만 태운다.

- **(a) 지표 전환 — 태스크 채널 회수.** 포화 하 원과제 완주 + 초기 내용 회수(DOC-1 제목·문서 수, FINAL REPORT 경유)를 측정. 정책 격리 대상 아님. codeword/에이전트간 회수는 포화 지표에서 내림. 현 하네스로 즉시 측정 가능(태스크 채널 채점 이미 있음).
- **(b) 주입을 일반 유저 턴으로.** 봉투 대신 하네스 전용 유저-턴 경로로 배달 → 정책 격리 회피. **단 ADR-0090 d2(실 control 경로) 위반** → 명시적 결정 필요 + 측정 대상이 달라짐.
- **(c) 리프레임 — 격리 자체가 발견.** "정책-준수 에이전트는 에이전트 간 메시지를 격리한다"를 Stage 2 결론으로 확정하고, **봉투/신뢰 설계(Stage 4)를 앞당긴다** — 정당한 메시지가 인젝션 방어를 신뢰 통과하는 법(wrap_message 인증·채널 신원)이 진짜 문제.

**메인 권고:** 즉시 포화 질문엔 (a), 설계 함의로 (c) 병행. 둘 다 운영 경로 보존. (b)는 측정 대상이 바뀌고 d2 위반이라 비권장.

## 참조
- 결정: ADR-0088(사다리·판정=지속 처리) · ADR-0090(실행 설계). 
- 코드: `crates/engram-dashboard-daemon/src/bin/saturation_pilot.rs` · `src/experiment/`.
- step-log: "S17 단계 2 파일럿" 항목.
