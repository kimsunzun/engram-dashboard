# S10 — 백엔드 추상화 설계 (AgentTransport / OutputEvent)

근거: GPT+Gemini 구성 검토 수렴(`web 검토 2026-06-12`) + 사용자 결정.
범위: 멀티 백엔드(claude/codex/gemini 콘솔 + API) 통합 인터페이스. **장기·저위험은 충분히 추상화(over-engineer 허용), API는 껍데기만, 콘솔 3종은 구현.**

## 결정 사항 (사용자)
- 장기로 가니 위험 없는 추상화는 지금 충분히 깐다.
- **API transport는 shell(껍데기)만** — 인터페이스만 만족, 내부는 나중(종량제 API ~한 달 뒤).
- **콘솔 3종(claude / codex / gemini CLI)은 만들어둔다** — 전부 터미널 기반.

## 수렴 피드백 반영 (GPT+Gemini 일치)
1. **seam을 `emit(bytes)` → `emit(OutputEvent)`로 상향.** xterm은 유일 진실이 아니라 projection 중 하나.
2. replay raw-bytes 취약(ANSI 잘림/부분메시지) → **콘솔은 terminal-bytes ring 유지(알려진 한계)**, semantic event log/이중저장/projection은 **API 등장 때 채움**.
3. capability **bool 폭증 금지** → 영역별 descriptor(input/output/control/session/model).
4. 입력도 bytes면 leaky → **`InputEvent`** 개념(PTY=Raw, API=Message).
5. 빠진 control 동사: **interrupt(≠kill)** 지금 추가, reconfigure/snapshot/graceful-shutdown은 단계화.

## 레이어 (목표 구조)

```
AgentManager
  └ HashMap<AgentId, AgentSession>            수명 + 복원 오케스트레이션
       │
   AgentSession  (에이전트당 1, 상태 독립)
     ├ OutputCore  : seq · replay · fanout   ← OutputEvent 단위로 동작(공용 코드 1벌)
     ├ capabilities: Capabilities (영역별)
     └ transport   : Box<dyn AgentTransport>
                          │
   trait AgentTransport  (seam)
     데이터: send_input(InputEvent) / (emit OutputEvent → OutputCore)
     제어  : start · interrupt · kill · resize        [지금]
             reconfigure · snapshot · graceful_shutdown [단계화/후일]
     질의  : capabilities()
        ├ PtyTransport  : master/slave·pump스레드·ConPTY·kill6·JobObject  (콘솔 공용)
        └ ApiTransport  : 껍데기(unimplemented), HTTP 스트림은 나중
```

## 핵심 타입 (개념 — 시그니처 확정은 구현 시)

```
enum OutputEvent {
    TerminalBytes(Vec<u8>),         // 콘솔(claude/codex/gemini) — 지금 쓰는 유일 variant
    // ── API 등장 때 채움(지금은 미정의 또는 정의만) ──
    // TextDelta(String) · MessageDone · Usage{..} · ToolCall{..} · Error{..}
}

enum InputEvent {
    Raw(Vec<u8>),                   // PTY: 키 입력 바이트
    // Message(String) · Cancel · Reconfigure{..}   (API용, 나중)
}

struct Capabilities {               // bool 나열이 아니라 영역별 묶음
    input:   { raw, message, attachment }
    output:  { terminal_bytes, markdown, tool_events, usage }
    control: { resize, interrupt, cancel, graceful_shutdown }
    session: { resume, snapshot, cwd_env }
    model:   { select, temperature, max_tokens }
}
```

### 콘솔 백엔드 (AgentBackend — PtyTransport가 파라미터로 보유)
- `ClaudeBackend`  : `--session-id`/`--resume` + `sessions/<pid>.json` 추적. **구현·검증 완료(S9)**.
- `CodexBackend`   : build_command/tracking — **CLI 구독일(며칠 뒤) spike로 플래그 확정**. 그 전엔 best-guess + 표식.
- `GeminiBackend`  : 동일 — gemini CLI 플래그 spike 후 확정.
- 셋 다 `PtyTransport` 공유, 차이는 args·세션 추적뿐.

### ApiTransport (껍데기)
- `AgentTransport` 구현은 하되 메서드는 `Unsupported`/`unimplemented!` 반환. capability는 전부 false/미지원으로 보고. HTTP 스트림·이벤트 변환은 API 모델 붙는 날 채움.

## 단계 구분 (지금 vs 후일)
| 항목 | 지금 | 후일 |
|---|---|---|
| OutputEvent seam | ✅ enum + TerminalBytes | TextDelta/Usage/ToolCall variant |
| InputEvent | ✅ enum + Raw | Message/Cancel/Reconfigure |
| Capabilities 영역별 | ✅ 구조 + 콘솔 값 | API 값 채움 |
| control: interrupt | ✅ (Ctrl+C / 콘솔 즉시 유용) | — |
| control: reconfigure/snapshot | 정의만 | API 때 구현 |
| replay | 콘솔 terminal-bytes ring(현행) | semantic event log + display/event 분리 + projection |
| 콘솔 백엔드 | claude✅ / codex·gemini 구조+stub | CLI 붙는 날 spike로 flag 확정·검증 |
| ApiTransport | 껍데기 | 내부 구현 |

