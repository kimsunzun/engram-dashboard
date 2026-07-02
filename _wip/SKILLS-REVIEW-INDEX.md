# 스킬 개선 검토 — 인덱스 (research 결로 나머지 스킬 리팩토링 준비)

> 2026-07-02 새벽 자율 세션 산출. **research는 완성(제외).** 나머지 커스텀 스킬(review·qa·adr·new-skill)을 research 재설계 결에 맞춰 **개선 검토만** 해둔 것 — rewrite 아님. 실제 재설계는 **사용자가 아래 thesis를 승인한 뒤** 각 스킬 `_wip/<skill>/`에서 실행하고, 다 되면 한꺼번에 `.claude/skills/`로 병합.

## 한 줄 상태 + 다음 첫 액션
스킬 4개를 `_wip/`에 스테이징(복사)하고 각 폴더에 `REVIEW-NOTES.md`(개선 제안 상세) 작성 완료. **다음 액션 = 사용자가 스킬별 재설계 thesis(아래 표) 승인/수정 → 승인된 스킬부터 `_wip/<skill>/`에서 재설계(코더·`/review`·`/qa` 게이트) → 전부 되면 batch merge.**

## 무엇을 했나 (방법)
- **스테이징:** `.claude/skills/{review,qa,adr,new-skill}` → `_wip/` 복사(research는 이미 있음). 원본 불변.
- **검토:** 스킬마다 general-purpose 서브에이전트 1명 병렬 스폰 → 원본 + `_wip/research/`(품질 기준) + 하우스 규약을 읽고, **research 설계 DNA 대비 갭**을 파일·줄 인용으로 적출 → `_wip/<skill>/REVIEW-NOTES.md`에 영속화(결론만 회수). 기계적 스킬(adr)엔 억지 이식 금지를 명시 지시.
- **평가 축(research DNA):** ① SSOT+rot방지 ② 역할→모델 배정표(Fable-ready) ③ 정직한 ⚠️ 검증 상태 ④ evidence-grounded 판정 ⑤ calibration ⑥ cross-family 적대 ⑦ 프로젝트 바인딩 분리 ⑧ mode-aware ⑨ self-improvement feedback ⑩ 용어/마크다운 핀셋.

## 스킬별 판정 (상세 = 각 REVIEW-NOTES.md)

| 스킬 | 판정 | 재설계 thesis (제안) | 상세 |
|---|---|---|---|
| **review** | **중간** | rewrite 아닌 **표적 패치** — 모델명을 배정표 한 곳으로 걷어내고(교체성), 적대 판정을 evidence-grounded로 조이고, 자율 모드 에스컬레이션 추가 | `_wip/review/REVIEW-NOTES.md` |
| **qa** | **중간(하단, 경미 근접)** | 대재설계 불요 — 강도표 이중화 제거 + review와 게이트 명령·경계 정합 + 실측 한계 정직 note | `_wip/qa/REVIEW-NOTES.md` |
| **adr** | **경미** | 큰 재설계 불요(하이브리드 seam 이미 정답) — 미세 개선만(supersede 예시·정직 앵커·index 오퍼레이션 노출) | `_wip/adr/REVIEW-NOTES.md` |
| **new-skill** | **큼** | "골격 생성기"→**"설계 DNA 전파 장치"로 격상** — 새 스킬이 research 원칙을 디폴트 상속하게 템플릿·게이트 강화 | `_wip/new-skill/REVIEW-NOTES.md` |

## 스킬별 상위 개선 (요약)

**review [중간]**
1. 역할→모델 배정표 미분리 **[높음]** — opus/Codex 모델명이 §2 역할표 셀·§3·산문에 직박(교체성 위반). 단 PRD "둘 다 blind" 예외 한 줄 필요.
2. evidence-grounded verdict 부재 [중~높음] — Adversary가 구체 파괴사례 없이 FIX/BLOCK 가능 → "모든 FIX/BLOCK은 입력/레이스/줄/위반불변식 지목"으로 이식.
3. mode-aware 에스컬레이션 부재 [중] — 불일치를 항상 사용자에게 물음(자율흐름 끊김). 마이너=태그+로그+진행.
4. abstention≠contradiction 미구분 [중] · 5. deep 정량 모호("다관점/다회") [중].
- *전이 금지:* 라우팅·grounding(claim↔source)·BLIND·확신도 태그는 research 고유 — 억지 이식 X.

