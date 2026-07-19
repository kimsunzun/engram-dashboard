# ADR-0086: 제어 채널 = 듀얼 typed 입구(MCP+CLI) + SQLite 메일박스-first — 마커(M3) 폐기

- 상태: 확정 (2026-07-16, 근거: 두 독립 조사 수렴(메인 /research 팬아웃 + 사용자 별도 Codex 조사 — cross-family 확증) + 스파이크 실측 4건 + 사용자 풀 브리핑 승인)
- 관련: Supersedes ADR-0085(마커=주채널 — 전체 폐기) · ADR-0080(engram-ctl ingress — typed CLI 발상은 부활, 이름은 재사용 금지) · ADR-0081(UI opaque-relay·권위 2도메인 존속 — 이 번복과 무관하게 유효) · ADR-0007(epoch — 토큰 회전 연동) · ADR-0029(데몬=에이전트 호스트) · ADR-0002/0030(capability matrix) · CLAUDE.md §5(LLM-우선 제어) · 결정 노트 `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` · Codex 조사 원본 `I:\claude-code-multiagent-messaging-research.md`(repo 밖) · Amended by ADR-0087 (스텝 사다리 ②③④ 분할 순서 → 2-min 최소 전송 일괄 선행 + SQLite 메일박스 보류(사용자 학습 후 재개))

## 맥락
에이전트 A→B 메시지(및 향후 제어 커맨드)의 채널을 정해야 한다. ADR-0085는 "데몬이 이미 stdout을 소유하니 in-band 출력 마커(M3)가 최단 경로"로 채택했으나, 근거였던 38/38 실측은 엄격 통제 조건이었다. 이후 두 독립 조사(메인 /research 3갈래 팬아웃 + 사용자 별도 Codex 조사)가 같은 결론으로 수렴했다:

- 자유 텍스트 마커 stdout 스크랩을 **주채널로 채택한 OSS 사례 0건**. PreToolUse 훅 주채널 0건. 원시 tmux send-keys는 안티패턴.
- 생태계 지배 패턴 = **typed 전용 인터페이스 + durable mailbox**. Orca(20.1k⭐)는 Bash carrier + typed CLI(매직스트링 아님)를 실증.
- 후속 Bash 센티널 스파이크(haiku)에서 컴플라이언스 66%(6회 중 4회 clean-parse — 따옴표 body 깨짐·nonce 오기재·모호 수신자 되묻기).
- Codex 핵심 통찰: MCP냐 Bash냐는 2차 문제. 진짜 분기점 = **"typed ingress + durable mailbox를 오케스트레이터가 소유하는가"**.

## 결정
**제어 채널 = 듀얼 typed 입구 + 공통 파이프라인 + SQLite 메일박스-first.** 마커(M3)는 폐기한다.

```
[스폰 시 1회] 데몬: (AgentId, epoch)별 토큰 발급
   ├→ MCP용: 에이전트별 mcp-config 생성 (127.0.0.1:PORT + Bearer 토큰 헤더)
   └→ CLI용: PTY env 주입 (ENGRAM_TOKEN)   ※epoch 회전=구토큰 폐기, kill=즉시 폐기

[런타임] 에이전트
   ├─[입구① McpIngress] send_message tool_use → 웜 연결(시작 시 1회 바인딩) — 기본 후보
   └─[입구② CliIngress] Bash("engram send --to B ...") → 콜마다 접속+env토큰 (스텝5)
        ▼ ControlIngress seam — 둘 다 정규화
   ControlCommand { from*, cmd, args }   *from = 페이로드 아님, 토큰에서 파생(사칭 차단)
        ▼ Validator (수신자 실존·스키마·상한 · 후일 authz)
        ▼ Mailbox: SQLite WAL append = ★source of truth★
          (id·from·to·sequence·body·created_at·delivered_at·read_at)
        ▼ Dispatcher: 온라인→ write_stdin push / 오프라인→ 장부 대기, 복귀 시 배달
        ▼ 동기 enqueue-ACK (입구 무관 동일 JSON)
```

- **two-level ACK:** 동기 ACK = "장부에 실렸다"까지만("읽었다" ACK 없음). 의미적 응답 = 별도 메시지/스레드. (생태계 지배 패턴)
- **커맨드 = 의도별 전용 툴 분리**(제네릭 메가툴 채택 사례 0). 이번 스코프 = send 하나, spawn/창이동 등은 additive로 후속. **워커에겐 send_message만 노출**(least-privilege = authz + MCP 컨텍스트 세금 동시 해결).
- **인증:** 스폰 시 (AgentId, epoch)별 토큰 → 레지스트리. MCP = mcp-config에 박아 연결 시 1회 바인딩(토큰이 모델 셸 밖 — claude.exe 전송층이 자동 처리) / CLI = env로 콜마다 제시(콜=프로세스라 신원 고정 불가). `from`은 항상 토큰에서 파생 — 페이로드 from 무시.
- **TUI 모드 = 제어채널 제외**(사용자 결정 — 별도 처리 없이 "파싱 안 되니 자연 제외").
- **스텝 사다리(한 스텝 = 한 검증 게이트):** ①토큰+MCP 입구 연결 ②send_message 툴+Validator+enqueue-ACK ③SQLite 메일박스 ④Dispatcher push+delivered_at+오프라인 복구 ⑤CLI 입구 ⑥A/B 실측 → **기본 입구 결정 = 사용자**.
- 의존성 추가: **rmcp(공식 Rust MCP SDK)**.

