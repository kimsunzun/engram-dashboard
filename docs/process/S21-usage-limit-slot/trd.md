# TRD — 사용량 한도 슬롯 (Claude·Codex 5시간·주간 남은 양)

| 항목 | 값 |
|---|---|
| **상태** | 리뷰 통과(`/review trd full` 3회) — 구현 착수는 `json-midturn-queue` 머지 뒤 |
| **날짜** | 2026-09-27 |
| **PRD** | [prd.md](prd.md) — 요구 R1–R32 · 결정 D1–D21 |
| **근거** | [UI 서베이](../../research/usage-limit-display-ui-survey-2026-09-26.md) §4 · [조회 서베이](../../research/claude-usage-query-method-survey-2026-09-27.md) §3·§7 · 탐침 `.claude/handoff/attachments/get-usage-probe.mjs`(claude 2.1.280 — 1.2–1.4초) · codex 실측 2026-09-27(codex-cli 0.156.1, 우리 기동 인자·악수 그대로 3회 — 전체 0.74–0.94초, stderr 없음) |
| **리뷰 기록** | 2026-09-27 `/review trd full` 1차 — codex blind(Designer) FIX · Claude doc-aware(Architect-breaker) FIX · 상충 없음 → MUST 5(버스 핸들러 모양 · 병행 브랜치 · 벤더 정책 누수 · 조회 수명 · 거절 저장) + 결정 6(줍기 큐 · 비기본 프로필 · 운영 기동 경로 · 권한 인자 제거 · ⟳ 간격 · 병합 순서 없음) + 낮음 5 반영 · 2차 — Architect-breaker PASS(L4 부분 + 낮음 N1–N6) · codex Designer FIX 2 → 종합 FIX: 줍기 합침 맵(N4·N5) · 조회 마감이 쓰기·읽기·종료 전부를 덮음 · 메뉴 ctx 로 상태 전달(L4) · 버스 대기 = std 채널(N1) · 임시 폴더 배타 생성·정리(N2) · 키 정규화(N3) · 무패닉 파서(N6) 반영 · r3 — Architect PASS · codex FIX 1(대기 값 칸 단위 병합) → 반영, 권고 그대로라 재리뷰 생략 |
| **줄 포인터 기준** | 작성 시점 트리(HEAD `dc59d01` + 미커밋 문서). ★구현은 `v0.3.2/feat/json-midturn-queue` 머지 뒤 착수 — 그 트리에서 전부 다시 잰다(§5 체크리스트)★ |
| **정책 주의** | ★출시 전 법무·보안 확인 필요(PRD D3)★ — A 도 Agent SDK 를 이름으로 든 API 키 문장의 검토 범위에 든다. 이 문서는 판단하지 않는다(표지만) |

## 1. 아키텍처 — 요청형(pull)

