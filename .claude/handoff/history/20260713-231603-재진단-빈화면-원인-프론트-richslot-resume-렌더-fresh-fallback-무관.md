# 핸드오프: **재진단 완료** — JSON resume 빈 화면의 원인은 **fresh-fallback이 아니라 프론트 `RichSlot` resume 렌더**. 백엔드·core·seed·replay·데몬 WS 전부 정상(실측). 다음 = 프론트 라이브 이등분(C1/C2/C3) → 수정 → review → qa

## 한 줄 상태 · 다음 첫 액션
- **상태:** ADR-0079 seed 코드는 미커밋(워킹트리) 그대로. 이번 세션은 **원인 실측만** 수행 → 핸드오프의 기존 진단("resume 조기종료 → fresh-fallback이 seed 폐기 → 빈 화면")이 **틀렸음**을 실측으로 확정. 진짜 원인 층 = **프론트엔드 전용**. 코드 변경 없음(진단 서브에이전트 5회 스폰, 전부 throwaway·정리 완료).
- **다음 첫 액션:** 워킹트리 앱을 띄워(`npm run tauri dev`, 디버그포트 9223) JSON 에이전트를 **resume** 시킨 뒤 `scripts/cdp.mjs eval`로 **① 그 슬롯에 마운트된 렌더러가 `RichSlot`인지 xterm `TerminalSlot`인지 ② `RichSlot` accumulator에 아이템이 있는지(`acc.snapshot()`) ③ DOM 텍스트**를 확인 → 아래 C1/C2/C3 판별 → 원인 확정 후 `/implement`(코더→`/review code`→`/qa`)로 수정.

## ★ 재진단 결과 (이번 세션 핵심 — 실측 근거 체인) ★
**원인 층이 아래로 한 단계씩 좁혀지며 확정됨. 각 층은 실측(headless 하네스/CLI)으로 통과 판정:**

1. **claude CLI 행동 (CLI 스파이크):** `claude -p --input-format stream-json …--resume`는 **input-driven** — stdin 열려 있으면 입력 대기하며 계속 생존(stdin 8s hold→9.2s, 20s→21s), **입력 전 출력 0**(`--replay-user-messages`도 시작 시 아무것도 안 뱉음), stdin EOF/턴완료 시에만 종료. 시작 오버헤드 ~4.3s. `cmd.exe /c claude` 셸 shim은 stdin 수명에 **투명**(EOF 원인 아님). ⇒ claude는 stdin write-end가 닫혀야만 조기 종료.
2. **백엔드/core resume 경로 (headless 통합 테스트, `-p engram-dashboard-core`):** SID `476c4317-d388-4d18-9439-ebfa91b73b68` resume 시 `resume_transcript_events` **256 이벤트** 파싱 → `OutputCore::seed`(publish 전) → 링 버퍼 → `manager.subscribe()` replay **256개 그대로**(seq 0..255 연속, Error 0). `activate_profile(Resume)` = **`Resumed` 반환(fallback 발동 안 함)**, epoch 0 유지, claude 5s+ 생존, stderr 무출력. ⇒ **fresh-fallback은 이 버그와 무관**(핸드오프 기존 전제 반증). `EARLY_EXIT_WINDOW`(3s)도 무관 — claude가 3s에 살아있어 `early_terminal_status`가 None→Resumed.
3. **데몬 WS 층 (headless `-p engram-dashboard-daemon` 통합 테스트):** WS 클라이언트가 resume 구독 시 **256 프레임 전부 수신** — 전부 tag=1 StructuredEvent, seq 0..255 연속, 중복 0, decode 에러 0, `ReplayComplete` 도착(JSON **text** AgentEvent로 옴 — binary tag 255 아님; qa-daemon-probe 주석은 이 점이 틀림), `SubscribeAck{action:Reset, oldest:0, latest:255}`. 직렬화·전송 완전 정상.
   - **수신 페이로드 실제 형태(사용자 "양식 이상" 가설 검증):** malformed 아님, live와 동일 shape. `TextDelta{type,text,turn_id,message_id}`(assistant 텍스트=여기로 옴, 64개) · `ToolCall{name,args_json,id,turn_id,message_id}`(59개) · `Structured{kind:"user",json}`(133개; json은 user 턴 nested-JSON 문자열). **유일한 seed-vs-live 차이 = 모든 이벤트 `turn_id: null`**(transcript엔 턴 그룹 정보 없음 — 백엔드측, 예상된 것).

