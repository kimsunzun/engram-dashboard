# S17 제어 채널 — 조사 결과 통합 보고서

> 작성일: 2026-07-16  
> 범위: 이틀간의 제어 채널 논쟁을 종결한 두 독립 조사(메인 `/research` 팬아웃 + 사용자 별도 Codex 조사)의 핵심 발견을 영속화한다.  
> **최종 결정 정본 = [ADR-0086](../../decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md).** 이 문서는 결정의 근거가 된 조사 내용 자체를 보존하는 것이 목적이다.  
> Codex 조사 원본: `I:\claude-code-multiagent-messaging-research.md`(repo 밖 — 유실 위험 있어 핵심 내용 이 문서에 박제).

---

## 1. 조사 질문

에이전트 A→B 메시지 및 제어 커맨드를 전달하는 채널로 무엇이 검증된 패턴인가? 구체적으로:

- 자유 텍스트 마커(in-band output marker, M3)를 주채널로 사용한 성숙 OSS 구현이 존재하는가?
- `PreToolUse` 훅을 주 전송 경로로 사용한 사례가 있는가?
- 생태계 지배 패턴은 무엇이며, engram-dashboard의 분리 데몬 구조(ADR-0029)와 어떻게 맞는가?

---

## 2. 조사 방법 — 두 독립 조사 + cross-family 확증 구조

### 2-1. 메인 세션 `/research` 팬아웃

메인 Claude Code 세션이 `/research` 스킬로 3개 갈래의 서브에이전트를 병렬 스폰했다. 각 갈래가 서로 다른 생태계 표본(Orca, claude-squad, Claude Swarm, Agent Teams, MCP Channels, Tmux-Orchestrator, claude-flow 등)을 코드 수준에서 조사했다.

### 2-2. 사용자 별도 Codex 조사 (독립)

사용자가 메인 조사와 무관하게 별도로 Codex를 돌렸다. Codex의 `web_search` 툴은 이 시점에 미노출 상태(`NO_NATIVE_WEBSEARCH`)였으며, 순수 추론으로 동일한 OSS 표본군을 분석했다. 원인 미판정 — **"web_search 미노출이 설정 결함인지 Codex 빌드 회귀인지는 확인되지 않았다"(불확실).**

### 2-3. Cross-family 확증 성립

두 조사가 서로 모르는 채 독립적으로 수행되었고, 동일한 핵심 결론으로 수렴했다. 이로써 cross-family 확증이 성립하여 **결론 등급 = 확실**로 격상되었다. 추가로 Codex는 메인 조사의 2개 항목을 교정했다(§4 참조).

---

## 3. 수렴 결론

두 조사가 공통으로 도달한 결론은 다음과 같다.

