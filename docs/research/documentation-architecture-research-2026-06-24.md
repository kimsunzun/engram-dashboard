# AI 시대 문서 아키텍처 리서치 (상세 기록)

> **상태:** 조사 완료(Claude 2트랙 + Codex 1 교차). engram 적용 **권고 제시 — 사용자 결정 대기**. 통합 가이드 작성은 별건.
> **단계:** 컨설/리서치 (조사 → 권고 → 사용자 결정). 구현 아님.
> **방법:** Claude(Sonnet) 2트랙 독립 BLIND 조사(① doc-type/IA/메타가이드 ② AI 효율) + Codex 1 독립 교차(전체) → opus 교차 대조·종합. (= `research` 스킬 패턴.)
> **날짜:** 2026-06-24
> **확신도 표기:** (확실) 다수 수렴·1차 출처 · (가능성 높음) 정황·단일 출처 · (불확실) 미확인.

---

## 0. 왜 (Context)

engram docs/는 여러 문서 종류(README/index · decisions=ADR · process=step-log · tracking.md · research/ · reference/ 예약)를 쓴다. 전체를 **업계 표준 + AI 시대 효율**에 맞추고 "우리 문서 시스템은 이렇다"는 통합 가이드를 만들 수 있는지 조사. (주석 컨벤션·ADR 배치는 별도 리서치 완료.)

---

## 1. doc-type 전체 지도 + 불변 vs 살아있음 (3트랙 수렴 · 확실)

| 타입 | 목적/청중 | 수명 | 배치 |
|---|---|---|---|
| README / index | "무엇·왜·시작법", 진입점 | 살아있음 | repo root + `docs/README.md` 포털 |
| **ADR** | 되돌리기 어려운 결정의 맥락·결정·결과 | **불변(append/supersede)** | `docs/decisions/NNNN-*.md` |
| Coding convention | 반복 코드 규칙·리뷰 기준 | 살아있음 | `docs/handbook/` 또는 `reference/style/` |
| Reference(정설) | API·설정·계약 | 살아있음(코드 동기화) | `docs/reference/`, 가능하면 코드/스키마서 생성 |
| Design doc | 구현 전 설계·트레이드오프 | 승인 시 snapshot | `docs/designs/` |
| Research/spike | 미확정 조사·실험 | freeze 후 archive | `docs/research/` (+ `status:` 표기) |
| Changelog | 변경 이력 | **append-only** | root `CHANGELOG.md`(또는 step-log) |
| Tracking | 미결정·TODO | 매우 살아있음 | issue tracker(SoR) 권장, docs엔 인덱스만 |
| Runbook | 운영·장애 절차 | 살아있음 | `docs/runbooks/` |
| Handbook | 팀 운영·문서 시스템 정의 | 살아있음 | `docs/handbook/` |

**경계 명문화(가능성 높음):** ADR·changelog=append / design=승인 snapshot / reference·runbook·convention·handbook·README=living. living은 git history가 이력(파일 내 변경이력 섹션 금지), 신선도는 `last-reviewed:`·`status:`로.

## 2. 정보 아키텍처 (확실)
- **Diátaxis**(tutorial/how-to/reference/explanation)를 기본 IA로. ADR·design·research·runbook은 "engineering artifact"로 별도 보강(Diátaxis에 안 깔끔히 안 맞음).
- **docs-as-code**: 문서도 VCS·PR 리뷰·CI(link/orphan/lint)·owner. GitLab은 청중별 최상위 분리(`doc/user`·`doc/development`).
- **SSoT**: "복사하지 말고 링크하라" — 중복=stale 비용.
- **고아 방지**: `docs/index` + 폴더 README + 링크그래프 + **orphan check를 빌드 실패로**(Read the Docs). engram "고아 금지" 규약과 동형.

## 3. 통합 가이드(handbook) 메타패턴 (확실)
- 성숙 조직은 **문서를 제품/코드처럼**(owner·source control·review·freshness). canonical source 지정(Google SWE Book).
- **단일 거대 파일 < 폴더+분할 마크다운 + landing index.** 목적 혼재한 monolithic wiki는 실패.
- 사례: **GitLab Handbook**(handbook-first, MR 기반 변경=process control plane, SSoT, CODEOWNERS) · Google **g3doc**(코드 옆 문서)+**eng-practices**(별도) · **Microsoft Engineering Playbook**(MkDocs 다중섹션 + "playbook을 알고·따르고·고친다" meta-rule).
- 가이드 자체 위치 권고: **`docs/handbook/documentation-system.md`** ("우리 문서 시스템은 이렇다"), `docs/README.md`·`AGENTS.md`에서 링크.

## 4. AI 시대 효율 (확실, 일부 단일출처)
- **에이전트 지침 파일:** `AGENTS.md`(agents용 README, Linux Foundation 표준화, 60k+ repos) · `CLAUDE.md`(org/user/project/local 계층, nearest-file-wins, 디렉터리 lazy-load).
- **always-load는 짧게:** Anthropic 권고 CLAUDE.md ≤200줄, 실무 체감 20~80줄. 80줄~ recall 저하, 긴 파일=context 잠식·준수율↓. "명령·금지·표준위치·테스트·링크"만, 상세는 **on-demand 링크/path-scoped rule**. (가능성 높음 — vendor 문서+실측 보고)
- **auto-gen 역효과(가능성 높음):** LLM이 생성한 AGENTS.md는 성공률 -0.5~2%·비용 +20%(ETH Zurich arXiv 2602.11988), 상세할수록 탐색 단계↑. **수작업 최소 분량**이 최선.
- **soft vs hard guardrail 짝:** 문서(soft, 의도·이유) + lint/typecheck/test/CI/hooks/CODEOWNERS(hard, 강제). memory는 context지 enforcement 아님(Claude 문서 명시). 불변식은 반드시 hard로도.
- **spec-driven:** 요구→설계→task→구현이 agent 추측을 줄임(2025~2026 흐름, 가능성 높음).
- **llms.txt:** 공개 docs/website엔 유효하나 **내부 코드베이스엔 AGENTS.md+docs index가 더 직접적**. 주요 LLM 공식 지원 없음.

