# 핸드오프: S17 봉투 포맷 확정(ADR-0095·colon 기본) + 라우팅 v3 실증 — 구현 슬라이스 착수 대기(갈림길 2개 미답)

## 한 줄 상태 · 다음 첫 액션
- **상태:** 발신 라우팅 신뢰화 완료(프라이밍 v3 실증) + 봉투 포맷 **사용자 확정 = ADR-0095**(스위칭 구조·기본 colon·대체 xml·bracket 기각) + 리치-메타 스트레스 실측(sonnet colon 6/6 = xml 6/6 동률). 사용자가 **구현 슬라이스 진행을 승인**했으나("colon 기본으로 구현 슬라이스 진행해") **갈림길 2개가 미답인 채 핸드오프**됨. 전부 커밋됨·워킹트리 클린(`.vs/`만 IDE 노이즈). **이번 세션 커밋 7개 미push**.
- **다음 첫 액션:** 사용자에게 갈림길 2개 확인 → `/implement standard` 착수.
  1. **xml 스위치 위치**: 데몬 전역 설정 하나(추천 — 용법상 필요충분·확장 쉬움) vs 전역+수신자 프로필별 오버라이드.
  2. **프라이밍 v3 정식 채택**: `prompts/experiments/agent-priming-routing-v3.md`를 정본 `prompts/agent-priming.md`에 반영할지(추천 채택 — 라우팅 개선의 실체).
- **구현 슬라이스 범위(확정분):** `wrap_message` 기본 템플릿 bracket+uuid → colon `{sender}: {body}` 교체 + 운영 스위치(colon/xml 선택, §5 LLM 제어 표면 고려) + 단위테스트 갱신. `ENGRAM_WRAP_FORMAT` env는 스파이크 전용으로 존치(ADR-0095 불변식). 코더→`/review code full`→`/qa standard` 게이트.

## 이번 세션 커밋 (전부 로컬 — 미push)
`7940a41` 포맷 스파이크 2차 step-log → `b388a33` 오버헤드 정정 → `e8c989c` 라우팅 리서치+실측 v1→v3(보고서+프라이밍 3종) → `41f3947` haiku v3 실측 정정 → `c750e9c` 언어 백로그 → `347cae8` 리치-메타 스파이크 → `d6af20c` **ADR-0095**+인덱스+step-log.

## 핵심 결과 (자세한 수치·근거 = ADR-0095 + `docs/research/agent-send-routing-reliability-2026-07-21.md`)
- **라우팅 문제 해결 = 프라이밍 v3** (출력 불가시성 + 원칙자 앵커 + 사전승인 귀속, 메타주석 0): sonnet 4/4·opus 2/2·haiku 4/5 (baseline haiku 2/9·sonnet 5/6). 봉투 포맷은 수용에 무관(전 실측 0 파싱 실패).
- **조직 보안 지침 상속 발견**: 스폰된 에이전트가 이 계정의 조직 지침을 상속, sonnet이 가장 문자적 집행. "인젝션 아니다" 항변·봉투 내 지시문·프라이밍 파일 메타주석이 인젝션 플래그 유발(전부 로그 원문 확인). 해결 = 회피 아닌 정렬(원칙자 앵커).
- **리치-메타(그룹/cc/스레드 가짜 주입)**: 발신자 오인 0(전 포맷). haiku만 리치에서 colon/bracket 0/4·xml 2/2 → xml이 스위치 대상으로 남은 근거. sonnet 증량 6/6=6/6 동률 → colon 기본 확정 조건 충족.

## 다음 세션 커뮤니케이션 주의 (사용자 혼선 반복 지점 — 정확히 이 언어로)
- **발신 에이전트는 봉투를 모른다** — send_message(받는사람, 본문) 툴 호출뿐. 조립 주체 = **데몬 wrap_message 한 곳, 배달 직전 1회**. "중간 변환·에이전트 조립" 표현 금지(colon을 파싱해 xml로 바꾸는 변환기 아님 — 조립 시점 템플릿 선택).
- 사용자 정확한 멘탈모델: "보낼 땐 의사코드(툴 호출), 데몬이 조립". 이 문장 기준으로 설명.
- 미시 코드명 나열 금지·거시명(wrap_message는 예외적으로 사용자에게 이미 노출된 용어) 유지.

## 검증 상태 (쌍)
- **한 것:** 실측 스파이크 총 55런+ (포맷 2차 21 · 라우팅 v1/v2/v3 21 · 리치-메타 12+증량8 — 전 런 VALID·SETUP-FAIL 0). /research medium(수집 3·grounding 원문 2건·codex 적대 리뷰 BLOCK→반영). ADR lint 깨끗(error 0). **코드 변경 없음 — build/test 게이트 해당 없음**(docs+prompts만).
- **재실행:** `cargo build -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` → `ENGRAM_WRAP_FORMAT='{sender}: {body}' target/debug/roundtrip-smoke.exe --priming prompts/experiments/agent-priming-routing-v3.md --model sonnet`.
- **안 한 것(정직):** ① 구현 슬라이스 미착수(코드 무변경 — 기본 템플릿은 아직 bracket+uuid). ② 셀당 n=1~6 소표본 — 통계 보증 아님. ③ 전 실측 한국어 대화 기준 — 영어 전환(백로그) 후 재검증 필요. ④ 데몬 detect-and-nudge 백스톱(보고서 O3) 미구현. ⑤ 포화 등 비정상 미착수. ⑥ CLI-only 격리 재실측(이전 핸드오프 잔여) 미착수.

## do-not (누적 + 신규)
- **프라이밍 파일에 메타/실험 주석 금지** — 파일 전체가 시스템프롬프트로 주입(v2 오염 사고). "인젝션 아니다" 선제 항변 금지. **봉투 본문에 행동 지시문 금지**(Anthropic 반증+실측 차단). 권위는 원칙자(사용자) 귀속.
- 봉투 조립은 wrap_message 단일 지점 유지(ADR-0086/0095). `ENGRAM_WRAP_FORMAT`은 스파이크 전용 — 운영 스위치로 전용 금지.
- 루트 bare `cargo test` 금지(멤버별 `-p`). 릴리즈 `--all-targets` 금지. `--allowedTools` 그룹은 args 맨 끝 유지. 발신 권한 넓히지 말 것(ADR-0094 최소권한).
- 회수 프로브를 인젝션-모양으로 만들지 말 것. happy-path baseline 통과 전 비정상 본격 착수 금지.

## 정지 조건
- **갈림길 2개(스위치 위치·v3 채택) = 사용자 결정** — 임의 확정 금지. 답 받은 뒤에만 구현 진입.
- 에이전트 간 언어 영어화는 **사용자 지시로 백로그 등재됨**(step-log 다음 절) — 임의 착수 금지, 착수 시 colon 수치 재검증 포함.
- 리뷰어 정면 대립(FIX vs BLOCK)은 사용자 에스컬레이션.

## 참조
- **ADR-0095**(봉투 포맷 — 이 슬라이스의 정본) · ADR-0092/0093/0094(맥락).
- 보고서: `docs/research/agent-send-routing-reliability-2026-07-21.md`(리서치+실측 종합·옵션 O1~O5).
- 프라이밍: `prompts/experiments/agent-priming-routing-v3.md`(채택 후보 — **주석 없음 유지**) · 코드: `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message`(~:550).
- step-log 최하단 S17 항목 5개 + "다음 (미진행)" 최상단 언어 백로그.
