# 핸드오프: 터미널 wiring 실측 통과·커밋 완료 → 다음 = 슬롯 UI 이주 PRD (slotStore→viewStore)

> master, HEAD = **66c1d12**. 터미널 출력 wiring 슬라이스 완결(구현+리뷰+커밋+cdp 실측 PASS). 이번 세션 큰 발견 = **프론트 레이아웃 2-store 이주 미완**이 사람 UI를 죽여놓음. 다음 세션 = 그 이주 PRD.

## 한 줄 상태 + 다음 첫 액션
모듈③ 터미널 출력 wiring = **완료·검증·커밋(66c1d12)**. 실측서 **핵심 발견**: 트리·슬롯 UI가 옛 `slotStore`에 걸려 있고 화면 캔버스는 새 `viewStore`라 **사람 클릭이 캔버스에 안 먹음**(LLM/invoke 경로는 됨).

**다음 첫 액션:** **스코핑** — `slotStore`가 어디까지 쓰이나 + 이주 대상 UI 액션 목록 + RichSlot(lab) 물리는 지점을 Explore로 매핑 → 그걸 근거로 **"슬롯 UI 이주" PRD**(컨설→옵션셋→**사용자 결정**). 굵은 이주라 메인이 임의 확정 금지.

## repo 상태
- HEAD = **66c1d12** (master) "모듈③ 터미널 wiring 1차 + 리뷰 FIX 3건". 이전 = 09ee5b8.
- **working tree = `_wip/` 만** (미커밋 스크래치, **격리 유지 = 커밋 금지**): `_wip/research/`(research 스킬 재설계 스크래치) + `_wip/slot-check.png`(실측 스샷). ★`git add -A`/`.` 금지 — 타깃 경로만 add.★ (이번 세션에 _wip 파일 하나가 실수로 staged 됐던 것 unstage 처리함.)

## 이번 세션 완료
- **FIX 3건 반영 + 커밋 66c1d12:**
  - FIX 3(stale-unsubscribe, 코더 opus 스폰): `ProtocolClient.subs`에 owner `token` 도입 — `unsubscribe`를 "현재 엔트리가 내 token일 때만 delete"로 가드. 재구독(epoch 교체/StrictMode) 시 옛 unsubscribe가 새 구독 지우던 "재시작 후 빈 터미널" 회귀 차단. 회귀 테스트 2개.
  - FIX 1: `<TerminalSlot key={node.agent_id}>` (죽은 `?? node.id` 제거 — 리뷰 nit).
  - FIX 2: 미사용 jest-dom 미포함(`@testing-library/react`만 devDep).
- **리뷰 /review code (light→full 승격 — 동시성 트리거):** opus doc-aware + Codex blind 2인. **FIX 판정, BLOCK 없음.** 불일치 1건(stale-SET)은 선재·범위 밖으로 로그 처리(아래 do-not).
- **cdp 실측 PASS:** `assign_agent(view,slot,agent)` invoke로 슬롯 배정 → **터미널 실제 렌더**(cmd 프롬프트가 xterm에 뜸, hasXterm:true). 배관 end-to-end 확인.

## ★핵심 발견 — 프론트 레이아웃 2-store 이주 미완 (다음 작업의 근거)★
- **화면 캔버스 = 새 `viewStore` + `ViewLayoutRenderer`**(UUID slot id, 백엔드 권위 ADR-0035). `AppLayout.tsx:55`이 이걸 렌더.
- **트리·슬롯 UI = 옛 `slotStore`로 dispatch**(number id, 프론트 전용, focus 기본=1). 예: `AgentTree.tsx:57-58,212` "포커스 슬롯에 배치" → `useSlotStore` dispatch. `SlotContextMenu.tsx:37`도 slotStore dispatch.
- → **사람 UI 액션이 안 보이는 옛 store를 건드려 새 캔버스에 반영 안 됨** = 사용자가 겪은 "배치했는데 무응답 / 동작이 다 이상해". LLM 경로(`viewStore.assignAgent`=`assign_agent` invoke, `eventBus.ts:60` 노출)는 정상.
- **이주 = 사람 UI를 `viewStore`(실 슬롯)로 옮기고 invoke→emit(`layout:updated`/`view:list-updated`)→렌더 이벤트 배관 잇기.** ViewLayoutRenderer 주석이 예고한 "전면 이주".

## lab 자산 = RichSlot 스파이크 (다음 PRD 범위에 물림)
- `src/lab/richslot/` — `claude -p --output-format stream-json` 파싱→구조화 렌더(마크다운·Monaco 코드·diff·툴콜·thinking). 터미널(raw) 반대편 = **JSON 모드 렌더러**. 파싱층(순수 TS `types.ts`/`parse.ts`)/렌더층(React) 분리 + 실측 fixtures + `npm run dev:richslot`(1430) 독립 실행. §5 핸들 `window.__lab`.
- README 통합 계획이 **옛 slotStore** 기준(`{kind:'rich'}` 추가)으로 적혀 있음 → 이주와 한 몸. 백엔드측(stream-json spawn + `OutputChunk` 구조화 variant)은 굵은 설계/ADR.

## 다음 PRD 범위 (A/B/C — 묶을지 쪼갤지 사용자 결정)
- **A. UI 제어 이주** (slotStore→viewStore): 배정·split·close·포커스 클릭이 실 슬롯 조작. ← 지금 "안 됨" 해소, 비교적 기계적.
- **B. RichSlot 통합**: lab → 실 슬롯.
- **C. 렌더러 선택**: 슬롯이 출력 capability(`terminal_bytes` vs `markdown`/structured)로 xterm/RichSlot 분기. 백엔드 stream-json + OutputChunk 구조화 = ADR. (ADR-0002/0030 "출력 터미널 가정 금지"의 실체.)