## 5. 교차검증표 (Claude ↔ Codex)

| 항목 | 수렴 | 발산/보완 |
|---|---|---|
| doc-type 지도·불변/living | 완전 일치 | Codex가 CHANGELOG(Keep a Changelog)·"tracking=issue tracker SoR" 추가 |
| Diátaxis·docs-as-code·SSoT·고아방지 | 일치 | Codex가 orphan=빌드실패(Read the Docs) 추가 |
| 메타가이드(handbook) 패턴 | 일치(GitLab/Google) | Codex가 MS Engineering Playbook·`handbook/documentation-system.md` 위치 추가 |
| AI 효율(always-load 짧게·soft/hard·llms.txt) | 강하게 일치 | Claude=ETH auto-gen 역효과·200/500 임계 / Codex=20~80줄 실무치 |

**결론: 두 family 실질 충돌 0(수렴).** 차이는 전부 한쪽이 더 길게 다룬 보완 → landscape 신뢰도 높음(확실). 단 "auto-gen 역효과 수치"·"80줄 recall 저하"는 단일출처라 (가능성 높음).

## 6. engram 적용 권고 (현 시스템 대비)

engram 현 docs/ = README(포털)·decisions/(ADR)·process/(step-log)·tracking.md·research/·reference/(예약 비어있음). **이미 표준 구조에 ~정렬.** 갭·권고:

- **(권고 A) CLAUDE.md 슬리밍 검토** — 현재 **~12k 토큰(≈300+줄)** 으로 always-load 권고(≤200, 실무 20~80)를 크게 초과. 항상 로딩되는 비용↑. 상세(아키텍처 불변식 본문·모듈맵·세션복원 등)는 on-demand 문서로 빼고 CLAUDE.md는 **얇은 라우터**(명령·금지·표준위치·핵심 불변식 한 줄 + 링크)로. ※ 큰 변경이라 별도 결정.
- **(권고 B) 통합 문서 가이드 신설** — "engram 문서 시스템은 이렇다"가 지금 CLAUDE.md+docs/README.md에 흩어짐. `docs/handbook/documentation-system.md`(또는 docs/README.md 확장)로 doc-type 배치·불변/living·always-load vs on-demand·고아금지를 한 곳에. (이번 리서치 결과가 그 초안 재료.)
- **(권고 C) reference/ 채우기** — 예약만 된 정설 폴더를 살아있는 캐논(주석 컨벤션 등)으로 첫 입주.
- **(권고 D) 문서 CI 최소셋** — markdown lint·link check·**orphan check(빌드 실패)**·generated reference freshness. 지금 "고아 금지"가 규약(soft)뿐 → hard(CI)로 보강.

## 7. 쟁점 (사용자 판단)
- **tracking.md vs issue tracker:** Codex는 tracking을 issue tracker(SoR)에 두고 docs엔 인덱스만 두라 권고(장기 docs tracking=stale 위험). engram은 `docs/tracking.md`를 살아있는 backlog(T-/D-)로 의도적으로 씀. → 솔로/소규모면 docs/tracking 유지가 단순, 팀 확장 시 재고. **사용자 결정.**
- **CHANGELOG vs step-log:** Codex는 root CHANGELOG(Keep a Changelog) 권고. engram step-log(언제/무엇)가 사실상 그 역할 → 별도 CHANGELOG 불요로 보임. 다만 "사용자/운영자용 릴리스 노트"가 필요해지면 분리.

## 8. 공백 / 한계
- **frontmatter 표준(type/audience/status/last-reviewed) 미확립** — 도입은 가능하나 업계 단일 표준 없음(불확실).
- auto-gen 역효과·줄수 임계는 단일출처(가능성 높음) — engram 실측 전 절대 신뢰 금지.
- 통합 가이드의 실제 작성·CLAUDE.md 슬리밍은 이 리서치 범위 밖(권고만).

## 9. 주요 출처
- Diátaxis https://diataxis.fr/ · docs-as-code https://www.writethedocs.org/guide/docs-as-code/
- GitLab Handbook https://handbook.gitlab.com/handbook/about/handbook-usage/ · Google SWE Book ch10 https://abseil.io/resources/swe-book/html/ch10.html · MS Engineering Playbook https://microsoft.github.io/code-with-engineering-playbook/
- ADR https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions · Keep a Changelog https://keepachangelog.com/en/1.1.0/
- AGENTS.md https://agents.md/ · CLAUDE.md memory https://code.claude.com/docs/en/memory · auto-gen 역효과 arXiv 2602.11988 · spec-driven arXiv 2604.05278 · llms.txt https://llmstxt.org/ · Read the Docs orphan https://docs.readthedocs.com/
