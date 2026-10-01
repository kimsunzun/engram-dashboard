# ADR-0238: claude JSON 끊기는 통로에 주입한 제어 줄로 보내 턴이 열려 있을 때만 나가고 끊긴 턴은 오류가 아닌 중단 턴 끝으로 닫는다

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 U1 (TRD `docs/process/S21-chat-ux/trd.md` §1) + 메인 판정 ①②③④ (TRD §11) + TRD 5판 리뷰 FIX 반영(TRD §12) · 구현 전 · B2 스파이크 전 · 멈추면 해당 결정은 새 ADR 로 개정) · 부분 폐기 by ADR-0243 (결정 4 프론트 무변경 서술과 결정 7 증상과 영향의 끝은 중단 행 서술 claude 는 끊김 표시 행이고 중단 행은 대비로만) · 부분 폐기 by ADR-0257 (결정 3과 5) · 부분 폐기 by ADR-0262 (결정 2의 줄 함수 반환형, 결정 3의 턴 열림 문, 결정 5의 끊기 기록, 영향의 락 순서 줄)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§1 U1 · §3-4 · §3-5 · §3-6 · §6 · §7 B2 · B3 · §9 · §10 0238 · §11 ①–④) · 조사 `docs/research/chat-ux-four-features-2026-09-27.md`(§2) · ADR-0237(Esc 키와 명령 `agent.interrupt` — 이 통로를 부른다) · ADR-0241(끊긴 claude 도구 행의 표시) · ADR-0004(backend 지식 격리) · ADR-0006(락 순서 — 새 간선 없음) · ADR-0127(턴 관측 · 30 분 fail-open) · ADR-0231(N11 오류 뒤 멈춤) · ADR-0234(뜻 모를 턴 끝은 멈춤을 건드리지 않는다) · ADR-0235(결정 2 · 4 · 5 — 개정 아님, 같은 뜻을 claude 에 편다) · ADR-0044(결정 6 — interrupt 후속 · 「통로에 땜질하지 말 것」과의 정합은 맥락) · ADR-0045(stream-json · 정제는 백엔드) · ADR-0226(첫 제출 래치 — 기대지 않는다) · step-log S21 · Amends ADR-0044 (결정 6 interrupt 후속) · Amended by ADR-0243 (결정 4 프론트 무변경 서술과 결정 7 증상과 영향의 끝은 중단 행 서술 claude 는 끊김 표시 행이고 중단 행은 대비로만) · Amended by ADR-0257 (결정 3과 5) · Amended by ADR-0262 (결정 2의 줄 함수 반환형, 결정 3의 턴 열림 문, 결정 5의 끊기 기록, 영향의 락 순서 줄)

## 맥락
오늘 claude JSON 통로(`StdioTransport`)는 끊기를 모른다 — `interrupt()` 가 `Unsupported` 를 돌려주고(`transport/stdio.rs:371-376`) 능력 `control.interrupt` 가 거짓이다. 그대로면 ADR-0237 의 Esc 가 claude 슬롯에서 아무 일도 안 한다. 기본 백엔드가 claude 라 체감 대부분이 빈다.

사용자는 이번에 짓기로 했다 [사용자 U1 2026-09-27] — 「claude 쪽 기능 확인해서 적용」(스파이크 먼저 — 설계 그대로).

