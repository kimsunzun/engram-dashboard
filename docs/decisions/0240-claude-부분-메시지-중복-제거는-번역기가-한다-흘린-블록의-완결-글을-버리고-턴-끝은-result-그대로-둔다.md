# ADR-0240: claude 부분 메시지 중복 제거는 번역기가 한다 — 흘린 블록의 완결 글을 버리고 턴 끝은 result 그대로 둔다

- 상태: 확정 (2026-09-27, 근거: TRD 5판 §5 · §9 + TRD 5판 리뷰 FIX 반영(TRD §12) · 구현 전 · 코드 무변경)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§5-1 ~ §5-5 · §6 · §7 B1 · §9 F4 · §10 0240) · 조사 `docs/research/chat-ux-four-features-2026-09-27.md`(§4 — 벤더 형식 실측 §4-2 · 피어 §4-3 · 선택지 §4-4) · ADR-0238(끊김 직전의 잘린 `assistant` 줄 · 턴 열림 문이 흘린 `TextDelta` 로도 열린다) · ADR-0044(결정 6 — partial 델타 스트리밍 후속) · ADR-0079(이어받기 기록 `.jsonl` 을 읽어 되감는 길 — 결정 5 와 B1 이어받기 멈춤 조건이 선 자리) · ADR-0045(stream-json · 정제는 백엔드) · ADR-0004(backend 지식 격리) · ADR-0127(턴 관측 · 30 분 fail-open) · step-log S21 · Amends ADR-0044 (결정 6 partial 델타 후속)

## 맥락
claude JSON 에서 글이 글자 단위로 흐르게 하려면 스폰 인자에 `--include-partial-messages` 가 든다(오늘 StreamJson 갈래 `claude/mod.rs:184-194` 에 없다). 그러면 벤더는 `stream_event` 로 글 델타를 흘리고, **그와 별도로** 비지 않은 블록마다 완결 `assistant` 메시지 하나를 그 블록의 `content_block_stop` 보다 먼저 보낸다(조사 §4-2 — 실측 + 공식 문서 = 확실).

프론트 누산기의 `TextDelta` 갈래는 마지막 글 항목에 이어 붙이기만 하고 중복 제거가 없다(`structuredAccumulator.ts:130-141`). 흘린 델타와 완결 본문이 같은 글 항목으로 모이므로, 누군가 완결 본문을 버리지 않으면 **모든 claude 답이 두 벌이 된다**. TRD 는 이 규칙을 load-bearing 이라 적고 ADR 후보로 올렸다(TRD §5-5 끝).

## 결정
표기 — **[고름]** = 사용자 체감이 없는 내부 구현이라 TRD 가 고른 것.

1. **스폰 인자** — StreamJson 갈래에서 `--verbose` 뒤 `--include-partial-messages`. 터미널 갈래는 그대로다.
2. **중복 제거는 번역기 몫이다 — 흘린 블록의 완결 글을 버린다.** `ClaudeStreamDecoder` 에 칸 하나 `PartialMessage { id: Option<String>, open: Option<u64>, streamed: BTreeSet<u64> }` [고름 — 줄 버퍼와 같은 수명(화신 하나)]. `consume_line` 이 인자 `partial: Option<&mut PartialMessage>` 를 받는다 — 라이브는 `Some`, 이어받기는 `None`.

   | 줄 | 규칙 |
   |---|---|
   | `stream_event` + `parent_tool_use_id` 가 null 아님 | 통째로 버린다(`message_start` 포함 — 하위 에이전트 흐름이 부모 상태를 덮지 않게) |
   | `message_start` | 상태를 `{ id: message.id, open: None, streamed: {} }` 로 새로 세운다 — 턴 도중 접힌 입력이 여는 새 메시지도 |
   | `content_block_start{index}` | `open = Some(index)` |
   | `content_block_delta` 의 `text_delta` | `id` 가 있고 글이 비지 않으면 `TextDelta{text, turn_id: None, message_id: id}` · `streamed` 에 `index` · `id` 가 없으면(`message_start` 없이 온 늦은 델타) 버린다 |
   | 그 밖의 델타(`thinking_delta` · `input_json_delta` · `signature_delta`) | 버린다 — 생각·도구는 완결 줄에서 오늘처럼 |
   | `content_block_stop{index}` | `open == Some(index)` 면 `open = None` |
   | `message_delta` · `message_stop` · 모르는 `event.type` | 아무것도 안 낸다 — ★`MessageDone`/`TurnEnd`/`Structured` 금지★ |
   | 완결 `assistant` 줄 | `message.id == partial.id` 이고 `open = Some(k)` 이고 `streamed ∋ k` 면 그 줄의 **`text` 블록만** 내지 않는다. 나머지 블록과 그 밖의 모든 경우(안 흘린 블록 · id 불일치 · `partial` 없음)는 오늘 그대로 |
   | `result` | 오늘 번역 뒤 상태를 비운다 |

   [고름] 「(메시지 id, 블록 번호)」를 여는 블록으로 잡는다 — 완결 줄에는 블록 번호가 없지만, 계약이 「그 블록의 `content_block_stop` 보다 먼저」라 완결 줄이 오는 순간 열려 있는 블록이 곧 그 줄의 블록이다. 빈 블록(완결 줄 없이 멈춤)은 자리를 차지하지 않는다. 벤더가 순서를 바꾸면(멈춘 뒤 완결) `open` 이 비어 **오늘처럼 전문을 낸다 — 잃지 않고 겹친다**(보이는 쪽으로 틀린다).
