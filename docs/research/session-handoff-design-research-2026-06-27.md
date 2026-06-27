# 리서치 보고서 — 세션 핸드오프 문서 설계 패턴

**날짜:** 2026-06-27  
**방법:** Claude(Sonnet) 4갈래 병렬 + Codex BLIND 교차 + opus 교차대조·적대검증  
**강도:** medium  
**목적:** 글로벌 공용 핸드오프 문서 시스템 설계 (현 `.ccb/history/` + `/continue` 개선)

확신도 범례: ✅확실 / ⚠️가능성 높음 / ❓불확실

---

## 핵심 결론 (3줄)

1. **현 CCB 구조는 옳다.** append-only 스냅샷 + 별도 레이어 + `/continue` 최신1개 주입 = 적대검증 통과 형태. "조약"한 원인은 구조가 아니라 **자유형식(섹션 스키마 부재) + 글로벌 작성규칙 미박제.**
2. **추천 D = 현 CCB의 형식화·글로벌화.** 위치/스키마는 글로벌(개인 작업방식), 내용은 프로젝트별 append-only 폴더, `/continue` 무변경, 스킬 2층에 그대로 이식.
3. **Verification Gaps 섹션 추가.** 현 CCB에 없는 AI 특화 필수 섹션 — 검증 안 된 항목 명시로 환각 방지.

---

## 1. OSS 도구 패턴 (갈래 A + Codex 교차)

| 도구 | 컨텍스트 저장 | 글로벌/프로젝트 분리 | 핸드오프 지원 | 확신도 |
|---|---|---|---|---|
| Claude Code | CLAUDE.md 5계층 + Auto Memory + session transcript | ✅ managed/user/project/local/subdirectory | ✅ session resume/export/compact | ✅확실 |
| Cline | Memory Bank(`activeContext.md`) + `.clinerules/` | ✅ workspace > global rules | ✅ (방법론적, 내장 아님) | ✅확실 |
| Devin | 클라우드 Knowledge DB + `AGENTS.md` | ✅ 엔터프라이즈/사용자/repo 3계층 | ✅ `/handoff` CLI (가장 명시적) | ✅확실 |
| Aider | `.aider.conf.yml` + chat history | ✅ home→repo→현재 3단계 | ⚠️ `/save`+`/copy-context` (경량) | ✅확실 |
| GitHub Copilot | `.github/copilot-instructions.md` + path별 | ✅ personal/repo/path/org | ⚠️ CLI session resume | ✅확실 |
| Continue.dev | `~/.continue/config.yaml` + `.continue/rules/` | ✅ (`.continuerc.json` 버그 있음) | ✕ | ✅확실 |
| OpenHands | `AGENTS.md` + `.agents/skills/` | ✅ 사용자/프로젝트/조직 | ✕ (conversation_id resume ≠ handoff) | ✅확실 |
| Cursor | `.cursor/rules/*.mdc` + Memories | ✅ Team/Project/User 3계층 | ✕ | ❓불확실 (내부 구조 불명) |

**공통 패턴 3가지:**
1. 항상 읽는 프로젝트 지침 (`AGENTS.md` / `CLAUDE.md` / `.clinerules/`)
2. 범위가 좁은 조건부 지침 (path/glob 기반 rules)
3. 세션 상태 재개 장치 (Claude Code session / Cline Memory Bank / Aider history)

---

## 2. 글로벌 / 프로젝트 경계 (갈래 B + Codex 교차)

| 레이어 | 들어가는 정보 | 예시 |
|---|---|---|
| **글로벌** | "나는 이렇게 일한다" — 개인 선호, 응답 방식, 모든 프로젝트 작업방식 | 응답 언어, 커밋 규칙, 서브에이전트 위임 방식 |
| **프로젝트** | "이 저장소는 이렇게 동작한다" — repo 사실, 빌드 명령, 아키텍처, 팀 규칙 | build/test 명령, 모듈 경계, 코딩 규칙 |
| **★핸드오프 (별도 레이어)** | "이번 세션의 현재 상태" — 매 세션 휘발성 | 진행 상태, 열린 이슈, 다음 스텝 |

