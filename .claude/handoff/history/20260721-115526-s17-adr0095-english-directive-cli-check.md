# 핸드오프: S17 봉투 확정(ADR-0095) + 라우팅 v3 — 구현 슬라이스 대기(갈림길 2 + 영어화 확정 지시 + CLI 검증 잔여)

> 직전 save(20260721-114735) 대비 델타: ① 에이전트 대면 텍스트 **영어화 = 사용자 확정 지시**로 승격(백로그 아님, 슬라이스 포함) ② CLI 입구 검증 = 사용자 재확인 요청으로 우선순위 승격 ③ 사용자 혼선 최종 해소 문답 추가. 나머지 맥락은 동일하되 이 파일이 최신 정본.

## 한 줄 상태 · 다음 첫 액션
- **상태:** 발신 라우팅 신뢰화 완료(프라이밍 v3 실증) + 봉투 포맷 **사용자 확정 = ADR-0095**(스위칭 구조·기본 colon·대체 xml·bracket 기각, sonnet 스트레스 colon 6/6=xml 6/6 동률로 조건 충족). 전부 커밋·워킹트리 클린(`.vs/`·`.codex/`만 노이즈). **이번 세션 커밋 9개+ 미push.**
- **다음 첫 액션:** 사용자에게 갈림길 2개 확인 → `/implement standard` 착수.
  1. **xml 스위치 위치**: 데몬 전역 설정 하나(추천) vs 전역+수신자 프로필별 오버라이드.
  2. **프라이밍 v3 정식 채택**: 실험 파일을 정본 `prompts/agent-priming.md`에 반영할지(추천 채택).
- **구현 슬라이스 범위(사용자 승인·확정 지시 포함):**
  - `wrap_message` 기본 템플릿 → colon `{sender}: {body}` + 운영 스위치(colon/xml — §5 LLM 제어 표면 고려). `ENGRAM_WRAP_FORMAT` env는 스파이크 전용 존치(ADR-0095 불변식).
  - **에이전트 대면 텍스트 영어화(사용자 확정 지시 2026-07-21):** 프라이밍 md + 하네스 시드(`roundtrip_smoke.rs`의 TASK_PROMPT_B/SEED_A_TO_B) 영어화. 봉투 양식 자체는 이미 영어/언어중립. **영어판으로 v3 라우팅 재검증 필수**(현 수치는 한국어 대화 기준). 언어 온오프 토글은 후속.
  - **CLI 입구 클린 검증(사용자 재확인 요청):** MCP는 실측 완료(이번 라우팅 성공 전부 entrance=mcp), CLI는 grant confound로 단독 격리 미실측. 하네스에 스폰 인자 주입 노브(예: `--disallowedTools mcp__engram__send_message`) 추가 → CLI-only 재실측(B의 CLI 사용 + Bash 풀경로 패턴 동시 확인).
  - 게이트: 코더→`/review code full`→`/qa standard`.

## 이번 세션 커밋 (전부 로컬 — 미push)
`7940a41` 포맷 스파이크 2차 → `b388a33` 오버헤드 정정 → `e8c989c` 라우팅 리서치+실측 v1→v3 → `41f3947` haiku 정정 → `c750e9c` 언어 백로그 → `347cae8` 리치-메타 스파이크 → `d6af20c` **ADR-0095** → `997afc8` 핸드오프 1차 → (이 save + step-log 승격 커밋).

## 핵심 결과 (수치·근거 정본 = ADR-0095 + `docs/research/agent-send-routing-reliability-2026-07-21.md`)
- **라우팅 해결 = 프라이밍 v3**(출력 불가시성 + 원칙자 앵커 + 사전승인 귀속, 메타주석 0): sonnet 4/4·opus 2/2·haiku 4/5 (baseline haiku 2/9·sonnet 5/6). 수용(파싱)은 전 포맷 0 실패.
- **조직 보안 지침 상속**: 스폰 에이전트가 계정 조직 지침 상속, sonnet 최엄격. 플래그 유발 3종 = "인젝션 아니다" 항변·봉투 내 지시문·프라이밍 메타주석(전부 로그 원문 확인). 해결 = 원칙자 앵커 정렬.
- **리치-메타**: 발신자 오인 0(전 포맷). haiku만 colon/bracket 0/4·xml 2/2 → xml이 스위치 대상인 근거.

