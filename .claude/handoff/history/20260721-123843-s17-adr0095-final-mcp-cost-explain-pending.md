# 핸드오프: S17 봉투 확정(ADR-0095)·라우팅 v3 — 구현 슬라이스 대기 + MCP 비용 설명 재시도 필요

> 직전 save(20260721-115526) 대비 델타: ① 세션 말미에 개념 Q&A 진행(툴 호출 메커니즘·colon/xml 전체 플로우·전송 포장 주체·MCP 호환성·커스텀 툴 등록) — 아래 "커뮤니케이션" 절에 요지 ② **MCP 토큰 비용 질문 미해결** — 사용자가 설명을 이해 못 한 채 세션 종료("아 이해안돼"). 다음 세션 첫 대화 후보. ③ 새 결정·코드 변경 없음. 이 파일이 최신 정본.

## 한 줄 상태 · 다음 첫 액션
- **상태:** 발신 라우팅 신뢰화 완료(프라이밍 v3 실증) + 봉투 포맷 **사용자 확정 = ADR-0095**(스위칭 구조·기본 colon·대체 xml·bracket 기각). 전부 커밋·워킹트리 클린(`.vs/`·`.codex/`만 노이즈). **이번 세션 커밋 ~11개 미push.**
- **다음 첫 액션(둘 중 사용자 흐름 따라):**
  - (a) **MCP 토큰 비용 재설명** — 사용자가 "send_message가 MCP라서 추가로 드는 비용"의 감을 못 잡음. 직전 시도: ①툴 정의 = 세션 1벌 상주(포스트잇 비유, 추정 200~400자·토큰 50~150, 캐시 얹힘) ②발신 1건당 툴 블록+ACK(문장 한두 개) ③수신 봉투(colon 글자 2개) — 이 3분류 자체가 안 꽂힘. **다른 각도 제안: 실측 스파이크로 숫자를 직접 보여주기**(stream-json usage 필드, MCP 유/무 스폰 대조 4런) — 개념 설명보다 실수치가 통할 사용자.
  - (b) 구현 슬라이스 착수 — 갈림길 2개 확인 먼저: ① xml 스위치 위치(데몬 전역 하나(추천) vs 전역+수신자 프로필별) ② 프라이밍 v3 정식 채택(추천 채택).
- **구현 슬라이스 범위(승인·확정 지시 포함):** `wrap_message` 기본 → colon + 운영 스위치(colon/xml, §5 LLM 제어 표면) · `ENGRAM_WRAP_FORMAT`은 스파이크 전용 존치(ADR-0095 불변식) · **에이전트 대면 텍스트 영어화**(프라이밍 md + 하네스 시드 — 사용자 확정 지시, 영어판 v3 라우팅 재검증 동반) · **CLI 클린 검증**(하네스에 MCP disallow 노브 추가 → CLI-only 재실측) · (옵션) MCP 토큰 측정 스파이크. 게이트: 코더→`/review code full`→`/qa standard`.

## 이번 세션 커밋 (전부 로컬 — 미push)
`7940a41` 포맷 스파이크 2차 → `b388a33` 오버헤드 정정 → `e8c989c` 라우팅 리서치+실측 v1→v3 → `41f3947` haiku 정정 → `c750e9c` 언어 백로그 → `347cae8` 리치-메타 스파이크 → `d6af20c` **ADR-0095** → `997afc8`·`d5739eb` 핸드오프 → (이 save).

## 핵심 결과 (수치·근거 정본 = ADR-0095 + `docs/research/agent-send-routing-reliability-2026-07-21.md`)
- **라우팅 해결 = 프라이밍 v3**(출력 불가시성 + 원칙자 앵커 + 사전승인 귀속, 메타주석 0): sonnet 4/4·opus 2/2·haiku 4/5 (baseline haiku 2/9·sonnet 5/6). 수용(파싱) = 전 포맷 0 실패.
- **조직 보안 지침 상속**: 스폰 에이전트가 계정 조직 지침 상속. 플래그 유발 3종 = "인젝션 아니다" 항변·봉투 내 지시문·프라이밍 메타주석. 해결 = 원칙자 앵커 정렬.
- **리치-메타**: 발신자 오인 0(전 포맷). haiku만 colon/bracket 0/4·xml 2/2 → xml이 스위치 대상 근거. sonnet 증량 colon 6/6 = xml 6/6 동률 → colon 기본 확정 조건 충족.