**★ 양 family 독립 수렴:** 핸드오프/세션 상태는 글로벌도 프로젝트도 아닌 **별도 레이어**가 적합. 확신도: ✅확실 (수명·쓰기주체·주입시점이 기조 문서와 직교. engram CLAUDE.md "기조 vs 상태 분리" 원칙이 1차 증거).

**실무 함정:**
- 글로벌 과적재: Anthropic 200줄 이하 권고 (상시 컨텍스트 노이즈↑)
- 프로젝트 고립: 개인 선호 drift, 매 세션 작업방식 재설명
- ~~ETH Zurich 3% 감소 수치~~ → **폐기** (arXiv 2602.14690, 검증 불가 — 미래 날짜 형식)

---

## 3. Living Doc vs 이력 누적 (갈래 C + Codex 교차)

| 방식 | AI 에이전트 적합도 | 장점 | 단점 |
|---|---|---|---|
| Living document (단일 덮어씀) | ⚠️중간~높음 | 핸드오프 속도 최소, 현재 상태 가시성 최고 | 결정 이력 소실, 150~200줄 초과 시 성능↓, Codex 32 KiB 제한 |
| 이력 누적 (세션마다 새 파일) | ⚠️중간 (단독) | 완전한 감사 추적, 롤백·디버깅 | 현재 상태 파악에 여러 파일 읽어야 함 |
| **하이브리드 (append-only 스냅샷)** | ✅높음 | 두 장점 결합, 모순 없음 (과거 동결) | 없음 (동기화 규율 불필요 — 과거 안 건드리므로) |

**Amp 사례 (✅확실):** 무감독 자동 compaction → 재귀 요약 왜곡 발생 → Amp가 compaction 폐기, 명시적 핸드오프(새 파일)로 전환. 문제는 "하이브리드"가 아니라 **기존 내용 덮어쓰기(무감독 재요약)**였음.

**핵심 발견:** "하이브리드가 최선"의 정확한 조건은 **append-only 스냅샷(과거 파일 동결) + 최신 1개를 live로 취급**. 현 CCB가 이미 이 형태.

---

## 4. 핸드오프 문서 필수 섹션 (갈래 D + Codex 교차)

모든 도메인(의료 SBAR/I-PASS, 군사 SITREP, NOC, SW) 공통 4개:  
**현재상황(Situation) → 배경(Background) → 행동(Actions) → 다음/권고(Recommendation)**

| 섹션 | 필수/선택 | 글로벌/프로젝트 | 현 CCB 보유 여부 | 확신도 |
|---|---|---|---|---|
| Current Goal / Status | 필수 | 프로젝트(세션별) | ✅ (`## ★★ 다음 세션 첫 행동`) | ✅확실 |
| Background / Context | 필수 | 글로벌+프로젝트 | ✅ (한줄 상태) | ✅확실 |
| Decisions Made + Rationale | 필수 | 프로젝트(ADR 포인터) | ✅ | ✅확실 |
| What to Avoid (Traps / Failed) | 필수 | 프로젝트 | △ 일부 | ✅확실 |
| Next Steps | 필수 | 프로젝트 | ✅ | ✅확실 |
| Environment / Commands | 필수 | 글로벌+프로젝트 | △ 포인터 있음 | ✅확실 |
| Files / Artifacts Changed | 필수 (AI 특화) | 프로젝트 | ✅ (커밋 목록) | ✅확실 |
| **Verification Gaps** | **필수 (AI 특화)** | **프로젝트** | **✕ 없음** | ✅확실 |
| Source of Truth Pointers | 필수 (AI 특화) | 글로벌+프로젝트 | △ ("본문이 항상 우선") | ✅확실 |
| Context-Window Survival Notes | 필수 (AI 전용) | 글로벌+프로젝트 | △ 일부 | ✅확실 |
| Session Identity / Resume Path | 필수 (AI 특화) | 프로젝트 | ✅ | ✅확실 |

**Verification Gaps** = "아직 검증 안 한 항목" 명시. 환각 방지 목적. 양 family·전 도메인이 AI 특화 필수로 꼽았고 현 CCB에 없는 유일한 중요 누락.

---

## 5. 적대 검증 결과