3. **턴 끝은 `result` 한 줄 그대로다.** 스트림 부속 줄(`message_delta` · `message_stop` 등)을 `Structured` 로 내지 않는다.
4. **기본은 합치지 않는다**(codex 와 같다 — 조사 §4-4). ★단 수치 문턱을 넘으면 B1 안에서 합친다★: B1 채취의 긴 답(≥ 2,000 자) 한 턴의 흘린 델타 수가 **`REPLAY_MAX_EVENTS`(4096 — `output_core.rs:1291`)의 10 %(≈ 410)를 넘으면** 한 `decode()` 호출이 돌려주는 사건 목록 안에서 이웃한 같은 `message_id` 의 `TextDelta` 를 하나로 잇는다(backend 만 · seq 는 emit 때 매겨지므로 구멍이 안 생긴다). 판정은 **실 펌프 청크 기준**이다 — 이웃 합치기는 펌프 한 번 읽기에 여러 줄이 올 때만 줄어들기 때문이다(fixture 청크 기준 아님 · TRD §12 1 라운드 low 3). 판정 수치는 B1 반환에 싣는다.
5. **이어받기 기록(transcript)은 `stream_event` 줄을 통째로 건너뛴다** [고름] — 오늘의 `_ => {}`(`:1003`)다. 기록에 그 줄이 있든 없든 완결 `assistant` 줄이 전문을 내므로 한 벌이다.
6. **선 타입 · 프론트는 바뀌지 않는다** — 흘린 글도 같은 `TextDelta` 어휘다.

## 거부한 대안
- **교체**(새 선 변형 + replay 교체 — vibe-kanban) — 「새 선 변형 + replay 가 교체까지 되감아야 함」(조사 §4-4) · 「주소 붙은 화면 항목이 필요」(조사 §4-3).
- **꼬리 비교**(paseo) — 「상태가 더 많고 중복 모서리」(조사 §4-4 · TRD §10). 피어 비교는 조사 §4-3 에 있다.
- **무조건 합치기** — 수치 문턱(결정 4)을 넘을 때만 짓는다.
- **`message_stop` 을 턴 끝으로 옮긴다** — 도구 호출마다 턴이 끝난다(조사 §4-4 함정 · TRD §5-3).
- **이어받기 기록마다 번역기 상태를 새로 세운다**(지시의 방식) [고름] — 결정 5 가 더 단순하고 기록 모양에 기대지 않는다(TRD §5-4).

## 근거
- **벤더 계약(확실)** — 비지 않은 블록마다 완결 메시지 하나, 그 블록의 `content_block_stop` 보다 먼저(조사 §4-2 — claude 2.1.280 실측 + 공식 문서).
- **링 상한** — `REPLAY_MAX_EVENTS = 4096`(`output_core.rs:1291`). 문턱 10 % 는 TRD §9 가 정한 수치다.
- **리뷰** — TRD 1 라운드 light 재검 low 3(합치기는 실 펌프 청크 기준)이 결정 4 에 반영됐다(TRD §12).
- ★**검증 상태**★ — B1 구현 · 실측 끝(2026-09-27). 시험 = TRD §5-5 의 1–14 · 새 fixture `backend/claude/fixtures/partial_stream_p1.jsonl`(우리 실제 스폰 인자 · `MAX_THINKING_TOKENS=8000` · 제어 채널 없음 · `extra_args` 없음으로 떴다). 채취 = claude CLI 2.1.280 · 모델 claude-opus-5-5(CLI 기본값 — 스폰에 `--model` 이 없다). 실측(채취 한 세션 + 그 세션 이어받기 한 번):
  - **이어받기** — 플래그를 켠 세션의 기록(`~/.claude/projects/<slug>/<sid>.jsonl`)에 `stream_event` 줄이 **0** 이고, 완결 `assistant` 글 블록은 라이브 완결 글과 같은 글자 수로 남아 있다. 같은 인자에 `--resume <sid>` 로 띄운 이어받기가 **같은 session id** 로 이어졌고 앞 대화의 낱말을 되물어 맞혔다.
  - **한가 구간** — 각 `result` 뒤 다음 입력을 쓰기 전 다섯 구간(32.0 s · 6.0 s · 6.0 s · 6.0 s · stdin 을 닫고 끝날 때까지 16.4 s)과 이어받기 턴 뒤 구간에서 최상위 `stream_event` 줄이 **전부 0** 이다(각 구간엔 `command_lifecycle completed` 한 줄뿐). 「턴 끝 뒤 진행 없음」의 벤더 전제(영향 절)가 이 채취에서 섰다.
  - **링 압박**(턴마다 흘린 글 델타 수 / 글자 수) — T1 3 / 17 · T2 9 / 70 · T3 15 / 62 · **T4(긴 답) 519 / 2,428** · T5 2 / 16 · 이어받기 턴 1 / 9. T4 의 519 가 문턱 410 을 넘어 결정 4 의 합치기를 지었다. 그러나 실 펌프와 같은 4096 B 읽기로 흉내 낸 청크에서 합치기는 **519 → 519**(줄지 않음)였다 — 델타마다 제 읽기에 따로 왔다(델타 간격 약 50–80 ms). 흉내는 채취 도구(Node)의 파이프 청크를 4096 B 로 잘라 낸 것이지 우리 Rust 펌프 자체는 아니다. **메인은 이 링 압박을 후속으로 받아들였다**(codex 가 오늘 같은 처지다) — 긴 답 하나가 `REPLAY_MAX_EVENTS` 의 약 13 % 를 쓴다.