⇒ **결론: 256개 well-formed·처리가능 타입 이벤트가 프론트까지 도달하는데 화면이 빔 = 순수 프론트 버그(`RichSlot` resume 구독/렌더).**

## 프론트 후보 3종 (라이브 이등분으로 즉시 판별 — 코드 앵커 포함)
accumulator는 TextDelta·ToolCall·Structured를 **다 처리**하므로, "도달 못 함" 또는 "도달해도 렌더 트리거 안 됨"이 원인.
- **C1 (유력): resume 시 렌더러 오선택.** ADR-0078로 렌더 모드가 생성 단계로 이동 → resume에서 slot이 json 모드를 못 받아 `RichSlot` 대신 xterm `TerminalSlot`을 마운트하면 tag1 구조화 이벤트가 버려짐. spawn 응답 caps.output.structured=true는 오는데, **프론트가 caps로 렌더러를 고르는지·그게 resume 경로에 전달되는지** 확인. (앵커: 슬롯 렌더러 선택 로직 + `RichSlot.tsx:85-128` 마운트 조건.)
- **C2: 구독 키/레이스.** `RichSlot.tsx:96-97` `agentClient.subscribeOutput(viewId, agentId, onChunk)`, deps `[viewId, agentId, epoch]`, line 87 `acc.reset()` 후 feed. viewId(slot id)/epoch가 resume에서 불일치하거나 ProtocolClient가 replay flush를 onChunk 부착 전에 하면 RichSlot accumulator가 256을 못 봄. (앵커: `src/api/protocolClient.ts:205-240` replay 버퍼/flush, `eventBus`.)
- **C3: `turn_id: null`.** accumulator/렌더가 turn_id로 그룹·트리거하면 null이 깨뜨림. (단 TextDelta는 message_id 보유 → 텍스트 병합은 OK일 수 있음.) 부분적으로 사용자 직감과 일치. (앵커: `src/components/slot/structuredAccumulator.ts:79-159` consume, `StructuredTextView.tsx:416-465` renderItem.)
- **반증됨(재시도 금지):** Explore의 "assistant가 `Structured{kind:assistant}` 블록으로 와서 `GenericItemRow` 접힘 박스로 렌더된다" 가설 → WS 실측이 반박(assistant=TextDelta). renderItem에 label==='assistant' 핸들러 추가는 헛수고.

## 재사용 진단 자산 (재조사 불필요)
- **테스트 SID:** `476c4317-d388-4d18-9439-ebfa91b73b68`, transcript `~/.claude/projects/I--Engram-apps-engram-dashboard/476c4317-….jsonl`(~1.9MB, cwd 일치). 256 이벤트 seed됨.
- **JSON resume를 WS로 스폰(프론트 없이):** 프로덕션 변경 0으로 가능 — `server.manager.profiles().upsert(profile)`에 `AgentCommand::Claude{output_format: StreamJson}` + `profile.claude_session_id = Some(sid)` 심고, WS `SpawnProfile{profile_id, resume:true}` → `activate_profile(Resume)` → seed → `WsOutputSink`. (register_shell_profile 헬퍼와 동형.)
- **exe 파일락 우회(빌드 중 앱 실행 시):** `cargo build -p engram-dashboard-daemon --tests`가 데몬 exe 재링크에서 `os error 5`(락) 나도 **테스트 하네스 바이너리는 빌드됨** → `target/debug/deps/ws_e2e-*.exe <test>::<name> --ignored --nocapture --test-threads=1` 직접 실행. (taskkill 불필요.)
- 두 throwaway 테스트 소스는 이 세션 대화에 전문 있음(diagnostic_resume_spike = core 통합, spike_resume_replay_ws_wire_format = daemon ws_e2e append). 필요 시 재생성.