## 다음 세션 커뮤니케이션 주의 (사용자 혼선 반복 — 이 언어로 고정)
- **"에이전트가 xml을 만드나?" → 절대 아니오.** 발신 에이전트는 어느 모드든 send_message(받는사람, 본문)만 호출 — xml 문자를 한 글자도 안 씀, 스위치 존재도 모름. **xml/colon을 만드는 주체 = 데몬 wrap_message(Rust) 단 한 곳, 배달 직전 1회.** xml 존재 구간 = 데몬→수신자 화면 사이뿐.
- "중간 변환·캐치해서 바꿈" 표현 금지 — 변환기 아님, 조립 시점 템플릿 선택.
- 사용자 정확 멘탈모델(이 문장 기준으로 설명): "보낼 땐 의사코드(툴 호출), 데몬이 조립."

## 검증 상태 (쌍)
- **한 것:** 실측 스파이크 총 55런+ 전 VALID(포맷 2차 21 · 라우팅 v1/v2/v3 21 · 리치-메타 12+8). /research medium + codex 적대 리뷰(BLOCK→반영). ADR lint error 0. **코드 무변경 — build/test 게이트 해당 없음**(docs+prompts만).
- **재실행:** `cargo build -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` → `ENGRAM_WRAP_FORMAT='{sender}: {body}' target/debug/roundtrip-smoke.exe --priming prompts/experiments/agent-priming-routing-v3.md --model sonnet`.
- **안 한 것(정직):** ① 구현 미착수(기본 템플릿 아직 bracket+uuid) ② 소표본(셀당 n=1~6) ③ **영어판 재검증**(전 실측 한국어 기준 — 이번 슬라이스에서) ④ **CLI-only 격리**(하네스 노브 필요 — 이번 슬라이스에서) ⑤ 데몬 detect-and-nudge 백스톱(O3) 미구현 ⑥ 포화 등 비정상 미착수.

## do-not (누적)
- **프라이밍 파일에 메타/실험 주석 금지**(파일 전체가 시스템프롬프트 주입 — v2 오염 사고) · "인젝션 아니다" 항변 금지 · **봉투 본문 행동 지시문 금지**(Anthropic 반증+실측 차단) · 권위는 원칙자 귀속.
- 봉투 조립 = wrap_message 단일 지점(ADR-0086/0095). `ENGRAM_WRAP_FORMAT` 운영 전용 금지.
- 루트 bare `cargo test` 금지(`-p`) · 릴리즈 `--all-targets` 금지 · `--allowedTools` 그룹 args 맨 끝 · 발신 권한 확대 금지(ADR-0094).
- 회수 프로브 인젝션-모양 금지 · happy-path 전 비정상 착수 금지.

## 정지 조건
- **갈림길 2개(스위치 위치·v3 채택) = 사용자 결정** — 답 받은 뒤에만 구현 진입.
- 리뷰어 정면 대립(FIX vs BLOCK) = 사용자 에스컬레이션.

## 참조
- **ADR-0095** · ADR-0092/0093/0094 · 보고서 `docs/research/agent-send-routing-reliability-2026-07-21.md`.
- `prompts/experiments/agent-priming-routing-v3.md`(채택 후보 — 주석 없음 유지) · `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message`(~:550) · `crates/engram-dashboard-daemon/src/bin/roundtrip_smoke.rs`(시드 상수·spawn_named — 영어화·CLI 노브 대상).
- step-log 최하단 S17 항목들 + "다음 (미진행)" 최상단(영어화·CLI 항목).
