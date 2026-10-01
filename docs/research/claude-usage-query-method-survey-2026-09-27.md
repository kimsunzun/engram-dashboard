# Claude 사용량을 능동 조회하는 방법 — 피어 서베이 (medium)

| 항목 | 값 |
|---|---|
| **상태** | 적대 리뷰(FIX) 반영판 — 반영 기록은 §6 · A 탐침 결과 = §7(2026-09-27) |
| **날짜** | 2026-09-27 |
| **방법** | 수집 2갈래(피어 코드 · 공식 문서·정책) + 메인 grounding 스팟체크(§5) + cross-family 적대 리뷰 1회(§6) |
| **앞 문서** | `docs/research/usage-limit-display-ui-survey-2026-09-26.md` 「사용자 결정」 — 갱신 = 대화 중 줍기 + 쿨타임(5분) 뒤 자동 조회 + 수동 ⟳. 이 문서는 그 **자동 조회를 Claude 에서 무엇으로 하나**를 본다 |
| **확신도** | 확실 = 독립 출처 둘 이상 · 가능성 높음 = 출처 하나 · 불확실 = 미지지·추론 |
| **정책 주의** | 이 문서는 약관·정책 문장을 **인용만** 한다. 해석·적용 판단은 법무·보안 담당 부서 몫이다 |

## 0. 결론