## 커뮤니케이션 노트 (사용자 이해 상태 — 이 언어로 이어갈 것)
- **정착된 멘탈모델(사용자 자기 언어):** "보낼 땐 의사코드(툴 호출), 데몬이 조립." 봉투 조립 주체 = 데몬 wrap_message 1회(배달 직전), 에이전트는 xml/colon을 만들지도 알지도 못함 — **"에이전트가 조립·중간 변환" 표현 절대 금지**(반복 혼선 지점이었음).
- **세션 말미에 설명 완료된 개념:** ① colon/xml 전체 플로우(포맷 분기 = wrap_message 한 곳뿐, 표로 정리해줌) ② 전송 포장(stream-json 유저턴 JSON 라인) = 우리 데몬 세션 인코더가 조립(claude 규격 준수), 출력 방향은 claude가 조립 ③ 툴 호출 = LLM이 이름·인자를 특수 토큰 경계 안에 직접 생성(치환도 산문 파싱도 아님 — haiku 실패가 정확히 "산문 채널에 말만 함") ④ CLI(engram-send)도 같은 툴 채널(Bash 툴 안 명령 문자열 — 한 겹 간접) ⑤ MCP는 개방 표준(OpenAI·Google 채택 — Anthropic 전용 아님), 진짜 리스크는 런타임별 grant 번역(ADR-0094 seam 대비)과 MCP 없는 로컬 런타임(CLI 폴백 존재 이유) ⑥ 커스텀 1급 툴 등록은 claude CLI엔 MCP뿐, "직접 등록+tool_choice 강제"는 API 직결(ApiTransport 껍데기) 층.
- **미해결:** MCP 토큰 비용 감(위 다음 첫 액션 (a)).

## 검증 상태 (쌍)
- **한 것:** 실측 스파이크 총 55런+ 전 VALID. /research medium + codex 적대 리뷰(BLOCK→반영). ADR lint error 0. **코드 무변경 — build/test 게이트 해당 없음**(docs+prompts만).
- **재실행:** `cargo build -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` → `ENGRAM_WRAP_FORMAT='{sender}: {body}' target/debug/roundtrip-smoke.exe --priming prompts/experiments/agent-priming-routing-v3.md --model sonnet`.
- **안 한 것:** ① 구현 미착수(기본 템플릿 아직 bracket+uuid) ② 소표본(셀당 n=1~6) ③ 영어판 재검증(전 실측 한국어 기준) ④ CLI-only 격리(하네스 노브 필요) ⑤ **MCP 토큰 실측**(usage 필드 대조 — 미실행) ⑥ 데몬 detect-and-nudge(O3) ⑦ 포화 등 비정상.

## do-not (누적)
- 프라이밍 파일 메타/실험 주석 금지(전문이 시스템프롬프트 주입) · "인젝션 아니다" 항변 금지 · 봉투 본문 행동 지시문 금지 · 권위는 원칙자 귀속.
- 봉투 조립 = wrap_message 단일 지점(ADR-0086/0095) · `ENGRAM_WRAP_FORMAT` 운영 전용 금지.
- 루트 bare `cargo test` 금지(`-p`) · 릴리즈 `--all-targets` 금지 · `--allowedTools` 그룹 args 맨 끝 · 발신 권한 확대 금지(ADR-0094).
- 회수 프로브 인젝션-모양 금지 · happy-path 전 비정상 착수 금지.

## 정지 조건
- **갈림길 2개(스위치 위치·v3 채택) = 사용자 결정** — 답 받은 뒤에만 구현 진입.
- 리뷰어 정면 대립(FIX vs BLOCK) = 사용자 에스컬레이션.

## 참조
- **ADR-0095** · ADR-0092/0093/0094 · `docs/research/agent-send-routing-reliability-2026-07-21.md`.
- `prompts/experiments/agent-priming-routing-v3.md`(채택 후보 — 주석 없음 유지) · `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message`(~:550) · `crates/engram-dashboard-daemon/src/control/mcp_server.rs`(SEND_MESSAGE_TOOL·SendArgs — MCP 툴 정의 실물) · `crates/engram-dashboard-daemon/src/bin/roundtrip_smoke.rs`(시드 상수·spawn_named — 영어화·CLI 노브 대상).
- step-log 최하단 S17 항목들 + "다음 (미진행)" 최상단(영어화·CLI 항목).
