# AI 에이전트 과잉 주석 — 원인·실무 제약·자동 검사 (research medium)

**상태:** 완료 · grounding(메인) + cross-family 적대 리뷰(Codex, effort high, web_search) 반영 · 2026-08-01
**목적:** 주석 기준 개정의 근거 기반. 개정 결과는 `~/.claude/skills/clean-comment/references/`(정본).
**확신도:** 확실(독립 교차확증) / 가능성 높음(grounding 지지·단일 출처) / 불확실(보류)
**적대 리뷰 판정: BLOCK → 12건 전건 반영 후 아래가 정정본이다.** 초안의 중심 부정 주장 3개가 틀렸다.

---

## ★결론부터 — 근거로 쓸 수 있는 것과 없는 것★

**쓰면 안 되는 것:** "AI 가 사람보다 주석을 많이 단다"는 **정설이 아니다.**
- 통제 비교 연구가 **존재하며**(초안은 "없다"고 단언 — 오류), 그 결과는 오히려 반대 방향이다:
  AICodeDetect 는 Java/Python/OCaml 약 24,000 샘플로 comment density 를 비교해 **human Python 이
  더 높고 AI 샘플은 일관되게 낮다**고 보고한다. [PDF](https://svijayakumar2.github.io/aicodegen.pdf) ·
  [Springer](https://link.springer.com/chapter/10.1007/978-3-032-20362-5_10) — *메인 grounding: PDF
  본문 추출 실패로 직접 확인 못 함 → **불확실**로 유지. 단 "없다"는 초안 주장은 폐기.*
- arXiv 2506.06069 부록도 human APPS 해답 vs Claude 3 Haiku/CodeLlama/GPT-3.5 의 comment-to-code 비율
  분포를 직접 비교한다. [arXiv](https://arxiv.org/abs/2506.06069) — 가능성 높음.
- 떠도는 "인간 18.42% vs AI 100%" 통계 → **폐기.** 원문(ResearchGate 388091783) 403 으로 접근 불가,
  같은 주제 검색은 반대 서술("주석 문서화는 인간 코드가 더 포괄적")을 낸다.
- arXiv 2508.21634(대규모 human vs AI) — 직접 확인: **주석을 아예 측정하지 않는다**(defects·
  vulnerabilities·complexity 만). 근거로 못 쓴다. — 확실.
- **방향이 모델·프롬프트마다 갈리고, 이 연구들은 벤치마크 스니펫 1회 생성이라 "지시 없이 실 코드베이스를
  여러 턴 편집하는 에이전트"와 세팅이 다르다.**

**쓸 수 있는 것(좁힌 명제):** **"Claude 계열 코딩 에이전트가 무지시 상태에서 중복 주석을 과잉 생산"**
- `anthropics/claude-code#65961` — "[MODEL] Claude verbose code comments by default — ignores
  instructions to stop". **open · 담당자 없음 · 메인테이너 응답 없음**(2026-08-01 직접 확인, Codex 재확인).
  본문: *"adds far too many code comments by default … mostly redundant, restating what the adjacent
  code already makes obvious"* / *"A clear, mandatory rule in `CLAUDE.md` does not reliably suppress
  it."* / *"Reinforcing the rule via the memory system does not stop it either."* — 확실(인용 검증됨).
  **★한계(적대 리뷰 적출)★:** 이는 **한 사용자·한 환경(Opus 4.8)의 일화적 보고**다. 다중 모델 실험·
  표본·대조군·메인테이너 확인이 없다. **전 에이전트의 행동 속성으로 일반화하지 말 것.**
- 중복 이슈 `#61305` — 리포터가 "Default to writing zero comments…" 라는 더 강한 CLAUDE.md 문구로도
  실패했다고 보고. **★한계★:** 이는 *규칙 내용의 부실*이 아니라 *지시 준수 실패*를 보인다. "더 구체적인
  금지 목록이 준수도를 높인다"는 **검증되지 않은 가설**이다(적대 리뷰 적출 — 초안의 논리 공백).
- **각 프로젝트의 자체 실측** — 이 repo 표본: 인터페이스 파일 주석 −58.5%, 이미 스윕한 파일 −20% 추가,
  반면 바이트 레이아웃 파일 −6.9%(기준이 분별함을 보이는 대조).

---

## 1. 원인 — 근거 있는 건 하나뿐

- **[가능성 높음] RLHF verbosity bias** — arXiv 2310.10076 *Verbosity Bias in Preference Labeling by
  LLMs*: GPT-4 가 사람보다 긴 답변을 더 선호함을 정량 제시. **단 일반 응답 길이 연구이고 코드 주석
  대상이 아니다** — 전이는 유추. [arXiv](https://arxiv.org/abs/2310.10076)
- **[불확실 — 초안에서 강등]** arXiv 2402.13013 *Code Needs Comments*: 주석 증강 **학습 데이터**가
  HumanEval/MBPP 성능을 올린다. **적대 리뷰 적출:** 이 논문은 결과 모델이 *생성 시 주석을 더 뱉는지*
  측정하지 않는다. 학습 데이터 밀도 → 출력 장황함의 인과는 성립 안 함. [arXiv](https://arxiv.org/abs/2402.13013)
- **[기권]** 훈련데이터 튜토리얼 편중 · chain-of-thought 누출 · 불확실성 헤징 · 지시 문자주의 —
  검증 연구 없음. CoT 누출은 실무자 관찰 1건(#65961 댓글: 주석이 채팅 히스토리·구현 "phase" 를 참조).

## 2. 실무 룰셋 — "why not what" 은 사실상 표준

- **[확실] 수렴:** GitHub 공식 `github/awesome-copilot` 의
  [`self-explanatory-code-commenting.instructions.md`](https://github.com/github/awesome-copilot/blob/main/instructions/self-explanatory-code-commenting.instructions.md)
  — *"Write code that speaks for itself. Comment only when necessary to explain WHY, not WHAT."*
  **금지 6종 명시: Obvious / Redundant / Outdated / Dead Code / Changelog / Divider.**
  Changelog 항목 = *"Skip historical notes like 'Modified by John on 2023-01-15'"* (Codex 재확인 완료).
  허용 예외: 복잡한 비즈니스 로직 · 알고리즘 선택 이유 · 정규식 · API 제약 · 공개 API 문서 · 설정 근거.
- **[가능성 높음]** 더 강한 극단도 쓰인다 — Boris Cherny(Claude Code 원저자) *"Avoid code comments
  unless your are explicitly asked to add comments"*(트윗 — WebFetch 402, 스니펫만 확인 → **verbatim
  미검증**). SamHatoum CLAUDE.md *"Do not add comments."*
- **[정정됨]** 초안의 "최소 4개 독립 출처 수렴 — 확실"은 **과청구**였다(적대 리뷰): 실제 제시된 건 3개고
  하나는 "gist 계열"이라 출처로 특정되지 않았다 → **가능성 높음**으로 강등.
- **[정정됨]** 초안 "벤더 공식 문서엔 주석 규칙이 없다 — 확실" → **범위 한정:** Anthropic
  `code.claude.com/docs/en/best-practices` 와 `openai/codex` 의 AGENTS.md **두 건을 확인한 결과 없음**.
  벤더 전체에 대한 보편 부정은 성립 안 하고, 같은 보고서의 GitHub(`awesome-copilot`) 사례가 반례다.
- **수집 중 환각 2건 자체 적발:** ① "Anthropic 공식 문서가 '명백한 코드 설명 금지'라고 말한다"는
  검색 요약 → 원문 전문 대조 결과 **그 문구 없음** ② "설계 결정은 PR 설명으로 옮기라"는 규칙 →
  후보 출처 2곳 직접 대조 결과 **실재하지 않음**. 둘 다 채택하지 않았다.

## 3. 자동 검사 — 성숙한 "저가치 주석" 게이트는 없지만, **작동하는 좁은 게이트는 있다**

**[확실 — 초안 정정] 실사용 가능한 도구가 존재한다:**
- **`comment-checker`** — **Claude Code `PostToolUse` 훅.** Go + **tree-sitter AST 파싱**(휴리스틱·LLM
  아님), `Write`/`Edit`/`MultiEdit` 마다 검사, 허용 패턴(린터 지시·shebang·BDD 마커) 밖이면 **exit 2**
  로 반려 → Claude 가 다음 턴에 스스로 고친다. 30+ 언어. npm/brew/go 설치.
  [pkg.go.dev](https://pkg.go.dev/github.com/code-yeongyu/go-claude-code-comment-checker)
  **우리 용도엔 그대로 못 쓴다** — 허용 목록이 무뎌 load-bearing WHY 까지 쳐낸다.
- **Dart Sentinel** — `redundant_comment` 규칙 + IDE 진단 + CI JSON/ratchet. [pub.dev](https://pub.dev/packages/dart_sentinel)
- **CRAIC** — 메서드 주석의 **의미적 중복**을 점수화(존재 여부가 아니라). [arXiv](https://arxiv.org/abs/1806.04616)

**[확실] 그러나 범용·검증된 의미 게이트는 없다:**
- 존재 여부만 세는 커버리지 도구(`interrogate`·`docstr-coverage`)는 `def f(): """f"""` 도 통과.
- 저가치 주석 분류기 정확도 미달 — arXiv 2504.18956: Random Forest **69%** / GPT-4 34%→증강 후 **55%**.
- 코드-주석 불일치(stale) 탐지는 F1 88~90(C4RLLaMA ICSE'25 · CCISolver 89.54)이나 **전부 연구 프로토타입**,
  CI 배포 사례 미확인.
- **[정정됨]** PMD `CommentSize` 를 Checkstyle 이 제외한 사유는 **"too much false positives" 가 아니다**
  (적대 리뷰 적출 — 그 문구는 다른 규칙 `DataflowAnomalyAnalysis` 의 것). 실제 사유 = **Checkstyle 이
  클래스 주석을 xdoc 원본으로 써서 주석이 의도적으로 크기 때문.** *단 "줄 수로 자르면 정당하게 큰
  주석이 걸린다"는 교훈 자체는 이 실제 사유가 오히려 더 잘 뒷받침한다.*
- Rust Clippy 주석-처리-코드 lint 는 2016 이슈 #1348 이래 미해결(메인테이너: "코드인지 판별하려면
  사실상 파싱해야 한다") — **메인 검증 실패, Codex 도 직접 확인 못 함 → 불확실.**
- comment-to-code 비율 = vanity metric 비판이 널리 퍼져 있고 SonarQube 도 기본 quality gate 조건으로
  흔히 쓰지 않는다 — 가능성 높음.
- LLM 리뷰봇의 오탐 누적 → 개발자가 알림을 무시/suppress → 워크플로 신뢰 붕괴 패턴이 여러 산업 소스에서
  공통 서술 — 가능성 높음.

---

## 4. 이 근거가 지지하는 결론 (개정 반영분)

1. **정당화 근거를 좁혀라.** "AI 는 주석을 많이 단다"를 기준 문서에 쓰지 않는다. 쓸 수 있는 건
   ① Claude 계열 무지시 기본 동작에 대한 **미해결·일화적 보고** ② **자체 실측** 뿐이다.
2. **"why not what" 만으로는 차별화가 안 된다** — 이미 표준이고 그걸 쓴 사람들도 실패를 보고한다.
   차별화는 **금지 유형의 구체성**에 있다. **단 그 가설은 미검증이다**(적대 리뷰 적출).
3. **기계 강제는 "판단이 필요 없는 것"에만.** 위치·분량(비공개 함수의 doc 존재 · 정의별 doc>코드 비율 ·
   주석 속 `파일:라인`)은 tree-sitter 로 객관 판정된다. **의미 판정을 기계에 시키면 죽는다** —
   Clippy 10년 미해결·분류기 69%·오탐 누적에 의한 신뢰 붕괴가 그 증거다.
4. **사후 적대 리뷰는 유지한다.** 이 프로젝트 실적: 라운드마다 FIX 적출(7건·6건) — 다만
   **"기계 증명이 못 잡는 부류였다"는 주장은 비교 프로토콜 없이 성립하지 않는다**(적대 리뷰 적출).
   정직한 서술 = *돌린 기계 게이트(주석 전용 diff 증명·보호 토큰 수지·build/test/fmt/격리)는 전부
   통과한 상태에서 리뷰가 결함을 냈다.*

## 5. 쟁점 / 한계

- AICodeDetect 원문 PDF 미독(추출 실패) → 그 수치·방향은 **불확실**로 남긴다. 결론이 이 논문에
  의존하지 않도록 §결론을 "정설 아님"까지만 주장하게 조정했다.
- Boris Cherny 인용 verbatim 미검증(402). SamHatoum 규칙의 출처 맥락 미검증.
- **"라인번호·심볼명을 주석에 박지 마라"는 AI 지시서 코퍼스에 선례 미발견** — 사람 대상 OSS 관행으로만
  존재(Linux LKML·PostgreSQL 메일링리스트의 file-path stale 논의). Codex 도 근접 사례만 찾았다
  (Coolify 가 "brittle line number references" 를 교체한 기록 — 단 그게 소스 주석 안이었는지 미확정).
  → **이 항목은 실측 기반 신설이다.**
- "심의 전사 금지" 는 **선례 있음**(에이전트용 가이드가 "긴 아키텍처 설명은 소스가 아니라 ADR 로" 명시)
  — 초안의 "원본 기여" 주장은 적대 리뷰가 반증해 철회했다.
- HN 스레드(id=49078710) 429 로 접근 실패. Anthropic 의 이 행동에 대한 공식 언급 자료 없음.
- Google/Ousterhout/Kernighan 원문 verbatim 미확보(2차 요약만).

## 6. 방법 메모

주계열 수집 = 3갈래 병렬(조사 수집자 · 주도 경량). grounding = 메인 외부 검증(load-bearing 클레임 전수 —
`#65961` 인용 대조, ResearchGate 통계 반증, arXiv 2508.21634 직접 확인). 적대 리뷰 = cross-family
(비주도 상급 · effort high · web_search) 1회, **레벨 2~3**, 판정 BLOCK · 12 findings.
**적대 리뷰가 초안의 중심 부정 주장 3개(통제 연구 부재 · 작동 게이트 부재 · 심의 전사 규칙 무선례)를
전부 반증했다** — cross-family 를 뺐다면 이 보고서는 확신에 찬 오류 3건을 그대로 기준에 실었을 것이다.
