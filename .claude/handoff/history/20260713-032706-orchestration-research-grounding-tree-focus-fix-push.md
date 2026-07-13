# 핸드오프: 트리 nesting 검증완료 + 드래그가드·"열기"포커스 2fix push(master 동기) — 다음 = **오케스트레이션(영상 데모), 단 기존 리서치 먼저 깔고 시작**

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0072 트리 계층(C1 백엔드 + C2 프론트)이 검증·안정화 완료. 이번 세션에 라이브 E2E 검증 + Codex cross-family 리뷰 + 저심각도 fix 2건(드래그 가드 일관성 · "열기" 트리 클로버)까지 커밋·**push 완료(origin/master = `ddf0f34` 동기)**. 다음 큰 갈래 = **오케스트레이션(영상 촬영용 데모)** — PRD 미착수.
- **다음 첫 액션(순서 중요):** ① **기존 오케스트레이션 리서치를 먼저 읽어 그라운딩**(아래 "참조" — 특히 `orchestration-survey-2026-07-12.md` + `llm-control-surface-message-command-scope-2026-06-28.md` + ADR-0014) → ② 이번 세션의 즉석 논의(아래 "오케스트레이션 즉석 논의 — 미확정")를 그 리서치와 **대조·조정**(riff를 정본으로 삼지 말 것 — 사용자 명시 지적) → ③ 명령 채널 + 최소 오케스트레이션 PRD를 사용자 결정으로 확정 → 구현.

## 완료 (이번 세션 — 전부 push됨)
- **C1+C2 라이브 검증(핸드오프 미검증 해소):** reparent 왕복 E2E(데이터·프론트 스토어 broadcast·orphan-to-root·디스크 직렬화) + **전체 데몬 재시작 영속**(stop/start 실프로세스 교체 PID 34396→13712, persistedOk) 전부 PASS. cdp(9223)로 실측.
- **C2 cross-family(Codex blind) 리뷰 완료**(이전 세션 사용자 중단분). 5 findings grounded 트리아지 → BLOCK 없음.
- **Fix 1 — 드래그 가드 일관성** `d72e2c0`(ADR-0072 C2 후속): `AgentTreeNode.hasProfile` + `disableDrag`/`disableDrop` 확장(ad-hoc 노드 드래그/드롭 차단 · same-parent no-op 차단 · 루트 pseudo-node isRootDrop 정규화) + 정렬 주석 정정. defer: #2(로드 normalize)·#3(dedup 무효).
- **Fix 2 — 제어 슬롯 포커스 제외("열기" 트리 클로버 fix)** `ddf0f34`(**ADR-0073**): 근본원인 = 트리 노드 좌클릭이 슬롯 pane click-to-focus(ADR-0066, 버블)로 트리 슬롯을 focused로 만들고, 우클릭 "열기"가 그 슬롯에 배정 → 트리 덮음. 수정(프론트만) = click-to-focus를 allowlist(콘텐츠 슬롯 empty/agent만)로 + `selectOpenTarget(layout, focusedSlotId)` 순수 함수(포커스가 콘텐츠면 그 슬롯→아니면 첫 empty→없으면 null). 구조적으로 제어 슬롯 클로버 불가.

## repo 상태
- 브랜치 master, **origin/master = `ddf0f34` 동기(ahead 0)**. 세션 중 사용자가 README 커밋 2개(`bfe86ae`·`0408454`) push해서 내 커밋들은 rebase로 선형 편입됨(무충돌 — docs vs code).
- 워킹트리: `.claude/handoff/latest.md`(이 save가 갱신) · `.tauri-dev-qa.log`(untracked QA 로그, 무해) · `.claude/handoff/history/`(이전+이번 기록). **미커밋 코드 없음** — `git log`가 정본.
- 앱: QA 실측용 `npm run tauri dev`는 종료됨. 볼 때 재기동 필요.

## 검증 상태 (게이트)
- **PASS(두 fix):** member-scoped `cargo test -p ...-core --lib`(182)·`-p protocol`·`cargo fmt --check`·코어격리(실 tauri import 0)·`npx tsc --noEmit`·`npm test`(vitest **598**). Fix2는 **실 UX cdp 실측**(트리 실클릭→포커스 안 옮김·우클릭→"열기" 실클릭→트리 미클로버) — 지난 C2 "invoke smoke" 과청구를 사용자 지적해 이번엔 실제 DOM 상호작용으로.
- **재실행:** Rust는 member-scoped만(bare `cargo test`=WebView2 크래시) · 프론트 `npx tsc --noEmit`+`npm test` · GUI `node scripts/cdp.mjs eval`(포트 9223, 앱 실행 중일 때).
- **검증 안 된 것(중요):** ① **사람 마우스 드래그 → nest 라이브 미검증**(가드 로직·onMove 배선·LLM reparentProfile invoke만 검증 — 실 드래그 nest는 한 번도 실측 안 됨). 사용자가 "드래그 안 된다" 보고 → 미검증 경로라 실재 가능. ② **release 모드 LLM 제어**(cdp/window은 dev·디버그포트 전용 — release exe에선 미노출).