| 주장 | 판정 | 근거 |
|---|---|---|
| "하이브리드가 최선" | **수정됨** | append-only 동결형은 동기화 규율 불필요. 문제는 "무감독 덮어쓰기 하이브리드". 현 CCB가 증거. | 
| "핸드오프 = 별도 레이어" | **강화됨** | 수명·쓰기주체·주입시점이 기조 문서와 직교. CLAUDE.md "기조 vs 상태 분리" 1차 증거. |
| "ETH Zurich 수치 (성공률 3% 감소)" | **기각됨** | arXiv 2602.14690 — 검증 불가(미래 날짜). 방향성("상주 오염 위험")만 위험으로 생존. |

---

## 6. 설계 선택지 + 추천

| 선택지 | 다 PC 공유 | /continue 호환 | 스킬 2층 정합 | 이력 추적 | 컨텍스트 오염 |
|---|---|---|---|---|---|
| A: 프로젝트별 living doc `_state.md` | △ | ✕ (메커니즘 변경) | △ | ✕ | ○ |
| B: 글로벌 living doc + 프로젝트 오버라이드 | ○ | ✕ (전체 상주화) | ○ | ✕ | ✕ |
| C: 글로벌 live + 프로젝트 archive (다른 위치) | △ | △ (주입 대상 모호) | △ | ○ | △ |
| **(추천) D: 글로벌 위치·스키마 + 프로젝트별 append-only** | ✅ | ✅ (무변경) | ✅ | ✅ | ✅ |

### (추천) D 구성

1. **위치·섹션 스키마는 글로벌** — `global-rules.md` 또는 continue 스킬 SKILL.md에 박제. "나는 이렇게 핸드오프한다" = 개인 작업방식 → 글로벌 레이어.
2. **내용은 프로젝트별 `.ccb/history/` 유지** — append-only, 과거 파일 동결. 모순 없는 감사 추적.
3. **섹션 스키마 고정** — 현 CCB에 있는 것 + **Verification Gaps 추가**.
4. **`/continue` 무변경** — 최신 1개 `@file` 주입 그대로. 스키마만 추가됨.
5. **스킬 2층** — 공용 SKILL.md(스키마·작성규칙) + 프로젝트 `bindings/`(폴더 경로·프로젝트 특수 섹션). 현 research/review/qa 스킬과 같은 패턴.

### 거부 대안 (ADR용)

- **A 거부:** living `_state.md` 덮어쓰기 → 결정 이력 소실 + `/continue` 메커니즘 불일치(고정 파일명 vs 시간순)
- **B 거부:** 글로벌 단일파일 전 프로젝트 상태 누적 → 상주 컨텍스트 오염, 글로벌/프로젝트 경계 위반
- **C 거부:** live(글로벌)와 archive(프로젝트) 다른 위치 → `/continue` 주입 대상 모호, 프로젝트 상태 글로벌 혼입
- **무감독 자동 compaction 거부:** 재귀 요약 왜곡(Amp 사례). append-only가 구조적 차단.

---

## 7. 교차검증 표

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 대부분 도구가 글로벌/프로젝트 2층 이상 분리 | ✅ | ✅ | 합의 |
| Claude Code + Cline이 가장 구조적 핸드오프 지원 | ✅ | ✅ | 합의 |
| 핸드오프 = 별도 레이어 | ✅ | ✅ | 합의 |
| 하이브리드 최선 | ✅ | ✅ | 합의 (수정형) |
| Devin이 가장 명시적 `/handoff` 지원 | ✅ | ✕ (미언급) | Claude 우선 (공식 문서) |
| Cursor Memories 내부 구조 | ❓ | ❓ | 공백 |
| Amp compaction 폐기 | ✅ | ✕ | Claude 우선 (방향 일치) |
| ETH Zurich 수치 | ✅ (Claude 보고) | ✕ | **기각** (검증 불가) |

---

## 8. 한계 / 공백

- Cursor Memories 내부 저장 구조 — 공식 문서 접근 실패. 불확실.
- Devin `/handoff` 내부 포맷 — 상업 서비스, 내부 구현 세부 미확인.
- "글로벌 핸드오프가 정말 성능에 영향을 주는가" — 정량치 없음. 방향성만.

---

*갈래별 원본 보고서: `docs/research/ai-coding-agent-context-management-research-2026-06-27.md`, `global-vs-project-context-research-2026-06-27.md`, `living-doc-vs-history-accumulation-research-2026-06-27.md`, `session-handoff-sections-research-2026-06-27.md`*
