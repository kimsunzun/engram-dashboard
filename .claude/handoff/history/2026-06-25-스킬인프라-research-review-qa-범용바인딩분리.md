# 핸드오프 — 스킬 인프라 구축(research·review·qa) + 범용/바인딩 2층 분리

작성 2026-06-25 (세션 "dashboard-research1"). 본문(`docs/`·`CLAUDE.md`·각 `SKILL.md`)이 항상 우선. **이 세션 = 스킬 5커밋 + 주석 컨벤션 적용 + tp/tr 삭제.** master HEAD = `bd59f2b`.

## 0. 한 줄 요약
개발 스텝(PRD→TRD→코드→검증)을 굴릴 스킬 셋을 `research` 검증 방식(리서치→설계→2자 적대리뷰→정련)으로 구축. `review`·`qa` 신규 + 셋 다 **강도** 보유 + **범용 골격 + `bindings/engram.md` 2층 분리**. 주석 컨벤션도 문서로 적용(ADR-0032). **다음 세션 0순위 = 사용자가 "qa·review 관련 재논의"를 예고함(주제 미상 — 먼저 물어볼 것).**

## 1. git 상태 (이 세션 커밋, 전부 master 로컬·push 안 함)
- `6af6e37` research 스킬 정련 — light=**mini-crosscheck**(Claude1+Codex1, 적대검증만 생략)로 재정의, tier 활용 명문화, 자기보고/학습노트 섹션.
- `1570198` 주석 컨벤션 적용(문서만, 코드 0줄) — **ADR-0032**(선택지 B) + 살아있는 캐논 `docs/reference/commenting-conventions.md`(첫 reference 입주, **위치 잠정**) + CLAUDE.md ##컨벤션 라우터화 + docs/README 온보딩 라우터.
- `bc71979` review 스킬 신규 — 2축 = 단계(prd/trd/code/doc)×강도(self/light/full/deep). dogfood 검증.
- `318762c` qa 스킬 신규 — 강도 quick/standard/full. dogfood 검증. CLAUDE.md 빌드명령 보강(`cargo fmt --check` + 프론트 게이트).
- `bd59f2b` (HEAD) qa·review **범용 골격 + engram 바인딩 2층 분리**.

**미커밋(이전 세션 산출, 무관):** `docs/research/documentation-architecture-research-2026-06-24.md`, `multi-agent-hosting-orchestration-research-2026-06-22.md`. (code-commenting research는 1570198에 포함 커밋됨.)

**글로벌 별건:** `tp`/`tr` 스킬 삭제(다른 설치에 묻어온 AutoFlow) — core(`claude-global-shared`)+user 양쪽 제거. **core repo 커밋은 사용자 몫**(무관 변경 섞여 있어 AI가 안 함).

## 2. 핵심 결론 (확정 — 재론 금지)
- **스킬 구축 방식:** research에서 검증된 흐름 = 방법론 리서치(이미 한 건 재활용, 매번 X) → 설계안(사용자 확인) → 코더(opus) → **opus+Codex 2자 적대 리뷰(dogfood)** → 정련 → 커밋. **모든 스킬은 강도(intensity)를 가진다. research=선행, review·qa=후행.**
- **2층 구조(불변):** 스킬 = **범용 골격**(`SKILL.md`+`references/flow.md`, 스택 흔적 0) + **프로젝트 바인딩**(`references/bindings/<project>.md`). 다른 프로젝트는 바인딩 파일만 추가하면 골격 재사용. CLAUDE.md 1번 원칙(추상 위 swappable). 바인딩은 정본을 베끼지 않고 포인터(qa→CLAUDE.md 빌드명령 절 / review→코드 `// ADR-` 앵커).
- **기존 스킬 처분:** consult(ADR-0031로 폐기됨, 파일 잔재 **삭제 예정**)·all-plan(웹 rubric 구버전, **폐기 판단**)·prior-art(웹 구버전, **재작성**). 전부 웹 GPT·Gemini·Opus 3종 기준이라 못 씀. research/review/qa만 신뢰.
- **통짜 오케스트레이터 안 만듦** — tp/tr 전철. PRD/TRD 스텝은 기존 스킬 조합 + CLAUDE.md 규약. review의 단계 인자(prd/trd)가 "PRD/TRD 리뷰"를 흡수(별도 스킬 X).
- **research vs review:** research=바깥 지식 *조사*(수렴, 보고서) / review=우리 산출물 *검증*(대척 Advocate vs Adversary, PASS/FIX/BLOCK). 메커니즘만 "2 family 교차" 공유, 방향 반대 → 별개 스킬.
- **스킬 위치:** 프로젝트 `.claude/skills/`(repo 커밋=engram 팀 공유). 범용이어도 검증 전까진 프로젝트, 다른 프로젝트서 쓸 때 user 글로벌 승격.
- **검증 상태(정직):** 스킬들이 단일 모델 대비 실제로 나은지는 **대조검증 전 = "근거 있는 가설"**. dogfood 리뷰는 "review 절차를 수동 opus+Codex 스폰"으로 한 것(스킬 정식 호출 아님).

## 3. 즉시 다음 작업 (우선순위)
1. **★ qa·review 재논의 ★** — 사용자가 "qa review 스킬 관련해서도 다시 얘기해야 될 게 있다"고 명시. **주제 미상** — 다음 세션 시작 시 사용자에게 무엇을 재논의할지 먼저 물을 것. (현 두 스킬을 직접 열어 보고 컨텍스트 잡은 뒤 질문 권장.)
2. **로드맵 잔여 스킬:** `adr`(강추 — 번호/템플릿/인덱스 양방향/supersede 자동, 이 세션 ADR-0032 수작업 + 인덱스 rot가 동기) → `prior-art 재작성` → `wrap`(세션 종료 체크리스트 자동) → `consult 삭제` → `all-plan 폐기 판단`. 전부 강도 + 범용/바인딩 2층으로.
3. **프론트 본작업(프로젝트 메인 라인, 별개):** step-log "다음" 참조 — D-7 레이아웃 영속·§5 LLM 제어표면. review/qa를 실전 도입(`/review code`·`/qa`)해 dogfood 아닌 실사용 검증.

## 4. 미결정 / 주의
- **review/qa 글로벌 승격 시점** — 현재 프로젝트. 다른 프로젝트서 쓰려면 user `~/.claude/skills/`로(검증 후).
- **주석 캐논 위치 잠정** — `docs/reference/`는 잠정(ADR-0032 명시). adr 등 문서 프로세스 정립 시 재조정.
- **범용/바인딩 분리 후 풀 재리뷰 안 함** — grep engram-free 검증만(flow.md 스택 키워드 0 확인). 새 세션이 bindings 정합·내용 유실 점검 가능.
- **문서 아키텍처 권고 A~D**(`documentation-architecture-research`) 미적용 — CLAUDE.md 슬리밍·handbook·문서 CI. 미커밋 상태.
- 스킬 dogfood는 "근거 있는 가설" 단계 — 실사용 대조 전.

## 5. 종료 체크리스트
1. 새 ADR → **ADR-0032**(주석 컨벤션) 작성·인덱스 갱신 완료. 스킬 인프라 결정은 step-log에 기록(스킬 내부 결정이라 ADR 미작성 — adr 스킬 생기면 재고).
2. 폐기 ADR → 없음.
3. `docs/decisions/README.md` 인덱스 → ADR-0032 등재 완료.
4. `docs/process/step-log.md` → 이 세션 5개 엔트리(research 정련·주석 컨벤션·review·qa·범용바인딩 분리) 추가 완료.
