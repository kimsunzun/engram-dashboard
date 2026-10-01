# ADR-0243: claude 끊김 합성 줄은 말풍선이 아니라 중단 표시로 그리고 그 턴의 중단 행은 그리지 않는다

- 상태: 확정 (2026-09-28, 근거: 사용자 결정 ⑫ (TRD `docs/process/S21-chat-ux/trd.md` §11) + 메인 판정 a–d (사용자 위임) + B2 스파이크 실측(`docs/research/claude-interrupt-spike-2026-09-28.md` §1 · §10-1) · 구현 전 · 코드 무변경)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§3-4 · §7 B3 · FE-2c · 접점 표 · §8-2 F2 ② · §10 0243 · §11 ⑪ ⑫) · 조사 `docs/research/claude-interrupt-spike-2026-09-28.md`(§1 · §10-1) · ADR-0237(결정 6 U8 — 강조된 「중단됨」 행) · ADR-0238(결정 4 — 끊긴 `result` → `TurnEnd{Interrupted}` · 이 ADR 은 그 분류를 바꾸지 않는다) · ADR-0241(결정 12 — claude 끊긴 도구 행) · ADR-0004(backend 지식 격리) · ADR-0127(턴 관측 · 30 분 fail-open) · step-log S21 · Amends ADR-0237 (결정 6 claude 는 표시 행이 강조를 지고 그 턴 중단 행 생략) · Amends ADR-0238 (결정 4 프론트 무변경 서술과 결정 7 증상과 영향의 끝은 중단 행 서술 claude 는 끊김 표시 행이고 중단 행은 대비로만) · Amends ADR-0241 (결정 12 claude 끊긴 도구 행 아래 맥락은 끊김 표시 행이 준다)

## 맥락
B2 스파이크가 TRD 가 예상하지 못한 줄 하나를 찾았다. 끊긴 claude 턴마다(관측 14/14) CLI 가 그 턴의 `result` **앞에** 합성 사용자 줄 `{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},…}` 을 낸다 — 도구가 돌던 중이면 글이 `[Request interrupted by user for tool use]` 다. transcript 에도 남는다(조사 §1 · §10-1).

오늘 claude 번역기는 이 줄을 `Structured{kind:"user"}` 로 내고, 프론트는 **사용자 말풍선**을 그린다 — 라이브에서도, 다시 연 이력에서도. 그래서 TRD §3-4 의 「codex 끊김과 같은 모양 · 프론트 무변경」과 U8(ADR-0237 결정 6)의 화면이 이 말풍선 하나만큼 달라진다.

이 줄의 알려진 역할은 둘이다. ① 다음 턴에 모델에게 「앞 요청이 끊겼다」를 알린다 — claude CLI 가 스스로 모델에 보내므로 우리 표시 선택과 무관하다(가능성 높음 — 추론: transcript 에 사용자 턴으로 남는다 · 스파이크는 재지 않았다). ② Claude Code 자신의 UI 는 이 줄을 알아보고 말풍선 대신 「Interrupted · What should Claude do instead?」를 보인다. 같은 줄이 다른 상황에도 쓰이는지는 **모른다**(스파이크는 끊기 두 경우만 쟀다).

끊김 판정 자체는 이 줄에서 오지 않는다 — `result` 의 `terminal_reason ∈ {"aborted_streaming","aborted_tools"}` → `TurnEnd{Interrupted}`(오류 아님 · 우편 멈춤 불변 — ADR-0238 · TRD §3-4).

## 결정
표기 — **[사용자]** = 사용자가 답한 결정 · **[메인]** = 오케스트레이터 판정(사용자 위임 — 「너가 생각하는 방향으로 하고 나중에 보고 개선하면 되지」 · 뒤집을 수 있다).

사용자 원문(2026-09-28, Claude Code 의 「Interrupted · What should Claude do instead?」 화면 스크린숏을 가리킨 뒤): 「이게 정석이긴한데」 · 「어쨋든 그냥 표기하는게좋을것같음. 물론 말풍선은 제거하고, 그리고 오히려 응답이 중단되었습니다.를 빼는게 맞는것같은데? Claude에 한해서」.

1. **claude 한정 — 합성 줄은 사용자 말풍선이 아니라 그 줄의 원문을 싣는 중단 표시 행으로 그린다** [사용자] — 라이브와 다시 연 이력 둘 다.
2. **claude 한정 — 그 표시가 이미 있는 턴에는 `result` 에서 온 「응답이 중단됐습니다」 결말 행을 그리지 않는다** [사용자]. codex 는 바뀌지 않는다(그런 줄이 없다 · 그 중단 행과 U8 강조는 그대로).
3. **ADR-0238 의 턴 끝 분류는 그대로다** [사용자] — 표시 행은 표시일 뿐이다. 끊김 판정 · 오류 뒤 멈춤 · 턴 관측은 `result` 가 정한다.

구현 · 표시 세부 [메인]:

