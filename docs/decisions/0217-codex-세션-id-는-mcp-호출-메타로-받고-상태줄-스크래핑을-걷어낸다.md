# ADR-0217: codex 세션 id 는 MCP 호출 메타로 받고 상태줄 스크래핑을 걷어낸다

- 상태: 확정 (2026-09-21, 근거: 실측 3라운드 — 아래 「근거」) · 부분 폐기 by ADR-0218 (회수 채널을 락 파일 홀더 PID 로 교체) · 부분 폐기 by ADR-0226 (결정 7 값 정책)
- 관련: CLAUDE.md 「백엔드 확장」 · ADR-0185(codex 는 id 를 발급하지 않는다) · ADR-0208(훅 회수) · ADR-0203(벤더 저장 포맷을 읽지 않는다) · ADR-0210(사용자 기기에 파일을 만들지 않는다) · ADR-0214(MCP 우편) · ADR-0213(`eg_` 접두) · ADR-0207(프로필 쓰기) · ADR-0004(백엔드 지식 격리) · `crates/engram-dashboard-agent/src/backend/codex/mod.rs:711` · `crates/engram-dashboard-daemon/src/control/mcp_server.rs:437` · `crates/engram-dashboard-daemon/src/control/registry.rs:24` · `crates/engram-dashboard-agent/src/profile.rs:709` · step-log S21 · Amends ADR-0216 (상태줄 회수 채널과 스캐너 대체) · Amended by ADR-0218 (회수 채널을 락 파일 홀더 PID 로 교체) · Amended by ADR-0226 (결정 7 값 정책)

## 맥락

codex 는 자기 스레드 id 를 스스로 발급하고(ADR-0185), 터미널 모드에는 그 값을 우리에게 말해 줄 창구가 없다(ADR-0208 맥락). ADR-0216 이 그 창구로 **TUI 상태줄**(`-c tui.status_line=['thread-id']`)을 골랐고, 회수 자체는 실제로 돈다.

★**대가가 승인 전제와 달랐다**★ — 사용자는 「정보가 한 번 뜨고 사라진다」로 이해하고 승인했는데, 상태줄은 **매 프레임 다시 그려지는 상주 한 줄**이다. 회수는 화신당 한 번이면 되는데 그 한 번을 위해 세션 내내 화면 한 줄을 내주는 셈이다. 사용자가 이 전제 어긋남을 확인하고 채널을 다시 고르기로 했다(사용자 결정 2026-09-21).

★**회수 시점은 늦어도 된다**★(사용자 지적 2026-09-21) — id 가 필요한 것은 **최초 화신 한 번뿐**이다. 그 뒤 화신은 우리가 `resume <id>` 를 인자로 넘기므로 id 가 보장된다. 그래서 「스폰 0.3초 안에 잡아야 한다」는 제약이 애초에 없었고, 첫 턴 뒤에 받아도 된다. 이 완화가 아래 채널을 가능하게 한다.

## 결정