## 현재 S9 코드 → 목표 마이그레이션
- `PtySession` 분해 → `PtyTransport`(master/slave·pump·kill6·job) + `OutputCore`(seq/replay/fanout를 AgentSession으로 hoist).
- `pump` 스레드(현 drain의 read 부분)는 `OutputEvent::TerminalBytes`를 emit, OutputCore가 후처리.
- `PtyManager` → `AgentManager`(`Arc<dyn AgentSession>` 보유). restore/fallback은 `capabilities().session.resume` 기반 generic.
- `claude.rs` → `ClaudeBackend`. `CodexBackend`/`GeminiBackend` 신설(stub). `ApiTransport` 껍데기 신설.
- write_stdin → `send_input(InputEvent::Raw)`. kill에서 interrupt 분리.

## 미해결/검토 포인트 (③ 상세 웹 질문 후보)
- pump→OutputCore 전달: 채널 vs 콜백(Arc OutputCore)? 성능·소유권.
- OutputEvent enum을 지금 어디까지 정의할지(미사용 variant 미리 vs 나중 추가).
- AgentBackend(콘솔 args) trait을 PtyTransport에 합칠지 별도 둘지.
- interrupt의 PTY 구현(Ctrl+C=0x03 주입 vs 시그널).
- capability를 trait 메서드 vs 데이터 descriptor vs Extension Trait(`as_interactive()`).

## 3자 고수준 검토 취합 (GPT + Gemini + fable, 2026-06-12)

**수렴(3자 일치):** seam을 event 관점으로 · capability bool 폭증 경계 · interrupt/reconfigure 동사 누락.

**★fable 단독 발견 (실제 코드 grounded — 핵심 보강)★:** leak은 입력(emit bytes)이 아니라 **종료 경계**다.
- kill 6단계(manager.rs:442-503)는 transport-private 자원(child/job/master)과 공용 OutputCore의 pump 종료 동기화가 섞여 있어 통째로 `transport.kill()`에 못 넣는다.
- `transition()`(drain.rs:110-140)의 exit-code 판정은 transport별로 다르다(PTY=exit code / API=stream closed·cancelled·error).
- **→ 분해:** `transport.shutdown()`(자원 강제 종료, 멱등, child→job→master 순서만 transport-private) + `core.join_pump(timeout)`(pump 종료 동기 대기, 공용) + `TerminalReason`(Exited{code}/Killed/StreamClosed/Error — transport가 산출, status 전이 idempotent 규칙은 공용).
- **kill = `shutdown()` + `join_pump()` 합성.** 이 분해가 `remove_session` vs `kill_agent` 중복(manager.rs:357-387 vs 442-503)도 자연 제거 → 분리선이 옳다는 방증.
- shutdown flag 소유권 명시 필요(transport set / pump 종료부 read).

**capability 결정 (fable, 현 규모):** bool 몇 개면 충분. session/resume만 작은 descriptor(메커니즘 다름). Extension Trait/full negotiation은 과설계(컴파일타임 capability 상실 → "추상 위에만 구현" 원칙과 충돌). capabilities를 `AgentInfo` 스냅샷에 실어 프론트 결합점 일원화.

**control 동사 확정안:** `start` · `send_input(InputEvent)` · `resize` · **`interrupt`**(≠kill, PTY=`\x03` 주입 / API=HTTP cancel) · `shutdown`(자원) · **`reconfigure`**(PTY=respawn / API=param 변경) · `capabilities`. pump 종료는 `core.join_pump()`.

## 저수준 취합 (fable + GPT + Gemini, 2026-06-12)

### 3자 수렴 — 확정
1. **TerminalReason**: flat enum, transport 산출 → core가 `AgentStatus` 매핑(idempotent + **finalize 정확히 1회**: OnceCell/compare_exchange). variant: `Exited{code:Option<i32>}` · `Killed` · `Interrupted` · `StreamClosed` · `Cancelled` · `Error(String)`. ※ **raw lib error(reqwest/nix) 직접 노출 금지** → 도메인 문자열로 매핑(Gemini). `StreamClosed`≠성공(GPT — 보조 last_error 여지).
2. **shutdown()/join_pump() 명시 2동사** (RAII 금지 — async Drop 없음[Gemini] + master drop 타이밍 못 잡음[fable]). 순서: `transport.shutdown()`(master drop/cancel token → pump read가 EOF/Err로 깨어 탈출) → `core.join_pump(timeout)` → status finalize 1회. Drop은 안전망만. **소유권**: transport=master/writer/child/shutdown flag/job, core=subscribers/replay/seq/status/drain_handle/drain_done_rx. Killed 판정은 transport가 reason으로 올려 **atomic flag 공유 제거**. 인과는 타이밍이 아니라 자원 폐쇄로 보장(Gemini).
3. **InputEvent**: `Raw(bytes)` only 지금. **Cancel은 variant 아님** → control verb `interrupt()`(3자 일치).
4. **args 격리**: PtyTransport는 claude/codex/gemini를 **모름**. 순수 `CommandSpec{program,args,env,cwd}` 주입. claude/codex/gemini 각자 CommandSpec 산출(얇은 backend layer). dyn/제네릭 과추상화 금지(Gemini). SpawnMode(Fresh/Resume) 중립 유지, 플래그명 매핑만 backend 안.
5. **control 동사 최종**: start·send_input·resize·interrupt·shutdown·reconfigure·capabilities. kill = shutdown+join_pump 합성.