- **a. 알아보기는 claude 번역기(`backend/claude`)에 산다**(CLAUDE.md 「백엔드 확장」). 조건 = `user` 줄의 `message.content` 가 **배열이고 `text` 블록 정확히 하나**이며 그 글이 `[Request interrupted by user` 로 시작하고, **`isReplay: true` 가 아니다**. 평문 문자열 content 는 보지 않는다 — 오늘 `consume_line` 이 배열이 아닌 content 의 `user` 줄을 버리고(`backend/claude/mod.rs` 의 `content` 배열 매치 — `:973-976`@HEAD) 그 모양은 관측된 적이 없다. ★`isReplay: true` 는 뺀다★ — 사용자가 그 접두로 시작하는 글을 치면 우리 uuid 를 단 되울림(replay)이 오는데, 그것을 바꾸면 누산기의 대기 행 검사 · uuid dedup 을 건너뛰고 진행 신호도 사라진다. 진짜 합성 줄에는 `isReplay` 가 없다(조사 §0 · §1). 그 줄은 `Structured{kind:"user"}` 대신 **`Structured{kind:"interrupted", json:{"text":"<원문>"}}`** 로 낸다(`json` 칸은 오늘처럼 직렬화된 JSON 글 — `crates/engram-dashboard-agent/src/types.rs:91` — 이고 그 내용이 이 객체다). 벤더 문자열 판별이다 — 벤더가 문구를 바꾸면 오늘의 말풍선으로 돌아간다(여전히 보이고 잃는 것은 없다).
- **b. `classify_turn` 에서 이 사건은 `None`**(진행 아님). 끝에 대한 표시이지 진행이 아니다. 만약 `result` 뒤에 오면 진행이 「턴 중」을 다시 켜 30 분 동안 우편이 막힌다(ADR-0127). 이 줄은 턴이 이미 열려 있을 때만 온다(실측 14/14 · claude 2.1.280 · 가능성 높음).
- **c. 프론트 누산기는 이것을 `interruptNote` 항목 `{ kind:'interruptNote'; text: string; itemId: number }` 으로 옮긴다.** `TurnEnd{Interrupted}` 에서 **현재 턴(마지막 구분선 뒤의 항목)에 `interruptNote` 가 이미 있을 때만** 결말 행을 건너뛴다(구분선은 그대로). 대비 = 그 줄이 끝내 안 오면 중단 행이 그대로 선다. ★순서 전제★ — 표시 행은 그 턴의 `result` **앞에** 온다(실측 14/14 · claude 2.1.280). 만약 그 턴의 구분선 뒤에 오면 다음 턴에 앉아, 그 턴의 대비 중단 행을 지울 수 있다. ★프론트는 표시 행의 유무로 가른다 — 「claude 인가」로 가르지 않는다★.
- **d. 표시 행은 U8 중단 행과 같은 모양**(아이콘 `CircleStop` · 굵게 · `var(--accent)` — ADR-0237 결정 6)이고, 글은 원문 그대로(마크다운 없이) 그린다.

## 거부한 대안
**사용자가 거부** (사유는 위 원문이 말하는 것뿐이다 — Claude Code 식으로 보이고, 말풍선은 없애고, claude 에서는 우리 행을 뺀다):
- **번역기에서 그 줄을 숨긴다**(메인의 앞선 추천) — 다시 연 이력에 끊김 표지가 남지 않는다.
- **라이브에서는 숨기고 다시 연 이력에서만 중단 행으로 보인다.**
- **사용자 말풍선으로 둔다**(오늘 모양).

**메인이 거부:**
- **위치로 알아본다**(`result` 바로 앞의 사용자 글 줄) — 다시 연 transcript 에는 `result` 가 없어 이력에서는 표시가 영영 서지 않는다.
- **새 선 변형으로 싣는다(ADR-0241 `ToolResult` 선례)** — `Structured` 의 새 `kind` 는 wire 타입 · 생성물 · `PROTOCOL_VERSION`/판차 처리가 필요 없고, 옛 셸은 탈출구 항목으로 받아 접힌 `interrupted` 상자(`GenericItemRow` — `StructuredTextView.tsx:336-360`@HEAD)를 보이고, 펼치면 글이 JSON(`{"text":…}`)으로 보인다 — 읽을 수는 있다.
- **「claude 한정」을 프론트에서 에이전트 종류로 가른다** — backend 지식이 프론트로 샌다(ADR-0004) · 줄이 안 왔을 때의 대비(결정 c)를 잃는다.

## 근거
- **사용자 결정 ⑫(2026-09-28)** — 원문은 「결정」 머리. 기록 = TRD §11 ⑫.
- **B2 스파이크 실측** — 끊긴 턴 14/14 에서 `result` 앞에 온다 · transcript 에 `user` 줄로 남는다 · 오늘 번역은 `Structured{kind:"user"}`(조사 §1 · §9 · §10-1).
- **메인 판정 a–d** — 사용자 위임(위 인용). 뒤집을 수 있다.
- ★**검증 상태**★ — 구현 전(코드 무변경). 이 줄이 끊기 밖의 상황에도 쓰이는지는 **모른다**. GUI 실측 계획 = TRD §8-2 F2 ②.

