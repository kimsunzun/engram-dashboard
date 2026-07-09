# 핸드오프: 스킬 리팩토링 — research 완성, review·qa·adr·new-skill 개선검토 스테이징 완료

> 이번 세션 = 스킬 리팩토링 트랙. research는 **완성(사용자 확인)**. 나머지 커스텀 스킬을 research 결로 맞추는 **개선 검토**만 자율로 해둠(rewrite 아님). 메인 프로젝트(슬롯 UI 이주)는 별개 트랙 — 아래 [이월] 유지.

## 한 줄 상태 + 다음 첫 액션
`.claude/skills/`의 커스텀 스킬 4개(review·qa·adr·new-skill)를 `_wip/`에 스테이징하고 각 폴더에 `REVIEW-NOTES.md`(개선 제안) 작성 완료. **인덱스 = `_wip/SKILLS-REVIEW-INDEX.md`(먼저 이것만 읽으면 전체 파악).**
**다음 첫 액션:** 사용자가 인덱스의 **스킬별 재설계 thesis 승인/수정** → 승인된 스킬부터 `_wip/<skill>/`에서 재설계(코더 서브에이전트 → `/review` → `/qa`) → 전부 되면 한꺼번에 `.claude/skills/`로 병합.

## 무엇을 했나 / 판정
- **방법:** 스킬마다 general-purpose 서브에이전트 병렬 스폰 → 원본 + `_wip/research/`(품질 기준) 대비 갭을 파일·줄 인용으로 적출 → `REVIEW-NOTES.md` 영속화, 결론만 회수.
- **판정:** review=**중간**(표적 패치: 모델 배정표 단일화·evidence-grounded verdict·mode-aware) · qa=**중간 하단**(강도표 이중화·review와 게이트 경계·정직 note) · adr=**경미**(하이브리드 seam 이미 정답, 미세만) · new-skill=**큼**("설계 DNA 전파 장치"로 격상 — 모델 배정표 미전파·리서치 게이트 부재·`/review trd` 강제화).
- **교차 실:** ① 역할→모델 배정표 SSOT(review·new-skill 공통 최상위 갭) ② review↔qa QA명령 복붙 SSOT 모순 ③ study-notes/ 위치 방침 세 곳(research 폴더·_shared·new-skill) 불일치 — 병합 전 정리.

## repo 상태 (미커밋 — 유실 주의)
- **HEAD = `c59daf7` (master).** 이번 세션 새 커밋 없음(전부 `_wip/` 스테이징 = untracked, 커밋 대상 아님).
- **미커밋 변경(이번 세션 것 아님·이전 세션서 넘어옴 — 직전 research 핸드오프가 놓쳤던 것):** `package.json`+`package-lock.json`(xterm 6.1 beta + `@xterm/addon-webgl`) + `src/components/slot/TerminalSlot.tsx`(WebGL 렌더러 부착 — 분수 DPI 첫 행 픽셀 깎임 수정). **완료·cdp 실측 스샷까지 있음**(`_wip/webgl-render.png`·`claude-cutoff.png`) but 미커밋·미검증 게이트. → 별도로 `/review`+`/qa` 거쳐 커밋할지 사용자 판단.

## 검증 상태 (쌍)
- **돌린 것:** 4개 REVIEW-NOTES 작성(서브에이전트, 파일·줄 인용 grounding) · research 3파일 정합 확인 · `_shared` 점검 · 각 REVIEW-NOTES 파일 실재 확인(10~15K).
- **검증 안 됨:** REVIEW-NOTES의 갭은 **제안이지 재검증·적용 전** — "근거 있는 가설"로 취급(후속 `/review`나 사용자 확인 필요) · 서브에이전트 findings 메인이 개별 재확인 안 함(파일인용은 신뢰하되 판단은 미재검) · 재설계 실제 코드 변경은 0(스테이징만).

## 하지 말 것 (do-not)
- ★`_wip/` 커밋 금지★(스테이징 전용 — `git add -A`/`.` X). 
- research 원본 덮어쓰기 금지(사용자가 "완성"이라 했으나 병합은 스킬 파일 3개만·SPEC 제외 — 그건 별건).
- 기계적 스킬(adr)에 research 고유 개념(cross-family 적대·calibration·grounding·모델배정표·강도축) 억지 이식 금지(REVIEW-NOTES가 왜 N/A인지 이미 정직 기록).
- research 5단 적대 사다리를 review에 통째 이식 금지(review는 대상을 읽어야 해 독립도 축이 부분만 맞음).

## 읽을 파일 (이것만)
- `_wip/SKILLS-REVIEW-INDEX.md` = 전체 인덱스·판정표·사용자 결정 대기 목록 (먼저).
- `_wip/<skill>/REVIEW-NOTES.md` (review·qa·adr·new-skill) = 스킬별 갭 상세.
- `_wip/research/{SKILL.md,references/flow.md}` = 품질 기준(완성본).

---

# [이월] 메인 프로젝트 미결 — 슬롯 UI 이주 PRD (스킬 트랙과 별개)
> 전문 = `history/20260701-183331-terminal-wiring-verified-slot-ui-migration-next.md`. 스킬 리팩토링이 끝나거나 사용자가 트랙 전환하면 여기로.
- **다음 첫 액션(메인):** 스코핑 — `slotStore` 사용처 + 이주 대상 UI 액션 + RichSlot(lab) seam Explore 매핑 → "슬롯 UI 이주" PRD(옵션셋→사용자 결정).
- **핵심 발견:** 화면 캔버스=새 `viewStore`(UUID) / 트리·슬롯 UI=옛 `slotStore`(number id) → 사람 클릭이 캔버스에 안 먹음(="배치해도 무응답"). LLM/invoke 경로는 정상.
- **do-not:** `slotStore` 이름에 속지 말 것(옛 프론트 전용) · stale-SET 엣지(범위 밖).