### ✅ 해결 — OutputEvent/InputEvent: 확장 enum으로 인터페이스화 후 넘어감 (사용자 결정)
**결정:** `OutputEvent`(`TerminalBytes`만)·`InputEvent`(`Raw`만)를 **확장 가능한 enum**으로 정의하고, core를 variant-agnostic(`_ => ignore`)로 둔다. API variant는 붙을 때 한 줄 추가(교체 가능 = 인터페이스화 완료). "지금 dead variant 이름까지 쓸지"는 비용 0의 곁가지라 비결정 — 안 쓰고 진행. 원칙: "나중에 교체만 되면 인터페이스화하고 넘어간다."

## 구현 순서 (④ — 단계마다 build/test/commit, 끝에 fable 게이트)

검증된 S9 코드 재편이라 **회귀 0**이 목표. 각 단계 후 `cargo test --lib` + `cargo run --example headless` 통과 유지.

1. **중립 타입/enum 정의** — `OutputEvent{TerminalBytes}` · `InputEvent{Raw}`(확장 enum) · `TerminalReason{Exited{code}/Killed/Interrupted/StreamClosed/Cancelled/Error(String)}` · `Capabilities`(영역별, 콘솔 값) · `CommandSpec{program,args,env,cwd}`. `PtyChunk`→`OutputChunk` 중립화.
2. **OutputCore 추출** — PtySession에서 seq/replay/subscribers/status를 분리한 구조체. `emit(event)` · `finish(reason)`(idempotent + finalize once) · `join_pump(timeout)` · subscribe/unsubscribe. 단위 테스트.
3. **AgentTransport trait + PtyTransport** — master/writer/child/shutdown/job + pump 스레드를 PtyTransport로 이동. `shutdown()`(반환 전 master drop 보장) · pump가 `OutputEvent` emit + 종료 시 `TerminalReason` 산출→`core.finish`. headless 통과.
4. **AgentBackend(CommandSpec 산출)** — `ClaudeBackend`(현 claude.rs) + `CodexBackend`/`GeminiBackend` stub. SpawnMode 중립 유지, 플래그명 매핑만 각 backend.
5. **AgentSession** — OutputCore + transport 보유. `write_input`/`resize`/`interrupt`/`kill`(=shutdown+join_pump)/`reconfigure`/`capabilities` 노출.
6. **AgentManager**(PtyManager 개명) — `Arc<dyn AgentSession>` 보유. spawn/restore/fallback을 `capabilities().resume` 기반 generic으로. S9 복원 로직 이식. `remove_session` 중복 소멸.
7. **ApiTransport 껍데기** — AgentTransport 구현하되 `Unsupported`/`unimplemented!`. capability 전부 false.
8. **commands/lib/프론트 재배선** — `send_input`/`interrupt` 커맨드, `capabilities`를 `AgentInfo` 스냅샷에 포함, TS 미러.
9. **fable 게이트** + headless + 전체 테스트.

**불변식 유지(회귀 금지):** kill의 "master drop→reader EOF→join" 인과(이제 shutdown→join_pump 2동사로), §10 락 규칙, drain send 시 lock 미보유, epoch 재구독, best-effort tracker.

## (참고) 검토 당시 갈림: OutputEvent API variant를 지금 정의?
- **fable + GPT (2): 미리 정의 X.** wire 포맷(Serialize, 프론트 공유)이라 dead variant가 TS로 샘 + API shape 불명 = 추측 over-engineer. 지금은 `TerminalBytes`만 + 이름 중립화. 대신 **core를 variant-agnostic**(consumer `_ => ignore`)으로 = 확장 안전 구조만 확보.
- **Gemini (1): 미리 정의 O(강력).** LLM primitive(TextDelta/Usage/ToolCall)는 저위험·준표준. core fanout/replay match-arm 미리 안정화 → API는 transport만 끼움. `Extension(Value)` 탈출구.
- **매니저 권고: 미리 정의 X.** "저위험 over-engineer"는 "확장이 안 깨지고 retrofit이 비싼 것"에만 적용 — OutputEvent는 나중에 variant 추가가 싸므로(consumer ignore) 그 바에 못 미침. 단 3자 진짜 공통점인 **"core를 variant-agnostic 확장안전으로"** 는 지금 확보.
