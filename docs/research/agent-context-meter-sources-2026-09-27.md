# 조사 — 에이전트 슬롯 하단 표시(컨텍스트 사용량·모델·effort)의 데이터 원천

- **상태:** 조사 적립 — 적대 리뷰(FIX) 반영 완료. **결정 없음**(PRD 선택은 다음 세션 — 사용자 결정 2026-09-27: 「조사만 해 놔, 다음 핸드오프 때」).
- **요구(사용자, 2026-09-27):** Claude 데스크톱 앱처럼 입력창 아래에 **컨텍스트 용량 · 현재 모델 · effort** 를 보이고 싶다(그 앱은 「Opus 5.5 · 엑스트라 · 작은 링」).
- **방법:** research medium · 설계 서베이 모드 — 갈래 셋(피어 · Claude · Codex) 수집 → 메인 grounding(아래 「grounding」) → cross-family 적대 리뷰.
- **대상 버전:** Claude Code CLI **2.1.280**(설치 바이너리 직접 확인) · codex-cli **0.156.1**(app-server 스키마 추출) · openai/codex 소스 HEAD `41f9084b3081`(태그와의 차이 미확인).
- **확신도 범례:** 확실 = 1차 자료 둘 이상이 맞음 · 가능성 높음 = 1차 자료 하나 · 불확실 = 추론이거나 미실측.

## 0. 결론

1. **GUI 피어(Paseo·t3code·vibe-kanban)는 입력창 옆 작은 링/게이지 하나 + 툴팁 used/window 로 모인다** — CLI 쪽(Claude statusline·Codex TUI·ccusage)은 텍스트다. **색 문턱은 제각각**이다(Paseo 70/90 · vibe-kanban 50/75/90 · ccusage 기본 50/80). 모델·effort 는 대개 **자기가 스폰할 때 넘긴 값**을 보인다. (가능성 높음 — ★「전부 같은 모양」으로 적었던 초안은 적대 리뷰가 과장으로 잡았다★)
2. **Claude(JSON 모드)는 필요한 원자료가 이미 스트림에 흐르는데 우리가 버린다.** 사용량 = 가장 최근 `assistant` 메시지의 `message.usage` · 창 크기 = 턴 끝 `result.modelUsage[<모델>].contextWindow` · 모델 = `system/init.model`. 우리는 `result.usage` 의 입력·출력 두 칸과 `rate_limit_event` 만 읽고 `system/init` 은 통째로 건너뛴다(`crates/engram-dashboard-agent/src/backend/claude/mod.rs:863`). (확실)
3. **Codex(app-server)는 한 알림이 사용량과 창 크기를 함께 준다** — `thread/tokenUsage/updated` 의 `tokenUsage.last.totalTokens` · `tokenUsage.modelContextWindow`(nullable). 모델·effort 는 스레드 시작 응답의 `model`·`reasoningEffort`. 우리는 그 알림에서 `last` 의 입력·출력 두 칸만 역직렬화해 나머지를 버린다(`crates/engram-dashboard-agent/src/backend/codex/protocol.rs:634-640`). (확실)
4. **effort 는 두 회사 모두 「실제로 쓰인 값」 신호가 없다** — 설정값만 되돌아온다. Claude 는 `init.effort` 가 **호스트가 싣는 경우에만** 있고 `-p` 에서 실리는지 미실측. (가능성 높음)
5. **터미널 모드는 기계가 읽을 통로가 사실상 없다** — Claude 는 statusline 명령·transcript 파일, Codex 는 세션 기록 파일뿐이고 둘 다 이 저장소가 이미 쓰지 않기로 한 부류다(statusline = 메시지마다 프로세스 · Codex 기록 파일 = ADR-0203). 두 TUI 는 자기 화면에 이미 이 값을 그린다. (확실 — 통로 목록 · 판단은 PRD 몫)

## 1. 피어 (직접 서베이)

