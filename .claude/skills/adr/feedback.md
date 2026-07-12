# adr — 개선 히스토리

이 스킬을 쓰다 발견한 결함·개선점을 누적한다(덮어쓰기 금지). 반영은 사용자 승인 하에. 규약 = `SKILL.md` "자기개선 피드백" 절.

출처: `/review trd full` dogfood(opus Architect-breaker, 2026-06-26) — 실데이터 반례 0016·0024 대조.

## 이력

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-06-26 | **부분 폐기(supersede-in-part) 미지원** — 실데이터 0016/0024는 상태 "확정" 유지 + 단서절. supersede가 통째 "폐기"로 덮어 살아있는 조항까지 죽일 위험. | 미반영 |
| 2026-06-26 | **lint 상태 비교가 복합 상태 문자열에서 거짓양성.** "상태 어휘(확정/제안/폐기/거부)만 비교, 단서절 자유서술 무시" 기준 필요. | 미반영 |
| 2026-06-26 | "상태 정본=본문 헤더"가 부분 폐기에선 깨짐 — 0024는 인덱스가 본문 상태줄보다 정보가 많음. | 미반영 |
| 2026-06-26 | **채번 race/번호 빠짐 무방비.** 단일 오케스트레이터 가정 명시 + lint가 중복 번호를 backstop으로 검출하게. | 미반영 |
| 2026-06-26 | 고아 앵커 `rg "ADR-"`가 `docs/`까지 긁어 거짓양성. `// ADR-` + 코드 경로(`crates/ src/`) 한정, `docs/` 제외 필요. | 미반영 |
| 2026-06-26 | 본문 H1 제목 ↔ 인덱스 제목 drift를 lint가 검사 안 함(0016에 실제 drift). | 미반영 |
| 2026-06-26 | **큰 방향:** adr을 하이브리드(결정적 스크립트 + 얇은 스킬)로 재설계 검토 중. 부분 폐기 형식은 OSS 리서치(adr-tools/MADR/log4brains) 대기 — 리서치 채번 갈래만 회수, 나머지 중단 상태. | 미반영 |
| 2026-06-26 | **하이브리드 재작성 완료** — `scripts/adr.mjs`(결정적: 채번·스캐폴드·supersede 양방향·index 재생성·lint) + 스킬 3파일 얇게. 위 6개 구멍 전부 닫힘(부분폐기→partial Amends / lint거짓양성→상태어휘만 / 채번→재스캔+lint중복 / rg노이즈→코드경로한정 / 제목drift→index재생성+보존 / 상태정본→index가 본문서 파생). 실데이터 32 ADR 무손실 검증(index --write idempotent). **남은 긴장(메인 결정 대기):** ① 레거시 단방향 0023→0026·0027→0029는 자연어("본 ADR이 대체")라 lint이 양방향 누락으로 잡음 — 정형 키워드로 마이그레이션할지 lint을 자연어 허용으로 완화할지 결정 필요. ② 0016/0024 레거시 부분폐기 본문엔 Amends 링크가 없어 인덱스 단서가 본문보다 정보 많음 — supersede --mode partial 재적용으로 본문 양방향 박을지 보존 유지할지. ③ 인덱스 제목 16건이 본문 H1보다 풍부한 큐레이션 — "본문=단일 출처"를 신규에만 적용(보존+경고)할지 레거시도 본문으로 통일할지. | 검토 대기 |
| 2026-07-03 | **검증 상태** (SKILL.md ⚠️절에서 이동 — 방침 C). *보장하는 것 = 기록 정합성*(채번 충돌 없음 · supersede 양방향 일치 · 인덱스↔헤더 일치 · 템플릿 형식 준수) — 스크립트가 결정적으로 검사·강제한다. **이 정합성 보장은 `adr.mjs` 정확성에 전적으로 의존하며, 실데이터 32 ADR 무손실 + `index --write` idempotent 검증이 그 근거**(위 2026-06-26 하이브리드 재작성 항목). *보장하지 않는 것 = 결정의 옳음* — 빈약하거나 틀린 결정도 형식만 맞으면 기록은 PASS한다. 결정 타당성은 호출자·review(적대 검증)·앞단(prd/trd)의 몫이다. *LLM 환각*(본문에 없는 API·가짜 근거·지어낸 대안)은 검증된 실패 모드 — 거부한 대안·근거는 사용자 제공분만 박제하고 채운 본문은 호출자가 fact-check한다. | 기록 (검증 상태 정본) |
| 2026-07-06 | **공용화 개조: 스크립트를 스킬 내장(`scripts/adr.mjs`)으로 이동 + 플래그 파라미터화**(--root/--dir/--index-name/--status-vocab/--default-status/--template/--anchor-roots — 기본값 = dashboard 무플래그 동일 동작, 바이트 동일 회귀 검증). 소비처 2곳: dashboard(기본값) + skill-factory(경량 형식 — `bindings/skill-factory.md` 신설, 결합 메타줄 파싱 지원, full supersede는 결합줄에서 안전 거부·수동 안내). 팩토리 인덱스(doc/decisions/README.md) 신규 생성·멱등 확인. 잔여 후보: 경량 형식 supersede 자동화 · --amend-only(1신→다구 부분개정). | 기록 (개조) |
| 2026-07-07 | **바인딩 이식 개조 파일럿 (ADR-0004 규약)** — 바인딩을 스킬 폴더(`references/bindings/<project>.md`) → 소비처 프로젝트 트리(`<project>/.claude/skill-bindings/adr.md`, cwd-상대 Read)로 이관. factory 바인딩을 `.claude/skill-bindings/adr.md`로 옮기고(절대경로 `I:/…/doc/decisions` → 상대 `doc/decisions`), flow.md "바인딩 1회 선언" 블록을 cwd-상대 Read 지시로 개정(작업본 = `staging/adr-binding-rework/references/flow.md`). **dogfood PASS:** fresh Sonnet가 개정 flow만 보고 cwd-상대로 바인딩 로드 → lint 명령·플래그 자력 추출 → 실행 → 정합(4건, 0/0). **적출 갭(미반영):** flow.md가 `<skill>`(스킬 자기 폴더 경로) 해석을 미명세 — global 심링크+다중 사본 상황에서 근거 부족. 실 Claude Code 호출은 하네스가 스킬 base dir 제공(부분적 dogfood 아티팩트)이나 flow에 "`<skill>` = 호출 시 제공된 스킬 base dir" 한 줄 명시 권장. **미완(배포 전):** ① 개정 flow를 done/·live·dashboard로 반영 ② dashboard 바인딩도 프로젝트 트리로 이관 ③ 구 `references/bindings/` 제거는 global 이사 시. 배포 시점 = 사용자. | 검토 대기 (파일럿 통과·미배포) |
| 2026-07-07 | **문구 담백화(사용자 지시):** flow §역할 분담 말미의 SKILL.md §불변 중복 문장 삭제 · SKILL.md 오퍼레이션 절의 헤더 재서술 문장 삭제. 의미 불변. | 반영 (2026-07-07) |
| 2026-07-07 | **최종 보고 피드백 한 줄 의무(사용자 결정):** flow 최종/결과 보고 절에 "피드백: 없음"도 보고하는 한 줄 의무 추가(파일엔 발견 시만 — 조용한 스킵 관측). 규약 정본 = _shared/self-improvement-feedback.md. 게이트 = review doc full(Opus PASS · Codex FIX 반영: 축약 + "최종 보고" 통일) + qa 등가 실행 PASS. | 반영 (2026-07-07) |
| 2026-07-07 | **4렌즈 감사 기각:** "base dir 미제공 시 전 오퍼레이션 차단"(fresh 감사 HIGH 주장) — Claude Code 하네스가 스킬 호출 시 "Base directory for this skill"을 항상 제공하므로 실위험 아님. 하네스 밖 실행은 비지원 환경. | 기각 (하네스 보장) |
| 2026-07-08 | **스크립트 날짜 = UTC 결함:** `adr.mjs`가 메타줄 날짜를 UTC로 찍음 — KST 새벽(예: 07-08 02:42)에 `new` 실행 시 07-07로 하루 어긋남(ADR-0007 채번 시 실측, prose 단계에서 수동 정정). 수정안 = 로컬 날짜 사용 또는 `--date` 플래그. | 미반영 |
| 2026-07-11 | **부분 폐기 마커가 폐기당한 ADR의 상태줄에 안 박힘** — `supersede --mode partial`이 옛 ADR에 `Amended by`(관련줄) + 인덱스 단서만 넣고 **상태줄은 "확정" 그대로**. CLAUDE.md rot-방지 규칙("폐기는 폐기당한 ADR의 `상태:` 줄에 박는다")이 상태줄을 지목하는데 스크립트는 관련줄에만 → 본문 헤더만 읽는 세션이 폐기된 조항을 살아있는 줄 안다. 실측 = ADR-0066 부분폐기 by 0067(doc 리뷰 FIX-1 적출, 상태줄 수동 보강). 선례 0007/0064도 index-only라 systemic. 수정안 = partial이 상태줄에도 `· 부분 폐기 by ADR-N (조항)` 스탬프(단 index --write 재파생과 이중마킹 안 나게) 또는 CLAUDE.md 규칙을 "관련줄+index 허용"으로 완화. | 미반영 |