클라이언트가 묻는다 → 데몬 캐시가 아직 유효하면 즉시 답, 기한이 지났으면 조회한 뒤 답, 강제 새로고침이면 조회한 뒤 답. 화면은 그동안 이전 값을 들고 있다. 방송·수요 신호·데몬 타이머는 없다. 멀티 클라이언트는 범위 밖(§6 #15).

```
agent ── decoder ─(UsageGate: 기본 계정 에이전트만)─ take_usage() ─► pump ─► StatusSink::usage_observed (신설 · 기본 no-op)
         backend::usage_probes() = [claude, codex] — 각 probe 가 key()·policy()·query() 를 가진다(벤더 지식은 backend 안)
           ├ claude : 짧은 claude -p 에 get_usage 한 줄 (A — 확정)
           └ codex  : 짧은 codex app-server --stdio 에 initialize → account/rateLimits/read
daemon ─ MessagingFlushSink → UsageInboxSink(신설 decorator) → DaemonStatusSink
                                   └ 대기 맵((회사, 창)당 한 칸 · 칸 단위 병합) ─► 요청 때 병합
         UsageService::request(key, force) : 캐시 판정 → 즉시 답 | 조회 시작·합류 → REPLY_WAIT_MAX 안에서 기다림
           조회 = 분리된 std 스레드(완료 가드) · UsageClock(mono·wall) · RejectStore(usage_rejects.json — 조립 때 먼저 읽음)
         입구 둘 → 같은 서비스: wire GetUsageLimits·RefreshUsageLimits{vendor}(웹뷰, async) · 버스 usage.get·usage.refresh{backend}(LLM, blocking)
shell ── forward_daemon_command 가 그대로 통과 · SlotContent::Usage{show_claude, show_codex} · layout.setSlotContent(+show_*)
front ── UsageSlot 이 실제로 보이는 동안 회사별 60초 요청(대기 중이면 건너뜀) · ⟳ = refresh · usageStore
```

### 1-1. agent — 중립 타입 (신설 `crates/engram-dashboard-agent/src/usage/` — 벤더 이름·정책 없음)

| 타입 | 모양 · 뜻 |
|---|---|
| `UsageKey` | `vendor: UsageVendorKey`(불투명 낱말 — ★정본은 `usage_probe_for(word)?.key()` 가 주는 `&'static str` 하나★, 들어오는 낱말의 대소문자와 무관) + `account: UsageAccountKey`(`String`, 지금은 늘 `"default"` — D13, §3 #33). 데몬은 이 키를 해석하지 않는다 |
| `UsagePolicy` | `cooldown: Duration` · `timeout: Duration` — 값은 각 벤더 파일이 정한다(§1-3) |
| `WindowObs` · `ScopedWindowObs` | `used_pct: Option<f64>`(0–100 정규화, §3 #32) · `resets_at: Option<i64>`(epoch 초) · 빠짐 ≠ 0(R21) · 모델별 주간 창 = `label` + `WindowObs`(D8) |
| `UsageObservation` | `vendor: UsageVendorKey` · `five_hour`/`weekly: Option<WindowObs>` · `model_scoped: Option<Vec<…>>`(None = 안 실림, Some = 전량 교체) · `plan: Option<String>` · `source: Passive\|Active`. 관측 시각은 싣지 않는다 — 데몬이 찍는다(디코더는 시계가 없다 — `backend/codex/decoder.rs:13-14`) |
| `ProbeError` | `NotInstalled` · `Unauthenticated` · `RateLimited{retry_after: Option<Duration>}` · `Unsupported` · `Timeout` · `Spawn(String)` · `Io(String)` · `Parse(String)` · `Upstream(String)` — 보이는 상태로의 접기는 §1-4 |
| `trait UsageProbe` | `key() -> UsageVendorKey` · `policy() -> UsagePolicy` · `query(&self, env: &ProbeEnv) -> Result<UsageObservation, ProbeError>`(blocking) · `ProbeEnv` = `spawner: &dyn ProbeSpawner` + `deadline`(조회 하나 전체의 마감) + `scratch_root`(데몬이 주는 부모 폴더 — §1-3 임시 폴더) |
| `trait ProbeSpawner` / `ProbeChild` | `spawn(&ProbeCommand)` → `write_line(deadline)` · `read_line(deadline)` · `wait_exit(deadline)` — ★세 동작 모두 같은 마감 아래★: 쓰기·읽기는 도우미 스레드 + `recv_timeout`, 종료 대기는 `try_wait` 를 마감까지 폴링(stdout 을 닫고 살아 있는 자식도 마감에 끊긴다). 마감이 지나면 트리째 kill. Drop = 트리 kill + 짧은 유계 종료 대기(`KILL_WAIT = 2초`). `ProbeCommand{program, args, cwd, env_set, env_remove}` — 에이전트용 `CommandSpec`(`types.rs:185-191`)은 env 를 더하기만 해서(`transport/stdio.rs:82-84`) 따로 둔다 |
| `OsProbeSpawner`(`usage/process.rs`) | `transport/stdio.rs:79-117` 의 모양(파이프 셋) + stdout 줄 리더 스레드 + stderr 는 읽어 버린다(진단 꼬리만, base `mask_secrets` 후 로그). ★OS 분기는 이 한 모듈 안★(「플랫폼 중립」): 기동 플래그(Windows `CREATE_NO_WINDOW`) · 트리 kill(Windows = `JobObjectHandle` `platform/windows.rs:19-77`, KILL_ON_JOB_CLOSE `:35` · POSIX = 새 프로세스 그룹 + `killpg`) · 종료코드 해석(§1-3 미설치) |
| `UsageGate` | 디코더 감싸개 — `collect=false` 면 `take_usage()` 를 늘 비운다(§1-2 비기본 프로필) |
| `StatusSink::usage_observed(&self, _obs)` | `types.rs:878-900` 의 `turn_ended` 옆, 기본 no-op. 계약도 같다 — 논블록·비재진입, pump 스레드에서 불린다 |

### 1-2. agent — 줍기 (passive)

- **출구 = StatusSink 훅, 운반 = pump drain.** 디코더는 관측을 자기 `Vec` 에 모아 두고 새 trait 메서드 `OutputDecoder::take_usage(&mut self) -> Vec<UsageObservation>`(기본 빈 벡터 — `transport/mod.rs:34-41`)로 내준다. pump 가 `decode`·`flush` 뒤마다 비워 `status_sink.usage_observed(obs)`(`output_core.rs:65` 가 이미 쥔다)로 넘긴다. 손대는 pump 자리 = `transport/stdio.rs:289-294`·`:312-316` · `backend/codex/transport.rs:2231-2234`·`:2323`.
- **훅만으로는 안 맞는 이유:** 디코더는 `open_spawn` 안에서 만들어져(`backend/claude/mod.rs:411` · `backend/codex/mod.rs:1067`) pump 로 move 되고 `OutputEvent` 만 돌려준다. 포트를 쥐여 주려면 `open_spawn` 에 칸이 늘고 구현 셋 + 호출 약 열 곳(`backend/mod.rs:1555` · `backend/codex/mod.rs:2381·2456·2575·2748·3529·3626` · `manager.rs:1479·3283·3307`)이 따라 바뀐다. drain 은 디코더를 순수하게 둔다.
- **비기본 프로필은 줍지 않는다:** `open_spawn` 이 `spec.env` 를 보고 계정 env(Claude `CLAUDE_CONFIG_DIR` · Codex `CODEX_HOME`)가 데몬 자신의 값과 다르면 디코더를 `UsageGate{collect: false}` 로 감싼다. 판정 함수는 각 벤더 `usage.rs` 에 둔다(벤더 지식). `output_decoder`(`backend/mod.rs:587`) 시그니처는 안 바뀐다.
- **무패닉 해석(줍기·조회 공통):** 릴리즈는 `panic = "abort"`(루트 `Cargo.toml:35`)이고 모양이 EXPERIMENTAL 이다 — 외부 데이터에 `unwrap`·`expect`·인덱싱 금지, 수 변환은 `checked`/`try_from`. 비유한(NaN·inf) 사용률 → 그 칸 None · 음수·100 초과 → 절단 · 범위 밖 epoch·ISO 아님·타입 틀림 → 그 칸 None(창 전체를 실패로 만들지 않는다) · 봉투 자체가 깨지면 조회는 `Parse`, 줍기는 그 줄을 버린다.
- **Claude** — 신설 `backend/claude/usage.rs`. `mod.rs` 는 skip 팔(`:834`)에서 `"rate_limit_event"` 를 떼어 넘기는 것만 바뀐다(`consume_line` `:744` 시그니처에 관측 벡터 칸 하나).
  - `rate_limit_info.rateLimitType ∈ {five_hour, seven_day}` + `utilization`(0–1 → ×100, [0,100] 절단) + `resetsAt`(epoch 초) → 그 창. 모르는 type 은 버린다(debug 1회). `unifiedWindows.{five_hour,seven_day}.{utilization,resetsAt}` 도 느슨하게 받는다(R20) — 둘 다에 있으면 `unifiedWindows`, 칸 타입이 틀리면 그 칸만 버린다.
  - 옛 모양(`fixtures/claude_text.jsonl:3` — `utilization` 없음) → `used_pct: None, resets_at: Some`(§1-5). `mod.rs:2757` 시험(`rate_limit_info:{}` → skip)은 그대로 초록.
- **Codex** — 신설 `backend/codex/usage.rs`. `decoder.rs:443-481` 에 `account/rateLimits/updated` 팔을 더하고 `KNOWN_UNTRANSLATED_METHODS`(`:135`)에서 빼며 개수 문장(`:125-126`)을 고친다. `protocol.rs` 에 메서드 상수와 인바운드 구조체(`RateLimitSnapshot`·`RateLimitWindow{usedPercent, windowDurationMins, resetsAt}` — 읽는 칸만, `deny_unknown_fields` 없음 — `protocol.rs:11-20`). 창 분류 = `windowDurationMins` 300 → 5시간 · 10080 → 주간 · 그 밖 버림(R18). null 창 = `None`. `OutputEvent` 는 내지 않는다(`decoder.rs:18-25`).

### 1-3. agent — 능동 조회 (probe) · 벤더 정책은 backend 안

- **등록부 하나:** `backend::usage_probes() -> [&'static dyn UsageProbe; 2]` + `backend::usage_probe_for(word: &str)`(대소문자 무시 — `commands.rs:376` 선례)를 정적 백엔드 표(`backend/mod.rs:660-671`) 옆에 둔다 — ★벤더 이름은 그 표·`backend_for`·싱글턴 선언에만 적는다★(`backend/mod.rs:677-678` · ADR-0004). probe 싱글턴은 각 벤더 `usage.rs` 에 산다. `AgentBackend` trait 에는 칸을 더하지 않는다(두 겹 디스패치 없음).
- **Claude A — 확정(D2)** (`backend/claude/usage.rs` — `CLAUDE_COOLDOWN = 15분`(R15) · `CLAUDE_PROBE_TIMEOUT = 15초`(R30))
  - 명령 = `console_command(CLAUDE_PROGRAM, args)`(`mod.rs:48` · `backend/mod.rs:44-57` — 운영은 `cmd.exe /c claude`). args = `-p --input-format stream-json --output-format stream-json --replay-user-messages --verbose --no-session-persistence --settings <파일> --strict-mcp-config --mcp-config <파일>`. `--permission-mode bypassPermissions` 는 뺀다(물을 프롬프트가 없다). 두 설정은 인라인 JSON 이 아니라 조회마다 새로 만든 임시 폴더에 쓴 파일 경로로 준다 — `cmd.exe` 를 지나는 JSON 인용을 피한다. `--mcp-config <경로>` 는 운영 스폰이 이미 쓰는 형태다(`mod.rs:226-230`) · `--settings` 가 경로를 받는지는 CLI 도움말로 확인(미검 — 안 받으면 인라인 + 스모크). cwd = 그 임시 폴더.
  - **임시 폴더(두 벤더 공통 — `usage/process.rs` 의 `ScratchDir`):** `scratch_root`(= 데몬 데이터 폴더의 `usage-probe/`) 아래에 무작위 이름(uuid v4)으로 **배타 `create_dir`**(이미 있으면 실패 → 새 이름) — claude 가 믿고 읽는 `--mcp-config` 파일이 들어 있어서, 남이 미리 심은 폴더를 재사용하지 않게 한다. 끝나면 트리 kill → 자식 종료를 유계로 기다린 뒤(`KILL_WAIT` — Windows 는 열린 파일을 못 지운다) 지우고, 지우기 실패는 무시한다. 남은 폴더는 데몬 기동 때 쓸어 낸다 — `control::mcp_config::sweep_stale_configs(&data_dir)`(`lib.rs:535`) 옆, 데이터 폴더당 데몬 하나라 안전하다.
  - env = 데몬 env 에서 `CLAUDE*`(대소문자 무시)를 벗기되 허용 목록 `CLAUDE_CONFIG_DIR`·`CLAUDE_CODE_OAUTH_TOKEN` 은 남기고 + 탐침과 같은 네 키 `MAX_THINKING_TOKENS=8000 ENABLE_CLAUDEAI_MCP_SERVERS=false CLAUDE_CODE_AUTO_CONNECT_IDE=0 CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL=1`(탐침 = `get-usage-probe.mjs:31-40` — 그쪽은 `CLAUDE*` 전부 벗김).
  - ★운영 명령 그대로의 실 Windows 스모크가 3단계 완료 조건★ — 탐침은 `claude.exe` 를 직접 띄웠고(`get-usage-probe.mjs:16`) 인자·env·shim 이 다르다. `#[ignore]` 라 CI 는 안 돌지만 로컬에서 돌려 결과를 step-log 에 적는다.
  - 한 줄 `{"type":"control_request","request_id":<uuid v4>,"request":{"subtype":"get_usage","skip_behaviors":true}}` 을 쓰고 stdin 은 연 채로, `type=="control_response"` 이고 `request_id` 가 같은 줄까지 읽는다(다른 줄은 버림). 받으면 kill.
  - 해석: `response.subtype=="success"` → `response.response.rate_limits`(null → 창 전부 없음) `.five_hour`/`.seven_day.{utilization 0–100, resets_at ISO 8601}` · `model_scoped[]` 와 값이 있는 `seven_day_opus`/`_sonnet`/`_oauth_apps` → 모델별 창 · `subscription_type` → plan.
  - 오류 분류(한 함수, §3 #6): 한도 문구 → `RateLimited{None}` · 로그인·인증 문구 → `Unauthenticated` · "not supported in this context" → `Unsupported` · 그 밖 → `Upstream`. 문구 표는 추정이다(§6 #4). 떠 있는 에이전트 프로세스에는 보내지 않는다(D7).
- **미설치:** 응답 줄 없이 stdout EOF 를 보면 `wait_exit(deadline)` 으로 종료 상태를 받아 해석한다 — Windows = `cmd.exe` 종료코드 9009 · POSIX = spawn `NotFound` → `NotInstalled`(`usage/process.rs` 안). 마감까지 안 끝나면 `Timeout` + 트리 kill.
- **교체 대안 B**(A 가 깨질 때만 — D2): 같은 `UsageProbe` seam 뒤 둘째 구현이다. 바뀌는 것 = ① 첫 TLS 의존 + 데몬의 첫 인터넷 송신(HTTPS 는 주입 포트로) ② 자격증명 찾기 OS 분기 한 함수(Windows·Linux = `config_dir()` `mod.rs:993` 의 파일 · macOS = Keychain · env 토큰) ③ 토큰 취급(읽기만·저장·로그 없음 — D5). 그 밖 설계는 전환 때 TRD 를 새로 쓴다.
- **Codex** (`backend/codex/usage.rs` — `CODEX_COOLDOWN = 5분`(R15) · `CODEX_PROBE_TIMEOUT = 30초` = 멈춤 방지(평소 1초 미만 실측 · 대화 이어받기에서 약 10초 관측 1회 — 안전 여유))
  - 명령 = `console_command(CODEX_PROGRAM, [APP_SERVER_SUBCOMMAND, APP_SERVER_STDIO_FLAG])`(`mod.rs:169`·`:461-462`·`:971`). env = 데몬 env 상속. cwd = 같은 방식의 임시 폴더.
  - **재사용:** `protocol.rs` 의 `classify`(`:151`)·`request_line`·`notification_line`·`error_response_line`(`:278-310`)·`InitializeParams`/`ClientInfo`/`InitializeResponse`(`:316-341`)·`method::INITIALIZE`/`INITIALIZED` + `CLIENT_NAME`(`transport.rs:288` — `protocol.rs` 로 옮긴다). **재사용 안 함:** `request_blocking`·`Pending`·`Waiter`(`transport.rs:742-797`·`:426`) — 대화 통로의 라이터 스레드·공유 상태에 묶여 있다.
  - 순서 = `initialize`(id 1) → 응답 대기(서버 요청은 `error_response_line` 으로 거절 — `transport.rs:1903` `refuse` 와 같은 뜻 · 알림은 버림) → `initialized` → `account/rateLimits/read`(id 2) → 응답 → kill.
  - 해석 = 최상위 스냅숏 `primary`/`secondary` 를 창 길이로 분류 + `planType` → plan + `rateLimitsByLimitId` 비기본 버킷 주간 창 → 모델별 창(label = `limitName`, 없으면 limit id). RPC 오류 → 인증 문구면 `Unauthenticated`, 그 밖 `Upstream`.
  - ★응답에 `accountId` 가 실린다 — 절대 로그에 남기지 않는다★(stderr·파싱 오류 꼬리 포함). Codex 쪽 `account_key` 의 장래 출처이지만 지금은 `"default"` 그대로.

### 1-4. daemon — `UsageService` (신설 `crates/engram-dashboard-daemon/src/usage_service/` — 벤더 이름·정책 없음)

`book.rs`(순수 상태기 — `UsageKey` 당 한 칸) · `mod.rs`(요청·합류·조회 구동) · `clock.rs` · `reject_store.rs` · `inbox.rs`(줍기 대기 맵 + decorator). 키 목록·쿨타임·시한은 `backend::usage_probes()` 에서 받는다 — 데몬 코드에 벤더 이름이 없다. 태스크·타이머 없음 — 모든 판정은 요청이 올 때 한다.

**기한 식 — 하나뿐(R29):** `next_auto = max(max(last_query, passive_reset) + policy.cooldown, reject_until)` · 기준점이 없으면(시동 뒤 첫 요청) `reject_until`(없으면 즉시). 거절 종료·리셋 경과는 식에 없으므로 조회를 앞당기지 못한다. 데몬 상수 = `REPLY_WAIT_MAX = 5초`(§3 #16) · `REJECT_FALLBACK = 5분`(D4) · `REJECT_MAX = 24시간`(§3 #37) · `REFRESH_MIN_SPACING = 30초`(§3 #38).

| 요청(회사별) | 판정 | 답 |
|---|---|---|
| `get`, `now < next_auto` | 캐시 | 즉시 캐시(`served = Cached`) |
| `refresh`, `now < reject_until` | 거절 | 캐시 + `Rejected{retry_in_secs}` — 조회 안 나감(R24) |
| `refresh`, 직전 조회 끝 뒤 30초 안 | 간격 | 캐시(`Cached`) — 조회 안 나감(D11 개정) |
| `get` 기한 지남 · 그 밖의 `refresh` | 조회 시작(`refresh` 는 쿨타임 무시 — D11), 이미 진행 중이면 합류 | 끝나면 새 상태(`Fresh`) · 실패면 캐시 + 실패 상태 · `REPLY_WAIT_MAX` 넘으면 캐시 + `in_flight=true`(조회는 계속 — 결과는 다음 요청에) |

| 사건 | 동작 |
|---|---|
| 요청 도착(판정 전) | 대기 맵을 비워 병합 → `wall_now ≥ resets_at` 인 창에 **만료 래치** set(한 번 — R32). 조회 시각은 안 바꾼다 |
| 줍기 관측(부분) | 대기 맵 항목마다 — 합친 항목의 mono 시각(= 합쳐진 마지막 관측) ≥ 그 창에 마지막으로 적용한 줍기 시각일 때만 적용(늦게 비워진 옛 항목이 새 값을 되돌리지 않게). 창 단위 병합(R16·D18): 관측에 **없는 창 = 유지** · `used_pct` Some → 교체 + 그 창 래치 해제 + fresh · None 이고 `resets_at` 바뀜 → `used_pct` 지움 · None 이고 같음 → 유지. 기준점 뒤 fresh 가 5시간·주간 둘 다 서면 `passive_reset = 그 항목의 mono 시각`, fresh 초기화(D12) — `reject_until` 은 안 건드린다. 화면에는 다음 요청(≤ 60초)에 닿는다 |
| 능동 성공(전체) | 먼저 대기 맵을 비워 병합 → 두 창·모델별 창·plan 전량 교체 — **null·빠진 창 = "없음"**(R16 하위·D18) · 래치 전부 해제 · 상태 `Ready`. 줍기와의 순서 규칙은 두지 않는다 — 나중 쓰기가 이긴다(사용자 결정 2026-09-27, §6 #17) |
| 능동 실패 | 값 유지(R22) · 상태 = 아래 표 · `RateLimited` → `reject_until = now + min(retry_after, REJECT_MAX)`(없음·깨짐·과거·0 → `REJECT_FALLBACK` — R23, 덧셈은 `checked_add`) + 저장 |
| 조회 끝(성공·실패·시한) | ★완료 가드가 진다★ — 조회를 도는 스레드의 Drop 가드가 `in_flight = false` · `last_query = now`(D11) · 합류자 깨우기(async = `watch` 세대 +1 · 버스 = 등록된 std 송신단마다 send)를 한다. 요청이 `REPLY_WAIT_MAX` 에 포기해도 `in_flight` 가 안 남는다. 마감(쓰기·읽기·종료 전부) + `KILL_WAIT` 안에 반드시 돈다 |
| 시한 초과 | 쓰기·읽기·종료 대기 어디서든 마감에 걸리면 트리 kill · `Timeout` = 실패로 셈 · 다음 기한 = 다음 쿨타임(R30) |

| 보이는 상태(회사별 — R31) | 계기 `ProbeError` | 문구 |
|---|---|---|
| `NotInstalled` | `NotInstalled` | 설치 안 됨 |
| `NeedsLogin` | `Unauthenticated` | 로그인 필요 |
| `Failed{next_attempt}` | `Timeout`·`Unsupported`·`Spawn`·`Io`·`Parse`·`Upstream` | 조회 실패 — 다음 시도 HH:MM(= `next_auto`) |
| `Rejected{until}` | `RateLimited`(`now < reject_until` 동안) — 지나면 `Failed{next_attempt}` | 거절됨 — N분 뒤 |

- **동시성·락:** `Mutex<UsageBook>` 은 잎 락 — 쥔 채 I/O·await·다른 락 금지(ADR-0006 원칙). 합류 = 키마다 `watch` 세대 번호 — 시작한 요청과 합류한 요청이 같은 완료를 `timeout(REPLY_WAIT_MAX)` 로 기다린다. 회사끼리는 서로를 기다리지 않는다.
- **조회 구동·종료:** 조회는 분리된 `std::thread`(이름 `usage-probe`)에서 돈다 — tokio blocking 풀이 아니므로 런타임 drop 이 기다리지 않고, 자식은 KILL_ON_JOB_CLOSE Job 안이라(`platform/windows.rs:4`·`:35`) 데몬 프로세스가 끝나면(정상·크래시 모두) OS 가 트리째 거둔다. 그래서 graceful 종료(`lib.rs:754-782`)에 단계를 더하지 않는다(§3 #39). 버스 대기는 std 채널 `recv_timeout` 이라 런타임 타이머와 무관하다(아래 「버스」).
- **줍기 경로(pump 뜨거운 길 밖):** 신설 decorator `UsageInboxSink`(`inbox.rs`)가 `usage_observed` 를 받아 관측을 창별로 쪼개 **대기 맵** `(vendor 키, 창) → (mono 시각, 값)` 에 항목 하나로 **칸 단위로 합쳐** 둔다(창 = 5시간 · 주간 · 모델별 묶음 · plan — 키 수가 유한해 버릴 것이 없다). ★합치기는 책의 창 병합과 **같은 순수 함수** `merge_window` 를 쓴다(호출자 둘, 규칙 하나)★ — 그래서 「비우기 전에 둘을 합친 뒤 적용」 = 「책에 차례로 적용」이 구조로 선다: 빠진 칸 = 대기 값 유지(`used_pct` 40 뒤 리셋만 온 관측 → 40 유지 + 리셋 갱신) · % 없이 리셋만 바뀐 관측이면 책과 같이 % 를 지운다(§3 #3). 합친 항목은 mono 시각(합쳐진 마지막 관측) 하나와 fresh 표시(이번 대기 중 들어온 `used_pct` 가 남아 있음)를 든다 — 시각 가드는 칸별이 아니라 **합친 항목 단위**다(칸마다 시각을 두면 같은 함수를 못 쓴다). 나머지 훅은 전부 안쪽으로 넘긴다. 맵의 락은 덮어쓰기·비우기만 하고 poison 을 견딘다(`PoisonError::into_inner`). `DaemonStatusSink` 는 손대지 않는다 — 그 머리의 「인코딩해 흘리기만」(`status_fanout.rs:12-15`)이 그대로 참이다. 사슬 = `MessagingFlushSink(UsageInboxSink(DaemonStatusSink))`(`lib.rs:275-279`) — 두 decorator 모두 새 훅(병행 브랜치의 `inputs_drained` 포함)을 넘긴다(decorator 계약 `messaging_host.rs:883-888`).
- **시계(R29·R32):** seam `UsageClock{ mono() → 잠든 시간 포함 단조 경과, wall() → epoch 초 }`. 쿨타임·거절 대기·값 나이·⟳ 간격 = `mono` · 리셋 비교·거절 파일 = `wall`. 실물의 `mono` 는 한 함수이고 OS 차이가 있으면 그 안에서만 갈린다(§6 #2). 복귀 뒤 첫 요청이 기한 지남을 보면 조회 한 번 — 동시 요청은 합류.
- **만료 래치(R32):** `UsageBook` 이 창마다 소유 — 요청 시각에 평가해 한 번 set 되면 벽시계가 되감겨도 유지, 그 창의 새 값(줍기 `used_pct` · 능동 성공)만 해제. 프론트는 그리기만 한다.
- **거절 저장(R29):** `reject_store.rs` — `trait RejectStore{load, save}` · 실물 `FileRejectStore` = 데몬 데이터 폴더(`lib.rs:61-63`)의 `usage_rejects.json` `{schema_version:1, entries:[{vendor, account_key, reject_until_epoch_s}]}` · tmp+rename(`persistence/presets.rs:18-20` 선례). `vendor` 는 정본 키로만 쓰고, 읽을 때 `usage_probe_for(..)?.key()` 로 정규화한다(손으로 고친 대소문자도 같은 칸 · 모르는 낱말 → 버림). 복원 = `until > wall_now` 만, 남은 시간 = `min(until − wall_now, REJECT_MAX)`, `mono` 에 `checked_add`(넘치면 그 항목 버림 + warn — 릴리즈는 `panic = "abort"` 라 조립 중 넘침은 매 부팅 중단이 된다, 루트 `Cargo.toml:35`). 지난 항목 무시 · 깨진 파일 = 빈 표 + warn · 쓰기 실패 = warn 후 계속. ★저장 = 전용 `save_lock` 안에서 책을 다시 스냅숏해 전량 쓰기★ — 락 순서 `save_lock → book`(책은 여전히 잎: 책을 쥔 채 `save_lock` 을 잡지 않는다) — Claude·Codex 거절이 동시에 나도 한쪽이 지워지지 않는다. **긴장(수용):** 재시작을 건너는 대기만 벽시계에 기댄다.
- **조립:** `build_daemon_wiring`(`lib.rs:246-261`)만 실물을 넘긴다 — `FileRejectStore`(`FilePresetStore::new` `:253` 옆) + 실물 조회기(`backend::usage_probes()` + `OsProbeSpawner`). `build_daemon_wiring_with_store`(`:264`)는 **저장소와 조회기를 인자로** 받는다 — 테스트 서버(`:861`)는 메모리 저장소·가짜 조회기를 넘기므로 시험이 실 CLI 를 띄울 길이 없다. 그 함수가 동기로 `load` 한 상태로 `UsageService` 를 만들고, `UsageInboxSink` 를 사슬에 끼우고, `DaemonWiring`(`:301`)에 `usage` 칸 · `AgentConnections::new`(`agent_conn.rs:455`) → `ConnectionCore`. 조립(`:609`)은 `run_accept_loop`(`:738`)보다 앞이다.
- **버스:** 포트 `UsageCommandHost{get, refresh}` 는 **동기** — `blocking_handler` 로 등록한다(`command/src/table.rs:359-364`). 데몬 입구(`control/commands.rs:73-91`)는 첫 poll 에서 끝나는 핸들러만 몰고(`drive_to_completion` `:104-125`, `now_or_never` `:109`) 기다리는 async 핸들러는 `OUTCOME_UNKNOWN` 이 되기 때문이다 — 입구는 손대지 않는다. 데몬 어댑터는 이미 blocking 풀 위라(`:64-68`) 서비스의 동기 메서드 `request_blocking` 을 부른다 — 판정은 같은 책에서 하고, 기다려야 하면 그 조회의 합류자 목록에 std `mpsc` 송신단을 올린 뒤 `recv_timeout(REPLY_WAIT_MAX)` 로 기다린다(완료 가드가 보낸다). ★`Handle::block_on(timeout(..))` 은 쓰지 않는다★ — 종료 중 런타임이 시간 드라이버를 내리면 tokio 타이머 poll 이 패닉할 수 있고 릴리즈에서는 곧 abort 다. std 채널은 런타임을 모른다. agent crate 도 tokio 를 모른 채로 남는다. `make_daemon_table`(`control/commands.rs:44-49`) → agent `make_table`(`agent/commands.rs:441`)에 포트가 는다 — 호출 자리: 운영 `lib.rs:621`·`:875` · 실험 bin 셋(`priming_smoke.rs:150`·`roundtrip_smoke.rs:458`·`saturation_pilot.rs:760`) · 시험 여덟(`control/commands.rs:290·306·322·471` · `connection_core.rs:2324·3458` · `tests/control_agent.rs:370` · `tests/mail_gate.rs:207`) — 운영 밖은 no-op 포트.

### 1-5. Claude 줍기에 사용률이 없을 때 (R14·D12)

CLI 가 옛 모양을 내면 Claude 는 줍기로 fresh 가 서지 않는다 → 쿨타임 초기화가 안 되고, 슬롯이 보이는 동안 15분마다(그 뒤 첫 요청에) 조회가 돈다. 2.1.280 캡처 모양(`utilization` + `unifiedWindows` — UI §4-2)이면 R14 가 선다. 최상위 한 창만 오면 창이 하나씩 fresh 가 된다(D12 가 받는다).

### 1-6. wire (`protocol`) · 버스

- **타입(`domain.rs`):** `UsageLimitSnapshot{vendor: AgentBackendKind(domain.rs:209), account_key: String, five_hour/weekly: Option<UsageWindow>, model_scoped: Vec<UsageScopedWindow>, plan: Option<String>, in_flight: bool, served: UsageServed, state: UsageVendorState}` · `UsageWindow{used_pct: Option<f64>, resets_at: Option<u64>, age_secs: u64, expired: bool, source: UsageSourceKind}` · `UsageScopedWindow{label, window}` · `UsageVendorState`(`#[serde(tag = "kind")]` — `domain.rs:167` 선례) = `Ready | NotInstalled | NeedsLogin | Failed{next_attempt_in_secs: Option<u64>} | Rejected{retry_in_secs: u64}` · `UsageServed{Fresh, Cached}` · `UsageSourceKind{Passive, Active}`. u64 에는 `#[ts(type = "number")]`(`domain.rs:424`).
- **키 변환·정규화:** 웹뷰 낱말은 소문자(`AgentBackendKind` `rename_all = "lowercase"` `domain.rs:207-208`), 버스 낱말은 대문자 시작(`AgentBackend` — `commands.rs:71`)이다. 두 입구 모두 `usage_probe_for(낱말)`(대소문자 무시)로 probe 를 찾고, 책·거절 파일은 **그 probe 의 `key()` 하나로만** 키를 삼는다 — 두 입구가 같은 칸을 친다. 답의 `vendor` 는 그 키를 `AgentBackendKind` 로 역직렬화해 싣는다. 데몬에 벤더 match 를 새로 두지 않는다.
- **시간 표현:** 절대 시각은 서버가 준 `resets_at` 하나뿐. 나이·대기·다음 시도는 **답을 보낸 시점 기준 상대 초**(데몬 `mono`) — 프론트는 받은 순간의 `performance.now()` 에서 흐른 만큼 더한다(R32).

| 방향 | variant (`messages.rs`) | 짝 |
|---|---|---|
| 클→데몬 | `GetUsageLimits{vendor: AgentBackendKind, request_id}` | 전용 답 `UsageLimits{request_id, snapshot: UsageLimitSnapshot}` |
| 클→데몬 | `RefreshUsageLimits{vendor: AgentBackendKind, request_id}` | 같은 `UsageLimits` |

- `command_request_id`(`messages.rs:769`)·`event_reply_request_id`(`:827`)에 쌍을 더한다. ★`dispatch_order`(`connection_core.rs:228-275`)는 둘 다 `Detached`★ — 최대 `REPLY_WAIT_MAX` 기다리므로 줄에 두면 같은 연결의 `WriteStdin` 등이 밀린다(판정 기준 = `:202-224`).
- 셸은 새 코드 없이 나른다 — `forward_daemon_command`(`src-tauri/src/commands/agent.rs:207`)에 variant allowlist 가 없다(`messages.rs:559-563`). 답장 상한 30초(`agent.rs:263`).
- **`PROTOCOL_VERSION` = 머지 시점 master 값 + 1 — 필요하다.** 새 요청 variant 둘 때문이다: 구데몬 + 신셸이면 요청이 매번 실패해 슬롯이 조용히 빈다 — `protocol/src/lib.rs:70-103` 의 기준(조용한 오작동). 병행 브랜치가 이미 5 → 6 을 쓴다(§5).
- **버스(agent `commands.rs` 블록, catalog = 머지 시점 master 값 + 1):** `usage.get {backend: AgentBackend}` → `UsageVendorRow`(Read) · `usage.refresh {backend: AgentBackend}` → `UsageVendorRow`(Write). 백엔드 어휘는 `agent.new` 의 블록 지역 enum 을 그대로 쓴다(`commands.rs:71` · 낱말 = `backend_word` `:383-386`). 행 = `UsageVendorRow{backend, account_key, plan, windows: Vec<UsageWindowRow>, in_flight, served: Fresh|Cached, state: UsageStateWord, next_attempt_in_secs, retry_in_secs}` · `UsageWindowRow{window: FiveHour|Weekly|ModelWeekly, label, used_pct, left_pct: Option<u64>, resets_at, age_secs, expired}` — ★`left_pct` = UI 와 같은 `floor(clamp(100 − used))`★. 매크로 enum 은 문자열 열거뿐이라(`command/src/macros.rs:45`) 상태는 단어 + 선택 칸으로 편다.

### 1-7. shell (`src-tauri`)

- **슬롯:** `SlotContent::Usage{show_claude: bool, show_codex: bool}`(`layout/types.rs:36-46`, serde 기본 true) · `agent_id()`(`:53-57`) · `resolve_spawn_slot`(`manager.rs:794-801`)은 점유로.
- **버스:** `SlotContentKind::Usage`(`commands.rs:106-111`) + `layout.setSlotContent`(`:302-309`)에 `show_claude`/`show_codex: Option<bool>`. `slot_content()`(`:921-940`)는 Usage 가 아닌데 실리면 반려. Usage 는 새 `apply::set_usage_slot`(`apply.rs:449` 옆)로 가서 **락 안에서** `mgr.slot_content`(`manager.rs:586`)를 읽어 병합한다 — 빠진 칸 = 이미 Usage 면 그 값, 아니면 true. shell catalog = 머지 시점 값 + 1(지금 8 — `:74`).
- **링크 권한:** opener 플러그인은 초기화돼 있으나(`lib.rs:39`) 권한이 없다 → `capabilities/default.json`·`popup.json` 에 두 URL 로 좁힌 `opener:allow-open-url`.
- 요청 중계·수요 파생·방송 중계는 없다. 생성물: `src-tauri/bindings/` 재생성(CI sync 게이트).

### 1-8. frontend

- **표면:** `agentClient`(`api/agentClient.ts:183-188` 꼴)에 `getUsageLimits(vendor)`·`refreshUsageLimits(vendor)` → `ProtocolClient.sendCommand`(`listPresets` `protocolClient.ts:973-974` 꼴) + `handleEvent` 의 `UsageLimits` 답 갈래(`PresetList` `:692-696` 옆).
- **스토어:** 신설 `store/usageStore.ts`(zustand) — 회사별 마지막 스냅숏 + 받은 `performance.now()` + `pending`(웹뷰 안 슬롯끼리 공유).
- **폴러(`useUsagePoll`):** 보임 = `document.visibilityState === 'visible'`(`visibilitychange`) ∧ 슬롯 루트의 IntersectionObserver `isIntersecting && intersectionRatio > 0`(숨은 keep-alive 탭 = `display:none` `WindowLayout.tsx:200` · 선례 `TerminalSlot.tsx:230-236`) ∧ 연결 상태 connected. 보이는 동안 켠 회사마다 `USAGE_POLL_MS = 60_000` 간격 `getUsageLimits` · 보이게 된 순간·connected 전이(첫 연결 포함) 즉시 한 번 · `pending` 이면 건너뜀 · ⟳ = 켠 회사마다 `refreshUsageLimits`(30초 간격은 데몬이 건다). 실패(연결 끊김 포함) = 이전 값 유지.
- **순수 `usageFormat.ts`(R10·D17):** `left = clamp(100 − used, 0, 100)` · **보이는 수 = `Math.floor(left)`** · 색은 보이는 수로 — `> 50` ok · `20–50` warn · `< 20` danger + ⚠ · 없음 → 회색 "—"(R21) · 나이 = `age_secs + (performance.now() − 받은 시각)/1000` · `STALE_AFTER_SECS = 1800`(R32) · 남은 시간 = `resets_at` − 로컬 벽시계(R29).
- **컴포넌트:** 신설 `components/slot/UsageSlot.tsx`.
  - 폭 단계(R3·D16): ResizeObserver 로 실제 폭, 단계별 자연 폭은 숨은 렌더(`visibility:hidden`)에서 잰다. 들어가는 가장 넓은 단계 · 남은 시간은 1·2단만 · 3단보다 좁으면 `text-overflow: ellipsis` — px 상수 없음.
  - 막대 = `role="meter"` + `aria-valuenow`(보이는 수)·`aria-valuetext`(D14). 값·시간은 DOM 텍스트(R27) + `data-usage-vendor`/`data-usage-window`(cdp QA 용). "갱신 중" = `pending` 또는 답의 `in_flight` — 값은 이전 것 그대로.
  - 만료: `expired`(데몬 래치) 또는 `resets_at ≤ 지금` → %·카운트다운 숨기고 "리셋됨 — 갱신 대기"(R32). 오래됨 → 흐리게 + "N분 전" · 분 단위 tick 하나. 상태 문구(R31) 네 가지를 `t()` 로 — 실패·거절이어도 들고 있는 값은 그린다(R22).
  - 팝업(R5–R7·D8): 요약 영역이 `<button>`(클릭·Enter·Space — D14) · `position: fixed` + `clampMenuPosition`(`SlotContextMenu.tsx:33`) · Esc·바깥 클릭으로 닫힘 · 창별 절대 리셋 시각(`Intl.DateTimeFormat`) · 값마다 나이 상시 · plan(있을 때만) · 모델별 창 · 「사용량 페이지 ↗」(`@tauri-apps/plugin-opener` `openUrl` — `package.json:22`, `title` 로 목적지 설명) · ⟳(거절 중 비활성 + "거절됨 — N분 뒤"). 토글 없음. 두 회사 다 꺼짐 → 안내 한 줄(R4).
- **메뉴(R8):** `registerSlotMenu('usage', …)` = `usageSlot.refresh` · `usageSlot.toggleClaude` · `usageSlot.toggleCodex`. ☑ 는 `SlotMenuItem`(`commands/slotMenu.ts:21-35`)에 선택 칸 `checked?: (ctx) => boolean` 을 더한다. ★상태는 메뉴 ctx 로 들어온다★ — ctx 를 만드는 자리(`LayoutLeaf.tsx:302-307`)가 이미 쥔 `node.content` 를 ctx 에 싣고, `checked(ctx)` 는 ctx 만 읽는다(스토어를 뒤지지 않는다 — ADR-0064 「메뉴에서 직접 store 호출 금지」·실행 컨텍스트는 ctx 로). `SlotContextMenu` 가 `role="menuitemcheckbox"`·`aria-checked` 로 그린다(`checked` 칸 추가 = §7 #8). 토글 실행 = `useViewStore.setSlotContent` 전량 교체(`store/viewStore.ts:213-219`). `'*'` hideOn(`slotContentCommands.ts:115`)에 `'usage'` · 빈 슬롯 「새 콘텐츠」 자식(`:94-105`)에 `slot.fill.usage`.
- **등록점:** `SLOT_CONTENT_TYPES`·`validateSlotContent`(`tabCommands.ts:147-172`) · `LayoutLeaf.tsx:206-207`·`:283-289`. 포커스 제외는 allowlist 라 자동(`LayoutLeaf.tsx:78-86` · `selectOpenTarget.ts:14`).
- **테마:** `styles/theme.css:1-3` 세 블록에 `--usage-ok`(= `--status-running`)·`--usage-warn`(= `--status-blocked`)·`--usage-danger`(dark `#f85149` · light `#cf222e`) + 채움·테두리 토큰. e-ink = warn 빗금 · danger 검정 채움 + 굵은 테두리(R11).
- **문구·도움말:** `i18n/ko.ts` 에 `usage:` 이름공간(`preset:` `:116` 옆) · `Intl`(R28). `prompts/engram-help.md` `## window`(`:65`) — `layout.setSlotContent` 줄(`:94`)에 `Usage`·`--show_claude`·`--show_codex`, 그리고 `usage.get --backend <Claude|Codex>`·`usage.refresh --backend <…>`. 구획 다섯 규칙(`:8`)은 그대로.

### 1-9. 기동·최초 연결·재연결·종료 시나리오 (새 메시지 없음)

| # | 시나리오 | 동작 | 앵커 · 시험 |
|---|---|---|---|
| 1 | 데몬 기동 직후 첫 요청 | 거절 파일은 조립(`lib.rs:609` → `:264`) 안에서 동기로 읽히고 서비스는 그 상태로 만들어진다 → `run_accept_loop`(`:738`) 전이다. 첫 요청도 저장된 거절을 못 넘는다 | 시험: 미래 항목이 든 저장소로 조립 → 첫 `get`·`refresh` = 캐시 + `Rejected`, 조회 0 |
| 2 | 데몬이 뜨기 전 요청 | 셸 `send_command` 는 소켓이 없으면 대기열 없이 즉시 `NOT_CONNECTED`(`daemon_client/mod.rs:756-763`) → 슬롯은 이전 값 또는 "없음". 폴러는 비연결이면 안 보내고 connected 전이에 즉시 요청 — 목록·프리셋의 재끌어오기(`eventBus.ts:181-195` · 부팅 재시도 `App.tsx:41-50`)와 같은 꼴 | vitest: 비연결 중 0건 · 전이 → 즉시 회사당 1건 |
| 3 | 빈 캐시에 첫 요청 | 기한 지남 → 조회 1회, 답 ≈1초(Claude 1.2–1.4초 · Codex 0.74–0.94초 실측). 슬롯 여럿·두 회사 동시 → 회사별 합류(회사당 조회 1회), 회사끼리는 안 기다린다 | 시험: 동시 요청 N → 조회 회사당 1 · Codex 가 막힌 가짜여도 Claude 답은 안 늦음 |
| 4 | 클라이언트는 남고 데몬 재시작 | 대기 중 요청은 끊김 drain 으로 오류(`connection.rs:649-656` · 영구 hang 아님 — `src-tauri/tests/daemon_client_pending.rs`) → 이전 값 유지 → 재연결 전이에 재요청. 캐시는 비지만 거절 파일은 남는다(#1) | vitest: 요청 실패 → 값 유지 · 전이 → 재요청 |
| 5 | 기동 경합 | 부팅 자동 복원은 기본 꺼짐(`lib.rs:725-731` — no-op). 사용자가 곧바로 에이전트를 띄워도 조회는 별 프로세스·별 Job — 특별 처리 없음 | — |
| 6 | 조회 중 데몬 종료 | 분리 스레드라 런타임 drop 이 안 기다리고, Job 이 닫히며 자식 트리가 죽는다(§1-4) | 시험: 가짜 조회기가 막혀 있어도 서비스 drop·런타임 종료가 즉시 끝남 |
| 7 | 레이아웃 영속(미래) | 복원된 슬롯이 부팅 즉시 요청해도 #1–#3 이 그대로 덮는다 | — |

## 2. 사용자 결정

**확정(2026-09-27)**
- **Claude 능동 조회 = A**(D2) — 짧게 뜨는 CLI 에 `get_usage` 한 줄. B 는 A 가 깨질 때의 교체 대안으로만(§1-3).
- **쿨타임 = Claude 15분 · Codex 5분**(D1·R15) — 각 벤더 파일의 이름 붙인 상수(§1-3).
- **요청형(pull)** — 보이는 슬롯이 회사별로 1분마다 묻고 데몬은 기한이 지났을 때만 조회. 멀티 클라이언트 범위 밖(R29·D19).
- **줍기↔조회 병합 순서 규칙 없음** — 나중 쓰기가 이긴다(§6 #17).
- 출시 전 법무·보안 확인(D3)은 A 에도 그대로 걸린다 — 표지만.

**대상:** 없음. PRD D16–D21 의 「확인 필요」 기본값대로 짠다.

## 3. 내가 정한 내부 선택 (보고)

1. 줍기 운반 = StatusSink 훅 + pump drain — `open_spawn` 포트는 호출 약 열 곳이 바뀌고 디코더를 부수효과 있게 만든다.
2. 관측 시각은 데몬이 찍는다(대기 맵에 든 시각) — 디코더는 시계를 안 갖는다.
3. `used_pct` 없이 `resets_at` 만 바뀐 창은 % 를 지운다 — 새 창의 옛 % 는 틀린 값이다.
4. D12 의 「새로 들어옴」 = `used_pct` 가 실린 창만.
5. `ProbeError` 에 `Upstream`(분류 불가)·`NotInstalled`(R31 첫 상태) 추가.
6. Claude A 의 모르는 오류 → `Upstream`(= 조회 실패), 한도로 읽히는 문구만 `RateLimited{None}` — 모르는 오류를 "거절됨" 으로 보이면 R31 을 속이고 ⟳ 를 막는다. 자동 조회 시각은 어느 쪽이든 같다(max 식).
7. Codex RPC 오류 → 인증 문구면 `Unauthenticated`, 그 밖 `Upstream`.
8. 쿨타임·시한 상수는 각 벤더 `usage.rs` 가 `UsagePolicy` 로 낸다 — 데몬·`usage/` 에는 벤더 정책이 없다(「백엔드 확장」 · ADR-0004).
9. probe env = `CLAUDE*` 제거 + 허용 목록(`CLAUDE_CONFIG_DIR` · `CLAUDE_CODE_OAUTH_TOKEN`) + 탐침과 같은 네 키 — 계정·인증까지 벗기면 D13 기본 로그인이 바뀔 수 있다. Codex 는 상속 그대로.
10. probe cwd·설정 파일 = 조회마다 새 임시 폴더(끝나면 지움) — 작업 폴더에 기대지 않고, JSON 을 `cmd.exe` 인용에 태우지 않는다.
11. ISO 8601 → epoch 초 = `chrono`(이미 `Cargo.lock` 0.4.45) — **agent 직접 의존으로 새로 든다(의존성 보고 대상)**.
12. 시각 = epoch 초 정수(core `i64` · wire `u64`) · 경과는 상대 초.
13. 모델별 창 = Claude `model_scoped[]` + 값 있는 `seven_day_opus`/`_sonnet`/`_oauth_apps`, Codex 비기본 버킷의 주간 창(D8).
14. 입구 둘 → 같은 서비스: 웹뷰 = 전용 wire 요청 · LLM = 버스. 웹뷰에는 버스 생산자도 `handleEvent` 의 `CommandReply` 갈래도 없다(`messages.rs:555-576`).
15. wire 요청 = `Detached` — 기다리는 요청이 같은 연결의 터미널 입력을 막지 않게.
16. `REPLY_WAIT_MAX = 5초` — 버스 마감 7초(`command_delivery.rs:464` · CLI 침묵 한도 10초 `types.rs:256`)와 셸 답장 상한 30초 아래. 넘으면 캐시 + `in_flight`.
17. 버스 핸들러 = 동기 포트 + `blocking_handler`, 데몬 어댑터가 blocking 풀 위에서 std 채널 `recv_timeout` 으로 기다린다 — 데몬 입구가 첫 poll 에서 끝나는 핸들러만 몰고(`control/commands.rs:104-125`), `Handle::block_on(timeout)` 은 종료 중 타이머 패닉 → abort 위험이 있다.
18. 버스 선언 = agent `commands.rs` 블록 — 어휘(백엔드 낱말·행 타입)의 생산자가 agent 이고(ADR-0155 결정 1) `agent.new` 의 `AgentBackend` 를 그대로 쓴다. 실물은 데몬 포트.
19. 번호는 전부 「머지 시점 master 값 + 1」 — `PROTOCOL_VERSION` · agent catalog · shell catalog.
20. `setSlotContent` 의 빠진 show_* = 기존 값 유지(락 안 read-modify-write) — 사람 경로는 전량 교체.
21. 메뉴 ☑ = `SlotMenuItem.checked` 선택 칸 + ctx 에 슬롯 내용 — title 이 빌드 때 고정이라(`slotMenu.ts:43-52`) 상태를 실을 곳이 없고, 기여가 스토어를 읽으면 ADR-0064 를 어긴다.
22. 폭 단계 = 숨은 렌더 실측 비교 — 글자 수·px 상수는 로케일에서 깨진다(R3).
23. `STALE_AFTER_SECS = 1800`(프론트) — 나이가 프론트에서 흐른다.
24. danger 색 = github 계열(`#f85149`/`#cf222e`) — 기존 초록·노랑이 github 계열이다(ADR-0062).
25. 벤더 파일 분리(`backend/claude/usage.rs`·`backend/codex/usage.rs`) — `mod.rs` 충돌면을 줄인다.
26. 프론트 registry id `usageSlot.*` 에 `help` 를 달지 않는다 — 버스 `usage.refresh` 와 겹치지 않는다(`viewCommandBridge.ts:40-58`).
27. OS 분기(기동 플래그·트리 kill·종료코드) = `usage/process.rs` 한 모듈 — 기존 `CREATE_NO_WINDOW` 인라인 두 벌은 병행 브랜치 충돌면이라 합치지 않는다.
28. 시계 seam `UsageClock` — 타이머가 없어 요청 시각에 시계로 잰다.
29. 만료 래치 = 데몬 소유, 요청 때 평가 — 프론트 벽시계로 판정하면 되감기에 옛 % 가 되살아난다.
30. 거절 저장 = `usage_rejects.json` · `usage_service/reject_store.rs` · 조립 때 먼저 읽음 — 값·쿨타임은 저장하지 않는다(R29).
31. Windows 미설치 = EOF 뒤 `cmd.exe` 종료코드 9009 — 경로 탐색 crate 를 들이지 않는다.
32. 사용률 정규화 = 0–1 ×100 뒤 소수 넷째 자리 반올림 — 0.55×100 = 55.00000000000001 → 내림 44(참값 45)를 막는다.
33. 상태 키 = `(vendor, account_key)`, `account_key` 는 늘 `"default"` — 여러 데몬에 붙는 미래 클라이언트가 출처를 가르게. Codex `accountId` 가 장래 출처.
34. 폴링 = 프론트 `USAGE_POLL_MS = 60_000` · 보임 = 창 보임 ∧ 슬롯 교차 ∧ 연결 · `pending` 은 웹뷰 스토어 공유.
35. 줍기 → 화면 지연 ≤ 60초 수용 — 방송 없이 가는 대가.
36. 줍기 = `(vendor 키, 창)` 당 항목 하나의 대기 맵 + 요청 때 병합 — pump 스레드는 책과 같은 `merge_window` 로 칸 단위 합치기만 한다(통째 덮어쓰기면 리셋만 온 관측이 앞 % 를 지운다). 키 수가 유한해 넘침·버림이 없고, 적용은 「항목 mono ≥ 그 창의 마지막 적용 시각」일 때만이라 비우는 순서가 뒤바뀌어도 옛 값이 새 값을 되돌리지 않는다. D12 는 창별 최신값만 있으면 서므로 잃는 것이 없다.
37. `REJECT_MAX = 24시간` — 관측된 가장 긴 `Retry-After` 는 1시간(조회 §0-3)이라 정상값은 안 자르고, 깨진 헤더·재시작 사이 시계 이동이 만드는 터무니없는 대기만 자른다.
38. `REFRESH_MIN_SPACING = 30초`(회사별, 사람·LLM 공통) — LLM 루프가 ⟳ 를 연타해 상류 한도를 태우지 않게. 쿨타임(15분)은 여전히 무시한다(D11).
39. 조회 = 분리 std 스레드 + Job 종료 kill — 대안 `UsageService::shutdown` 은 이미 순서 위험이 적힌 종료 절차(`lib.rs:763-765`)에 단계를 늘리고 크래시는 못 덮는다. Job 은 둘 다 덮는다.
40. 비기본 프로필 줍기 차단 = `open_spawn` 의 `UsageGate` — 다른 계정의 값이 `"default"` 칸에 섞이지 않게.
41. 조회 마감 = 쓰기·읽기·종료 대기 전부를 덮는 하나 — stdout 을 닫고 안 죽는 자식이 완료 가드를 붙잡지 못하게. `KILL_WAIT = 2초`는 kill 뒤 파일 잠금이 풀리기를 기다리는 상한이다.
42. 임시 폴더 = 데이터 폴더 `usage-probe/` 아래 무작위 이름 배타 생성 + 기동 때 쓸기 — `--mcp-config` 파일을 claude 가 믿고 읽는다. 공유 OS 임시 폴더보다 데이터 폴더가 데몬 하나의 것이라 쓸기가 안전하다.
43. 키 정본 = `usage_probe_for(..)?.key()` — 입구마다 대소문자가 다른 낱말이 한 칸으로 모인다.
44. 무패닉 해석 = 칸 단위로 버림 — 릴리즈 abort 에서 모양 하나 바뀐 응답이 데몬을 죽이지 않게.

## 4. 테스트 계획

| 모듈 · seam | 무엇을 | PRD §4 | 실프로세스 |
|---|---|---|---|
| Claude 디코더 | 합성 `rate_limit_event` — 새 모양 · 옛 모양(리셋만) · 빈 `rate_limit_info` · 타입 틀린 칸 · 모르는 type → `take_usage`. 0.55 → 55.0 · `UsageGate{false}` → 빈 벡터 · 깨진 입력(칸 빠짐·타입 틀림·거대 수·NaN·음수) → 패닉 없이 그 칸 None | R13·R16·R20 | 없음 |
| Codex 디코더 | 합성 `account/rateLimits/updated` — 300/10080 분류 · `primary` 가 주간 · null 창 · 모르는 길이 · `OutputEvent` 0개 · 같은 깨진 입력 묶음 → 패닉 없음 | R13·R18 | 없음 |
| 비기본 프로필 판정 | spec env 에 다른 `CLAUDE_CONFIG_DIR`/`CODEX_HOME` → `collect=false` · 같거나 없음 → `true` | — | 없음 |
| 등록부 | `usage_probes()` 두 키가 `agent.new` 낱말과 대소문자 무시로 맞음 · 정책 값(15분·15초 / 5분·30초) · `usage_probe_for` 모르는 낱말 → None | R15·R30 | 없음 |
| probe 해석기·대화 | 합성 JSON(가짜 수치) — 성공·`rate_limits: null`·`model_scoped`·ISO 아님 → 그 칸 None·봉투 깨짐 → `Parse` · 깨진 입력 묶음(칸 빠짐·타입 틀림·거대 수·NaN·음수·ISO 아님) → 패닉 없음 · 오류 분류 표 · Codex 응답·RPC 오류 · `accountId` 가 로그·오류 문자열에 안 실림 · 가짜 `ProbeSpawner` 대본 — 다른 줄·`request_id` 불일치·시한 → `Timeout` + kill · ★stdout 을 닫고 안 죽는 가짜 자식 → 마감에 `Timeout` + 트리 kill + 완료 가드 풀림★ · 안 읽는 자식(쓰기 막힘) → 마감 · 서버 요청 거절 · env 제거 목록 · 인자에 `bypassPermissions` 없음 | R21·R30·R31 | 없음 |
| `OsProbeSpawner`·임시 폴더 | 실 자식 줄 왕복·시한·트리 kill · stdout 닫고 살아 있는 실 자식 → 마감 kill · 없는 프로그램 → `NotInstalled`(Windows 9009 · POSIX `NotFound` — 시험 파일 통째 `cfg(windows)` 금지, ADR-0230) · 임시 폴더 배타 생성(같은 이름이 있으면 새 이름)·kill 뒤 삭제·삭제 실패 무시·기동 쓸기 | R30·R31 | **있음** — agent(`--test-threads=4`) |
| 실 CLI 스모크 | ★운영 명령 그대로★ claude `get_usage` · 빈 `CLAUDE_CONFIG_DIR` 로 로그아웃 오류 모양 수집 · codex `account/rateLimits/read` — `#[ignore]`, 로컬 실행·결과 기록이 3단계 완료 조건 | R17·R31 | **있음** |
| `UsageBook` 병합 | 줍기 빠진 창 유지 · 능동 null·빠진 창 → 없음 · fresh 두 창 따로 도착 시 초기화(D12) · 실패 시 값 유지 · null ≠ 0 · 능동 완료 전 대기 맵이 먼저 비워짐 | R16·R21·R22 | 없음 |
| `UsageBook` 판정(가짜 시계) | 기한 전 `get` = 캐시·조회 0 · 거절 종료·리셋 경과가 쿨타임 전이면 조회 안 함 · 거절 중 줍기 초기화가 거절을 못 줄임 · `refresh` = 쿨타임 무시·거절 중 `Rejected`·30초 안 재요청 = 캐시·조회 0 · `retry_after` 있음/없음/0/과거/100일 → 5분·24시간 절단 · `mono` 3시간 점프 뒤 첫 요청 → 1회 · `wall` ±2시간 → 불변 · 시한 → `Failed` + 다음 = 쿨타임 | R14·R24·R29·R30 | 없음 |
| 요청·합류·수명(tokio 가짜 시간) | 동시 요청 N → 회사당 조회 1 · Codex 가 막혀도 Claude 즉시 · 5초 넘으면 캐시+`in_flight`, 완료 뒤 다음 요청 = `Fresh` · 요청 future 를 버려도 완료 가드가 `in_flight` 를 푼다(시험 빌드에서는 가짜 조회기 패닉에도 — 릴리즈는 abort 라 해당 없음) · 버스 동기 대기(`recv_timeout`) 가 런타임 종료 중에도 패닉 없이 캐시로 끝남 · 막힌 조회기가 있어도 런타임 종료가 즉시 끝남 | R25·R29·R30 | 없음 |
| 대기 맵·decorator | ★비우기 전 「% 40 → 리셋만(같은 리셋)」 → % 40 유지 + 리셋 갱신★ · 「% 40 → 바뀐 리셋만」 → 책에 차례로 적용한 결과와 같음(% 지움) · 합친 결과 = 차례 적용 결과(속성 시험) · 옛 mono 항목이 늦게 비워져도 적용 안 됨(N5) · poison 뒤에도 덮어쓰기·비우기 · `UsageInboxSink` 가 다른 훅을 모두 넘김 · `MessagingFlushSink` 가 `usage_observed` 를 넘김 | R13·R16 | 없음 |
| 래치·상태 | 리셋 지난 뒤 첫 요청이 래치 → `wall` 되감기에도 유지 · 새 값만 해제 · 네 상태 접기 · 거절 끝 → `Failed` · 나이 = `mono` | R31·R32 | 없음 |
| `RejectStore`·기동 | 파일 왕복 · 미래 항목만 복원 · 24시간 절단 · `checked_add` 넘침 항목 → 버림(패닉 없음) · 깨진 파일 → 빈 표 · ★Claude·Codex 동시 거절 → 다시 읽기에 둘 다 남음★ · ★파일 항목 정규화(대소문자 다른 낱말 → 같은 칸 · 모르는 낱말 버림)★ · 조립 직후 첫 요청이 저장된 거절을 못 넘음 · 테스트 조립은 가짜 조회기만 받음 | R29 | 없음 |
| wire | codec golden 새 variant 셋 + `UsageVendorState` 다섯 · 상관 함수 쌍 · `dispatch_order` = `Detached` · ts-rs sync · ★wire(`"claude"`)·버스(`"Claude"`) 요청이 같은 칸을 침★(N3) | — | 없음 |
| 버스 | `usage.get`/`usage.refresh` 스키마·catalog · ★`call_daemon_command` 로 불러 기다리는 경우에도 `OUTCOME_UNKNOWN` 이 아니라 행이 옴(blocking 풀 스레드에서)★ · `left_pct` 내림 | R26·R27 | 없음 |
| 셸 | `SlotContent::Usage` serde 기본값 · `slot_content()` 반려 · `set_usage_slot` 병합 · catalog | R9·R26 | 없음(`lib_unit`) |
| 프론트 폴러 | 가짜 타이머 + 가시성·IO 모의 — 숨은 탭·창 hidden·비연결 → 0건 · 보이게 됨·connected 전이 → 즉시 회사당 1건 · 60초마다 · `pending` 이면 건너뜀 · 슬롯 둘 → 안 겹침 · ⟳ → refresh · 꺼진 회사 0 · 실패 → 값 유지 | R8·R12·R29 | 없음(vitest) |
| 프론트 표시 | `usageFormat` — 19.6→19 빨강 · 20.4→20 노랑 · 50.9→50 노랑 · 51→초록 · 정확히 50·20 노랑 · 0 빨강 ⚠ · 100 초록 · −5→0 · 130→100 · 없음 → 회색 "—" · 나이 · 30분 경계 · 만료 · "갱신 중" · 폭 단계·말줄임·1–2단만 남은 시간 · meter ARIA · 팝업 키보드 · 네 상태 문구 · 둘 다 꺼짐 안내 · 색 토큰만(e-ink) · 메뉴 `checked` 가 ctx 의 슬롯 내용만 읽음(스토어 무접촉) · 등록점 | R1–R11·R21·R28·R31·R32 | 없음(vitest) |
| 화면 실측 | PRD §4 전 항목 — `/qa full`(cdp) · ★릴리즈 빌드에서 조회 때 콘솔 창이 튀지 않는다★(눈 확인) | 전부 | 앱 |

## 5. 이행 순서 (커밋마다 빌드·시험 초록 — 어디서 멈춰도 선다)

**착수 조건:** `v0.3.2/feat/json-midturn-queue` 가 master 에 머지된 뒤 — 그 브랜치가 `PROTOCOL_VERSION` 5→6 · agent catalog 4→5 · `StatusSink::inputs_drained`(`turn_ended` 바로 뒤) 를 더하고 codex transport·`output_core`·`connection_core`·`messages.rs`·`protocolClient.ts` 를 크게 바꾼다. 머지된 트리에서 이 문서의 줄 포인터를 다시 잰다.

**머지 체크리스트(각 해당 단계에서 확인):** ① `PROTOCOL_VERSION` = 머지 시점 master + 1 ② agent catalog = master + 1 · shell catalog = master + 1 ③ `MessagingFlushSink`·`UsageInboxSink` 가 `inputs_drained`·`usage_observed` 를 둘 다 넘김 ④ `dispatch_order` 에 새 갈래(`Detached`) ⑤ `command_request_id`·`event_reply_request_id` 쌍 ⑥ 생성물 세 폴더(agent `bindings/` 의 `commands.schema.json` 포함 — `agent/tests/ts_export.rs:94`) 재생성.

1. `S21: feat(agent,protocol): 사용량 중립 타입·wire 도메인 타입 + StatusSink::usage_observed 훅(기본 no-op)` — 데몬 decorator 전달까지 · ts-rs 생성물 함께.
2. `S21: feat(agent): Claude·Codex 줍기 해석 + take_usage · pump drain · UsageGate(비기본 프로필)` — 데몬 sink 는 no-op 이라 동작 불변.
3. `S21: feat(agent): 조회 seam + OsProbeSpawner + Claude A·Codex + backend 등록부(정책)` — `chrono` 의존. ★완료 = 운영 명령 그대로의 실 Windows 스모크 로컬 성공 + step-log 기록★.
4. `S21: feat(daemon): UsageService — UsageBook·UsageClock·RejectStore·줍기 대기 맵·요청 판정·합류·완료 가드(가짜 시계·조회기 시험)` — 아직 조립에 안 붙는다.
5. `S21: feat(protocol,daemon): 사용량 요청 wire(Detached) + PROTOCOL_VERSION + 조립(거절 먼저 로드·조회기 주입·UsageInboxSink) + 버스 usage.*(blocking)` — `dispatch_order`·상관 함수 쌍이 variant 와 같은 커밋에서 닫혀야 빌드가 선다 · `make_table`/`make_daemon_table` 호출 자리(시험 여덟 포함 — §1-4)를 함께 고친다 · agent `bindings/`(`commands.schema.json` 포함) 재생성. ★완료 = 실 데몬에서 `engram usage.get --backend Claude` 가 blocking 입구를 지나 `OUTCOME_UNKNOWN` 이 아닌 행을 돌려줌★.
6. `S21: feat(shell): Usage 슬롯 콘텐츠 + setSlotContent show_* + 링크 권한` — 프론트 타입 검사에 필요한 최소 분기만 함께.
7. `S21: feat(front): agentClient 요청·답 갈래 · usageStore · 폴러 · UsageSlot·팝업·메뉴·usageFormat·테마·문구` + `prompts/engram-help.md`.
8. `S21: docs: step-log · ADR(/adr) · TRD 「구현:」 표시`.

## 6. 위험·미결·확인 항목

1. **병행 브랜치 `v0.3.2/feat/json-midturn-queue`(wt1, `/qa full` PASS 단계)** — 번호 둘·`StatusSink` 훅·codex transport·`output_core`·`connection_core`·`messages.rs`·`protocolClient.ts` 가 겹친다. 대응 = 머지 뒤 착수 + §5 체크리스트. Claude `mod.rs` 는 skip 팔 한 곳 + 시그니처 한 칸으로 줄이고 probe 인자를 `build_spec` 에서 파생하지 않는다(대가 = 인자 두 벌 — 시험으로 stream-json 핵심 인자 일치를 잰다).
2. **확인 항목 — 잠든 시간을 세는 시계(R29).** 사실로 적지 않는다: Rust `Instant` 가 절전 시간을 세는지는 OS 마다 다르다고 알려져 있다(Windows QPC 는 센다 · Linux `CLOCK_MONOTONIC`·macOS 는 안 센다 — 미검). 다르면 그 분기는 `UsageClock` 실물의 한 함수 안에만 둔다.
3. **운영 기동 경로** — EXPERIMENTAL · "not supported in this context" 분기(모드 미식별) · 허용 목록 env·`bypassPermissions` 제거·`--settings <파일>` 은 탐침과 다르다 · `cmd.exe /c claude` shim · 9009 판정 · POSIX 트리 kill·stdin EOF 동작 미검 → 3단계 완료 조건의 로컬 스모크가 확인한다.
4. **A 의 오류 표면 미실측** — 429·로그아웃 문구를 모른다. §3 #6 의 문구 표는 추정이다.
5. **Codex** — `account/rateLimits/read` params 모양(`request_line` 은 늘 `params` 를 싣는다 `protocol.rs:278-288`)·로그인 안 됨 오류 모양은 스키마 재추출(`protocol.rs:3-6`)로 확인 · 데몬 재시작 직후 대화 이어받기에서 `initialize` 약 9.7초 관측 1회(원인 모름 — 30초 시한이 덮는다) · 알림 도착 빈도 모름(UI §9) · `accountId` 로그 금지.
6. **거절 저장과 시계의 긴장(수용)** — 재시작을 건너는 대기만 벽시계라 재시작 사이 시계를 옮기면 복원 대기가 틀어진다(`REJECT_MAX` 가 상한).
7. **만료 래치의 틈** — 래치는 리셋 뒤 **첫 요청** 때 선다(보이는 동안 ≤ 60초, 보이는 슬롯이 없으면 다음에 보일 때까지). 그 전에 시계를 되감으면 옛 % 가 보일 수 있다 — PRD §4 R32 항목을 「리셋 경과를 데몬이 확인한 뒤」로 좁혔다.
8. **계정(D13)** — 조회는 데몬 env 의 기본 로그인만 본다(`config_dir()` `mod.rs:993-1000`). 비기본 프로필 에이전트는 줍기도 막았다(§1-2). ★`/login` 으로 계정을 바꾸면 이미 떠 있는 에이전트는 재시작 전까지 옛 계정 값을 주울 수 있다★ — 조회 값과 한동안 섞인다.
9. **지시와 코드의 어긋남 — 기다림 상한.** 「기한이 지나면 조회하고 답한다」는 버스 마감 7초·셸 답장 상한 30초와 부딪친다(Claude 15초·Codex 30초 시한). 최소 조정 = `REPLY_WAIT_MAX` 뒤 캐시 + `in_flight`(§3 #16) · wire 요청 `Detached`(§3 #15).
10. **skeleton 과 어긋난 자리(리뷰 확인 요청)** — ① StatusSink 훅 + pump drain ② 버스 선언이 agent 블록(동기 포트) ③ 번호 셋 bump ④ `ProbeError` 두 칸 ⑤ ADR-0064 스키마에 `checked`(§7 #8) ⑥ opener 권한 = 앱 권한 설정 변경 ⑦ LLM `setSlotContent` 부분 패치 ⑧ `(vendor, account_key)` 키 ⑨ 웹뷰는 전용 wire 요청 ⑩ `REPLY_WAIT_MAX` ⑪ 새 decorator `UsageInboxSink`.
11. **`PROTOCOL_VERSION` bump** — 구데몬이 떠 있으면 기존 불일치 처리(discovery `check_acceptable`)를 탄다.
12. **멀티 데몬** — 데몬별 조회가 같은 계정의 조회 제한을 나눠 쓴다 · Claude 응답에 계정 식별자 없음 · 범위 밖. 단일 인스턴스 가드는 **데이터 폴더당 하나**라(`net/src/instance.rs:1-5`·`:216`) 같은 PC 도 폴더가 다르면 둘이 뜬다(릴리즈 데이터 폴더 = exe 옆 `data/`, env 재정의 가능 — `discovery/src/lib.rs:80-87`). 거절 파일도 폴더마다 따로다.
13. **강한 조회 제한** — 15분도 안전이 증명된 간격이 아니다(조회 §0-3). A·B 가 같은 버킷을 쓰는지 미확인(§0-4). ⟳ 는 30초 간격으로만 막는다.
14. **콘솔 창 사본 셋** — `CREATE_NO_WINDOW` 가 인라인으로 세 곳에 산다(기존 둘 + 스포너 모듈). 통합은 별건.
15. **멀티 클라이언트 범위 밖(사용자 결정)** — 여러 클라이언트가 각자 물어도 캐시가 조회를 합치지만 설계·검증 대상이 아니다. PRD R29 는 「모든 사용량 슬롯이 공유」로 고쳤다(2026-09-27).
16. **줍기 → 화면 지연** — 방송이 없어 대화 중 주운 값은 다음 요청(≤ 60초)에 보인다(수용).
17. **줍기↔조회 순서(규칙 없음 — 사용자 결정 2026-09-27)** — 조회 결과가 CLI 1분 공유 캐시 때문에 방금 주운 값을 잠깐 덮을 수 있다 — 피해 = 잠깐 조금 더 남은 것처럼 보임 · 실사용에서 이상하면 규칙 추가(리셋 시각은 소수점 아래가 호출마다 달라 허용 오차가 필요 — 실측).
18. **개발 빌드의 조회 cwd** — 임시 폴더가 데이터 폴더 아래라 개발 빌드에서는 저장소 안에 놓일 수 있다 → claude 가 위로 올라가 `.claude/settings.json`·CLAUDE.md·`.mcp.json` 을 읽을 수 있다(훅·MCP 는 이미 막았다). 3단계 실 스모크를 데이터 폴더가 저장소 안인 상태로 한 번 돌려 확인한다.

## 7. ADR 제안 (번호는 `/adr` 가 준다)

1. **사용량 = 데몬이 소유하는 `(회사, 계정)` 단위 상태, 벤더 정책은 backend 등록부** — 거부: 에이전트별 `OutputEvent` variant(에이전트별·replay 대상이라 수명이 다르다) · 셸 소유(여러 창이 한 계정을 따로 센다) · 데몬 쪽 벤더 상수·enum(「백엔드 확장」 위반).
2. **사용량은 요청(pull)형 — 데몬 캐시 + 오래되면 갱신** — 사용자 결정 2026-09-27. 거부: 수요 신호 + 방송(push — 셸의 활성 탭 파생·재연결 재전송·팝아웃 정리·데몬 타이머가 따라온다) · 클라이언트가 쿨타임을 세어 조회 시점을 정하는 타이머(판정이 클라이언트마다 갈린다). 대가 = 줍기 지연 ≤ 60초.
3. **줍기 출구 = StatusSink 훅 + pump drain + 데몬 대기 맵((회사, 창)당 한 칸 · 칸 단위 병합)** — 거부: `open_spawn` 포트 주입 · `Structured` 배출구(`decoder.rs:18-25`) · pump 스레드에서 책 락 잡고 병합 · 전역 FIFO 큐(넘침 버림·비우는 순서 역전).
4. **Claude 능동 조회 = A(짧게 뜨는 CLI 에 `get_usage`)** — 사용자 결정 2026-09-27. 지금은 거부: B(토큰 파일 + 비공개 OAuth 사용량 주소) — 앱이 자격증명을 만지고, 첫 TLS 의존·인터넷 송신·OS 별 자격증명 분기가 새로 생긴다. A 가 깨지면 같은 seam 뒤로 B 를 들인다.
5. **능동 조회 = 짧게 뜨는 전용 프로세스, 분리 스레드 + Job 종료 kill** — 거부: 떠 있는 에이전트 프로세스에 보내기(D7) · 대화 통로에 일반 RPC 표면(`transport/mod.rs:51-90`) · 명시 shutdown 단계(크래시를 못 덮는다).
6. **시계 = 잠든 시간 포함 경과, 거절만 벽시계로 디스크(24시간 상한)** — 거부: 전부 벽시계(되감기에 대기가 늘고 준다) · 저장 안 함(재시작이 거절 중인 주소를 다시 친다).
7. **`usage.*` = agent 블록 선언(생산자 옆 — ADR-0155 결정 1) + 동기 포트 + `blocking_handler`** — 거부: 데몬 자기 블록 + 표 합성(세 표면에 합성을 새로 깐다) · async 핸들러(데몬 입구가 `OUTCOME_UNKNOWN` 으로 만든다).
8. **`SlotMenuItem.checked` — ADR-0064 고정 스키마의 additive 개정**(ADR-0065 선례) — 범위 = `checked` 칸 추가만, 상태는 메뉴 ctx 로 들어온다. 거부: title 에 상태 문자 넣기(title 이 빌드 때 고정) · 토글마다 메뉴 재등록 · 기여가 뷰 스토어를 직접 읽기(ADR-0064 「메뉴에서 직접 store 호출 금지」).