| 제품 | 보이는 것 | 사용량·창 크기 원천 | 모델·effort | 출처 · 확신도 |
|---|---|---|---|---|
| Claude Code CLI statusline | 명령이 stdin JSON 을 받아 그림 | `context_window.{used_percentage, context_window_size, current_usage…}` — 문서 원문 "calculated from input tokens only: `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. It does not include `output_tokens`" | `model.{id,display_name}` · `effort.level`(effort 를 지원하는 모델일 때만, `/effort` 변경 반영) | code.claude.com/docs/en/statusline + 바이너리 빌더 · 확실 |
| Codex CLI TUI | "Context N% left" 등 상태줄 항목 | `last_token_usage.total_tokens` · `model_context_window`(없으면 config 값) · **`BASELINE_TOKENS = 12000` 을 분자·분모에서 빼는 정규화** | 상태줄 항목 `ModelWithReasoning` | openai/codex 태그 `rust-v0.156.1` `codex-rs/protocol/src/protocol.rs:2244-2286` · `tui/src/token_usage.rs` · (HEAD `41f9084b3081` 에선 `protocol.rs:2417,2451-2462` · `tui/src/chatwidget/status_controls.rs:405-427`) · 확실 |
| Paseo | 14px SVG 링, >90% 빨강 · ≥70% 앰버 | Claude: 스트림 `message_start`·`message_delta` usage(입력 3종 + 출력) · 창 = `result.modelUsage[*].contextWindow` 최대, 그 전엔 카탈로그 값 · Codex: `last.totalTokens`/`modelContextWindow` | 자기 설정값 · Codex `reasoningEffort` | 로컬 클론 `paseo/packages/app/src/components/context-window-meter.tsx:67-70` · `server/.../codex-app-server-agent.ts:928-930` · 확실 |
| t3code | 컴포저 옆 미터 | Claude(Agent SDK): 입력 3종 + 출력, `min(used, max)` · 창 = `result.modelUsage`, 없으면 마지막 값, 초기값 = 카탈로그(`standard 200k / expanded 1M`) · Codex: `last.totalTokens`/`modelContextWindow` | 자기 설정값 | 로컬 클론 `t3code/apps/server/src/provider/Layers/ClaudeAdapter.ts:706-759,2665-2677` — ★「Avoid getContextUsage because its token-count fallback can make extra model requests」(`:2677`)★ · 확실 |
| vibe-kanban | 게이지 <50/<75/<90/그 이상 4단 · 툴팁 "N% · used/total" | ★**CLI stream-json 을 직접 파싱 — 우리 구조와 가장 가깝다. 단 그 계산을 본받지 말 것**★ — `message_delta.usage` 입력 3종 + 출력(서브에이전트 이벤트 제외). Anthropic 스트리밍 문서의 `message_delta` 예시는 **`output_tokens` 만** 싣는데, 빠진 입력 칸을 0 으로 채워 합을 교체하므로 **출력만 센 과소 계상**이 날 수 있다(`claude.rs:1772` · platform.claude.com/docs/en/build-with-claude/streaming — 적대 리뷰 반증) · 창 기본 `200_000`, `system/init.model` 에 `[1m]` 이면 1M, `result.modelUsage[주 모델].contextWindow` 로 보정 · Codex `last.total_tokens`/`model_context_window` | — | 로컬 클론 `vibe-kanban/crates/executors/src/executors/claude.rs:723,1264` · 확실(`README` 가 서비스 종료를 알린다 — 설계만 본다) |
| Orca | statusline 훅은 `rate_limits` 수집용 | 컨텍스트 미터 구현은 grep 으로 **못 찾음**(없다는 뜻이 아니다) | — | 로컬 클론 `src/main/claude/statusline-script.ts:14-16` · 불확실 |
| herdr | 해당 기능 없음 | — | — | 로컬 grep · 가능성 높음 |
| ccstatusline · ccusage | statusline 스크립트(텍스트) · ccusage 기본 문턱 50/80 | statusline stdin JSON(컨텍스트 대체 계산을 문서화) | statusline stdin | github.com/sirmalloc/ccstatusline `docs/USAGE.md` · github.com/ccusage/ccusage `docs/guide/statusline.md` · ★**적대 리뷰가 누락으로 잡아 이름만 더했다 — 본문은 안 읽었다**★(불확실) |
| Claude 데스크톱/웹 | 모델 · effort · 작은 링 | 원천·임계값 **모름**(공개 자료 미조사) | — | — |

**「사용량」 합산의 두 갈래:** CLI 자신과 statusline 은 **입력 3종만**(출력 제외) · Paseo·t3code·vibe-kanban 은 **입력 3종 + 출력**. 기준은 모두 **마지막 API 응답 한 건**이고 `result.usage` 누적값은 쓰지 않는다. (확실)

## 2. Claude Code CLI 2.1.280