## 검증 상태 (쌍)
- **돌림(green, 실측):** core resume 경로 통합 테스트(256 seed→replay), daemon WS 통합 테스트(256 프레임 wire 수신), CLI 스파이크(claude stdin 수명·shim 투명). **재실행 = member-scoped(`-p engram-dashboard-core`/`-p engram-dashboard-daemon`) `--ignored --nocapture`만.**
- **검증 안 됨:** **프론트 라이브 동작** — resume 후 RichSlot에 스크롤백 실제 표시. C1/C2/C3 미확정(라이브 cdp 이등분 필요). 워킹트리 코드의 프론트 게이트(`npm test`/`tsc`)는 이전 세션 green이나 이 세션에서 재실행 안 함.

## 실패한 접근 / do-not
- **기존 핸드오프·ADR(0076/0077/0079) 전제 "fresh-fallback이 원인" = 반증됨.** fresh-fallback 폐기(고위험 kill/lifetime 재설계)는 **이 버그의 해결책이 아님** — 착수 금지(별개 개선으로 재론할 순 있으나 이 버그와 무관).
- **bare `cargo test`·`-p engram-dashboard` = WebView2 크래시.** member-scoped만.
- 실행 중 앱 있으면 `cargo build`가 데몬 exe 락 → 위 "exe 파일락 우회" 사용(taskkill 남발 금지).
- assistant 텍스트를 Structured 블록으로 가정하고 renderItem 고치기 = 헛수고(반증됨).
- 매직넘버/추측 수정 금지 — 라이브 이등분으로 C1/C2/C3 **확정 후** 수정(ADR-0038 정신).

## 정지 조건 (다음 세션)
- **워킹트리 미커밋 변경(ADR-0079 seed 코드 + 문서) discard 금지.** 커밋 여부는 사용자 판단(프론트 수정까지 끝나 E2E 통과 시 함께 커밋 검토).
- 프론트 수정이 §5(LLM 제어 표면)·ADR-0046(view-scoped replay·gen fence) 불변식을 건드리면 임의 확정 말고 사용자 확인.
- 원인이 C1(렌더러 오선택)로 확정되고 수정이 ADR-0078(렌더 모드 생성 단계 이동) 설계를 바꿔야 하면 → 새 ADR + 사용자 결정.

## 문서 영향 (다음 세션 처리 — 아직 안 함)
- 재진단이 확정되면 ADR-0076/0077(fresh-fallback)·0079(seed)의 "원인" 서술 정정 필요(seed 자체는 정상 작동하므로 0079는 유효, 단 "빈 화면 원인" 연결이 틀림). step-log에 이번 재진단 흐름 기록. 수정 ADR은 프론트 원인 확정·수정 결정 후.

## 참조 (읽을 것)
- **프론트 앵커:** `src/components/slot/RichSlot.tsx:85-128`(구독 effect·reset·onChunk tag1 gate) · `src/components/slot/structuredAccumulator.ts:79-159`(consume) · `src/components/slot/StructuredTextView.tsx:416-465`(renderItem: user/thinking/default→GenericItemRow, text→Markdown) · `src/api/protocolClient.ts:205-240`(replay 버퍼/flush) · 슬롯 렌더러 선택 로직(caps.output.structured → RichSlot vs TerminalSlot).
- **백엔드 앵커(정상 확인됨 — 재조사 불필요):** `manager.rs` resume_with_fresh_fallback:488/early_terminal_status:551/EARLY_EXIT_WINDOW:45 · `backend/claude.rs` resume_transcript_events/read_transcript_events(~630) · `output_core.rs` seed/emit · `transport/stdio.rs`(stdin struct 보유, 조기 close 없음).
- **ADR:** 0079(seed) · 0078(렌더 모드 생성 단계) · 0046(view-scoped replay·gen fence·seq dedup) · 0002/0030(caps 렌더러 선택) · 0076/0077(fresh-fallback — 원인 아님).
- 앱 실행: 미커밋 검증은 워킹트리 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` + `scripts/cdp.mjs`. `run-dashboard-clean.bat`은 HEAD 재빌드라 부적합.
