# S12 데몬화 — 상세 구현 설계 (consult 교차검증 병합, 구현 착수 가능 수준)

날짜: 2026-06-14. 근거: `/consult` 2회(라이브러리 실조사 → 세부 설계 비판) GPT·Gemini·Claude-Opus 블라인드 + judge. 원자료: `agents/web-runner/shared/20260614-104428-consult-daemon-detail-design/`. 라이브러리 조사: `ipc-library-consult.md`.

이 문서는 **판정으로 옳다고 확정된 입장만 병합**한 최종 설계다. 모순 쟁점은 judge가 단정한 쪽을 채택한다.

## 0. 확정 전제 (재논쟁 없음)
- 경로 B-direct(JS↔데몬 WS 직결, relay 금지), replay=데몬 in-process(OutputCore 재사용), 두-모드 토글(Embedded/Daemon, startup 선택), Cargo workspace 정적 링크, raw tokio-tungstenite, 1차 Windows.

## 1. 핵심 판정 (모순 쟁점 — judge 단정)

### 1-1. WS lane = 단일 연결, 단일 수신루프 (lane 분리 금지)
- control과 output을 **별도 WS 연결로 분리하지 않는다.** TCP/WS 순서보장은 **연결당**이라, 별도 연결 2개는 크로스 순서보장이 없어 "Killed status가 마지막 output보다 먼저 도착"하는 인과 역전이 생긴다 → ET seq 정렬 신뢰성 붕괴.
- **단일 127.0.0.1 WS 연결.** control 이벤트와 output을 같은 연결로, 클라는 **하나의 수신 루프**에서 처리(상대 순서 보존).
- "PTY 폭주 시 status/ack가 backlog 뒤에 갇힌다"는 우려는 실재하나, 해법은 lane 분리가 아니라 **backpressure(§1-5)+프론트 throttle(§3-a)**다.

### 1-2. wire codec = output hot path는 커스텀 고정헤더 binary frame, control은 JSON
- base64-in-JSON 폐기(33% 팽창+CPU, 현 base64는 Tauri Channel JSON 제약 우회였음 — WS는 binary opcode 지원).
- **output(hot path)**: WS **binary frame**, 고정 헤더 `[tag:1][agentId:16][epoch:4][seq:8][raw payload]`. 직렬화/파싱 0. `tag`가 OutputChunk variant 구분(0=TerminalBytes; 미래 API 출력은 throughput 낮아 control처럼 JSON).
- **control(저빈도)**: WS **text frame JSON**(또는 MessagePack). 디코더 단순.
- 종류 불가지(§2) 유지: wire 표현만 variant별로 갈릴 뿐 OutputChunk를 바이트로 굳히지 않음.
- (전면 MessagePack도 허용 가능하나 hot path에 가변 인코딩 비용이 남아 비권장.)

### 1-3. replay 기점 + ring 잘림 + raw≠화면상태 (★GPT가 잡은 결정적 보강)
- epoch **불일치** = full reset. epoch **일치** = afterSeq+1 resume.
- ★**replay buffer는 bounded(2MB/agent 상한 존재)** → `afterSeq < oldestSeq`(재연결 중 ring 밖으로 밀림)일 수 있다. **"seq 0부터 replay"는 물리적으로 불가능한 경우가 있다.** 분기 필수:
  - epoch 불일치 → Reset, oldestSeq부터 replay, `truncated = oldestSeq>0`.
  - epoch 일치 & afterSeq<oldestSeq → truncated replay(oldestSeq부터) + UI "output truncated" 표시.
  - epoch 일치 & afterSeq≥oldestSeq → Resume(afterSeq+1부터).
- ★**raw byte replay ≠ terminal 화면 상태**(alt-screen/cursor 위치/진행바/full-screen TUI는 중간부터 재생하면 깨짐). **v1 치명도: 치명 아님** — 정상 경로(처음부터 구독, ring 안)는 정확. truncated는 ring 밖 예외 경로에서만 degrade.
- **v1 계약 명문화:** "exact byte resume만 보장. afterSeq가 replay window 안이면 gap/dup 0. window 밖이면 terminal state exact 복원 미보장 → clear + tail replay + 'output truncated' 표시." VT-parser snapshot/disk spool은 v2.