1. **회수 정본 = 우리 MCP 서버가 받는 `tools/call` 의 `_meta`.** codex 가 `eg_*` 도구를 부를 때 그 요청에 스레드 id 가 실려 온다 — `_meta.threadId` 와 `_meta.x-codex-turn-metadata.{session_id, thread_id, turn_id, model, codex_version}`. ★**짝짓기가 없다 — 우리는 받는다.**★
2. **신원은 이미 있는 per-agent bearer 토큰이 준다.** 요청 extensions 의 `BoundIdentity { agent_id, epoch }`(`control/registry.rs:24`)를 그대로 쓴다. ★**epoch 까지 들고 있다**★ — 오늘 데몬에 하나뿐인 프로필 sid 쓰기 경로(`SessionTracker` 콜백)가 epoch 를 `None` 으로 넘기는 것보다 강하다.
3. **상태줄 채널을 걷어낸다.** 스폰 인자 `STATUS_LINE_OVERRIDE`(`codex/mod.rs:833-841`) 제거 · 스캐너 `backend/codex/thread_id.rs` 전체와 유일한 운영 호출부(`codex/mod.rs:1026`) 제거. **PTY 의 pre-input observer seam**(`transport/pty.rs:351-356`·`transport/mod.rs:61`)은 이 스캐너가 유일 소비자였으므로 함께 걷는다.
4. **보완 표식을 심는다 — 이번엔 심기만 한다.** 스폰 env 에 `CODEX_INTERNAL_ORIGINATOR_OVERRIDE=<에이전트별 고유 문자열>` 을 넣는다. 그 값이 세션 기록의 `originator` 칸에 그대로 남아, 나중에 행을 찾을 때 **정확 문자열 일치**로 소유를 판정할 수 있다. ★**읽는 쪽은 이 라운드에 만들지 않는다**★ — 후속이며, 읽을 때도 파일을 긁지 않고 app-server 에 묻는다(ADR-0203 이 허락한 경로).
5. **env 주입 조건은 control endpoint 가 있을 때만**이다 — 기존 `inject_cli_entrance` 와 같은 조건. 무조건 싣지 않는다: `without_an_endpoint_nothing_is_injected`(`codex/mod.rs:1548`)가 지키는 「엔드포인트 없으면 env 도 없다」를 깨지 않는다.
6. **쓰기 경로로 agent crate 에 공개 동사를 하나 낸다.** 오늘 MCP 핸들러(데몬)에서 프로필에 닿는 길이 **없다** — `AgentManager` 가 `profiles` 를 private 으로 쥐고 쓰기 위임 메서드가 없다. 새 동사는 `ProfileRegistry::observe_session_id`(`profile.rs:709`)의 **epoch 가드를 그대로 타야** 한다(우회로를 새로 만들지 않는다).
7. **값 정책은 덮어쓰기 유지**(ADR-0216 결정 7 승계). 같은 값이면 no-op, 다른 값이면 옛 값을 `old_session_ids` 로 밀고 기록한다. 매 턴 같은 id 가 다시 와도 무변화다.
8. **회수는 fresh 화신에만 돈다.** resume 화신은 우리가 id 를 인자로 넘기므로 받을 것이 없다. 기존 `fresh_spawn_release_session_id`(`manager.rs:467`)와 모순되지 않는다.
9. **못 받으면 못 받은 채로 둔다 — 폴백을 만들지 않는다**(ADR-0203·0216 승계). ★**알려진 갭**★: 도구를 한 번도 부르지 않는 세션은 id 를 못 받고 재개되지 않는다. 그 갭은 결정 4 의 읽기 경로가 후속으로 닫는다.

## 거부한 대안

- ★**상태줄을 그대로 둔다(ADR-0216 현행)**★ — 돈다는 것은 실측이고 결함도 아니다. **기각 = 사용자 결정**: 승인 전제(「한 번 뜨고 사라진다」)가 사실과 달랐고, 회수 1회의 대가로 상주 한 줄은 과하다.
- **상태줄 항목을 `thread` 로 바꾼다** — 항목 설명이 `Current thread title, or thread identifier when unnamed` 이고 codex 가 첫 턴 뒤 제목을 자동 생성하므로, **첫 턴 뒤 36자 hex 가 제목으로 바뀐다**(바이너리 실측). ★기각 = 줄 자체는 남는다★ — 사용자의 불만이 hex 가 아니라 상주였으므로 목적을 못 푼다.
- **`tui.terminal_title` 로 받는다**(채팅 본문에 안 보이고 OSC 로 깔끔히 온다) — ★**기각 = 실측**★: **32자로 잘린다**(29자 + 말줄임, 뒤 7 hex 손실), 잘림은 화면 폭과 무관하게 고정. ADR-0216 이 이미 실측으로 기각한 것을 이번 조사가 **최선으로 재추천**했다 — 조사는 절단을 미확인으로 남겼고 우리 실측이 이긴다. **다음에 또 올라오면 이 줄을 먼저 볼 것.**
- **`/statusline` 슬래시 커맨드로 런타임에 끈다** — 실재한다(바이너리 문자열 확인). ★기각 = 선택을 `~/.codex/config.toml` 에 **영속**한다★(저장 실패 문자열까지 있다) + 대화형 다중선택 픽커라 스크립트로 몰기 취약. ADR-0210 결정 1(사용자 기기에 쓰지 않는다)에 걸린다.
- ★**락 파일 + 시간창**★(`~/.codex/thread-writer-locks/<id>.lock` 은 기동 즉시 생기고 id 가 UUIDv7 이라 생성 시각이 이름에 박혀 있다) — **기각 = 실측**. 스폰→id 생성 지연 4회 = **0.735 · 0.879 · 0.907 · 1.891초**인데 ★**상한이 없다**★: 별도 라운드에서 **21초** 사례가 관측됐고 4회 재현 시도에서 다시 나오지 않아 원인 미규명이다. 2초 창은 그 사례에서 후보 0 개(못 받음), 25초 창은 그 25초 안에 사용자가 codex 를 하나만 띄워도 후보 2 개(거절)다 — **두 끝을 함께 만족하는 폭이 없다.** 충돌 실측 = 411ms 차로 띄운 둘의 id 생성 간격 **90ms**.
  - 곁가지 사실(기각을 뒤집지는 않는다): **codex 는 기동할 때 주인이 죽은 락을 쓸어낸다**(2회 관측 — 우리가 지운 것이 아니다). 후보가 누적되지는 않지만 창 문제는 그대로다.
