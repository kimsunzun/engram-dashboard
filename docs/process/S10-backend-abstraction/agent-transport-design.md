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