- **CLI 자신의 계산식(바이너리 확인):** `r = input_tokens + cache_creation_input_tokens + cache_read_input_tokens` · `pct = round(r / window × 100)` → 0–100 으로 자름. `e` 는 메시지를 끝에서부터 훑어 처음 만난 usage — **가장 최근 API 응답**. 출력 토큰은 따로 보고하고 % 에 넣지 않는다. (확실 — 바이너리 + statusline 문서)
- **창 크기:** 스트림에서 창 크기가 실리는 곳은 **`result.modelUsage[<모델>].contextWindow` 하나**(스키마 `contextWindow: int`). 첫 턴이 끝나기 전에는 스트림에 없다. CLI 의 판정 규칙: 모델 id 의 `[1m]`(끄는 env `CLAUDE_CODE_DISABLE_1M_CONTEXT`) · `context-1m` 베타 + 지원 모델 · 원래 1M 인 모델 → 1,000,000 · 모르는 모델은 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` · 기본 200,000. (규칙 코드 = 확실 · 보조 함수 각각의 뜻 = 가능성 높음) ★**이 값은 「CLI 가 믿는 한도」이지 검증된 공급자 한도가 아니다**★ — 호환 게이트웨이에선 Claude Code 가 네이티브 1M 을 확인하지 못해 `[1m]` 선택자 없이는 200K 로 보고·예산한다고 Anthropic 이 문서화했다(github.com/anthropics/claude-code-action `docs/configuration.md` — 적대 리뷰).
- **모델:** `system/init.model`(확실) · 각 `assistant.message.model`(API 표준 칸 — 가능성 높음). 표시용 이름(`display_name`)은 statusline 에만 있다. 세션 중 전환은 제어 요청 `set_model`. 전환 뒤 `init` 이 다시 오는지는 **미실측**.
- **effort:** 정하는 법 = `--effort <level>` · env `CLAUDE_CODE_EFFORT_LEVEL` · 설정 `effortLevel`(모델별 가능) · `/effort`. 되돌려 받는 곳 = statusline `effort.level` · **`init.effort`(값이 정의됐을 때만 싣는다 — 바이너리 `if(e.effort!==void 0)…effort=e.effort`, 스키마 설명 "absent on hosts that do not publish it … newest frame wins")** · 제어 요청 `get_settings` → `applied.effort`. `-p` 스트림의 `init` 에 실제로 실리는지 **미실측**. (가능성 높음)
- **압축:** `system/compact_boundary` 의 `compact_metadata.{trigger, pre_tokens, post_tokens?}`. (확실 — 스키마)
- **우리 코드가 읽는 것:** `result.usage.{input_tokens,output_tokens}` → `OutputEvent::Usage`(누적값이라 점유율이 아니다 · 캐시 칸 무시 — `mod.rs:802-825`) · `rate_limit_event`. `assistant.message.usage`·`message.model`·`system/init`·`modelUsage`·`compact_boundary` 는 버린다(`mod.rs:785-800,863`). (확실)

## 3. codex-cli 0.156.1 (app-server)

- **사용량:** `thread/tokenUsage/updated` `{threadId, turnId, tokenUsage:{last, total, modelContextWindow?}}` · `last`/`total` 칸 = `inputTokens · cachedInputTokens · outputTokens · reasoningOutputTokens · totalTokens`(+ `cacheWriteInputTokens` 기본 0). `last.totalTokens` = 최신 활성 컨텍스트 크기, `total` = 세션 누적(상류 TUI 주석 `tui/src/token_usage.rs:37-38` — 스키마엔 설명 없음). `modelContextWindow` 는 **null 일 수 있다**(상류 `TODO make this not optional`). (확실 — 스키마 + paseo·t3code·vibe-kanban 이 같은 칸을 읽음)
- **TUI 의 %:** `BASELINE_TOKENS = 12000`("prompts, tools and space to call compact") — 창 ≤ 12000 이면 0 · 아니면 `eff = window − 12000`, `used = max(last.totalTokens − 12000, 0)`, `left% = round(clamp((eff − used)/eff × 100))`. 그래서 `last.totalTokens ≤ 12000` 인 동안(보통 첫 프롬프트 직후)은 100% left 로 보인다 — 첫 프롬프트가 크면 그보다 낮다. (확실 — 태그 `rust-v0.156.1` 의 `codex-rs/protocol/src/protocol.rs:2244-2286` + `codex-rs/tui/src/token_usage.rs`, HEAD 에서도 같은 식)
- **모델:** `thread/start`·`thread/resume`·`thread/fork` 응답 최상위 `model`·`reasoningEffort`(스키마 확인) · 세션 중 변경 = `turn/start` 의 `model`/`effort` 덮어쓰기 · 변경 통지 = `thread/settings/updated`(`threadSettings.{model, effort}`) · 서버가 실행 모델을 바꾸면 `model/rerouted{fromModel,toModel,reason}`. 전부 **설정값**이고 턴마다의 실행 기록이 아니라고 스키마가 명시한다. (가능성 높음 — 수집자 스키마 판독 · `thread/settings/updated` 스키마 위치는 메인이 재확인 못 함)
- **effort 값의 형식:** 닫힌 enum 이 아니라 「모델이 광고한 비지 않은 문자열」. `model/list` 가 모델별 `defaultReasoningEffort`·`supportedReasoningEfforts` 를 준다(창 크기는 안 준다). (가능성 높음)
- **압축:** 스레드 항목 `type: "contextCompaction"`(옛 `thread/compacted` 는 폐기 예정) · 수동 `thread/compact/start` · 자동 임계는 config `model_auto_compact_token_limit`(알림에 안 실림). (가능성 높음)
- **우리 코드가 읽는 것:** `TokenUsageBreakdown` 이 `input_tokens`·`output_tokens` 두 칸만 선언 → `total`·`modelContextWindow`·캐시·추론·`totalTokens` 는 역직렬화에서 버려진다(`protocol.rs:634-640`) · `thread/settings/updated`·`model/rerouted`·`thread/compacted` 는 무시 목록(`decoder.rs:171,184,205`) · `Thread` 구조체와 시작 응답은 `model`·`reasoningEffort` 를 안 읽는다 · `turn/start` 는 모델·effort 를 덮어쓰지 않는다. (확실)

## 4. PRD 에서 고를 것 (결정 아님 — 다음 세션이 사용자에게 올린다)

1. **대상 모드** — JSON 모드(채팅 슬롯)만 / 터미널 모드까지. 터미널 쪽 통로는 §0-5.
2. **Claude 사용량 합산** — CLI 와 같게 입력 3종만 / 피어처럼 출력까지.
3. **Codex % 식** — TUI 와 같게 12000 기준선을 뺀 값 / 원값(`last.totalTokens / window`). 같은 대화를 TUI 와 대시보드에서 볼 때 숫자가 갈리는지가 걸린다.
4. **첫 턴 전 창 크기** — Claude 는 첫 `result` 전에는 스트림에 창 크기가 없다 → 「—」로 비움 / CLI 규칙(`[1m]`·200k)으로 추정.
5. **effort 의 출처** — 스폰 때 우리가 넘긴 값 / `init.effort`(실리면) / Claude 제어 요청 `get_settings` / Codex 시작 응답·설정 통지. 넘기지 않았고 보고도 없으면 무엇을 보이나.
6. **임계 색** — 사용량 슬롯의 색 토큰을 재사용할지, 피어의 70·90% 문턱을 따를지.

## 5. 거부 후보 (다음 세션이 다시 꺼내지 않게)

- **Claude Agent SDK `getContextUsage`** — ★거부 사유는 「우리 경로가 SDK 가 아니라 CLI stream-json 이라서」다★. t3code 가 「토큰 수 대체 계산이 모델 요청을 더 만든다」며 피했지만(`ClaudeAdapter.ts:2677`), SDK 변경 기록에 추가 요청 없이 마지막 응답 usage·로컬 추정만 쓰는 `getContextUsage({ detail: 'summary' })` 가 생겼다(github.com/anthropics/claude-agent-sdk-typescript `CHANGELOG.md` — 적대 리뷰). 그러니 「추가 요청 때문」을 사유로 되풀이하지 말 것.
- **`result.usage` 누적값으로 점유율 계산** — 턴·세션 누적이라 컨텍스트 점유가 아니다(피어 전원이 안 쓴다 · 우리 현행 `OutputEvent::Usage` 가 이 값이다).
- **터미널 모드용 statusline 명령·transcript·Codex 기록 파일 읽기** — 이 저장소가 이미 같은 부류를 거부했다(ADR-0203 등). 되살리려면 그 결정부터 다시 연다.

## grounding (메인 외부 대조 — 2026-09-27)

| 클레임 | 대조한 것 | 판정 |
|---|---|---|
| CLI % 식 = 입력 3종 / 창 | 바이너리 `…input_tokens+e.cache_creation_input_tokens+e.cache_read_input_tokens,s=Math.round(r/n*100),g=Math.min(100,Math.max(0,s))…` | 지지 |
| `init.effort` 는 정의됐을 때만 | 바이너리 `if(e.effort!==void 0)i.effort=e.effort` | 지지 |
| `modelUsage.contextWindow` 스키마 | 바이너리 `costUSD:k(),contextWindow:k().int(),maxOutputTokens:k().int()` | 지지 |
| 우리 Claude 디코더가 `system/init` 을 건너뜀 | `backend/claude/mod.rs:863` 주석 "system/init·thinking_tokens 등 메타 라인 … skip" | 지지 |
| vibe-kanban 200k 기본 · `[1m]` → 1M | `claude.rs:723` · `:1264` | 지지 |
| t3code 「Avoid getContextUsage」 | `ClaudeAdapter.ts:2677` | 지지 |
| paseo Codex = `last.totalTokens` · `modelContextWindow` · 링 90/70 | `codex-app-server-agent.ts:928,930` · `context-window-meter.tsx:67,70` | 지지 |
| Codex 알림에 `modelContextWindow` | 추출 스키마 `ThreadTokenUsageUpdatedNotification.json` | 지지 |
| Codex TUI `BASELINE_TOKENS = 12000` | 받아 둔 상류 `protocol.rs:2417,2452` | 지지 |
| 시작 응답에 `model`·`reasoningEffort` · `turn/start` 에 `effort` | 추출 스키마 `ThreadStartResponse.json` · `TurnStartParams.json` | 지지 |
| 우리 Codex 역직렬화가 두 칸만 | `backend/codex/protocol.rs:634-640` | 지지 |
| 무시 목록에 `model/rerouted`·`thread/settings/updated` | `backend/codex/decoder.rs:171,205` | 지지 |
| statusline 문서 원문(입력만) | 수집자 인용 — 메인이 웹 문서를 직접 열지는 않음(바이너리 식이 같은 내용을 지지) | 부분지지 |

## 적대 리뷰 (cross-family · codex · 2026-09-27)

판정 **FIX** → 7건 전부 반영했다(반증 출처가 붙은 것만 채택 — 반영 자리마다 「적대 리뷰」 표기).

| 지적 | 결함 유형 | 처리 |
|---|---|---|
| 「피어는 전부 같은 모양·문턱」 | 과장 | §0-1 을 GUI 피어로 좁히고 문턱을 제각각으로 고침 |
| vibe-kanban `message_delta` 합산을 모범으로 | confident-wrong · 논리 공백 | 표에 과소 계상 경고 · 「본받지 말 것」 |
| `modelUsage.contextWindow` 를 믿을 분모로 | vendor caveat 누락 | §2 에 「CLI 가 믿는 한도 ≠ 공급자 한도」 |
| `getContextUsage` 거부 사유가 낡음 | 과장 · 누락 | §5 사유를 「CLI 경로라서」로 교체 |
| Codex 「첫 프롬프트 직후 100%」 | 과장 | 조건부(`last.totalTokens ≤ 12000`)로 |
| 0.156.1 동작에 HEAD 줄 번호 | 오귀속 인용 | 태그 고정 줄 번호로 교체(HEAD 는 괄호 병기) |
| ccstatusline·ccusage 누락 | 완전성 | 표에 이름만 추가(본문 미독 — 불확실) · 공백에 남김 |

**리뷰어 재대조(스팟):** statusline 식(입력 3종·출력 제외) = 지지 · Codex 알림 `last`/`total`/nullable `modelContextWindow`(0.156.1 스키마) = 지지 · Codex 12000 기준선(0.156.1 TUI) = 지지(첫 프롬프트 결론은 조건부) · 우리 디코더의 `system/init` 건너뜀 = 지지 · vibe-kanban `message_delta` = 부분 지지.

## 공백 · 한계

- `-p` 스트림의 `init` 에 `effort` 가 실리는지 · `set_model` 뒤 `init` 이 다시 오는지 — **실 1회 실행으로 확인 필요**.
- Codex 첫 `thread/tokenUsage/updated` 가 언제 오는지(첫 턴 전? resume 직후?) · `thread/settings/updated` 가 언제 발화하는지 — 미실측. 푸터 초기값이 여기에 달렸다.
- Codex TUI 가 창 크기에 `effective_context_window_percent`(기본 95) 보정을 거친 값을 쓰는지 — 부분 추적.
- Claude 데스크톱 앱의 원천·임계값 · ccstatusline·ccusage 본문(컨텍스트 대체 계산) · Claude 내장 자동 압축 경고 임계 — 미조사.
- ★**Claude 사용량은 `assistant.message.usage`(완전한 한 벌)에서 읽는 것이 안전하다고 보이지만 미실측**★ — 스트림 `message_delta` 는 출력만 실을 수 있다(위 vibe-kanban 경고). 우리 JSON 모드에서 `assistant` 줄에 입력 3종이 다 실리는지 실 1회로 확인할 것.
- 우리 Codex 시험 고정값 `modelContextWindow: 272000`(`decoder.rs:1424`)이 실측값인지 모른다.
- 상류 openai/codex 는 HEAD 기준이다 — 0.156.1 태그와의 차이 미확인.