**qa [중간·하단]**
- 강도표 SKILL↔flow 이중화 제거(SKILL이 flow 정본 선언하고도 재수록) [중]
- **review↔qa 명령 복붙 SSOT 자기모순** [중·교차] — review 바인딩이 "안 베낀다" 선언 후 cargo/cdp 재수록.
- review §5 게이트 vs `/qa` 경계·중복실행 회피 미확정 [중·사용자결정]
- 핫패스 정직 note(cdp 1회 PASS≠race-free) [저~중] · 실행 중 드리프트 자기보고 결여 [저]
- *research 억지이식 금지 항목은 §5에 "qa 해당 없음" 정직 명시.*

**adr [경미]**
- full/partial supersede 판단에 `<examples>` 1쌍(유일 비자명 LLM 판단인데 예시 0) [중]
- ⚠️ 절 정직 앵커("정합성 보장은 adr.mjs 정확성 의존") [저~중]
- index 재생성 오퍼레이션이 트리거에 미노출 · flow §60 서술이 lint가 write하는 듯 오독 [저]
- *SSOT/rot방지는 오히려 research보다 강함(바인딩 "나는 스냅샷일 뿐" 명문화). deterministic 특성상 research 고유 개념 대부분 정당하게 N/A.*

**new-skill [큼]**
1. **역할→모델 배정표 미전파 [큼]** — `flow.template.md`가 opus/sonnet을 flow 본문에 인라인 하드코딩 = research가 폐기한 바로 그 모양을 생성(프로젝트 #1 불변 위반 방향).
2. **리서치 게이트(설계 전 `/research`) 부재 [큼]** — new-skill 자기 feedback.md가 이미 지목한 결함.
3. `/review trd`가 강제 아닌 권장 [중~큼] — 자기 초안을 자기가 통과시킬 여지(생산자≠리뷰어 위반).
4. 정량 SSOT 분리 미전파 [중] · 5. 마크다운 핀셋 규약 미참조 [중] · 6. study-notes를 스킬 폴더 *안*에 두라 가르침(안티패턴 회귀) [중].
- *잘 심긴 축(유지): 정직⚠️ 필수 섹션·feedback 루프·bindings·flow.md Read SSOT.*

## 교차 스킬 실 (cross-cutting — 병합 전 정리)
- **역할→모델 배정표 SSOT** — review·new-skill 공통 최상위 갭. 모델명 인라인 하드코딩을 배정표 한 곳으로. (research가 세운 표준 — 두 스킬로 전파.)
- **review ↔ qa 게이트 경계** — QA 명령 정본을 qa 바인딩 한 곳으로, review는 참조만. 이중/누락 실행 회피 규칙 필요(사용자 결정 요소 포함).
- **study-notes/ 위치** — research 새 설계는 "노트=스킬 폴더 밖(SSOT 심링크 읽기전용→프로젝트 데이터 경로)"인데, ① research 폴더에 아직 study-notes/ 있음 ② `_shared`가 "research는 study-notes로 usage-log 대체"라 명시 ③ new-skill이 "스킬 폴더 안에 두라" 가르침 — **세 곳이 한 방침으로 안 맞음.** 병합 시 위치 확정 필요.
- **정직 톤 전파** — "1회 PASS/통과 ≠ 증명"의 정직 라벨을 qa(핫패스)·review(만장일치≠정답 이미 있음)에 정합.

## 사용자 결정 대기 (굵은 것 — 임의 재설계 금지)
1. 스킬별 재설계 thesis 승인/수정 (위 표) — 특히 **new-skill[큼]** 은 범위가 크니 방향 확정 먼저.
2. review의 evidence-grounded·mode-aware·deep 사다리를 **얼마나** 이식할지(research 5단 사다리 통째는 과전이 — review는 대상을 읽어야 해 독립도 축이 부분만 맞음).
3. review↔qa 게이트 경계(누가 실행 주체·중복 회피) — 동작 정책이라 사용자 결정.
4. study-notes/ 위치 방침(세 곳 정합).

## ⚠️ 정직 상태
- 이건 **개선 제안(검토)** 이다 — *적용도 검증도 아님.* 각 REVIEW-NOTES의 갭은 서브에이전트가 파일·줄 인용으로 적출했으나 **사용자·후속 리뷰로 재검증 전엔 "근거 있는 가설"** 로 취급.
- 심각도(높음/중간/낮음)는 "결함 커버리지·교체성·정직성에 미치는 실제 영향" 기준 — 절대 척도 아님.
- 실제 재설계는 승인 후 프로젝트 규약(코더 서브에이전트 → `/review` → `/qa`)으로 실행. **`_wip/`는 커밋 금지(스테이징 전용).**
