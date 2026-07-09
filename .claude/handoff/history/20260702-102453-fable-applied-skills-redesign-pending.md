# 핸드오프(Fable 전환): Fable 적용 완료 + 스킬 재설계 대기 + 글로벌룰 토큰효율 문구 대기 (+JSON렌더·슬롯UI 이월)

> **▶ 다음 세션(너 = Fable)에게:** 이 프로젝트는 이제 **Fable 5로 뜬다**(`.claude/settings.json` model=fable, effortLevel=xhigh). 아래 "Fable 운영 주의" 먼저 훑고 시작해라. 여러 트랙이 병존하니 사용자에게 **어느 트랙부터 갈지** 확인하고 진행.

## 한 줄 상태 + 다음 첫 액션
스킬 리팩토링 트랙 진행 중 — research 완성, 나머지 스킬(review·qa·adr·new-skill) 개선검토 완료·`_wip/` 스테이징. **다음 = 사용자가 스킬별 재설계 thesis 승인 → 승인분부터 재설계 → batch merge.** 병행 대기: 글로벌룰 토큰효율 문구(사용자가 Fable로 작성 예정) · JSON 렌더(사용자 "중요") · 슬롯UI 이주.

## 이번 세션에 바뀐 것 (직전 핸드오프 대비)
1. **Fable 적용** — `.claude/settings.json`에 `"model": "fable"` 추가(effortLevel xhigh 유지). 다음 세션부터 Fable. 스코프=프로젝트 한정(전역 원하면 유저 글로벌로 이동).
2. **Fable effort 확인** — effort 5단(`low`/`medium`/`high`/`xhigh`/`max`), xhigh 포함(high↔max 사이). API 기본 high / Claude Code 기본 xhigh. xhigh 설정 유효. (근거: claude-api 레퍼런스)
3. **글로벌룰 토큰효율 문구는 보류** — 사용자가 "Fable로 직접 작성"하기로. 초안은 아래 [대기작업 A]에 보존.

## repo 상태 (실측)
- **HEAD = `c59daf7` (master).** 미커밋 **코드** 0.
- untracked: `_wip/`(스테이징·스샷·작업본 — ★커밋 금지·`git add -A`/`.` X★) · `docs/research/terminal-xterm-render-webview2-2026-07-02.md`(터미널 트랙 노트, 커밋 대상) · `.claude/settings.json`은 tracked이므로 model=fable 변경이 미커밋 상태(커밋 여부 사용자 판단).

---

## 앞으로 할 일 (우선순위 = 사용자 결정)

### [대기작업 A] 글로벌 룰 — 토큰 효율/모델 티어 위임 문구 (사용자가 Fable로 작성 예정)
`I:\Engram\core\claude-global-shared\rules\global-rules.md`에 추가. 현재 `## 컨텍스트 위생`이 비슷하나 결이 다름(근거=위생/범위=수집성 한정). 원하는 것 = **토큰 효율 근거 + 범위 확대(핵심 판단 외 전부) + 모델 티어링.** load-bearing 표준이라 `/review doc` 권장. 제안 초안(사용자가 다듬을 것):
> ## 모델 티어 · 토큰 효율 — 메인은 핵심 판단만, 나머지는 위임
> 메인 세션은 상위급(비싼) 모델로 돈다. 그러니 메인 토큰은 가장 핵심적인 판단·오케스트레이션·통합에만 쓰고, 그 밖에 실행 가능한 일(수집·대량 읽기·병렬 탐색·기계적 편집·검증 러닝 등)은 하위 에이전트에 위임한다 — 서브에이전트는 더 싼 모델로 돌릴 수 있어 토큰 효율이 크게 오른다. "컨텍스트 위생"이 *무엇을*(결론만 필요한 수집성)이라면, 이 규칙은 *왜*(메인이 비싼 상위급) + *범위*(핵심 판단 외 전부)다. 핀포인트 단발 확인은 인라인 허용.
>
> ⚠️ **Fable 특이점(중요):** Fable는 서브에이전트·검색·메모리·커스텀툴을 **기본적으로 덜 쓰는** 경향(claude-api 레퍼런스). 즉 "위임하라"는 이 규칙이 Fable에선 **더 명시적으로** 박혀야 실제로 위임한다(가만두면 메인이 다 떠안음). 룰 문구에 "언제 위임하는가"를 구체적으로.