- **app-server 목록으로 첫 턴 전에 찾는다** — ★기각 = 실측★. 평범한 TUI 는 app-server 에 **붙지 않는다**: TUI 4개가 살아 있는 상태에서 `thread/loaded/list` 가 빈 배열이었고, `netstat` 상 소켓을 쥔 것은 `--remote` 로 띄운 하나뿐이었다. 게다가 npm 설치본은 관리형 데몬 자체를 못 띄운다(`codex app-server daemon start` → standalone 설치 없음). 첫 턴 **전** 스레드는 디스크 목록에도 없다.
- **`codex resume --last` 로 id 없이 이어받는다** — 실측으로 **per-cwd 가 맞았다**: 다른 폴더에 더 최근 세션이 있어도 그 폴더 것을 잡았고 `-C` 로 조종된다(`--all` 설명이 「disables cwd filtering」이라 기본이 cwd 필터임을 확인해 준다). ★기각 = **같은 cwd 에 우리 에이전트가 둘이면 충돌**하고, 사용자가 그 폴더에서 자기 codex 를 띄워도 충돌한다★ — 하드 키가 둘이나 있는데 추측 경로를 남길 이유가 없다.
- **훅을 되살린다(herdr 의 고정 경로 래퍼)** — 선례가 있고, 승인 해시가 훅 정의를 덮지 스크립트 내용을 안 덮으므로 **평생 1회 승인**이 성립한다. 기각 = **비용과 선**: 되살릴 양이 약 1,900줄(`535cdba` 가 지운 것)이고, 사용자 홈에 파일을 까는 것은 ADR-0210 결정 1 에 걸린다. MCP 채널이 같은 값을 더 싸게 준다.
- **orca 방식(RPC 로 훅 신뢰 부여)** — ADR-0216 의 기각을 승계한다: 신뢰 키가 위치 기반이라 남의 훅을 우리가 대신 승인하는 꼴이 된다.
- **락 파일의 소유 PID 로 판정한다** — 락은 소유 프로세스가 열어 두고 있어 OS 수준 PID↔스레드 결속이 실재한다(`cat` 이 busy 로 실패하는 것으로 held 여부까지는 확인됐다). ★**미검으로 남긴다**★ — 어느 PID 가 쥐고 있는지 보는 경로(Restart Manager)가 이 환경의 정책에 막혔다. 되살리려면 그 확인부터 해야 한다.

## 근거