## 백로그 (사용자 제기 — 나중)
- **i18n:** 하드코딩 한글(`'포커스 슬롯에 배치'`·`'중단'`·`'종료'`·`'에이전트 없음'` 등) → 로컬라이징 테이블 분리. cross-cutting, 우선순위 낮음.
- **우클릭 통일 UX:** 생성·분할·배정을 화면 우클릭 컨텍스트 메뉴로 통일(트리 `+` 버튼 낭비). A 이주 때 반영 후보.
- **spawn 종류 차이:** ad-hoc `SpawnByCwd`=cmd 셸(resume:false) / profile spawn=claude(resume:true). "생성이 뭘 띄우나" 별개 정리.

## 이월 미결 (이전 핸드오프 계승)
- **평면 B (같은 창 다중 슬롯 같은 agent):** 결론 = **案 A(프론트 subs를 agentId당 다중 콜백 fan-out) = 작음, 백엔드 무변경.** subs가 렌더 등록만이라(ADR-0035/0037) 프론트 전용 변경으로 충분. 案 B(백엔드 슬롯 cursor)는 프레임에 슬롯 태그 필요=중간. 案 C(push→pull)는 큼. late-join 히스토리 seeding = `resubscribe_after_seq`/`getSnapshot`. (이번 세션 상세 논의 있음.)
- **ADR 앵커 부착 (이월):** ffcd766 코드에 `// ADR-0041~0043` 5곳 — `agent.rs:199` · `protocolClient.ts:298` · `output_router.rs:80` · `layout.rs:99` · `output_view_store.rs:158` · `connection.rs:1176`. 부착만 남음.
- **FreshFallback 복원실패 배너:** report는 프론트 도달(`restore-result`), `eventBus.ts:134`가 console.info로만 버림. 슬롯 배너만 추가. 작음.

## 검증 상태 (쌍)
### 돌린 것 (green)
- FIX 3건: `npx tsc --noEmit` OK + `npm test` **137 통과**(기존 135 + FIX3 회귀 2). 재실행 = 프로젝트 루트에서 그대로.
- cdp 실측: 터미널 렌더 확인(`assign_agent` invoke → xterm에 cmd 출력, hasXterm:true). 앱은 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev`(PowerShell은 `$env:...=...; npm run tauri dev`), 검증 `node scripts/cdp.mjs eval/shot`.
- 리뷰: /review code full → FIX(non-blocking).
### 검증 안 됨 (오신뢰 금지)
- **사람 UI 경로 전체 미검증** — 2-store 갭으로 트리·슬롯 클릭이 캔버스에 안 먹는 것만 확인. split/close/create-view 등 개별 UI 액션이 옛/새 어디에 걸리는지 **개별 실측 안 함(코드 추론)** → 다음 스코핑이 확정.
- **평면 B 案A 미구현·미검증.**
- **stale-SET known-issue 미해결**(아래).

## do-not / 주의 (known-issue·실패접근)
- **stale-SET 엣지 (선재·범위 밖):** 두 `subscribeOutput`의 async `set` 순서가 뒤집히면(연결 straddle) 옛(stale) 콜백이 subs 차지 가능. FIX 3 token 가드는 **늦은 delete만** 막음. Codex=reachable(`wsTransport.ts:240`/`tauriTransport.ts:241` 이미 connected면 즉시 resolve) 지적, opus=실무상 드묾. **FIX 3가 만든 게 아님(선재)** — 별도 슬라이스에서 "최신 *호출*이 resume 순서 무관 승리"(await 전 토큰 예약/per-agent 직렬화)로 고칠 것. 커밋 메시지에도 known-issue로 박힘.
- **`_wip/` 커밋 금지** — `git add -A`/`.` 쓰지 말 것.
- **`slotStore` 이름에 속지 말 것** — 옛 프론트 전용 store(number id). 실 슬롯 = `viewStore`(UUID, 백엔드 권위).
- **실행 중 앱에 테스트 에이전트 2개**(내 cmd `fdc8b6e9` + 사용자 "에이전트 테스트" `743db0b1`) 떠 있을 수 있음 — 앱 닫으면 사라지는 런타임 상태(영속 아님). cmd는 검증용이라 kill 무방.

## 참조 (읽을 것만)
- 2-store 갭: `src/components/agent/AgentTree.tsx:57-58,209-212`(옛 dispatch) · `src/store/slotStore.ts`(옛, number id) · `src/store/viewStore.ts`(새, 백엔드 권위 invoke·assign_agent) · `src/components/layout/ViewLayoutRenderer.tsx`(주석에 "전면 이주" 예고) · `AppLayout.tsx:55`.
- RichSlot: `src/lab/richslot/README.md`(통합 계획) · `types.ts`(ContentBlock) · `src/lab/main.tsx`.
- FIX 3: `src/api/protocolClient.ts`(subs=52 · SubState.token · subscribeOutput=290 · unsubscribe 가드 · handleOutput=122) · `protocolClient.test.ts`(회귀 describe).
- 출력 배관 지도(이번 세션 배선도): 에이전트→OutputCore(seq)→WS→src-tauri `OutputViewStore`(content 공유버퍼/per-(창,agent) cursor/deliverable)→창 Channel(다중화)→`handleOutput`(epoch·seq dedup)→xterm. 프레임=`[tag|agentId|epoch|seq|payload]`. 통로 4평면: control emit(eventBus) / command invoke(forward_daemon_command) / reply / output Channel(onmessage).
- ADR: 0035(레이아웃 권위) · 0011(seam) · 0002/0030(출력 capability) · 0007(epoch) · 0040~0043(출력 평면).
