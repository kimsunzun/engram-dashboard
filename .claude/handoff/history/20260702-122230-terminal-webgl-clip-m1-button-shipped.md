# 핸드오프: 터미널 WebGL 클리핑 해결 + 트리 배치버튼(M1) — 커밋·푸쉬 완료 (origin/master d863c6a)

> master, origin push 완료. HEAD = **d863c6a**. 이번 세션 = 터미널 렌더/UI 실사용화(클리핑·배치버튼·여백) 완결. **JSON 렌더(트랙① 본류)는 여전히 미착수.**

## 한 줄 상태 + 다음 첫 액션
터미널 상단 클리핑 해결(WebGL) + 트리 "포커스 슬롯에 배치" 살림(M1) + 좌우 여백 = /review code full 통과 + QA(tsc·npm test 137) + 커밋·푸쉬 완료.
**다음 첫 액션 = JSON 구조화 렌더(사용자 "중요"·본류) 스코핑** — `src/lab/richslot/` vs 백엔드(stream-json 스폰·OutputChunk 구조화·capability 렌더러분기, ADR). 첫 조사 = claude 대화형 멀티턴 `stream-json` I/O 실동작(추측 금지·스파이크).

## 이번 세션 완료 (커밋·푸쉬됨)
- **7ecebf2 feat(front):** ① 터미널 DOM→**WebGL 렌더러**(`@xterm/addon-webgl@0.19.0` = xterm 6.0.0 정식 짝) — `customGlyphs`가 블록글리프 상단 클리핑(분수 DPI) 해결. ② **폰트 CSS var→실값 해석**(canvas가 `var()` 못 읽음 = 밤새 "검은 webgl"의 진범). ③ WebGL 실패/컨텍스트소실 시 DOM 폴백(로깅). ④ 좌우 여백(wrapper padding 4px 8px). ⑤ `AgentTree` "포커스 슬롯에 배치"를 죽은 `slotStore`→**`viewStore.assignAgent`**(assign_agent invoke, ADR-0035) 재배선 + 실패 시 setError.
- **d863c6a docs:** research 노트(`docs/research/terminal-xterm-render-webview2-2026-07-02.md`) + 런처 `run-dashboard(-clean).bat`.
- 리뷰: **/review code full**(opus doc-aware + Codex blind) → **FIX 3건 반영**(F1 webgl catch 로깅 · F2 배치 실패 setError 피드백 · F4 sourceSlotId 미사용 주석). QA: `npx tsc --noEmit` + `npm test` 137 통과.

## ★밤새 헤맨 것 정리 (do-not 반복 금지)★
- **"검은 화면(빈 슬롯+커서만)" 진범 = 데몬/orphan 프로세스 오염**(앱 반복 재시작/`location.reload()`가 만든 것). 렌더러 아님. 해결 = engram **데몬만** kill → clean 재기동. `claude.exe` 무차별 kill 금지(내·유저 다른 세션 죽음).
- **"webgl 검음(커서만)" 진범 = 폰트 CSS var 버그**(canvas가 `var()` 미해석). WebView2 GPU 문제 아님 → 폰트 실값 해석으로 해결. (밤엔 이걸 몰라 webgl을 dead-end로 오판·`git checkout` 원복했으나, 이번에 baseline에서 재적용해 **성공·커밋**.)
- **xterm beta 승격 금지** — 밤에 6.1-beta로 올렸다 DOM 렌더까지 깨짐. webgl은 **baseline 6.0.0 + webgl 0.19.0(정식 짝)**으로 감.
- **CDP 스샷은 WebGL(GPU) 캔버스 못 잡음** — DOM/canvas만. webgl 렌더 검증은 사람 눈(사용자 확인함). 자율 작업 시 유의.

## 렌더러 결정 (ADR 후보)
DOM→WebGL. 거부: **canvas**(xterm6 미지원 dead-end) · **DOM**(분수 DPI 블록글리프 클리핑). 근거·트레이드오프·폴백사슬(WebGL→DOM, canvas 중간단은 xterm6 막힘) = research 노트. **borderline ADR-worthy** — 원하면 `/adr new`로 박제(현재 rationale = 코드 주석 + research 노트).

## 후속 / note (리뷰 발견 — 미수정, 별건)
- **focus_slot invoke 부재(선재 갭·실효성 제약)** — 슬롯 클릭→포커스 변경 UI가 없어 focused slot이 백엔드 기본(첫/마지막 split)에 고정. "포커스 슬롯에 배치"가 이 갭에 묶여 다중 슬롯 배치엔 focus 이동 UI 필요.
- **F3:** WebGL 컨텍스트 처닝 — 재배정마다 `ViewLayoutRenderer` key=agent_id로 TerminalSlot 리마운트=컨텍스트 생성/파기. 많은 슬롯/장기 세션서 일부 슬롯 조용히 DOM 강등 가능(F1 로깅으로 관측). 환경 의존.
- **F5:** 터미널 배경 `#0a0a0a` 하드코딩 — 테마 light/e-ink 무시(선재). 이번 여백도 같은 하드코딩.

## repo 상태
- **HEAD = d863c6a (master), origin push 완료.** 이번 커밋 = `7ecebf2`(feat) · `d863c6a`(docs).
- **미커밋(내 것 아님/유저 처리 예정):** `.claude/settings.json`(병렬 세션 fable 전환) · `_wip/*.png` D + `_wip/shots/`(스샷을 shots/로 이동 — 유저가 git 정리·gitignore 직접 하기로).

## 검증 상태 (쌍)
- **green:** /review code full FIX 반영 · `npx tsc --noEmit` · `npm test` 137 · WebGL 렌더+클리핑해결 **사용자 눈 확인** · 배치버튼 cdp 시뮬(slot 배정 확인).
- **미검:** JSON 렌더 미착수 · webgl DOM 폴백 실발동(GPU 죽는 상황) 미실측(표준 코드라 신뢰 높음) · focus 이동 UI 없음.

## 다음 우선순위
1. **JSON 구조화 렌더(트랙① 본류, 사용자 "중요")** — lab richslot 재사용 + 백엔드 stream-json/OutputChunk/capability 렌더러분기(굵은 설계+ADR).
2. focus 이동 UI(슬롯 클릭→focus) — 배치 실효성.
3. (원하면) 렌더러 ADR · F3/F5 정리 · `_wip` git 정리(유저).

## 참조 (읽을 것만)
- research 노트: `docs/research/terminal-xterm-render-webview2-2026-07-02.md`
- 코드: `src/components/slot/TerminalSlot.tsx`(webgl+폰트+여백) · `src/components/agent/AgentTree.tsx`(배치=viewStore) · `src/store/viewStore.ts`(assignAgent/selectActiveView) · `src/components/layout/ViewLayoutRenderer.tsx`(key=agent_id, F3).
- ADR: 0035(레이아웃 권위) · 0002/0030(출력 capability=렌더러 선택).
</content>