- ★**실측 정본이 이 ADR 이다**★ — 아래 수치는 2026-09-21 워커 3라운드의 측정이고 조사 문서로 승격되지 않았다. 방법: 이미 신뢰된 `I:\Engram_Workspace\qa-codex-probe` 에서 codex 를 자기 콘솔로 띄우고, 로그만 찍는 최소 stdio MCP 서버를 붙여 JSON-RPC 를 통째로 받아 봤다. codex-cli **0.155.1**.
- **Probe A(채널) — 확인.** `initialize` 에는 id 가 없고(`clientInfo` 뿐, `_meta` 없음) MCP 서버 환경에는 `CODEX_*` 가 하나도 안 닿는다(scrubbed). 실리는 자리는 **`tools/call` 의 `_meta`** 다. 받은 id 가 rollout 의 `session_id` 와 동일하고, 우리가 띄운 프로세스(PID 38052)의 것이며, **TUI 와 `exec` 양쪽에서 같은 모양**이었다.
- **Probe B(표식) — 확인.** `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` 에 임의 문자열 3종을 심어 전부 기록의 `originator` 에서 그대로 되읽었다. allowlist 거절은 관측되지 않았다.
- **Probe C(기록 행) — 확인.** 첫 턴 뒤 행에 `session_id · cwd · originator · cli_version · source · thread_source · model_provider · git` 이 실린다. ★**행 어디에도 PID 가 없다.**★ 첫 턴 **전**에는 스레드 id 가 생겨도 rollout 파일이 없다.
- **Probe D(PID) — 없음.** rollout JSONL 에 pid·hostname·명령줄이 없다.
- **코드 실측(탐색 라운드).** env 는 이미 자식에게 전달된다 — `build_spec` 이 `env` 를 받고(`codex/mod.rs:718`) `transport/pty.rs:168` 에서 적용된다. **새 배선이 필요 없다.** `_meta` 는 지금 버려지지만 rmcp 2.2.0 이 `RequestContext.meta` 로 이미 올려 준다. 데몬→프로필 쓰기 경로는 없다(결정 6 이 그것을 연다).
- **게이트.** MCP 도구 호출은 승인 정책을 탄다 — `-a never` 면 거부된다(실측). 우리 스폰은 `-a on-request` 라 해당 없다. **서버별 자동승인 설정은 존재하지 않는다** — `--strict-config` 로 `auto_approve`·`trust`·`trusted`·`approval_policy`·`tool_approval` 이 전부 unknown field 로 거절됐고, `mcp_servers.<name>` 아래 받는 것은 `enabled_tools` 와 `startup_timeout_sec` 뿐이다.
- ★**미검 셋**★ — ① **사람이 직접 승인한 일반 TUI 에서도 같은 `_meta` 가 오는지**는 측정하지 않았다(측정은 `--approve-for-me` 로 돌렸고, 그 플래그는 같은 cwd 에 **자동 리뷰용 형제 스레드를 하나 더** 만든다). ② `originator` 값이 벤더 telemetry·과금 귀속에 영향을 주는지 모른다. ③ `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` 는 **문서화되지 않은 내부 변수**라 업스트림이 없애면 조용히 안 먹는다 — 실패 모드는 「못 받음」이다.

## 영향 / 불변식

- **ADR-0216 의 회수 채널이 죽는다** — 상태줄 인자·스캐너·무장 창(첫 입력/예산/회수 성공)이 함께 나간다. 그 ADR 의 **덮어쓰기 정책(결정 7)과 fresh 반납은 살아 있고** 이 결정이 승계한다.
- ★**「창 안에 사용자·모델 내용이 구조적으로 없다」는 불변식이 불필요해진다**★ — 화면을 읽지 않으므로 첫 매치 tie-break 자체가 사라진다. ADR-0216 이 그 창을 지키려고 세운 세 조건(무장 유지 금지 등)도 함께 은퇴한다.
- **새 불변식: 쓰기는 epoch 를 들고 간다.** MCP 경로는 `BoundIdentity.epoch` 를 그대로 `observe_session_id` 의 화신 가드에 넘긴다 — `None` 으로 우회하지 않는다. 우회하면 죽은 화신의 보고가 산 프로필을 덮는다.
- **새 불변식: env 는 control endpoint 와 함께만 실린다**(결정 5). 어기면 `without_an_endpoint_nothing_is_injected` 가 깨지고, 엔드포인트 없는 스폰에 우리 표식이 새어 나간다.
- **`manager.rs:455-458` 의 주석 가정이 뒤집힌다** — 「codex 가 이 칸을 채우는 경로는 제어 평면이 아니라 PTY 상태줄」이라고 적혀 있다. 그 자리를 이 결정에 맞게 고친다.
- **ADR-0203 은 뒤집지 않는다** — 이 라운드는 벤더 저장소를 **읽지 않는다**(결정 4 는 심기만 한다). 후속에서 읽게 될 때도 파일이 아니라 app-server 질의여야 한다.
- **회귀망** — 상태줄 스캐너 테스트(`thread_id.rs` 단위군 · `codex/mod.rs` 의 PTY 경유 end-to-end · `transport/pty.rs` 의 pre-input observer 배선)가 삭제 대상과 함께 나간다. **그 자리를 MCP `_meta` 파싱과 새 쓰기 동사의 테스트가 대신 채워야 한다** — 순 감소로 두지 않는다.