## 영향 / 불변식
- **벤더 문자열에 기댄다** — 판별 접두 `[Request interrupted by user` 를 벤더가 바꾸면 말풍선으로 돌아간다(조용한 퇴행이지만 정보 손실은 없다). 그때 claude 턴은 말풍선 + 중단 행(대비)으로 보인다. ★TRD §11 ⑪ · ADR-0241 이 claude 끊긴 `tool_result` 를 본문 글로 판별하기를 거부한 것과 모순이 아니다★ [메인] — 못 알아보는 쪽(벤더가 문구를 바꿈)은 두 경우 모두 오늘 모양으로 돌아갈 뿐이라 차이가 아니다. 차이는 **잘못 알아보는 쪽**이다: 여기서는 잘못 알아본 줄도 그 원문을 그대로 보여 거짓 뜻을 말하지 않는다. ⑪ 이 거부한 규칙은 진짜 실패에 「중단됨」을 붙이게 되고, 그 본문 「The user doesn't want to proceed…」는 권한 거절의 글이기도 하다(가능성 높음 — 본문이 「…The tool use was rejected…」다 · 잰 적 없다).
- **다시 연 claude 이력에 끊김 표지가 남는다** — 이력에는 `result` 가 없어(`backend/claude/mod.rs` 의 `parse_transcript_events` 주석 — `:1536`@HEAD 「실측 2026-08-17 — 최근 transcript 12개 전부 0건」 · 메인의 2026-09-28 확인 — S1 스파이크 세션 transcript 에도 `result` 줄이 없다. fixture `claude_transcript.jsonl` 의 `result` 줄은 손으로 지은 것(자리표시 uuid `1111…`)이고 그 주석은 이를 「픽스처·미래 claude 가 result 를 남기는 경우」로 다룬다 — 이력에 `result` 가 와도 결정 c 가 결말 행을 건너뛰므로 표시는 같다) 오늘은 결말 행이 없지만, 이 줄은 transcript 에 남으므로 표시 행이 선다.
- **알려진 한계 — 다시 연 이력에서는 `isReplay` 제외가 듣지 않는다** — transcript 의 `user` 줄에는 `isReplay` 칸이 없다(메인 확인 2026-09-28 — S1 스파이크 세션 transcript 의 `user` 줄 6 개 전부 칸 없음). 그래서 사용자가 `[Request interrupted by user` 로 시작하는 글을 직접 쳤다면, 라이브에서는 말풍선이지만 다시 연 이력에서는 끊김 표시 행으로 보인다. 글은 그대로 보이고 턴 관측은 이력 경로를 타지 않으므로 잃는 것은 없다 — 받아들인다.
- **중단 행은 codex 에서, 그리고 claude 의 대비로 남는다** — ADR-0237 결정 6 의 강조는 그 행에 그대로다. claude 에서 표시 행이 있으면 강조는 표시 행이 진다.
- **턴 관측 불변** — `Structured{kind:"interrupted"}` 는 두 턴 관측 정리 지점 · 오류 뒤 멈춤 · 대기 입력 사건 어디에도 들지 않는다(`classify_turn` `None`). 진행으로 바꾸지 말 것(결정 b).
- **턴 열림 문(TRD §3-4)을 열지 않는다** — 문은 진행 신호에서만 열리므로 이 사건은 문을 건드리지 않는다. 줄이 오는 때는 문이 이미 열려 있다(실측 14/14 · claude 2.1.280 · 가능성 높음).
- **새 선 변형이 아니다** — 기존 `Structured` 의 새 `kind` 문자열이라 wire 타입 · 생성물 · `PROTOCOL_VERSION` 이 바뀌지 않는다. 옛 셸(FE-2c 전)은 모르는 `kind` 를 누산기의 탈출구 항목(`kind:'structured'` — `src/components/slot/structuredAccumulator.ts` `consume` 의 `Structured` 갈래 끝 push · `:252-253`@HEAD 「알 수 없는 종류(kind)도 흘려 유실 방지」)으로 받고 중단 행도 그대로 그린다. 그 항목은 `StructuredTextView.tsx` 가 `GenericItemRow`(label + json · 레일 `assistant` — `:463-467`@HEAD)로 그린다 — 접힌 `interrupted` 상자(`:336-360`@HEAD)이고, 펼치면 글이 JSON(`{"text":…}`)으로 보인다.
- **ADR-0241 의 탈출구 기각 사유(「같은 두 분류기가 `Structured` 를 통째로 진행으로 센다」)는 여기서 claude 분류기 안의 `kind` 예외 하나로 풀린다** — codex 분류기는 바뀌지 않는다. 같은 전제를 적은 `types.rs` 의 `QueuedInput` doc(`:94-95`) · `classify_turn` doc 은 B3 이 함께 고친다(TRD §7 B3 · §10).
- **`user` 갈래의 부수 동작을 타지 않는다** — 누산기의 `kind === 'user'` 갈래는 uuid dedup 과 `turnDone = false`(새 유저 턴 시작)를 한다(`structuredAccumulator.ts:233-251`). 합성 줄은 새 유저 턴이 아니므로 `interruptNote` 갈래는 이 둘을 하지 않는다.
