# 핸드오프: research 스킬 v3.2 재설계 (작업본 완성, 미병합)

## 한 줄 상태 + 다음 첫 액션
research 스킬을 **실측 기반으로 재설계**해 `_wip/research/`에 v3.2 완성. 원본 불변·미병합.
**다음 첫 액션:** 사용자가 다른 스킬들 검토 후 **batch merge** 때 — `_wip/research/`의 `SKILL.md`·`references/flow.md`·`feedback.md`만 `.claude/skills/research/`로 복사(스펙 파일 제외) → `/review doc` → 커밋.

## 무엇을 왜 바꿨나 (핵심 피벗)
- **옛 설계:** Claude(Sonnet)+Codex 둘이 BLIND 이중 수집 → Opus 심판.
- **실측이 뒤집음:** 사실조회는 단일 Claude ~100%(SimpleQA 30문항, 웹 켜면) → 이중 수집 군더더기(같은 웹→같은 오차, Codex 비판 실측 확인). 하드 멀티홉(FRAMES 8문항)에선 **cross-family 적대 리뷰**가 confident-wrong 적출 → 우리 스킬 **7/8(87.5%)** > 단일 Sonnet 5/8 > "최고점 따라하기"(Opus+자기검증) 4.5/8, 공개참조(멀티스텝 66%)도 상회.
- **v3.2 = Codex를 수집자→적대 리뷰어로 전환.** 부가: 라우팅(light/medium/deep)이 핵심 · calibration(모르면 기권) 1급 · grounding=메인 외부(+medium↑ Codex 이중) · abstention≠contradiction · 확신도는 grounding+리뷰서 파생("합의=확신" 폐기) · 모델→역할 배정표(Fable 나오면 표만 교체) · mode-aware 에스컬레이션 · **적대 강도 레벨 사다리(2~5, medium=2~3/deep=4~5)** · 반박은 반증출처 강제.

## repo 상태
- 브랜치 **master. 커밋 안 함.**
- `_wip/research/` = 작업본(untracked `?? _wip/`). 원본 `.claude/skills/research/` **손 안 댐**.
- 기타 미커밋(이번 작업 무관, 건드리지 말 것): `package.json` · `package-lock.json` · `src/components/slot/TerminalSlot.tsx`.

## 검증 상태 (쌍으로)
- **돌린 것:** SimpleQA 30문항(웹/무웹) 실측 · FRAMES 8문항 실측(우리 스킬 7/8) · **Codex 적대 설계리뷰 2라운드**(v2·v3 BLOCK 다 반영) · v3.2 §4/강도표 눈으로 read 검증(대충).
- **검증 안 됨(오신뢰 금지):** full `/review doc` 게이트 **안 돌림**(대충 read만) · 종합보고서 판(DeepResearch Bench류) **N-확대 미검** — medium+ 적대리뷰 값어치는 "근거 있는 가설"(FRAMES 8 = 방향성, 통계 아님) · 실제 skill invoke dogfood은 FRAMES 8회분뿐.

## 하지 말 것 (dead-end — 다시 꺼내지 말기)
- cross-family **"이중 수집"으로 되돌리기 금지**(사실조회 군더더기·오경보 — 실측).
- **"collector agreement = confidence" 부활 금지**(같은 웹 = 독립 아니라 연출).
- **2-blind-judge 전수판정(v2) 부활 금지**(과함 → Codex=단일 적대 리뷰어로 단순화가 결론).
- 병합 시 `_wip/research/`의 `REDESIGN-SPEC*.md`(v2·v3)는 **설계 메모지 스킬 아님 → 원본 복사 X**.

## 미결/미착수
- **batch merge 대기** — 사용자가 다른 스킬들 다 본 뒤 한꺼번에 병합 예정(research 단독 병합 X).
- **C7 평가 하네스**(N확대·종합벤치로 미검 해소) · **C9 PRD/TRD binding**(`research/bindings/engram.md`) = 별건 미착수.

## 읽어야 할 파일 (이것만)
- `_wip/research/SKILL.md` · `_wip/research/references/flow.md` — v3.2 스킬 본체
- `_wip/research/REDESIGN-SPEC-v3.md` — 설계 근거·실측 요약(정본)
- `_wip/research/feedback.md` — 변경 히스토리(v3.1·v3.2)