## 오케스트레이션 즉석 논의 — ★미확정, 리서치와 대조 필요★
> 사용자 지적: 기존 리서치를 안 깔고 즉석 토론만 함. 아래는 그 riff고 **정본 아님** — 참조 리서치와 대조해 확정할 것.
- **최소 스코프(영상 데모용):** "A→B 메시지가 화면에 뜨는" 정도. 자율(A가 스스로 판단해 보냄) = 비목표. 명시적 호출만.
- **sendMessage(잠정 PRD):** `sendMessage(표시명, 텍스트)` → 이름→agentId resolve(mergeTreeNodes가 이미 `{id, displayName}` 보유, 얇은 find) → 기존 `writeStdin(agentId, ...)`. 겹침/부재 시 에러. 새 백엔드·wire 0.
- **"클로드가 앱 조종" 채널 — 선호안 = CLI-via-Bash:** 스폰된 에이전트가 Claude Code라 **Bash 툴 보유** → 얇은 Node CLI(`cdp.mjs`/`adr.mjs` 결)가 `daemon.json`(host/port/token) 읽고 데몬 WS에 붙어 `AgentCommand`(JSON, 예 `{"SpawnByCwd":{...}}`) 전송 → 클로드가 `engram spawn/send/reparent`를 Bash로 실행. **release-safe**(데몬은 빌드 무관 실행 + portfile만 있으면 됨), 마커 파싱보다 견고, 데몬 프로토콜 재사용.
  - **거부/대안:** MCP 툴/로컬 엔드포인트 = 임시 데모엔 오버킬(서버+클로드 mcp 설정). 출력 마커 파싱 = 자유텍스트라 fragile → CLI가 더 견고. cdp/window = dev 전용이라 release 데모 불가(이 채널이 그 갭의 답).
  - **감안:** 스폰 클로드에 CLI 실행 allowlist · CLI는 daemon.json 토큰 인증 · "에이전트가 시스템 조종" 신뢰경계(님 소유 에이전트라 의도지만 명령 스코프 정의 필요).

## 데모 목표 관문 (오케스트레이션과 함께 풀 것)
- **release exe 제어 통로:** `npm run tauri build`로 exe는 나오나, 현 LLM 제어(cdp/window)는 dev/디버그 전용 → release에서 클로드가 조종 못 함. **CLI-via-Bash가 이 관문의 답 후보**(§5 정식 제어 표면의 실현).
- **트리 nesting 사람 UX:** 메뉴 없음, 드래그 아니면 LLM reparentProfile뿐. 사람 드래그는 미검증(위). **데모에선 클로드가 reparentProfile(또는 위 CLI)로 트리 구성** = 사람 드래그 불필요.

## 미결 결정 (사용자)
- 오케스트레이션 명령 채널 = CLI-via-Bash로 갈지(리서치 대조 후 확정) · 명령 세트(spawn/sendMessage/reparent) · 마커 스코프·신뢰경계.
- 백엔드 트리/프리셋 구성 재검토(사용자 메모) + 그때 focus_slot/fixup_focus 제어 슬롯 강제(ADR-0073 defer — 현재 시각 잔존만, 동작 무영향).

## do-not / 실패한 접근
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시.** member-scoped만.
- **오케스트레이션을 이 채팅 riff 기준으로 설계 확정 금지** — 기존 리서치(아래) 먼저 읽고 대조.
- **GUI 실측을 invoke smoke로 갈음 금지**(지난 C2에서 사용자 지적) — UI 변경은 실제 클릭/상호작용까지.
- **SendMessage 툴 없음** → Claude 코더/리뷰어 이어가기 = fresh 스폰 + 이전 산출 주입. Codex만 `codex-reply`. cross-family blind = `mcp__codex__codex`(sandbox read-only·approval never·`config:{model_reasoning_effort:"high"}`).

## 정지 조건 (다음 세션)
- 오케스트레이션은 리서치 그라운딩 + PRD 사용자 결정 전 구현 진입 금지(순서 불변).
- push는 사용자 승인 후만.

## 참조 (읽을 것 — 오케스트레이션 그라운딩)
- **리서치(먼저):** `docs/research/orchestration-survey-2026-07-12.md`(최신) · `docs/research/llm-control-surface-message-command-scope-2026-06-28.md`(제어 표면·명령 스코프 — CLI/마커/MCP 논의 직결) · `docs/research/agent-messaging-survey-2026-06-28.md` · `docs/research/multi-agent-hosting-orchestration-research-2026-06-22.md` · `docs/research/control-surface-and-fleet.md`.
- **결정:** `docs/decisions/0014-오케스트레이션-참조-후보.md`(앵커·제안) · **ADR-0073**(제어 슬롯 포커스 제외 — 이번) · ADR-0072(트리 계층) · ADR-0066(click-to-focus)·0067(우클릭 포커스 불변식)·0060(SlotContent)·0035(레이아웃 백엔드 권위) · §5(CLAUDE.md — LLM 제어 표면·손발/두뇌).
- **코드(`파일:심볼`):** `src/components/agent/selectOpenTarget.ts`(열기 대상 선택·isContentSlot) · `AgentList.tsx`(openInFocusedSlot·rowMenu·onMove·disableDrop/Drag) · `ViewLayoutRenderer.tsx`(click-to-focus 게이트) · `mergeTreeNodes.ts`(hasProfile·id↔displayName) · `crates/engram-dashboard-daemon/src/`(WS 서버·portfile daemon.json — CLI가 붙을 곳) · `src/api/agentClient.ts`(writeStdin·reparentProfile·spawn 시그니처) · `scripts/cdp.mjs`·`adr.mjs`(Node CLI 패턴 참고).
- **흐름:** `docs/process/step-log.md` 최근 2항목(ADR-0072 C2 검증+가드 · ADR-0073 포커스 제외).
- 미커밋 코드 없음 — `git log`가 정본.
