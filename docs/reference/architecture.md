# 아키텍처

모듈이 무엇을 하고 누구와 무엇으로 말하는지, 그리고 런타임 불변식. 지금 코드에서 뽑았다 — 코드와 어긋나면 불변식을 코드에 맞춰 조용히 고치지 않는다. 코드가 틀렸는지(회귀) 의도가 바뀌었는지 가려 보고한다. 멤버 목록 정본 = 루트 `Cargo.toml`, 각 crate 의 경계·격리 게이트 정본 = 그 crate `src/lib.rs` 헤더, 파일별 책임 정본 = 그 파일 헤더와 `// ADR-NNNN` 앵커. 설계 원칙은 `CLAUDE.md` 「설계 원칙」.

## 연결도

```text
[앱 프로세스 engram-dashboard.exe]
  프론트(React, 창마다 한 벌)
    │  Tauri invoke · event · Channel
    ▼
  셸(src-tauri)
    │  WS 127.0.0.1 — 연결 1개, daemon.json 으로 찾는다
    │  (데몬 띄우기 = WMI 로 Job 밖에서 · 끄기 = StopDaemon 프레임)
    ▼
[데몬 프로세스 engram-dashboard-daemon.exe]
  net(WS 행) · agent(런타임) · 명령 버스 · messaging(우편) · 제어 평면(HTTP)
    │  PTY(터미널 모드) · stdio(JSON 모드)
    ▼
[에이전트 N — claude · codex · 셸]
    │  engram.exe(CLI) · MCP 툴 → HTTP /control/* · /mcp (Bearer 토큰)
    └──▶ 데몬 제어 평면
```

crate 의존(화살표 = 의존한다, 운영 그래프):

```text
src-tauri ─▶ protocol · command · base · platform · transport(ws)      (agent 는 의존하지 않는다)
daemon    ─▶ agent · protocol · net(server) · messaging · command · base · platform
cli       ─▶ agent · command
agent     ─▶ command · base · platform
net       ─▶ protocol · platform   (platform only behind the server feature)
base · platform · command · messaging · transport · protocol ─▶ 워크스페이스 의존 0
```

## 모듈

**프로세스 셋.** 셸(앱) · 데몬 · 에이전트. 데몬이 에이전트를 소유하므로 앱을 껐다 켜도 에이전트는 산다.