1. **피어 대부분은 비공개 OAuth 사용량 주소를 직접 부른다(B).** `GET https://api.anthropic.com/api/oauth/usage` + `Authorization: Bearer <토큰>` + `anthropic-beta: oauth-2025-04-20`. Orca·Paseo·CodexBar·jean·claude-usage-widget·CodeZeno 가 1순위로, ClaudeBar 가 보조로 쓴다. 토큰은 **Claude Code 가 저장한 것을 앱이 직접 읽는다**(`~/.claude/.credentials.json`, macOS Keychain, `CLAUDE_CODE_OAUTH_TOKEN`). 여럿이 `User-Agent: claude-code/2.1.x` 를 흉내 낸다(Orca `src/main/rate-limits/claude-fetcher.ts:46-48,358-360`). 일부(ClaudeBar·jean·Orca 관리 계정)는 Claude Code 의 공개 client_id 로 **토큰 갱신까지 스스로** 한다. (확실 — Orca 스팟체크)
2. **t3code 는 SDK `get_usage`(A)를 쓴다.** 프롬프트를 절대 내보내지 않는 claude SDK 세션을 띄워 초기화만 끝내고 `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` 를 부른다 — 주석: "사용자 메시지가 stdin 에 한 번도 쓰이지 않아 … Anthropic 에 API 요청을 시작하지 않는다"(`apps/server/src/provider/Layers/ClaudeProvider.ts:318-367`). 로그인은 claude 프로세스가 스스로 하므로 **앱이 토큰을 만지지 않는다.** 5분 캐시. (확실 — 스팟체크) 단 t3code 는 **SDK 라이브러리를 거쳐** 부른다 — SDK 없이 CLI 에 직접 보내는 경로의 증거는 아니다(§1). 또 "API 요청 없음"은 **추론(모델) 요청이 없다**는 뜻이고, 사용량 값 자체는 SDK 타입 주석대로 "claude.ai usage endpoint 에서" 온다(`sdk.d.ts:4232`) — 네트워크 호출은 있다.
3. **사용량 주소는 조회 제한이 세다 — "벌칙 상자"처럼 군다.** ClaudeBar: "조용한 기간 뒤 단 한 번의 호출이 1시간짜리 `Retry-After` 를 부른 적이 있다 … 5분 캐시로도 걸렸다" → 성공값 15분 캐시·API 모드 하한 15분(`docs/providers/claude/design.md:61-62`). **이 15분은 운용 선택이지 안전이 증명된 간격이 아니다.** Orca: 기본 15분 폴링, 레이트리밋 중엔 이전 값을 24시간 유지. Claude Code 자신도 CHANGELOG 에 "usage endpoint 가 rate-limited 일 때 마지막 값을 'as of' 로 보여 준다"(2.1.208)·"한 기계의 세션들이 최근 1분 안의 읽기를 공유한다"(2.1.275)를 적었다. 사용자 이슈에는 10분 간격도 `Retry-After` 없이 한 시간 안에 계속 429 라는 보고(anthropics/claude-code#31637)와 `Retry-After: 0` 인 채 429 가 이어졌다는 보고(#30930)가 있다(공식 수치는 없음). (확실 — 조회 제한이 있다는 것 / 불확실 — 정확한 한도·안전 간격)
4. **`get_usage`(A)가 B 와 같은 제한을 공유한다는 간접 증거가 여럿이다 — 정확한 버킷은 미확인.** ① SDK 타입이 값의 출처를 "claude.ai usage endpoint" 라 적는다(`sdk.d.ts:4232`) ② ClaudeBar 는 CLI `/usage` 와 자기 직접 호출이 같은 백엔드라 보고 직접 호출 429 뒤 CLI 로 넘어가지 않는다(`docs/providers/claude/design.md` 18행 — 리뷰어 인용) ③ CLI 2.1.275 가 한 기계 세션들의 최근 1분 읽기를 공유한다. 어느 것도 서버 쪽 버킷을 증명하지 않는다. 그 1분 공유 캐시는 A 쪽에만 이득이다. (가능성 높음 — 공유 / 불확실 — 버킷)
5. **정책 문장(인용만):** Claude Code 법률·준수 문서(`code.claude.com/docs/en/legal-and-compliance`)는 OAuth 인증이 "Claude Code 와 다른 네이티브 Anthropic 앱의 일상적 사용을 위해 설계됐다"고 하고, "개발자는 Claude.ai 자격증명이나 세션 토큰을 수집·저장·중개할 수 없다"고 적는다. **같은 항목은 그보다 앞서 "Agent SDK 를 쓰는 개발자를 포함해 Claude 의 기능과 상호작용하는 제품을 만드는 개발자는 API 키 인증을 써야 한다(should)"고 적고, 이어서 제3자 개발자가 사용자 대신 Free·Pro·Max 플랜 자격증명으로 요청을 중계하는 것을 허용하지 않는다고 적는다.** 같은 문서는 "최종 사용자가 **수정하지 않은 Claude Code 바이너리**에 자기 구독으로 로그인하는 것을 막지 않는다"고도 적는다. B(앱이 토큰을 직접 읽어 호출)는 자격증명 문장이 이름 붙인 모양과 같고, API 키 문장은 Agent SDK 를 이름으로 들어 A 도 검토 범위에 든다. A 가 어느 쪽에 드는지는 이 문서가 판단하지 않는다 — **법무·보안 확인 대상.** (확실 — 다섯 문장 모두 존재, 메인이 원문 대조 2026-09-27)
6. **공개된 공식 조회 API 는 없다.** 문서화된 경로는 statusline 의 `rate_limits`(TUI 전용 — 앞 문서 결정으로 쓰지 않음)와 실험 기능 `get_usage` 뿐이다. (가능성 높음 — 없음의 증명은 아니다)
7. **C(짧은 프롬프트)·D(`/usage` 화면 긁기)·E(claude.ai 쿠키)는 보조로만 쓰인다.** C 는 CodeZeno 만(`max_tokens: 1` Haiku — 사용량을 쓴다), D 는 ClaudeBar 기본·Orca/CodexBar 보조(시작 대기 2초 고정 등 깨지기 쉬움), E 는 CodexBar 만.

## 1. 우리에게 주는 뜻

- **A 가 우리 제약에 가장 맞는다 — 단 raw stream-json 탐침이 A 를 고르는 선행 조건이다.** ★**→ 탐침 완료(§7, 2026-09-27): ①②③ 는 동작 확인(초기화 불요 · 첫 턴 없이 응답), ⑤ 는 모델 호출 흔적 없음(가능성 높음). 아래는 탐침 전 기록이다.**★ 토큰을 우리가 읽지 않고, 우리가 이미 띄우는 공식 CLI 에 묻는다. 그러나 t3code 는 SDK 를 거치므로 우리 경로의 증거가 아니다. 탐침으로 볼 것: ① 우리 데몬(Rust)이 SDK 라이브러리 없이 **CLI stream-json 에 `control_request{subtype:"get_usage"}` 를 직접 보내서** 되는지(CLI 2.1.208 릴리스 노트는 headless stream-json 이 control request 일반을 받는다고만 하고 이 subtype 은 적지 않는다 — 미실측) ② `initialize` 가 먼저 필요한지 ③ 첫 턴 전에도 되는지 ④ 응답 처리(§3 의 null·빠짐) ⑤ 사용량을 안 쓰는지(모델 호출이 없으니 안 쓸 것으로 **추론** — 공식 문장은 찾지 못함). 정책 쪽은 §0-5 의 Agent SDK·API 키 문장이 A 에도 닿는다 — 적용 판단은 법무·보안 몫.
- **Claude 쿨타임 5분은 피어 증거와 부딪힌다.** 성숙한 피어는 Claude 조회 하한을 15분에 둔다(Orca·ClaudeBar). A 는 같은 제한을 공유할 가능성이 높아(§0-4) 5분 주기 조회는 429 를 부를 수 있다. → **Claude 만 15분에서 시작**이 증거에 맞는 쪽이다. 단 15분은 **출발점이지 안전 간격이 아니다** — 조용한 기간 뒤 한 번 호출에 1시간 거절을 받은 사례가 있다(§0-3). 계정 단위로 조율한다(한 계정에 조회 하나만). Codex(로컬 app-server)는 5분 그대로.
- **429 처리는 `Retry-After` 존중에서 멈추지 않는다.** 값이 없거나·깨졌거나·과거거나·`0` 이면(#30930) 대체 대기를 쓴다(ClaudeBar = 5분). 거절 중엔 **자동·수동(⟳) 모두 즉시 재조회하지 않고** 이전 값을 유지한다.
- JSON 모드 Claude 에이전트가 이미 떠 있으면 **그 프로세스에 `get_usage` 를 보낼 수 있는지**도 볼 가치가 있다(새 프로세스를 안 띄운다 — TRD 확인 사항).

## 2. 피어 표

| 피어 | 방법(순서) | 인증 | 주기·429 |
|---|---|---|---|
| t3code | A (+ 대화 중 `rate_limit_event` 병합) | claude 프로세스가 스스로 | 5분 캐시·5분 헬스 갱신 · 429 처리 없음 |
| Orca | B → 자격증명 재읽기·갱신 후 B → D · statusline 수신 | Keychain → `.credentials.json` | 15분 · 디바운스 5분 · `Retry-After` 존중 · 레이트리밋 중 24시간 유지 |
| Paseo | B | `.credentials.json` → Keychain · 읽기만(갱신은 CLI 몫) | 5분 캐시 · 429 모름 |
| CodexBar | B → D → E | Keychain/`.credentials.json` · 갱신은 `claude /status` 에 맡김 | 적응형 · 토큰별 429 게이트(기본 5분) |
| ClaudeBar | D ↔ B | `.credentials.json` → Keychain → env · 스스로 갱신 | 성공 15분 캐시 · 하한 15분 · `Retry-After`(없으면 5분) |
| jean | B + 자기 실행의 `rate_limit_event` 병합 | Keychain → `.credentials.json` · 스스로 갱신 | 5분 캐시 · 이벤트가 오는 동안 B 는 30분에 한 번 · 429 쿨다운 |
| claude-usage-widget | B | env → `.credentials.json` → Keychain · 갱신 없음 | 60초 → 300초 백오프 · 429 두 번 재시도 |
| CodeZeno (Windows) | B → C | `.credentials.json` 등 · 만료 시 `claude -p .`(사용량 씀) | 설정형 · `Retry-After` 최대 24시간 |

## 3. `get_usage` 모양 (SDK 타입 `sdk.d.ts` 기준)

- 요청: `{type:"control_request", request_id, request:{subtype:"get_usage", skip_behaviors?:boolean}}` — `skip_behaviors` 는 "사용량 미터처럼 플랜 한도만 필요한 호출자용"(7일치 기록 스캔을 건너뜀).
- 응답: `subscription_type`(pro/max/team/enterprise/null) · `rate_limits_available` · `rate_limits`(null 가능){five_hour, seven_day, seven_day_opus, seven_day_sonnet, seven_day_oauth_apps: {utilization 0–100 | null, resets_at ISO 8601 | null}, model_scoped[](없을 수 있고 허용 목록으로 걸러짐 — `sdk.d.ts:4285-4295`), extra_usage} · `session{…}`.
- **null·빠짐은 정상 응답이다**(`sdk.d.ts:4234-4254,4303`) — 없는 값을 0% 로 그리지 않는다.
- 표시: "EXPERIMENTAL: this API is unstable and may change or be removed in any release without notice"(`sdk.d.ts:3041`).

## 4. 한계

- **미실측:** A 가 B 와 같은 429 예산을 쓰는지(간접 증거만 — §0-4) · 사용량 소모 여부(모델 호출 흔적 없음까지만 — §7) · 떠 있는 에이전트 프로세스에 보내는 경로. (CLI stream-json 직접 전송 · `initialize` 필요 여부 · 첫 턴 전 동작은 §7 탐침으로 확인됐다.)
- 조회 제한 수치는 피어·사용자 관찰뿐이다(공식 수치 없음). 관찰끼리도 갈린다(5분이 버텼다는 보고 vs 10분도 429 라는 보고). 15분도 안전이 증명된 간격이 아니다.
- 약관 해석은 하지 않았다. 언론 보도(2026-02 문구 변경·차단 조치)는 1차 출처로 확인하지 않았다.
- 피어 소스 시점: t3code `5378f87f9`(2026-09-18) · Orca `df7460af`(2026-08-20) · Paseo `bbe3f17`(2026-08-25) · 그 밖은 2026-09-27 기본 브랜치.

## 5. grounding (메인 대조)

| 클레임 | 대조 | 판정 |
|---|---|---|
| t3code 가 프롬프트 없는 SDK 세션으로 `get_usage` 를 부르고 "API 요청을 시작하지 않는다" | 로컬 클론 `ClaudeProvider.ts:318-367` 원문 | 지지 |
| Orca 가 OAuth 주소 + beta 헤더 + `claude-code/2.1.0` UA 로 부른다 | 로컬 클론 `claude-fetcher.ts:46-48,358-360` | 지지 |
| ClaudeBar 의 "벌칙 상자"·15분 하한 | `gh api` 로 `design.md:61-62` 원문 | 지지 |
| `get_usage` 요청 모양·출처 주석·EXPERIMENTAL | `sdk.d.ts:3041,4201-4205,4232` | 지지 |
| CLI 가 1분 안의 사용량 읽기를 공유 | CHANGELOG 원문 grep | 지지 |
| 법률·준수 문서의 세 문장 | 페이지 원문 grep | 지지 |
| 같은 문서의 API 키 문장·플랜 자격증명 중계 금지 문장(§6 반영분) | 페이지 원문 대조(2026-09-27) | 지지 |
| 나머지 피어(Paseo·CodexBar·jean·widget·CodeZeno) 세부 | 메인 미대조 — 수집자 보고 | 부분지지(단일 출처) |
| §6 반영분의 새 인용(ClaudeBar `design.md` 18·48행 · `sdk.d.ts` null 허용 행 · #30930) | 리뷰어 대조 — 메인 미대조 | 부분지지(단일 출처) |

기존 `sdk.d.ts` 줄 번호는 버전 미기록 사본 기준 — 0.3.276 대응 줄은 §7.

## 6. 적대 리뷰 결과

리뷰어 = codex(cross-family) · 판정 **FIX** · 원문 = `.claude/handoff/attachments/claude-usage-query-review-2026-09-27.md`.

| # | 지적(심각도) | 반영 |
|---|---|---|
| 1 | A 추천이 미실측 CLI 직접 경로에 기댄다 — raw stream-json 탐침을 선행 조건으로(높음) | §0-2 · §1 · §4 |
| 2 | 정책 인용에 인접 문장(Agent SDK 사용자도 API 키 인증) 누락 — A 에도 닿는다(높음) | §0-5 · §1 · §5 |
| 3 | 15분을 증거가 받친 간격처럼 적었다 — 출발점일 뿐(중간) | §0-3 · §1 · §4 |
| 4 | A·B 제한 공유의 간접 증거를 뺐다 — 버킷은 미확인(중간) | §0-4 · §1 · §4 |
| 5 | `get_usage` 응답의 null·빠짐을 뺐다 — 0% 로 그리지 말 것(중간) | §3 · §1 |
| 6 | 429 처리가 `Retry-After` 존중에서 멈춘다 — 대체 대기·즉시 재조회 금지(중간) | §0-3 · §1 |

ClaudeBar `design.md` 행 번호는 기준이 어긋난다 — 같은 "벌칙 상자" 내용을 §0-3 은 61-62행(`gh api`), 리뷰어는 47-49행(raw main)으로 적었다. 원인은 확인하지 않았다.

## 7. 탐침 결과 (2026-09-27)

- **대상** = 원시 `claude` CLI 2.1.280 — SDK 없음 · 프롬프트 없음 · `initialize` 없음. stdin 에 한 줄만 썼다: `{"type":"control_request","request_id":"<random>","request":{"subtype":"get_usage","skip_behaviors":true}}`
- **인자** = 우리 백엔드 JSON 모드 인자 `--permission-mode bypassPermissions -p --input-format stream-json --output-format stream-json --replay-user-messages --verbose` + 부작용 줄이기 `--no-session-persistence --settings {"disableAllHooks":true} --strict-mcp-config --mcp-config {"mcpServers":{}}`.
- **결과** = 기동 뒤 약 1.2초에 `control_response` success. stdout 전체가 그 한 줄이다(`system:init`·`assistant`·`result` 없음).
- **응답 모양** = `rate_limits.five_hour`/`seven_day` {utilization, resets_at} · `subscription_type` · `model_scoped`[] · 키는 있고 값이 null 인 창(`seven_day_opus`·`seven_day_sonnet`·`seven_day_oauth_apps`) · 타입에 없는 필드 여럿.
- **모델 호출 없음** — `session.total_api_duration_ms`=0 · `total_cost_usd`=0 · `model_usage`={}. (가능성 높음 — 증명은 아니다)
- **조회 예산** = `get_usage` 1회 사용 · 429 없음. A 에서 앱은 토큰을 만지지 않는다.
- **미검** = 부작용 줄이는 인자 없이 우리 인자 그대로 · 이미 떠 있는 에이전트 프로세스에 `get_usage` 보내기 · CLI 에 "get_usage is not supported in this context" 를 돌려주는 분기가 있다(어느 모드인지 미식별).
- **SDK 줄 번호(`@anthropic-ai/claude-agent-sdk@0.3.276` — t3code `pnpm-lock.yaml:1117` 의 버전)** = EXPERIMENTAL 메서드 2860-2868 · `SDKControlGetUsageRequest` 3995-4001 · 응답 타입 4006 · "claude.ai usage endpoint" 주석 4027 · null 가능 창 4039-4079 · `model_scoped?` 4083 · `rate_limits … | null` 4098.
- 원시 로그 = 세션 스크래치(저장소 밖, 식별자 없음).