## 영향 / 불변식
- **중복 제거가 빠지면 모든 claude 답이 두 벌이다** — 프론트에 제거가 없다. 회귀 시험이 흘린 첫 델타와 완결 본문이 한 벌인지 잰다.
- **턴 끝 뒤 진행 없음** — 흘린 델타는 `TextDelta` 라 진행 신호다. 턴 **안**에서만 나온다: `result` 가 상태를 지우고, `message_start` 없이 온 늦은 델타는 버리므로 턴 끝 뒤에 「턴 중」을 다시 켤 길이 없다. 스트림 부속 줄을 `Structured` 로 내면 claude 턴 분류기가 통째로 진행으로 세어(`:538-545`) 턴 끝 뒤 30 분 막힘 경로가 된다(CLAUDE.md 「대기 입력 상태」 끝 문단).
  - ★이 불변식은 벤더 전제 하나에 기댄다 — **벤더가 턴 밖에서 최상위(`parent_tool_use_id` null) `stream_event` 를 내지 않는다**★. 한가할 때 `message_start` + 글 델타가 오면 번역기는 새 메시지로 받아 `TextDelta` 를 내고, 「턴 중」(과 ADR-0238 의 턴 열림 문)이 켜져 30 분 막힘 경로가 된다. 번역기로는 막을 수 없다(턴 도중 접힌 입력의 새 `message_start` 와 모양이 같다). B1 채취가 확인한다.
- ★**B1 이 멈추고 메인에 올리는 조건**★(TRD §5-4):
  - 이어받기 — 플래그를 켠 세션의 기록에 완결 `assistant` 글 줄이 남아 있지 않으면(부분 줄만 남긴다) 이어받은 화면의 답이 빈다.
  - 한가 구간 — 각 `result` 뒤 다음 입력 전(≥ 30 초 한 구간 포함)에 최상위 `stream_event` 줄이 0 이 아니면.
- **기존 fixture 무수정이 증거다** — 부분 줄 없는 기존 fixture 전부의 사건열이 바이트 단위로 같아야 한다. 기존 시험을 고쳐야 하면 설계가 틀렸다(TRD §5-5 의 11).
- **링 압박** — 토큰 단위 `TextDelta` 로 링이 더 빨리 차 긴 대화의 앞 이력이 replay 에서 더 일찍 밀린다(결정 4 의 문턱이 이것을 잰다).
- **렌더 비용** — 델타마다 `setItems`(`RichSlot.tsx:215`) + 스크롤 따라가기 RO 가 돈다. codex 가 오늘 같은 경로를 이미 탄다(TRD §9).
- ★**알려진 한계 셋**★ (TRD §9 F4 와 같다):
  - (a) **완결 `assistant` 줄이 그 블록의 `content_block_stop` 뒤에 오면 글이 두 벌 나온다** — 벤더 계약(TRD §5-2) 위반일 때다. 의도된 동작이다(「잃기보다 겹친다」): 완결 줄에는 블록 번호가 없어 그 줄이 어느 블록의 것인지 열린 블록으로만 알 수 있고, 꼬리 비교는 거부했다(거부한 대안). 시험 `a_text_block_that_was_not_streamed_falls_back_to_the_completed_text`(멈춘 뒤 완결 갈래)가 이 동작을 못 박는다.
  - (b) **흘린 글은 되돌릴 수 없다** — CLI 가 실패한 스트림을 다시 시도하거나 스트리밍 아닌 요청으로 물러나면, 실패한 시도가 흘린 앞부분이 화면에 남는다. 벤더 동작은 확인하지 않았다.
  - (c) **`/compact` · 자동 compact 는 플래그를 켠 채로 채취하지 않았다** — compact 가 최상위 `stream_event` 를 내면 요약이 채팅에 흘러들 수 있다. 미확인 — GUI QA 에서 확인한다.
- **문서 갱신** — `claude/mod.rs:762-767`(「decoder 자신의 상태 = 줄 재조립뿐」)을 이 칸으로 고친다.
- **PTY 무변경** — 터미널 인자·`TerminalSlot` 은 그대로다.