- **프론트(`src/`)** — 화면 렌더링만 한다. 에이전트·데몬과의 통신은 `agentClient`(`ProtocolClient` + `TauriTransport`) 하나로 셸을 거친다 — 명령 = invoke(`forward_daemon_command` 등), 출력 = Tauri Channel, 방송 = Tauri event. 셸이 주인인 상태(레이아웃 · 창 · 설정 · 복원)는 셸 명령을 직접 invoke 한다. 조작 표면 = `window.__engramCmd` 레지스트리(`src/commands/`)이고, 버스 명령이 셸 → 웹뷰로 내려와 같은 `registry.run` 을 탄다.
- **셸(`src-tauri`)** — 데몬의 상주 클라이언트. 창 · 트레이 · 레이아웃(창·탭·슬롯의 권위) · 셸 설정 · 화면 상태 영속 · 데몬 찾기/띄우기/끄기(`discovery`)를 갖는다. 에이전트를 소유하지 않는다. 데몬과는 WS 연결 하나(`daemon_client` — 창이 몇 개든 1개)로 말하고, 출력 프레임을 창마다 Channel 로 중계한다(`output_router`). 레이아웃 명령(`window/tab/slot`)의 주인으로 데몬 명령 버스에 등록된다. 창은 main 하나이고 팝아웃은 실행 중 만든다.
- **daemon** — `AgentManager` 를 소유한 에이전트 호스트이자 조립실. WS 수락 루프에서 셸 연결을 받고, 명령 버스의 명부·배달(`command_roster` · `command_delivery`)을 돌리고, 메시징 커널에 실물 어댑터를 꽂는다(`messaging_host`). 제어 평면(`control`) = HTTP 서버 하나에 MCP(`/mcp`)와 CLI 라우트(`/control/*`)를 태운다 — 에이전트 신원은 Bearer 토큰에서만 나온다. 데이터 폴더 배치(`data_dir`) · 사용량 조회(`usage_service`)도 여기다.
- **agent** — 에이전트 런타임(Tauri import 0). 세션 수명 · 출력 펌프 · `transport`(PTY · stdio) · `backend`(claude · codex · 셸 · gemini 껍데기 — 백엔드 전용 지식은 여기만) · 턴 관측 · 대기 입력 · 프로필 영속 · `agent.*` 명령 선언. 출력·상태는 `OutputSink`/`StatusSink` trait 으로만 내보낸다.
- **cli** — `engram.exe`. 스폰된 에이전트가 `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL` 로 데몬 제어 라우트에 HTTP 로 붙는다. 인증 + 배송만 하고 help 화면도 데몬이 낸다. bin 이름 = agent `CLI_EXE_NAME`.
- **protocol** — 셸 ↔ 데몬 wire 계약(메시지 · 출력 binary frame codec · ts-rs 바인딩). 도메인 로직 없음.
- **command** — 명령 버스 도구(봉투 · 오류 · 선언 매크로 · 표 · 라우팅). 명령은 0개이고 생산자 옆에서 선언한다(`agent.*` = agent, `mail.*` = daemon, `window/tab/slot` = 셸, 웹뷰 명령 = 프론트).
- **net** — the daemon's network row (WS upgrade · Origin · token handshake · connection lifetime · single writer · keepalive · fan-out · single-instance guard · `daemon.json`). Everything sits behind the `server` feature; without it only `auth` remains, a re-export of protocol's handshake frame. Only the daemon depends on net — the shell's edge was cut by ADR-0310 decision 9.
- **messaging** — 에이전트 간 우편 커널(배달 · 파킹 · 반려 · 장부). 워크스페이스 의존 0, 접합은 커널이 소유한 포트 trait 뿐이고 실물 어댑터는 데몬이 갖는다.
- **base** — 바닥 기반층 잎 crate. 도메인 지식 0 의 범용 도우미(데이터 파일 읽기 판정·원자 쓰기 · 로깅 · 시계 seam · 패닉 가둠 등). 입주 조건 정본 = 그 `lib.rs` 헤더.
- **platform** — OS 층 잎 crate. OS 에 따라 갈리는 운영 코드(프로세스 · Job Object · 띄우기 · 창 · 입력 · 파일 잠금)는 여기에만 둔다.
- **transport** — reusable transport lib (ADR-0177). The shell depends on it (`ws` feature) for its daemon link `daemon_client/link/` (stage 3 3-2 · ADR-0310); the shell's daemon connection runs on it in every build (driver = `daemon_client/connection.rs`). The daemon attaches in 3-3.

## 런타임 불변식

「리뷰」 칸 = 그 영역을 바꾸는 변경에서 리뷰어가 확인할 것. ★ = agent·daemon 런타임 변경이면 영역과 무관하게 확인.