## 거부한 대안
- **in-band 출력 마커(M3, ADR-0085)** — 두 독립 조사에서 주채널 채택 사례 0건 + 스파이크 컴플라이언스 66%(엄격 지시로도 따옴표·nonce에서 깨짐). "완벽 컴플라이언스 전제" 채널은 약한 모델·긴 세션에서 무너진다.
- **PreToolUse 훅 주채널** — 사실상 수제 MCP 재발명(메시지마다 프로세스 스폰 + deny 시 간헐 idle 버그 #24327 + 부품 과다). 조사에서 주채널 채택 0건.
- **원시 tmux send-keys 릴레이** — 조사에서 안티패턴 판정(typed 계약 없음·오배달).
- **MCP Channels** — 에이전트 간 메시징 용도가 아님(ephemeral·배달 신뢰 이슈 #45563).
- **Agent Teams 파일 메일박스(`~/.claude/teams/...`) 계약 의존** — 비공식 역공학(v2.1.45 관찰)이라 공식 계약으로 간주 금지(Codex 교정).
- **claude-flow(ruflo) 참조** — 64k⭐에도 placeholder 다수·retry 로직 결함 추정(Codex 코드 확인).

## 근거
- **두 독립 조사 수렴 = cross-family 확증** — 메인 /research 3갈래 팬아웃과 사용자 별도 Codex 조사가 서로 모르는 채 같은 결론(typed ingress + durable mailbox 지배). Codex가 메인 조사 2건을 교정(Orca = typed CLI, Agent Teams 메일박스 = 비공식)했고 사용자 지시로 Codex 우선 채택.
- **Orca(20.1k⭐) 실증** — Bash carrier + typed CLI + sequence/delivered_at/read_at 분리 메일박스. 단 Orca는 앱-내장 런타임(Electron, 앱과 함께 사망) — 우리 분리 데몬(ADR-0029)이 복구 시맨틱에서 구조 우위. "데몬이 PTY 소유 + 클라이언트 탈착"은 tmux 서버가 실증한 고전 패턴.
- **스파이크 실측 4건(이 세션)** — ①stream-json 봉투: complete 봉투에선 tool_use input fully-formed(델타 재조립 불요), 봉투 타입 다양 → 방어적 파싱 필수 ②Bash 센티널 66% → 마커 접근 폐기 확증 ③훅: deny idle 버그 #24327 ④MCP 동적 로드 불가(서버는 시작 선언 필수) → "연결 성공"이 스폰 직후 검증 가능한 마일스톤.
- **사용자 승인** — 아키텍처 풀 브리핑(2층 브리핑) 후 승인, 스텝 1 범위 승인.
- **Bearer 헤더 실측 확인(2026-07-16, claude 2.1.170)** — mcp-config `headers`의 Authorization이 initialize·tools/list·tools/call 전 요청에 실전송됨 + `Mcp-Session-Id` 에코 동작(스파이크 ⑤, `control-channel-research-consolidated.md` §6). rmcp 2.2.0은 Streamable HTTP 서버 + axum 미들웨어 인증 패턴 공식 지원. 단 데몬 MCP 서버는 OAuth 메타데이터 광고 금지(#59467 — 광고 시 정적 Bearer 무시).

## 영향 / 불변식
- **Mailbox SQLite append = source of truth.** Dispatcher push는 배달 수단일 뿐 — delivered_at으로 재주입 방지, 오프라인 수신자는 복귀 시 배달.
- **`from`은 토큰에서만 파생** — 페이로드 from은 무시한다(프롬프트 주입/오작동 에이전트의 사칭 차단). 같은 OS 유저라 하드 격리는 원래 불가 — 최종 방어는 Validator.
- **토큰 수명 = (AgentId, epoch)** — epoch 회전(restart/fresh fallback) 시 구토큰 폐기, kill 시 즉시 폐기(ADR-0007 연동).
- **워커 노출 = send_message만** — 의도별 전용 툴을 additive로 늘리되 least-privilege 기본.
- **engram-ctl *이름* 재사용 금지** — typed CLI 입구 자체는 부활(스텝 5)하나 폐기된 크레이트명과의 혼동 방지를 위해 `engram-send` 또는 데몬 서브커맨드로 명명(메인 재량 + 보고).
- **ADR-0081(UI opaque-relay·권위 2도메인)은 존속** — 0085 폐기를 "UI relay도 죽었다"로 읽지 말 것(0085의 같은 경고 승계). UI 커맨드를 팔 때 0081 정합 확인.
- **기본 입구(MCP vs CLI) 결정 = 스텝 6 A/B 실측 후 사용자** — 그 전까지 MCP가 기본 후보일 뿐 확정 아님.
- 코드 앵커: 구현 진입 시 토큰 레지스트리·ControlIngress seam·Mailbox에 `// ADR-0086` 앵커를 박는다.