- **자유 텍스트 마커 stdout 스크랩을 주채널로 채택한 성숙 OSS 구현 = 0건.** 손실 허용 가능한 관찰 신호(텔레메트리·종료 신호)에 쓸 수는 있으나, routing·control message의 source of truth로는 부적합하다.
- **`PreToolUse` 훅을 주 전송 경로로 채택한 구현 = 0건.** 훅은 tool execution 관찰, 보안 검사, inbox 알림 같은 보조 기능에는 적합하나, 에이전트가 툴을 호출하지 않으면 발화되지 않으므로 임의 inbound message transport가 될 수 없다.
- **원시 `tmux send-keys` 릴레이 = 안티패턴.** typed 계약이 없고 오배달이 발생한다.
- **생태계 지배 패턴 = typed 전용 인터페이스 + durable mailbox.** Orca(20.1k⭐)가 Bash carrier + typed CLI + SQLite WAL mailbox 조합으로 실증하고 있다.
- **MCP Channels는 에이전트 간 메시징 용도가 아님.** ephemeral 특성 + 배달 신뢰 이슈(GitHub issue #45563)로 인해 에이전트 간 제어 채널로 부적합하다.

---

## 4. Codex가 메인 조사를 교정한 2건

사용자 지시에 따라 교정 건에서는 Codex 판단을 우선 채택했다.

### 교정 ①: Orca의 전송 기제

**메인 조사 초안의 이해:** Orca가 출력 마커(매직스트링)를 사용하는 것으로 해석될 여지가 있었다.

**Codex 교정:** Orca(20.1k⭐, v1.4.143)는 **Bash carrier + typed CLI**를 사용한다. 에이전트가 일반 Bash 툴에서 구조화된 `orca orchestration send/reply/ask/check` CLI 커맨드를 실행하는 방식이다. "Bash로 typed CLI 실행"은 강한 패턴이며, "매직스트링 stdout 밀항"만이 0건이다. 이 두 개념을 혼동하지 않아야 한다.

참고 permalink(Codex 조사 원본 수록):
- DB schema: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/orchestration/db.ts#L82-L115>
- send/fan-out/delivery: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/rpc/methods/orchestration.ts#L207-L283>
- ask/reply: <https://github.com/stablyai/orca/blob/8c1d13dc52a84b0b38213fe2e76e1a84780b206e/src/main/runtime/rpc/methods/orchestration.ts#L365-L385>

### 교정 ②: Agent Teams 파일 메일박스의 공식성

**메인 조사 초안:** `~/.claude/teams/<team>/inboxes/<agent>.json` 파일 mailbox 구조를 Agent Teams의 동작 방식으로 기술했다.

**Codex 교정:** 이 구조는 Claude Code 2.1.45에 대한 **비공식 역공학 관찰**이다. 내부 소스가 공개되지 않았으며, 2026년 중 tool/model 통합 변경 제보도 있어 실제 설치 버전에 따라 동작이 다를 수 있다. **공식 API contract로 간주해서는 안 된다.**

---

## 5. 핵심 통찰

### "MCP냐 Bash냐"는 2차 문제

Codex가 도출한 핵심 통찰: 실제 분기점은 전송 수단(MCP vs Bash)이 아니다.

> **"typed ingress + durable mailbox를 오케스트레이터가 소유하는가"**

- 강한 패턴: typed ingress + durable mailbox + optional push
- 약한 패턴: 자유 텍스트 marker나 terminal 상태에만 의존하는 best-effort injection

Bash를 transport entry로 사용해도 payload와 error contract가 구조화되어 있으면 강한 패턴이다(Orca가 실증).

### Durable mailbox 소유권 — 3-state 분리

Orca의 `sequence` / `delivered_at` / `read_at` 분리 설계는 채택 가치가 높다:

- `sequence`: deterministic ordering (SQLite autoincrement, recipient별 monotonic)
- `delivered_at`: PTY/context에 이미 push한 메시지의 중복 재주입 방지
- `read_at`: consumer가 실제 inbox를 소비했는지 별도 표시

이 세 상태를 분리함으로써 "terminal에 넣었음" / "context에 전달됨" / "agent가 읽음"을 독립적으로 추적할 수 있다.

### Two-level ACK

생태계 지배 패턴은 두 수준의 ACK를 분리한다:

1. **동기 enqueue ACK** — "mailbox에 실렸다"까지만 확인 (recipient 실존 · schema 유효 · DB append 성공 · message ID 반환)
2. **비동기 semantic reply** — 별도 reply message / thread response / worker result

"상대가 읽었다"를 동기 ACK로 요구하지 않는다.

### Engram 분리 데몬의 구조 우위

Orca는 Electron 앱 내장 런타임으로, 앱과 함께 사망한다. Engram의 분리 데몬(ADR-0029)은 클라이언트(src-tauri 셸) 탈착 후에도 에이전트와 mailbox가 살아 있으므로 복구 시맨틱에서 구조 우위가 있다. "데몬이 PTY를 소유하고 클라이언트가 탈착 가능"한 구조는 tmux 서버가 오랫동안 실증한 고전 패턴이다.

---

## 6. 스파이크 실측 요약 (5건)

①~④는 조사 세션 수행, ⑤는 스텝 1 착수 직전(2026-07-16) 수행. 스파이크 하네스는 throwaway로 삭제됨. 필요 시 아래 조건으로 재현 가능.

### 스파이크 ①: stream-json 봉투 구조

`claude -p --output-format stream-json --verbose` 실행 시 봉투 타입 분석.

**발견:**
- `complete` 봉투에서는 `tool_use.input`이 fully-formed (delta 재조립 불필요).
- `--include-partial-messages` 일 때만 `partial_json`이 발생.
- 봉투 타입이 예상보다 많음: `user` / `rate_limit_event` / `system:thinking_tokens` / `system:status` / `stream_event` — **방어적 파싱 필수**.
- `result` = 프로세스 완료(원샷), `stop_reason` 확정값은 `message_delta`에만.

### 스파이크 ②: Bash 센티널 컴플라이언스 (haiku 모델)

구조화된 Bash 마커를 모델이 얼마나 준수하는지 측정.

**발견:** 6회 중 4회 clean-parse(66%). 실패 원인: 따옴표 포함 body에서 깨짐 / nonce 오기재 1회 / 모호한 수신자에서 되묻기. **이 수치가 마커 접근 폐기의 결정적 실측 근거다.** 엄격 지시를 내려도 66%는 주채널로 신뢰하기 어려운 수준이며, 약한 모델·긴 세션에서는 더 낮아진다.

### 스파이크 ③: 훅 기제 (문서 확인)

**발견:** `PreToolUse` 훅은 headless(`claude -p`)에서 발화 OK. 단 `deny` 응답 시 간헐적 idle 버그 #24327 존재. 이로써 훅 주채널 접근도 폐기 확증.

### 스파이크 ④: MCP 동적 로드 가능성 (문서 확인)

**발견:** MCP 서버는 시작 시 선언 필수(`system:init` 후 불변). `tools/list_changed`는 지원하지만 이는 연결된 서버의 툴셋 변경이지 새 서버 동적 추가가 아니다. dormant MCP 서버 비용은 미미 (`tool search deferred`, v2.1.191+ 기본 — 이름만 로드). **"연결 성공"이 스폰 직후 검증 가능한 마일스톤**이 된다.

### 스파이크 ⑤: mcp-config Bearer 헤더 실전송 (claude 2.1.170 실측)

웹 조사에서 헤더 탈락 버그 리포트 다수(#48514 open·#50464 not-planned 닫힘·#59467 OAuth 우선·#32191 headless 무음 종료 — v2.1.71~140+ 대역)가 발견되어 정지 조건에 걸렸고, 사용자 승인 하에 로컬 실측으로 판정했다.

**방법:** 헤더 전부를 로깅하는 최소 streamable HTTP MCP 응답 서버(node, 127.0.0.1:9876)를 띄우고, `headers: {"Authorization": "Bearer <토큰>"}`이 든 mcp-config로 `claude -p --allowedTools mcp__engramspike__spike_ping` 실행.

**발견 (전면 그린):**
- **Authorization 헤더가 모든 요청에 전송됨** — initialize · notifications/initialized · SSE GET · tools/list · tools/call 전부. #50464가 탈락을 보고한 `(sdk-cli)` user-agent 경로에서도 전송 확인.
- 서버가 내려준 `Mcp-Session-Id`를 클라이언트가 후속 요청마다 에코 → initialize 시점 토큰→세션 바인딩 설계 유효.
- headless 무음 종료(#32191) 재현 안 됨 — 툴 콜까지 E2E 성공(`pong` 수신). 프로토콜 버전 `2025-11-25`.

**함의:** 스텝 1의 "Bearer 헤더로 토큰 전달" 설계 유지 확정. 단 **우리 데몬 MCP 서버는 OAuth 메타데이터를 광고하면 안 됨**(#59467 — 광고 시 정적 Bearer가 무시됨).

---

## 7. 생태계 사례 정리

### Orca (★★★ — 지배 패턴 대표)

- Stars: 약 20.1k (2026-07-16 기준)
- 전송 기제: 에이전트가 Bash에서 `orca orchestration send/reply/ask/check` 타입 CLI 실행
- 배달: 중앙 Orca runtime → SQLite WAL mailbox
- Mailbox 필드: `sequence`, `read`, `delivered_at`, `thread_id`, `id` (unique index)
- ACK: `send`는 message ID/수신자 수 반환. `ask`는 동기 요청-응답 지원
- 복구: SQLite WAL + 영속 DB로 재시작 복구 가능
- 한계: exactly-once 보장은 확인되지 않음 (불확실)
- 구조 참고 permalink: 위 §4 교정 ① 참조

### mcp_agent_mail (참조 — MCP send_message 시맨틱)

MCP `send_message` tool 시맨틱 참조 구현. 핸드오프에 참조 목록으로 수록됨.

### awslabs CAO (참조 — env 신원 패턴)

CLI 입구의 환경변수 신원 패턴(예: `CAO_TERMINAL_ID`)을 실증한 사례. engram의 `ENGRAM_TOKEN` env 주입 설계의 참조 근거.

### claude-flow / Ruflo v3 (참조 — 주의 필요)

- Stars: 약 64.5k (2026-07-16 기준)
- 내부 `MessageBus`는 중앙 프로세스의 에이전트별 in-memory priority queue + EventEmitter 구조.
- `enablePersistence` 설정이 있으나 실제 persistence 연결을 Codex가 코드에서 확인하지 못함.
- retry 로직에서 결함 추정: `addToQueue()`에서 `attempts`를 0으로 재생성하는 코드 → retry bound가 의도대로 작동하지 않을 가능성. **코드상 결함 추정이며 확정 아님(불확실).**
- **참조 금지 판정:** 여러 세대의 구현이 공존하고 placeholder 다수. Codex 검토로 코드 신뢰도 낮음.
- 참고 permalink: <https://github.com/ruvnet/ruflo/blob/a0c1ac4b4ff84360cb85b577e1da81eb661a078f/v3/%40claude-flow/swarm/src/message-bus.ts>

### MCP Channels — Anthropic (참조 — 에이전트 간 제외)

- 공식 research preview, Claude Code v2.1.80+
- inbound: MCP server가 `notifications/claude/channel` notification push
- outbound: 모델이 channel MCP reply tool 호출
- **한계:** session이 열려 있을 때만 event 도착. 내장 durable queue/replay 없음. ordering/dedup/재시작 복구는 channel server 또는 외부 플랫폼 책임.
- 에이전트 간 메시징 용도로는 ephemeral 특성 + GitHub issue #45563의 배달 신뢰 이슈로 부적합 판정.
- 공식 문서: <https://code.claude.com/docs/en/channels-reference>

### Tmux-Orchestrator 계열 (안티패턴)

- 원시 `tmux send-keys` + sleep: delivery guarantee 없음(keyboard injection에 불과).
- 개선된 구현(literal mode, idle handshake, status file)도 message dedup/transactional replay 없음.
- **판정:** active-session 웨이크업 transport로만 제한 사용 가능. source of truth 불가.

### Claude Swarm v1 / parruda (참조 — 계층적 RPC 패턴)

- 각 Claude Code 인스턴스에 다른 에이전트를 MCP tool/server로 노출. delegate MCP tool 호출 → 대상 `claude -p` 프로세스 실행 → 결과가 MCP `tool_result`로 반환.
- A가 B에게 일을 위임하고 결과 하나만 기다리는 계층적 RPC에 적합.
- 장기 mailbox나 자유 peer chat과는 다른 패턴.
- v1 최신 고정 SHA는 조사 시점 repo redirect/clone 불안정으로 **확보하지 못함(불확실).**

---

## 8. 최종 결정 포인터

위 조사 결과를 근거로 내려진 최종 결정은 **[ADR-0086](../../decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md)**에 박제되어 있다.

요약: **듀얼 typed 입구(MCP `McpIngress` + CLI `CliIngress`) + 공통 `ControlIngress` seam + SQLite WAL `Mailbox`(source of truth) + `Dispatcher` push.** 기본 입구(MCP vs CLI) 확정은 스텝 6 A/B 실측 후 사용자 결정.

ADR-0086이 폐기한 이전 결정: ADR-0085(마커=주채널 — 전체 폐기).

---

## 9. 출처 목록

1. **Codex 조사 원본 (primary):** `I:\claude-code-multiagent-messaging-research.md` — repo 밖, 유실 위험 있어 이 문서에 핵심 내용 박제. 조사 기준일 2026-07-16.
2. **세션 핸드오프:** `.claude/handoff/latest.md` — 두 독립 조사 수렴·Codex 2건 교정·스파이크 4건·아키텍처 승인 상태 기록.
3. **ADR-0086:** `docs/decisions/0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md` — 최종 결정 정본.
4. **M3 시절 검토 노트:** `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` — ADR-0085 결정 당시 쟁점·실측 스냅샷 (현재 superseded).