풀어야 할 것이 셋이었다.
- **줄이 통로에 닿는 길** — 지시는 「`cancel_async_message` 가 닿는 길을 따라 하라」였다. 그 길은 `MidTurnPolicy::SessionClassified { cancel_line }`(`types.rs:215`)을 세션이 입력 자물쇠 안에서 `send_input` 하는 것이다(`session.rs:563-580`).
- **턴이 없을 때의 끊기** — codex 는 턴이 없으면 끊기를 거절한다(`backend/codex/transport.rs:4045-4061` — `Unsupported("… 중단할 턴이 없다")`). claude 도 같은 버스 계약을 따라야 한다(ADR-0237 결정 2 매핑 = CONFLICT).
- **끊긴 턴의 `result`** — 지금 번역기(`claude/mod.rs:944-970`)는 `is_error || subtype.starts_with("error")` 면 `Error(RESULT_FAILURE_DETAIL…)` 를 낸다. 끊긴 턴의 `result` 는 `is_error: true` 에 `terminal_reason` = `aborted_streaming` | `aborted_tools` 로 올 수 있다(SDK `types.py` · SDK issue #429 — 메인 대조). 그대로 두면 Esc 한 번이 오류 행을 그리고 오류 뒤 멈춤(`last_end_failed`)을 세워 우편을 멈춘다 — 의도와 반대다.

**ADR-0044 결정 6 · 「통로에 땜질하지 말 것」과의 정합** [메인 — 오케스트레이터의 정합 판단]. ADR-0044 는 interrupt 를 후속으로 미루며(결정 6 — 「stdio엔 PTY Ctrl-C 없음」) 「interrupt 미지원(kill만)은 의도된 미구현 — 통로에 땜질하지 말 것」이라 적었다(0044 「영향 / 불변식」). 그 규칙이 막은 것은 통로 안에서 끊기를 흉내 내는 것(예: PTY Ctrl-C 를 에뮬레이트)이다. 이 ADR 에서 제어 줄을 짓는 것은 claude backend 이고 벤더 지식은 backend 에 남는다(ADR-0004). 통로는 요청을 받으면 backend 가 준 뜻 모를 줄 한 벌을 쓸 뿐이다 — 오늘 이미 하는 `structured` 주입과 같은 모양이다. 능력 `control.interrupt` 는 통로 caps 에서 온다(TRD §3-4 · §11 ①).

## 결정
표기 — **[사용자]** = 사용자가 답한 결정 · **[메인]** = 오케스트레이터 판정(TRD §11) · **[고름]** = TRD 가 고른 내부 구현.

1. **제어 줄 = `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n`** — 모양은 공식 Agent SDK(조사 §2-1). `cancel_line`(`claude/mod.rs:698-724`)처럼 typed struct 로 직렬화한다. ★`cancel_queued` 같은 선택 능력은 싣지 않는다★ — 뜻은 codex 와 같다: **도는 턴만 멈추고, 이미 받아 둔 대기 입력은 다음 턴으로 간다**(ADR-0235 결정 4 — `Interrupted` 는 멈춤을 세우지 않는다 · 결정 5 — 남은 글은 다음 한 턴에). 그것이 CLI 응답의 `still_queued` 의 뜻인지는 스파이크 S3 가 확인한다.
2. **줄이 통로에 닿는 길 = 통로 주입** [메인 ① 수락] — `structured` 주입(`stdio.rs:52-56`)과 같은 모양이다.
   ```rust
   pub type InterruptLine = Arc<dyn Fn() -> Option<Vec<u8>> + Send + Sync>; // None = 「지금은 끊을 턴이 없다」
   impl StdioTransport { pub fn with_interrupt(mut self, line: InterruptLine) -> Self { … } }
   ```
   - `interrupt()`: 주입이 있고 함수가 `Some(b)` 면 입력 큐에 넣는다 · `None` 이면 `Unsupported("끊을 턴이 없다")` · 주입이 없으면 오늘의 `Unsupported`.
   - 능력 `control.interrupt = self.interrupt.is_some()` — 「끊을 수 있는 통로」이지 「지금 턴이 있다」가 아니다.
   - 통로는 이 바이트의 뜻을 모른다(바보 파이프 유지). 줄 모양 · 턴 열림 판정 같은 claude 지식은 `backend/claude` 에만 산다(「백엔드 확장」 · ADR-0004). `open_spawn`(`claude/mod.rs:424-433`)이 `open` 이 돌려준 튜플을 풀어 꽂는다.
   - 줄은 입력 큐(`stdio.rs:46-47` — 라이터 스레드 하나)에 들어가 사용자 줄과 **통째로** 직렬화된다(줄 섞임 없음). 입력 자물쇠(`input_order`)는 안 탄다.
   - 응답 `control_response`(`request_id` 머리 `interrupt:`)는 **번역하지 않는다** — `cancel_response_event`(`:1215-1235`)는 `cancel:` 머리만 보므로 이미 `None` 이다. 응답이 왔다고 턴이 멈춘 것은 아니다(S4).
3. **턴 열림 문** [메인 ③ 해소] — `backend/claude` 안의 화신 공유 값 `turn_gate: Arc<TurnGate>`(`struct TurnGate { open: AtomicBool }`). `DeliveryAck` 공유와 같은 모양이다(`open_spawn` 에서 하나 만들어 decoder 와 끊기 줄 함수에 같은 `Arc` 를 준다). 선 타입·세션·통로는 이 값을 모른다 — `types.rs` 에 두지 않는다.
   - **세우는 쪽 = 라이브 decoder** — 한 라이브 줄이 턴 진행 신호 사건(`classify_turn` `:538-545` → `Progress` — 사용자 되울림 `Structured{user}` · `assistant` 블록 · 흘린 `TextDelta` · `command_lifecycle` `started` 의 `Delivered`)을 하나라도 내면 `open = true`. **`result` 에서 `open = false`**(오늘 번역 뒤). 이어받기(`LineSource::Transcript`)는 건드리지 않는다. [고름] 「턴 중」의 정의를 턴 관측과 같은 분류기 하나에서 뽑는다 — 두 정의가 갈리지 않는다.
   - **읽는 쪽 = 끊기 줄 함수** — `open` 이면 `Some(줄)`, 아니면 `None` → 통로 `Unsupported` → 버스 CONFLICT · WS `Interrupt` 는 오류 응답(`connection_core.rs:1222-1230`). 턴이 열리기 전의 Esc 는 정직하게 거절되는 무동작이다 — 사용자가 다시 누른다.
   - ★S4 가 「되울림이 `system/init` 보다 먼저 오고 그 틈의 끊기는 무시된다」를 보이면 문이 여는 지점을 **그 턴의 `system/init` 뒤 첫 진행 줄**로 늦춘다★(같은 값 · 같은 거절 — 여는 줄만 바뀐다 · 멈춤 사유 아님). 그 보정은 비트가 하나 더 든다(「마지막 `result` 뒤 init 을 봤나」) · `system/init` 이 턴마다 오는지 S4 가 기록한다(TRD §12 1 라운드 low 1).
4. **끊긴 턴 `result` → `OutputEvent::TurnEnd { turn_id: None, outcome: TurnOutcome::Interrupted }`** [메인 ② 수락] — `Error` 도 `MessageDone` 도 내지 않는다. `Usage` 는 오늘처럼 그 앞에 낸다.
   - 판정 `interrupted(result)`(스파이크 S2 가 확정) = `subtype == "interrupted"`(오늘도 오류 아님 — `:951-956`, 유지) **또는** `terminal_reason ∈ {"aborted_streaming","aborted_tools"}`.
   - 턴 분류기(`classify_turn` `:553-557`)가 이미 `Interrupted → Ended(Other)` 로 적는다 → 턴은 끝나고 · 오류 뒤 멈춤을 세우지도 풀지도 않는다(ADR-0234). 누산기(`structuredAccumulator.ts:213-220`)가 `outcome: 'interrupted'` 행과 구분선을 그린다 — codex 끊김과 같은 모양 · 프론트·선 타입 무변경.
   - 이주가 아니라 **끊김 한 갈래만** `TurnEnd` 로 간다. 오늘 `subtype:"interrupted"` 는 `MessageDone`(구분선만)으로 닫히는데 이 변경 뒤엔 중단 행이 붙는다(오류 아님은 그대로). `types.rs:70`(「`MessageDone` 을 이것으로 이주시키지 않는다 — claude 는 그대로 `MessageDone` 을 쓴다」)과 `claude/mod.rs:551`(「이 decoder 는 `TurnEnd` 를 내지 않는다」)이 이 변경과 부딪히므로 두 주석을 함께 고친다 — 같은 doc 이 「어느 쪽을 내는지는 각 decoder 가 정한다」고도 적는다.
5. **「우리가 끊기를 보냈다」 표식은 예비로만 둔다** [메인 ④] — S2 에서 두 칸(`subtype` · `terminal_reason`) 모두 끊김을 못 가를 때**만** `TurnGate.interrupt_sent` 를 짓는다. 끊기 줄 함수가 **문이 열려 있을 때만**(줄을 돌려줄 때만) 세우고, 번역기가 **그 턴의 `result`** 에서 읽고 지운다(문을 닫는 같은 자리) — 그래서 한가할 때의 끊기 호출은 표식을 세우지 못해 다음 턴의 진짜 오류를 가리지 않는다. 스파이크가 두 칸 중 하나로 가르면 이 칸은 짓지 않는다.
6. **대기분은 다음 턴에 돈다** — 끊기가 대기 입력을 취소하지 않는다(결정 1 · `cancel_queued` 안 씀).
7. **잔여 경합은 벤더 한계로 문서화한다 — 좁힐 뿐 닫지 못한다.** 문을 읽고 줄을 큐에 넣는 사이, 또는 줄이 CLI 에 닿기 전에 그 턴의 `result` 가 나오고 **CLI 가 스스로 다음 턴을 열면**(대기 중이던 B · 우편이 나른 턴) 그 줄은 **다음 턴**을 끊는다. 프론트 `streaming` 이 턴 끝 사건까지 늦게 꺼지므로 늦은 Esc · 버스 호출이 이 틈에 든다. claude `control_request` 에는 턴 id 가 없어(codex `turn/interrupt` 는 `turn_id` 를 싣는다) **벤더 프로토콜로는 닫을 수 없다**. 증상 = B 가 한 번 끊긴 턴으로 닫힌다(중단 행 · 오류 아님 · 멈춤 불변 — 우편이 멈추지는 않는다). 스파이크 S7 이 크기를 잰다.
8. **스파이크(B2)가 출구 조건을 못 채우면 claude 부분은 멈추고 사용자에게 돌아간다.** 방법 · 시행 S1–S8 · 통과 조건의 정본 = TRD §3-5(결과 = 새 보고서 `docs/research/claude-interrupt-spike-2026-09-2x.md` + fixture `backend/claude/fixtures/interrupt_s1.jsonl`). 버스 명령(ADR-0237)은 스파이크와 무관하게 간다(TRD §7 B3 ⓐ).

## 거부한 대안
- **세션 입력 자물쇠 경로**(`MidTurnPolicy::SessionClassified { cancel_line }` — 지시가 따르라 한 `cancel_async_message` 길) [메인 ① — TRD §11] — ① 능력 `control.interrupt` 는 **통로의 caps** 에서 온다(`session.rs:656-657` → `stdio.rs:425-455`) ② 끊기는 입력 id 에 묶이지 않아 입력 자물쇠가 지킬 순서가 없다. 리뷰어 확인: 통로 주입으로 ADR-0004 격리 유지 · 새 락 간선 없음 · 입력 임대 검문은 이미 덮인다.
- **끊긴 턴을 오늘처럼 `MessageDone` 으로 닫는다** [메인 ② — TRD §11] — 끊긴 `result` 가 `is_error: true` 로 오면 지금 번역기가 `Error` 를 내 오류 행을 그리고 오류 뒤 멈춤을 세운다(맥락). `TurnEnd{Interrupted}` 의 이득 = codex 와 같은 중단 표시 · 오류 뒤 멈춤 불변. 리뷰어 확인: 턴 관측 `Ended(Other)` · `last_end_failed` 불변 · 초인종 울림 · 누산기가 이미 결말 행을 그림 · 대기 입력 골든 무영향.
- **턴이 열리기 전의 끊기를 미뤄 두는 큐**(init 을 볼 때까지 끊기를 미룬다) [메인 ③ — TRD §11] — 미루기는 번역기↔통로 사이에 새 공유 상태가 필요하다. 턴 열림 문이 대신한다 — 턴이 열리기 전의 Esc 는 `Unsupported`(무동작 · 정직한 답)이고 사용자가 다시 누른다.
- **끊기와 함께 대기분을 취소한다**(`cancel_queued` 를 싣는다) — 뜻을 codex 와 같게 둔다: 도는 턴만 멈추고 받아 둔 대기 입력은 다음 턴으로 간다(ADR-0235 결정 4 · 5). CLI 가 끊으며 대기분을 버리면 B 가 말풍선 없이 목록에서 사라진다 — ADR-0235 결정 2 「글은 늘 보인다」와 claude 수명주기 번역표(`claude/mod.rs:1164-1181` — `cancelled` → `Dropped{Unknown}` 묘비)가 깨진다(TRD §3-5).
- **`StdioTransport::open` 에 인자를 늘린다** [고름] — `StdioTransport::open(` 호출이 시험 포함 9 곳이다(`rg "StdioTransport::open\(" crates`). 빌더로 두면 기존 단언(`stdio.rs:507` · `session.rs:1249` — 주입 없는 통로는 `Unsupported`·`false`)이 그대로 참이다.

## 근거
- **사용자 결정 U1(2026-09-27)** — 인용은 맥락. 추천안 그대로다. 대안(codex 만 먼저)의 대가 = Esc 가 claude 슬롯에서 아무 일도 안 한다(TRD §1).
- **메인 판정 ①–④(TRD §11)** — ① 수락 · ② 수락 · ③ 턴 열림 문으로 해소 · ④ 예비로만 유지. 각 리뷰어 확인은 위 「거부한 대안」에 적었다.
- **벤더 모양** — 제어 줄 = 공식 Agent SDK(조사 §2-1) · 끊긴 `result` 의 `terminal_reason` = SDK `types.py` · SDK issue #429(메인 대조) · 되울림이 `init` 보다 먼저 오는 틈 = SDK issue #429(2.1.241 보고 · ★2.1.280 미검★) · 끊김 직전의 잘린 `assistant` 줄 = SDK issue #338.
- **대조 실측(2026-09-27 · 확실)** — 대화형 Claude Code CLI 는 Esc 에 도는 Bash 도구를 죽인다(90 초 동안 줄을 찍는 명령이 10 초째 Esc 에 쓰기를 멈췄고 뒤에 그 프로세스가 없었다 · 화면은 곧바로 「Interrupted」). ★범위 = 대화형 CLI 의 Bash 도구다 — 우리 stream-json `control_request` 끊기가 도는 Bash 도구를 죽이는지는 미검★(S1 이 기록한다 · 출구 조건과 무관).
- **리뷰** — TRD 1–5판 `/review trd` 라운드(TRD §12).
- ★**검증 상태**★ — 구현 전 · **스파이크 전**이다. 판정 칸(결정 4) · 문이 여는 지점(결정 3) · 대기분의 뜻(결정 1)은 스파이크 결과에 기댄다(아래).

## 영향 / 불변식
- ★**스파이크 결과에 기대는 불변식** — 결과가 지저분하면 작업을 멈추고 사용자에게 돌아간다★(TRD §3-5):
  - 「턴 관측 정리 = 두 지점뿐」·30 분 fail-open(ADR-0127) — S1 · S6 · S8. 끊은 턴이 끝 줄 없이 남으면 `in_turn` 이 30 분 붙는다. 한가 끊기가 경합으로 CLI 에 닿아 턴 신호를 켜는 줄을 내면 30 분 막힘 경로다(S6).
  - 「오류 뒤 멈춤」 `last_end_failed`(ADR-0231 N11 · ADR-0234) — S2. 끊김이 실패로 읽히면 Esc 한 번이 우편을 멈춘다.
  - 「대기 입력 상태 = 링 사건 한 줄기」 환원 규칙 · claude 수명주기 번역표 · ADR-0235 결정 2 — S3.
  - ADR-0226 첫 제출 래치 — 기대지 않는다(claude 는 보내기 전에 센다 · 끊기는 래치를 안 건드린다).
- **락 순서(ADR-0006)** — 끊기 줄은 입력 큐만 탄다 · 입력 자물쇠를 안 탄다 · 새 락 간선 없음.
- **④ 예비 표식의 대가(지을 때만)** — 끊기와 겹친 **같은 턴의** 진짜 오류는 끊김으로 접힌다. 잔여 경합(결정 7)에 걸리면 표식은 앞 턴의 `result` 에서 쓰이고, 줄이 끊은 다음 턴은 오류로 읽힐 수 있다(그 턴에 오류 뒤 멈춤이 선다).
- **턴이 열리기 전 · 턴 끝 직후** — 앞은 CONFLICT 로 거절되는 무동작(다시 누르면 된다) · 뒤는 결정 7 의 잔여 경합.
- **끊김 직전의 잘린 `assistant` 줄** — ADR-0240 뒤에는 흘린 블록의 잘린 완결 본문은 버려지고(사용자는 흘린 만큼을 이미 봤다) 안 흘린 블록은 잘린 본문 그대로 나간다. 끝은 중단 행이 표시한다.
- **능력 표시** — claude JSON 의 `control.interrupt` 가 참이 되면 트리 `canInterrupt`(`mergeTreeNodes.ts:94`)도 참이 된다 — 오늘 소비자가 없다.
- **의도적으로 고쳐 쓰는 기존 시험 둘 — 회귀로 읽지 말 것**: `result_interrupted_subtype_emits_only_done_no_error`(`claude/mod.rs:3390`) · `result_interrupted_subtype_with_is_error_false_emits_only_done`(`:3402`). 둘 다 `subtype:"interrupted"` 에 `["done"]` 을 단언하는데 결정 4 뒤 기대값은 `TurnEnd{Interrupted}` 한 벌(Error 없음 그대로)이고 이름도 기대에 맞게 바꾼다. 진짜 오류 회귀망 = 기존 `result_error_handbuilt.jsonl` 은 여전히 `Error` + `MessageDone`.
- **끊긴 claude 도구 행**(`tool_result` `is_error:true`)의 표시 = ADR-0241(codex 의 늦은 실패와 같은 규칙).
- **함께 고칠 문서** — `types.rs:70` · `claude/mod.rs:551` 주석 · CLAUDE.md 「핵심 불변식」(claude 끝 어휘에 끊김 `TurnEnd` 한 갈래) — 착지 라운드에서 `/review doc`(TRD §10 끝).