### [대기작업 B] 스킬 재설계 실행 (트랙 ② — 메인 스킬 작업)
`_wip/SKILLS-REVIEW-INDEX.md`가 진입점. 사용자가 스킬별 thesis 승인 → 승인분부터 `_wip/<skill>/`에서 재설계(코더 서브에이전트 → `/review` → `/qa`) → 전부 되면 `.claude/skills/`로 batch merge(+research 병합: SKILL.md·flow.md·feedback.md만, SPEC 제외).
- 판정: review=중간(모델배정표 단일화·evidence-grounded verdict·mode-aware) · qa=중간하단(강도표 이중화·게이트 경계·정직note) · adr=경미 · new-skill=큼(설계DNA 전파장치로 격상).
- **Fable 연결:** review·new-skill 최상위 개선=`역할→모델 배정표` 추출. 이게 되면 Fable를 어느 역할 슬롯에 태울지 = 배정표만 교체(사용자 결정). research는 배정표 이미 있음.
- 사용자 결정 대기: 스킬별 thesis · review↔qa 게이트 경계 · study-notes 위치.

### [대기작업 C] JSON 구조화 렌더 (트랙 ① — 사용자 "중요"·본류)
JSON 스코핑 → PRD. `src/lab/richslot/`(렌더 절반 완성) vs 백엔드(stream-json 스폰·구조화 OutputChunk·capability 렌더러분기, 굵은설계+ADR). 첫 조사 = claude 대화형 멀티턴 `stream-json` I/O 실동작(추측 금지·스파이크). JSON은 xterm/PTY/webgl 경로 안 씀 = 터미널 gremlin 전이 없음.

### [대기작업 D·이월] 슬롯 UI 이주 PRD
스코핑(`slotStore` 사용처 + 이주 UI 액션 + RichSlot seam) → PRD. 캔버스=새 `viewStore`(UUID) / 트리·슬롯UI=옛 `slotStore`(number id, 죽은 경로) → 사람 클릭이 캔버스에 안 먹음(LLM/invoke는 정상).

---

## Fable 운영 주의 (다음 세션 = Fable라 특히)
- **위임을 덜 함(기본):** 서브에이전트·검색·메모리·커스텀툴 under-reach. engram은 "비자명 코드변경=코더/리뷰어/QA 서브에이전트 강제" 규약이니, **위임을 명시적으로** 챙겨라(Fable는 가만두면 직접 떠안음). = 대기작업 A와 직결.
- **나레이션 많음 + 더 자주 물음:** 자율모드("진행 쭉해")에선 장황·질문과다 될 수 있음. 필요시 "사소 결정은 알아서, 스코프/파괴적 변경만 질문"으로 조정.
- **effort:** xhigh 설정됨(유효). 레퍼런스는 "반사적 xhigh 말고 high 기본+스윕" 권하나, engram 규약(메인=xhigh, 무가드 통합노드)은 근거 있어 유지 OK.
- **Fable 전용 API 차이:** 명시적 `thinking:{type:"disabled"}` → 400(생략해야). effort·adaptive thinking은 Opus 4.7/4.8과 동일. 스킬에 모델분기 짤 때 참고.

## 검증 상태 (쌍)
- **돌린 것:** 4×REVIEW-NOTES 작성(서브에이전트 grounding) · research 정합확인 · settings.json model=fable 적용(Edit 성공) · Fable effort/설정 사실 = claude-api 레퍼런스로 확인.
- **검증 안 됨:** REVIEW-NOTES 갭은 제안(재검증·적용 전) · 재설계 코드변경 0 · Fable 실제 기동 확인 안 함(다음 세션이 뜨면서 검증됨) · 글로벌룰 문구 미작성.

## 하지 말 것 (do-not)
- ★`_wip/` 커밋 금지★ · research 원본 덮어쓰기 금지(병합은 스킬파일 3개만) · adr에 research 고유개념 억지이식 금지 · research 5단 적대사다리 review 통째이식 금지 · `slotStore`(죽은 옛경로) vs `viewStore` 혼동 금지 · WebGL/xterm-beta 승격 되돌리기 금지(이전 세션이 헛다리로 판명·원복함 — 검은화면 원인은 렌더러 아닌 데몬 오염).

## 읽을 파일 (이것만)
- `_wip/SKILLS-REVIEW-INDEX.md`(스킬 트랙 진입점) · `_wip/<skill>/REVIEW-NOTES.md` · `_wip/research/{SKILL.md,references/flow.md}`(완성 기준).
- 트랙별 전문(정본): history `20260702-015527-skills-refactor-review-staged.md`(스킬) · `20260702-015132-terminal-blackscreen-daemon-rootcause-baseline-restored.md`(터미널/JSON) · `20260702-005823-research-skill-redesign-v3.2.md`(research) · `20260701-183331-terminal-wiring-verified-slot-ui-migration-next.md`(슬롯UI).

---

## 트랙 요약 (정본=위 history 파일)
- **① JSON 렌더** [사용자 중요] = 대기작업 C. "검은화면=데몬오염" 규명, baseline 복원, 코드 순변경 0.
- **② 스킬 리팩토링** = 대기작업 B. review/qa/adr/new-skill 개선검토 스테이징 완료.
- **③ research 병합** [완성·미병합] = 대기작업 B에 흡수(batch merge 시 함께).
- **④ 슬롯 UI 이주** [이월] = 대기작업 D.