| # | 불변식 | 리뷰 |
|---|---|---|
| I1 | **kill 인과(2동사):** `transport.shutdown()`(child.kill+wait → 무리 주인 `GroupOwner::terminate` → master drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish`. `kill_agent` 의 첫 줄은 `session.begin_retire()` — 물러남 칸만 세우고 자원을 거두지 않아 위 인과는 그대로다. (ADR-0001 · ADR-0262) | kill·transport 변경 |
| I2 | **finalize 1회:** `OutputCore.finalized.swap(AcqRel)` — terminal 전이·알림은 정확히 1회(pump 단독). (ADR-0005) | ★ |
| I3 | **락 순서:** sessions RwLock 은 Arc clone 후 즉시 해제. status lock 을 쥔 채 외부 호출 금지. emit 은 subscribers 를 clone 한 뒤 락 없이 send. 입력 경로 = `input_order → replay → {QueuedInputs, InputsPendingTable}` — 명부·표는 잎이다. 펌프·emit·`finish` 는 `input_order` 를 잡지 않는다. codex 통로 = `Announcer.order → {상태 락, replay → …}`. 「락 없이 send」의 의도된 예외는 둘(세션 `input_order` · codex `order` 를 쥔 emit)이고, 교착이 없는 근거는 `OutputSink::send` 가 막히지 않는다는 trait 계약이다. claude 잔여물 정리(`backend/claude/leftover.rs`)의 잎 자물쇠 둘(`GateCell.state` · `Recorder.inner`)은 겹쳐 잡지 않고 쥔 채 다른 락·로그·emit·OS 호출을 하지 않는다(예외 = 단조 시각 읽기 · `TerminateProcess` 하나). (ADR-0006 · ADR-0231 · ADR-0234 · ADR-0262) | ★ |
| I4 | **상태 알림 분담:** 과도기 `Exiting` = manager(`enter_exiting`), terminal(`Killed`/`Exited`/`Failed`) = pump 단독. 프론트는 `status_changed` 로 terminal 을 판정하지 않고 `agent-list-updated` 로 판정한다. (ADR-0005) | ★ |
| I5 | **replay→live:** subscribers 락을 쥔 채 replay 전송. 프론트는 seq 연속(`마지막+1` 만 배달 · 이하는 버림 · 구멍 뒤는 붙든다)으로 라이브 팬아웃의 배달 역전을 맞춘다. 그래서 sink 와 셸 중계는 seq 를 건너뛰지 않는다 — 못 실은 사건 · 모르는 tag = 자리채움 `Error`(`placeholder_error_frame`) · seq 를 못 읽는 프레임 = 연결을 끊어 전량 replay. (ADR-0231) | 출력 경로·셸 중계·`protocolClient` 변경 |
| I6 | **화신 표식(필드명 `epoch`):** 화신마다 새 32비트 난수 — 비교는 일치/불일치만(대소 유도 금지). `agents.json` 디스크 serde 만 읽기 건너뜀 + 쓰기 `0` 자리채움이고 wire 로 넓히지 않는다(wire 의 `0` 은 정당한 값). 재부착 계기는 권위 명부 관측이고, 구독 effect deps 에 표식을 넣지 않는다. (ADR-0163 · ADR-0164) | ★ |
| I7 | **소유권 분할:** transport = master/writer/child/shutdown/무리 · core = subscribers/replay/seq/status/finalized + 대기 입력 명부 · 대기 목록 표 · 봉인 · session = id/cwd/epoch/cols/rows + 첫 제출 래치 · `mid_turn` · `delivery_ack` · `input_order`. 대기 입력 명부를 바꾸는 길은 코어가 replay 락 안에서 먹이는 환원 하나다 — 세션은 같은 `Arc` 를 읽기만 한다. codex 통로(`TransportOwned`)는 자기 입력 상태를 상태 락 하나 아래 쥐고 목록 사건도 스스로 낸다(정본 = `backend/codex/transport.rs` 의 `State`). 무리 주인(`GroupOwner`)은 `Clone` 이 아니라 무리당 하나다 — 타입이 강제하는 것은 여기까지다. 에이전트 무리의 주인이 통로라는 것은 배치가 지킨다(사용량 조회 자식의 무리는 그 조회의 `TreeRoot` 가 쥔다 — `usage/process.rs`). (ADR-0231 · ADR-0234 · ADR-0235 · ADR-0262 · ADR-0275) | ★ |
| I8 | **턴 관측 정리 = 두 지점뿐:** `finish` 의 forget + `emit` 의 finalize 재확인. 세 번째 호출자를 늘리지 않는다 — 빠지면 죽은 에이전트가 「턴 중」으로 남아 우편이 30분 상한까지 막힌다. 오류 뒤 멈춤 `last_end_failed` 도 이 두 지점에서 거두고, 진행 커서 `last_seq` 가 아니라 자기 커서(`last_end_seq` · `error_seq`)로 거른다. `inputs_pending` · `last_end_failed` 에는 30분 상한을 붙이지 않는다. 끊김 표시 `Structured{kind:"interrupted"}` 는 claude 분류기가 진행으로 세지 않는다. (ADR-0127 · ADR-0231 · ADR-0238 · ADR-0243) | 턴 분류·`output_core` 변경 |
| I9 | **대기 입력 상태 = 링 사건 한 줄기(`StructuredEvent::QueuedInput`):** 데몬 명부와 프론트 누산기가 같은 환원 규칙을 쓰고 둘 다 골든 `queued_input_golden.json` 을 먹는다 — 규칙을 고치면 골든과 TS 판을 같은 변경에서 고친다. 턴 관측에 드는 대기 입력 사건은 claude `Delivered` 하나이고 codex 는 전부 `None` 이다. codex 통로가 합성한 사용자 말풍선과 늦은 받음 뒤 경계는 `emit_without_turn_observation` 으로 낸다 — 보통 emit 으로 바꾸면 우편이 막히거나 흐른다. (ADR-0231 · ADR-0235 · ADR-0241) | 대기 입력·codex 통로 변경 |
| I10 | **등록 순서:** sessions insert 가 pump 시작보다 먼저 — 뒤집히면 즉시 종료하는 세션이 수거되지 않는다(reaper 시험이 잡는다). (ADR-0019) | spawn 경로 변경 |
| I11 | **세션 id 영속 = 첫 제출 래치 한 지점, 비교-교체로:** backend 세션 id 는 화신의 첫 제출 래치가 commit 할 때만 프로필에 영속된다(덮어쓰기 `observe_session_id` 가 아니다). 다른 기록 경로(스폰 때 발급해 영속 등)를 되살리지 않는다 — 0턴 id 가 영속돼 다음 활성화가 실패한다. 예외 = claude `/clear` 추적. codex JSON 모드는 통로가 상대의 첫 유저 메시지 되울림에서 `FirstTurnSink` 로 commit 한다(첫 턴 전 영속이 아니다). 저장 실패는 메모리 명부에 적고 dirty — 다음 성공 저장이 싣는다. (ADR-0226 · ADR-0233 · ADR-0291) | 세션 id·프로필 영속 변경 |

### 세션 복원 · 활성화

- 복원은 프로필에 저장된 backend 세션 id 하나에 기댄다 — 누가 발급하는지는 백엔드가 정한다(claude = 우리가 `--session-id` 로 발급 · codex = 스레드 생성 응답에서 받는다). (ADR-0008 · ADR-0185)
- 활성화 = 이어받기(resume) 전용이다. 실패하면 새 대화로 폴백하지 않고 관측된 종료 상태로 끝내고 실패를 프로필에 적는다. (ADR-0082 · ADR-0172)
- 활성화 입구(버스 `wake_existing` · WS `SpawnProfile`)와 부팅 복원은 같은 판정 `backend::can_resume_profile` 을 부른다. WS 의 `resume=true` 명시 요청은 그 판정을 타지 않는다.
- 공표되는 `Failed` 는 `TerminalReason::Error`(pump 패닉) 하나다 — resume 실패의 종점은 대개 `Exited` 이고, 실패는 활성화 결과(`RestoreOutcome::Failed`)와 프로필 「마지막 실패」 축에 선다.
- 백엔드별 세션 표 = `docs/reference/structure/agent-backend.md`.

## 프론트 규칙

- 에이전트·데몬 통신은 `agentClient` 하나를 지난다. 셸이 주인인 상태(레이아웃 · 창 · 설정 · 복원)만 Tauri invoke/listen 을 직접 쓴다. `WsTransport` 는 운영에서 쓰지 않는다(시험만 import).
- 구독(콜백) 수명은 `eventBus` 가 `agentClient` 의 추상 구독으로 소유하고, Tauri `listen` 을 직접 거는 쪽은 자기 disposer 를 소유한다. 거는 곳은 세지 말고 찾는다: `rg "from '@tauri-apps/api/event'" src/`.
- 구독 effect deps = `[viewId, agentId]`(화신 표식 제외 — I6). 구독 전 `terminal.reset()`.
- 붙듦 상한 `HELD_MAX_FRAMES` · `HELD_MAX_BYTES`(`src/api/protocolClient.ts`)는 데몬 링 상한 `REPLAY_MAX_EVENTS` · `REPLAY_MAX_BYTES` 보다 작게 둔다 — 크면 넘침 회복이 늘 잘림으로 끝난다. 넘치면 `startBuffering` 으로 간다. (ADR-0231)
- replay 경계 = gen 펜스 성공 마커(마커가 replay 머리를 싣고, flush 는 `max(마지막+1, 머리)` 부터).
- replay 요청은 그 창의 출력 Channel 등록 뒤에 낸다(`awaitOutputChannel`) — 셸은 미등록 창 몫을 말없이 건너뛴다.
- 구조화 뷰는 `'live'` 마다 대기 목록을 다시 대조한다(`ListQueuedInputs` → 스냅숏 `as_of_seq` 까지 배달받은 뒤 적용 · 대조 세대로 옛 답을 버린다). (ADR-0231)
- Channel 떼기는 `delete channel.onmessage`(null 대입 아님)이고 실 Tauri 에선 무동작이다 — 재등록 틈의 프레임이 그 무동작에 기대므로 떼기를 진짜로 만들지 않는다.
- 터미널 resize 는 50ms debounce.

## 창

- 정적 창은 main 하나 — `src-tauri/tauri.conf.json` 에 선언하지만 `"create": false` 라 셸이 부팅 때 저장된 자리로 만든다.
- 팝아웃은 실행 중 만든다(`/popup` 라우트 · `PopoutPage` — 탭을 소유한다, ADR-0057). label = `slot-popup-<실행 표식>-<n>` 이라 실행마다 바뀐다 — 재시작을 넘어 같은 창을 가리키는 것은 영속 id 다. (ADR-0302)
- 창을 만드는 코드는 전부 `WebviewEnv::finish`(`src-tauri/src/webview_env.rs` — 웹뷰 데이터 폴더 · 브라우저 인자 · 바탕 · OS 틀)를 지난다. 앱이 창 단추를 그리는 OS(지금 Windows)면 OS 틀을 걷고(`decorations(false)` · `shadow(true)`) 웹 제목 줄이 그 자리에 선다. (ADR-0054 · ADR-0283)
- 창 단추는 레지스트리 명령(invoke `window_control`)을 부른다 — 버스 창 상태 명령과 같은 서비스다. OS 창 제목 · 최소 창 크기 = 셸 `chrome` 모듈. (ADR-0300 · ADR-0301 · ADR-0302 · ADR-0303)

## 화면 상태 영속

- 레이아웃은 `<데이터 루트>\shell\state\state.json` 에 영속된다. 정상 종료 뒤 부팅은 묻지 않고 복원하고, 비정상 종료 뒤엔 `state.crash.json` 으로 떠 두고 기본 화면으로 시작한다.
- 복원 여부 답 = 버스 `restore.status` / `restore.answer` · 사람 = main 창 모달. 정본 = `docs/process/S21-storage/trd.md` §6.

## 현황

- 회귀망은 아직 Windows 에 묶여 있다 — CI 러너(`.github/workflows/ci.yml` 의 빌드·시험 잡)와 실행 스크립트가 Windows 전용이고, 실프로세스 시험 일부가 파일째 `#![cfg(windows)]` 다.