### 1-4. 토큰 전달 = ACL 포트파일만 (arg·query string·env 금지) ★spike #1 후 갱신
- 프로세스 커맨드라인은 `wmic process`/`Get-CimInstance Win32_Process`로 **같은 사용자 타 프로세스가 읽는다** → CLI arg 토큰 노출. query string도 프록시/로그 흔적(loopback이라 위험 낮으나 0 아님).
- ~~env~~ 도 폐기: **WMI `Win32_Process.Create`(spike #1 채택 spawn)는 환경변수 주입 불가** → env 경로 자체가 데몬에 안 통함.
- 토큰은 **ACL 잠근 port.json(현 사용자 only)**로만. 데몬이 부팅 시 256-bit 토큰 생성 → port.json 기록(ACL) → Tauri 가 읽어 첫 Auth frame 으로 제출. 검증은 **연결 직후 첫 Auth frame**(1초 내 없으면 close).

### 1-5. backpressure = bounded + emit try_send만 + slow consumer 끊기 (코어 변경 0)
- producer(pump)가 느린 client로 **await하면 PTY read 정지 → 에이전트 멈춤.** `OutputSink.send`는 반드시 **non-blocking(try_send)**.
- subscriber별 **bounded mpsc**. full → 그 subscriber **dead 마킹 + 소켓 close**(= 현 `output_core.rs:110-123` dead-sink 제거 메커니즘과 동형 → **코어 변경 0**). 재연결 + afterSeq replay로 회복(2MB ring 안전망).
- drop 금지(seq 연속성·ANSI 깨짐), 무한버퍼 금지(OOM). 답은 "끊고 재연결 회복".

## 2. 모듈/crate 레이아웃 (Cargo workspace, 정적 링크)
```
engram-dashboard/ (repo 루트 = workspace)
  crates/
    engram-dashboard-protocol/  [신규] AgentCommand·AgentEvent·OutputChunk·Hello·ID — serde, Tauri import 금지
    engram-dashboard-core/      [이동] pty/ 전부 + persistence + logging. AgentManager·OutputCore·AgentTransport·backend. Tauri import 0
  engram-dashboard-daemon/      [신규] main: lock/portfile·WS server·auth/version·AgentManager 소유·PTY Job 소유
  src-tauri/                    [얇아짐] Embedded: core 직접 / Daemon: discovery·spawn만 + AgentClient 모드 전달
  src/ (frontend)               AgentClient 인터페이스 신설, 컴포넌트·스토어는 인터페이스만 의존
```

## 3. protocol (engram-dashboard-protocol — linchpin)
- `AgentCommand`(UI→core): Spawn{profileId}·Kill{agentId}·Interrupt·WriteStdin{agentId,data,**requestId**}·Resize{agentId,cols,rows,viewportId}·Subscribe{agentId,epoch?,afterSeq?}·Unsubscribe·ListAgents·StopDaemon{force,killAgents}·Profile(CRUD).
- `AgentEvent`(core→UI): Hello{protocolVersion,daemonVersion,capabilities}·Ack{requestId}·SubscribeAck{action(Reset|TruncatedReplay|Resume),currentEpoch,oldestSeq,latestSeq,replayFrom,truncated}·Output{agentId,epoch,seq,chunk:OutputChunk}(binary frame)·ReplayComplete·StatusChanged{agentId,status,epoch}·AgentListUpdated{agents}·RestoreResult·Error.
- `OutputChunk`: TerminalBytes(serde_bytes Vec<u8>)·TextDelta·Usage·ToolCall·Structured — 종류 불가지 유지.
- 타입 생성: ts-rs(안정) vs tauri-specta(통합, RC 리스크) — **사용자 결정**.
- **command idempotency**: 모든 side-effect command에 `requestId`. 데몬은 짧은 TTL dedup table. **자동 재시도 금지**(writeStdin 중복=입력 중복). send 후 ack 전 끊김 → reconnect 후 QueryCommandResult(requestId).

## 3-a. 프론트 (AgentClient)
- 공통 인터페이스: `subscribe(agentId,onEvent)` + `onConnectionStateChange(connected|reconnecting|down)`. spawn/kill/interrupt/writeStdin/resize/unsubscribe/listAgents.
- EmbeddedClient: invoke/Channel 그대로, connectionState 항상 connected, dedup no-op.
- DaemonClient: WS + 지수 백오프 재연결 + 재연결 시 자동 Subscribe{epoch,afterSeq:lastReceivedSeq} + seq dedup(재연결 경계 1~2개 중복 흡수). 재연결·afterSeq·dedup은 **내부 격리**(인터페이스에 안 올림 — Embedded LSP 냄새 방지).
- **연결 직후 데몬이 전체 agent list 스냅샷 push**(재연결 중 AgentListUpdated 유실 방지 — terminal 판정 정확성).
- ★**프론트 렌더 throttle 필수**: 고속 PTY 출력이 그대로 React로 유입되면 가상 DOM 렌더 밀려 탭 freeze/OOM. rAF/16ms 단위 chunking. **주체 결정(사용자)**: 데몬 측 16ms 묶음 vs 프론트 rAF flush.
- ★**multi-client ControlLease**: 같은 agent에 desktop+mobile 동시 시 PTY size 하나뿐 → writeStdin/resize/interrupt는 lease owner만(ControlLease{agentId,ownerClientId,viewportId,expiresAt}). read-only viewer 분리.

## 4. 데몬 생명주기 (Windows)
- ★**spike #1 완료(2026-06-14) — 결과: `spike1-breakaway-result.md`. 판정 GO, 단 spawn 방식 변경.**
  - 측정 환경(IDE/CLI 셸) 부모 Job = `0x2000`(KILL_ON_JOB_CLOSE + breakaway 불허) = worst-case.
  - `CREATE_BREAKAWAY_FROM_JOB` 직접 → **os error 5 실패.** `cmd /c start /b` fallback → **동반 사망(설계 가정 폐기).**
  - **WMI `Win32_Process.Create` → in_job=false 분리 성공**(WmiPrvSE 부모, 호출자 Job 미상속).
- **spawn = WMI `Win32_Process.Create` 채택**(또는 적응형: Job flags 조회 → KILL_ON_JOB_CLOSE 아님=normal / breakaway_ok=CREATE_BREAKAWAY / worst-case=WMI). 데몬은 자기 수명 소유.
- ★**파생 제약: WMI Create 는 env 주입 불가**(CommandLine 만) → **토큰 전달 = ACL port.json 강제**(§1-4 env 경로 폐기). winmgmt 서비스 의존(상시 가동).
- ★**"Job 소유권 이전"은 환상 — 삭제.** Job 핸들은 프로세스 로컬, 이전 불가. 데몬 모드=데몬이 PTY를 직접 spawn하니 자기 Job(KILL_ON_JOB_CLOSE)에 넣으면 끝. 모드별 spawn 주체가 갈리므로 런타임 이전 시나리오 없음.
- 단일 인스턴스: **`Local\` named mutex**(현 사용자 한정 — `Global\`보다 안전) + ACL 잠근 `%LOCALAPPDATA%\Engram\daemon.json`{port,pid,token,protocolVersion} atomic write(persistence tmp+rename 재사용).
- 발견: **bind 127.0.0.1:0(랜덤 포트)** → port.json. Tauri가 읽어 /health 접속. 고정 포트 금지(충돌·stale).
- 부팅: stale 포트파일/mutex의 pid 죽었는지 확인 후 정리.
- 종료: UI 닫혀도 생존(목적). `StopDaemon` 커맨드(§5 LLM 제어). **idle-timeout 기본값 사용자 결정**(C=OFF / Gemini=30분; 양립안: "PTY 0 + 연결 0" N분 지속 시 자살, 활성 에이전트 있으면 무한 생존).
- **protocolVersion 협상**: 데몬은 앱과 독립 배포 수명 → Hello로 버전 확인. incompatible → StopDaemon(old)+new spawn(자동 vs 사용자확인 = 결정).

## 5. 보안
- 127.0.0.1만 bind(0.0.0.0/LAN 금지, 모바일 켜기 전). 256-bit 토큰(env/ACL port.json). 첫 Auth frame 검증. **Origin allowlist**(Tauri origin). message size/rate limit. permessage-deflate off.
- named pipe(ACL로 OS레벨 제한)는 데스크톱 보안 강화 v2 옵션. 모바일 v3 = wss/TLS + pairing + device allowlist.

## 6. 구현 phasing
- **★spike 0 (phase 1 이전):** Job Object breakaway 실측. 부모 job(IDE/터미널) breakaway 불허 시 분리 가능 여부 + fallback(`cmd /c start /b`) 검증. **안 풀리면 전체 무효.**
- **✅ phase 0(완료 2026-06-14):** engram-dashboard-protocol 독립 crate. AgentCommand/AgentEvent/OutputChunk/SubscribeAction + domain 미러 + codec binary frame + ts-rs 바인딩. 테스트 21건 PASS(codec golden/roundtrip/에러 + 타입별 export). 커밋 `61f2d0f`. ※이름 충돌(protocol AgentCommand ↔ core profile AgentCommand=spawn 종류) → phase 1+ 에서 spawn 종류 SpawnSpec 개명 필요.
- **✅ phase 1(완료 2026-06-14):** 
  - **1a 백엔드:** Cargo workspace(루트 Cargo.toml: protocol/core/src-tauri). pty/persistence/logging→`crates/engram-dashboard-core`(git mv, history 보존, 내부 crate:: 무수정). examples도 core 로. 격리 게이트 use tauri 0. 회귀 0(core unit 38 / headless·transport_smoke·session_smoke / 전체 빌드 / target 워크스페이스 재배치 tauri dev 정상). 커밋 `576c5e1`.
  - **1b 프론트:** AgentClient 인터페이스 + EmbeddedClient(Channel·base64·#13133 캡슐화) + clientFactory(싱글톤+§5 window.__ENGRAM_AGENT__). 컴포넌트 ptyApi→agentClient. GUI E2E: spawn→subscribe→writeStdin→디코드 277B/3청크, kill 후 0, UI 정상. 커밋 `5346240`.
- **phase 2:** 데몬 단독 + 격리 하네스(transport_smoke 확장). 필수 케이스:
  - 수명주기: single instance / stale port·lock / incompatible version / **UI kill→데몬 생존** / **데몬 kill→PTY child 정리** / duplicate spawn race / **breakaway 성공 확인**.
  - 보안: token 없음·오답·Origin mismatch·oversized·auth timeout 거부.
  - 출력: order exact / afterSeq 경계 off-by-one / afterSeq==latest / **afterSeq<oldest(truncated)** / epoch mismatch / replay중 live 경합 / **C4 gap·dup 0** / slow consumer disconnect / reconnect 복구 / ring memory cap.
  - PTY: high throughput / resize 폭주 / interrupt / child exit·crash / stdin close.
- **phase 3:** 하네스 CI 영구 보존(protocol golden / core contract / embedded contract / daemon contract / Windows lifecycle).
- **phase 4:** 접합부 스왑. startup config mode=embedded|daemon → AgentClientFactory. 라이브 핫스왑 안 함.
- **acceptance test:** Tauri 재시작 → 데몬 생존 + 무손실 복원.

## 7. 사용자 결정 필요 (미해결)
1. ✅ **idle-timeout = OFF, 항상 생존** (결정 2026-06-14). 사유: 나중에 스케줄러로 에이전트를 깨울 수 있으므로 데몬이 늘 살아 있어야 함. idle 자동 종료 없음. (명시적 StopDaemon 커맨드로만 종료.)
2. ✅ **타입 생성 = ts-rs** (결정 2026-06-14). 사유: 필요한 건 protocol 타입(AgentCommand/AgentEvent) 자동 생성뿐. tauri-specta의 invoke 바인딩 이점은 WS 데몬 경로엔 무효 + specta 2.0 RC 불안정. ts-rs 하나로 양 모드 타입 커버. u64 seq는 `#[ts(type="bigint")]` 또는 string 매핑 검토(JS number 2^53 한계).
3. **truncated replay UX** — tail+표시(v1) vs VT snapshot(v2) vs disk spool. v1 범위.
4. **로컬 transport** — TCP+토큰 단독(v1) vs named pipe+TCP 이중(강화 시점).
5. **프론트 throttle 주체** — 데몬 16ms 묶음 vs 프론트 rAF flush.
6. **version mismatch 시 old daemon** — 자동 종료 vs 사용자 확인.
7. **다중 Tauri 인스턴스** — 단일 데몬 다중 창 허용 정책(이중 spawn 가드 manager.rs:90 데몬 유지 확인).

## 8-b. 결정 로그 (Q&A 진행 2026-06-14)
- **#1 idle-timeout = OFF, 항상 생존** (스케줄러로 에이전트 깨움 대비). 명시적 StopDaemon만.
- **#2 타입 생성 = ts-rs** (WS라 tauri-specta invoke 이점 무효, RC 회피).
- **#3 truncated replay = A(clear+tail+마커)** + **버퍼 = 설정가능 기본값, 에이전트(LLM)가 런타임 조정**(SetReplayBufferSize). **구조화 출력(turn 단위)**: 핵심 필수지만 **TUI↔구조화 스위칭 구조로 나중 설계**. claude는 interactive TUI(바이트, 단위끊기 X) vs `-p stream-json`(구조화 turn/tool, TUI 포기) 택일 — SDK 아님. 데몬은 OutputChunk가 양쪽(TerminalBytes / 구조화 variant + turnId) 표현 가능하게 **열어만 둠**, 구현은 스위칭 설계 때.
- **#4 epoch 불일치 = full reset.** backpressure = bounded 큐 + **emit try_send(절대 블록 X = 코어 변경 0)** + 넘치면 **그 클라만 끊기**(watermark pause 안 씀 — 단순 disconnect 모델). **회복 = 클라 주도, 끊긴 원인별:**
  - 서버가 버퍼로 끊을 때 → WS close 사유 태그(`SlowConsumer`) → 클라 **refresh 요청**(밀린 백로그 안 받고 현재로 점프).
  - 그냥 에러(비정상 close) → 클라 **resume(afterSeq)** → 서버가 ring에 있으면 이어주고, 없으면 **refresh로 강등**.
  - refresh가 주는 "현재 상태": v1=clear+tail(근사), v2=VT parser 화면 스냅샷(정확, 나중 — far-behind refresh가 v2의 분명한 동기).
  - 원칙: **서버=버퍼 자율정책+상태 통보, 클라=resume/refresh 판단.** thrash 클라는 **서버가 rate-limit/블록 가능(나중)**.
  - **회복 로직 위치: DaemonClient 분리**(네트워크 끊김은 Daemon에만 존재). Embedded는 회복 0. 공용 `AgentClient` 인터페이스 뒤 → 나중에 DaemonClient 안에서만 채우면 됨(공용·Embedded 무영향). **상세 구현은 나중.**

## 8-d. 사용자 결정 7개 — 전부 완료 (2026-06-14)
1. **idle-timeout = OFF, 항상 생존**(스케줄러로 에이전트 깨움 대비). StopDaemon만.
2. **타입 생성 = ts-rs.**
3. **truncated replay = A(clear+tail+"truncated" 마커).** 버퍼 = 설정가능 기본 + 에이전트(LLM) 런타임 조정. 구조화(turn 단위) 출력 = TUI↔구조화 **스위칭 모드로 나중 설계**(§8-c), 데몬은 OutputChunk 양 표현 열어만 둠.
4. **epoch 불일치 = full reset.** backpressure = bounded + emit try_send(코어 변경 0) + 못 따라오면 처리 — **끊기 vs flow-control pause(클라 ACK)는 DaemonClient 구현 때 택1**(표준은 pause). 회복 = **클라 주도**: 끊긴 원인별(서버 close 사유 `SlowConsumer`→refresh / 비정상→resume), 서버는 resume 불가 시 refresh 강등. refresh "현재상태" = v1 clear+tail / v2 VT 스냅샷. 회복로직 = DaemonClient 분리(Embedded 회복 0). **상세 구현 나중.**
5. **throttle = 표준**: 데몬은 read 청크 중계(특별 배칭 X), 클라는 xterm 프레임(rAF, skip-frame, 50MB) 렌더. 적응형/프레임스킵·sync mode·flow control 다 xterm이 가짐. (검증: ttyd/gotty 중계 + xterm.js rAF + 일반 터미널 vsync 렌더.)
6. **버전 처리 = 한참 나중(deferred).** 지금 안 짬(둘 다 같이 띄움). 나중: protocolVersion(깨질 때만 +1) **또는/그리고** parse-error 시 **팝업 가이드**(사용자가 데몬 재시작) — auto-restart·협상 레이어 안 만듦.
7. **클라이언트 = 1개(Tauri 앱 = Rust 백엔드 1개) + React 창 여러 개(뷰).** 공유 상태(레이아웃·슬롯 창간 이동·관리창 싱글톤)는 백엔드 한 곳, 창은 뷰(Tauri 창 = 독립 React라 공유는 백엔드/이벤트로). multi-subscriber는 클라이언트(앱·모바일)당 1연결 기준, 창당 아님. **다중창 상세 + 다중 .exe 정책 = 나중.**
- **transport = v1 TCP(127.0.0.1)+토큰.** named pipe = v2 보안강화 옵션. (모바일 WS 직결 목표라 TCP 통일.)

**→ 설계 결정 완결. 다음은 구현 단계. ★최우선: Job Object breakaway spike(데몬화 성패 단일 장애점) — phase 1 이전 단독 실측.**

## 8-c. 모드 스위칭 아키텍처 (나중 구현, 짜고 사용자 검수)
2축 직교: **위치**(Embedded/Daemon=AgentClient) ⟂ **모드**(TUI/Structured=AgentMode). 원칙: **모드 = [실행법+transport+capability+렌더러] 묶음, 코어(OutputCore/Session/Manager/AgentClient)는 모드 불가지.**
- 백엔드: `AgentMode` enum→dispatch. `ClaudeTui`(interactive CommandSpec / PtyTransport / cap.output=terminal_bytes / emit TerminalBytes) vs `ClaudeStructured`(`-p stream-json` CommandSpec / **StreamJsonTransport**(신규, stdout JSON 라인 파싱) / cap.output=structured / emit TextDelta·ToolUse·ToolResult·Usage). 둘 다 같은 OutputCore.
- 프론트: Slot이 `capability.output`으로 렌더러 선택 — terminal_bytes→TerminalRenderer(xterm, 기존) / structured→**StructuredRenderer**(turn 벡터·접기·VS Code풍, 신규). StructuredRenderer는 claude-stream-json + 미래 HTTP API 양쪽 섬김.
- 이미 있음(S10): AgentTransport trait·OutputCore variant-agnostic emit·OutputChunk/Capabilities 슬롯·backend dispatch. 신규: AgentMode·StreamJsonTransport·StructuredRenderer·turnId/exchangeId.
- 스위칭 = spawn 시점 결정(라이브 변환 불가 — TUI vs stream-json은 다른 프로세스 실행). 모드 변경 = 해당 모드로 재spawn(필요 시 --resume, 출력모드 바꿔 resume 가능성은 실측). 갈아엎기 0 — 모드 추가 = transport 1 + 렌더러 1 + 묶음.

## 8. 모델별 기여 (참고)
- Claude-Opus(가장 신뢰, 사실오류 0, 코드 직접 검증): 단일 WS·binary frame·try_send=코어변경0·"Job 소유권 이전=환상"·breakaway 단일장애점·list 스냅샷·version 협상.
- GPT(결정적 보강): ★ring 잘림 truncated 분기·raw≠화면상태·command idempotency·control lease·named mutex 단독 위험.
- Gemini(단독 포착): ★프론트 렌더 throttle(rAF)·zombie daemon idle-timeout.
- 기각: GPT의 control/output lane 분리(크로스 순서보장 없음), Gemini의 "무조건 seq 0"·토큰 arg/query 허용.
