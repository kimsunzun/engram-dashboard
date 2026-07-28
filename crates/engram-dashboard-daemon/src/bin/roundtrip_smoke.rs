//! roundtrip-smoke — ADR-0092 A→B→A 왕복(reply round-trip) 실측 드라이버(검증 전용 bin).
//!
//! ## 역할
//! priming-smoke 는 A→B **수신**만 증명했다(합성 발신자 1명 → 실 에이전트 1명). 이 하네스는 그 위에
//! 두 가지를 **추가로** 실측한다:
//!   ① 실 에이전트 B 가 **발신 절반**(MCP `send_message` 툴 OR `engram-send` CLI)을 **스스로** 호출하고,
//!   ② A 가 B 의 답신을 자연스럽게 수용한다.
//!
//! 즉 실 primed claude **2개**(A·B, stream-json/Fresh)를 스폰하고:
//!   1. B 에게 짧은 원과제 턴을 줘 "일하는 팀원" 맥락을 만든다.
//!   2. A→B 로 자연스러운 질문 하나를 실 control 경로(`handle_send`, Entrance::Cli)로 **씨앗 주입**한다
//!      — B 는 봉투 `[message from alice id:..]` 에서 A 의 이름을 배운다(본문엔 "툴 X 를 써라" 같은 기계적
//!      지시를 넣지 않는다 — 발신 학습은 프라이밍 변형이 하고, 기본(no-priming/both) 은 순수 툴 발견을 본다).
//!   3. 하네스는 **B 의 답신에 대해 handle_send 를 부르지 않는다** — B(실 claude)가 스스로 MCP/CLI 입구를
//!      호출하고, 그 요청이 **실제 입구 → handle_send → wrap → A stdin** 으로 흐른다.
//!   4. 관측: (a) 기계적 = registry `DeliveryObservation`(from=B, to=A)이 실제로 생겼는지 + B 가 고른
//!      입구(Mcp/Cli). (b) 정성적 = A 의 `TurnObserver` 가 A 가 답신을 처리하며 낸 텍스트를 누적.
//!   5. 구조화 stdout 마커로 오케스트레이터 판정용 결과를 출력한다.
//!
//! ## 프라이밍 선택(발신 학습 변형 — priming 파일로 조절, ADR-0099 이후)
//!   - 미지정(또는 `--priming C0`) = `prompts/agent-priming.md`(운영 A = both-teaching: send_message +
//!     engram-send). "C0" 는 no-priming 기본과 동의어인 편의 별칭일 뿐이다(그 하나만 남았다).
//!   - `--priming <abs-or-rel-path>` 로 임의 프라이밍 파일 직접 지정(예: 운영 B `prompts/agent-priming-cli.md`).
//!     절대면 그대로, 상대면 repo 루트 기준.
//!   ※ 옛 C1~C3 케이스 별칭(`prompts/experiments/agent-priming-send-*.md` 로 매핑되던)은 ADR-0099 로 제거됐다
//!     — 실험 변형 파일이 정리됐고(git 이력 보존), 채널 변형은 이제 백엔드 capability 가 정적 2파일로 가른다.
//!     C1~C3 문자열을 `--priming` 에 넘기면 이제 "그 이름의 파일 경로"로 해석돼 하류에서 부재로 걸린다(특수
//!     매핑 없음).
//!
//! ## CLI-only 측정 노브 2종(측정 축이 다르다)
//!   - **`--disallow-mcp`(ADR-0094)** — `ENGRAM_DISALLOW_MCP_SEND` env 를 세워 MCP `send_message` **grant 만**
//!     뺀다. MCP 서버는 여전히 mcp-config 로 부착되고 프라이밍도 both-teaching 이라, 물리(MCP 살아있음)와
//!     교육/권한(CLI 만)이 **갈린다** — 프롬프트-도구 불일치를 일부러 만들어 순수 CLI 라우팅을 측정한다.
//!   - **`--cli-only`(ADR-0099 FIX 3)** — `ENGRAM_FORCE_CLI_ONLY_SEND` env 를 세워 provision 을 **비-MCP 로
//!     강제**한다. mcp-config 미부착 + CliOnly 프라이밍(`prompts/agent-priming-cli.md`) + [Cli] grant 가 함께
//!     움직여 정합 불변식을 **보존한 채** false path 전체를 실측한다(실 claude 를 비-MCP 백엔드처럼 굴림).
//!     이 모드는 `--priming` override 를 주지 않는다(상속된 `ENGRAM_PRIMING_FILE` env 도 거부 = SETUP-FAIL,
//!     조용히 clear 하지 않는다) — provision 이 CliOnly 파일을 auto-select 하는 걸 보는 게 목적이고,
//!     entrance=cli 를 기대한다(mcp 관측 시 SETUP-FAIL = 강제 seam 결함).
//!     ★엄격 판정(다른 모드와 다르다)★: `--cli-only` 는 B 가 실제로 CLI 입구로 보냈을 때만(b_sent=true AND
//!     entrance=cli) exit 0(PASS). 아무것도 안 보낸 경우(B_SENT=false/ENTRANCE=none)는 일반 모드의
//!     valid-negative 와 달리 **exit 1**(FAIL — 강제 false path 가 도는 걸 못 봤으니 목적 미달)이고, 전용
//!     `VERDICT [... --cli-only]:` 줄로 결과를 낸다.
//!   둘 다 test-only 노브(운영 스위치 아님)이고 CLI 입구(send_exe = engram-send 형제 빌드)가 필수다 — 없으면
//!   SETUP-SKIP. 미지정이면 오늘 동작(MCP 경로)과 바이트 동일.
//!
//! ## 씨앗 계약·B 과제 노브 2종(위 CLI-only 노브와 독립적으로 조합 가능)
//!   - **`--seed-request`**(+ 선택 `--seed-reply-by <dur>`, 예: `5m`) — 씨앗 A→B 를 **회신 계약**
//!     (`SendContract{request:true,..}`)으로 보낸다. 같은 실 control 경로(`handle_send`, `Entrance::Cli`)
//!     그대로 계약 축만 얹는다 — entrance/backend 불변. `--seed-reply-by` 를 `--seed-request` 없이 단독
//!     지정하면 **인자 오류**(exit 1, 다른 setup 전에 fail-fast — reply_by 는 request 전용, ingress.rs
//!     `validate_contract` 규칙 1 과 같은 정신). ★`--seed-reply-by` 값 자체의 오용(F5 와 같은 규율)★ 값
//!     누락 / 다음 토큰이 `--` 로 시작(플래그로 오인 가능) / 빈 값·공백만인 값도 전부 **인자 오류**(exit 1,
//!     스폰 전 fail-fast — `--b-task` 오용 가드와 같은 분업: parse_args 는 사실만 기록, 반려는 run() 이).
//!     조용히 넘기면 기한 없이(또는 엉뚱한 값으로) 돌아 오퍼레이터가 눈치채기 어렵다.
//!     결과 블록에 `SEED_KIND=request|plain` 을 항상 찍고, request 일 때만
//!     `SEED_REPLY_BY=<value|none>`·`REPLY_IN_REPLY_TO=<id|none>`·`REPLY_MATCHES_SEED=true|false`·
//!     `REPLY_POLL=matched|timeout|skipped-no-budget`(항목1, option b — 아래 F3 폴링 예산 절 참조)·
//!     `REPLY_POLL_BUDGET_MS=<ms>`(신규 — 폴링 스킵/타임아웃을 가른 결정 지점의 잔여 예산을 ms 로 그대로
//!     드러낸다. 0 = 예산 없음(스킵) **또는 1ms 미만 잔여**(ms 절사) — 스킵 판정 정본은 REPLY_POLL 라벨.
//!     이게 없으면 REPLY_POLL=timeout 뒤에 "100ms 짜리 굶주린 예산" 인지 "정상 예산을
//!     다 쓰고도 못 찾음" 인지가 안 갈린다)를
//!     추가로 찍는다(B 의 답신 봉투가 in-reply-to 를 실었는지 + 씨앗 msg_id 와 일치하는지 — 아래 관측 절의
//!     `DeliveryObservation.in_reply_to` 확장 필드로 판정).
//!     ★두 마커는 서로 다른 축이다(FIX)★: `REPLY_MATCHES_SEED` 는 baseline 이후 배달된(is_delivered)
//!     B→A 레코드 중 `in_reply_to == seed_msg_id` 인 것이 있는지의 **엄격 일치** 판정이고,
//!     `REPLY_IN_REPLY_TO` 는 **그 일치 레코드가 있으면 그 레코드의 값(= seed id)을 우선해서 찍고,
//!     없을 때만** baseline 이후 첫 배달 레코드 중 `in_reply_to.is_some()` 인 값으로 폴백한다 — 그래야
//!     "B 가 틀린/환각 id 로 회신함"(REPLY_MATCHES_SEED=false 인데 REPLY_IN_REPLY_TO 엔 값이 있음 — 계약
//!     위반 중 가장 유력한 형태)과 "B 가 in-reply-to 를 아예 안 실음"(REPLY_IN_REPLY_TO=none)이
//!     구분된다(이전엔 REPLY_IN_REPLY_TO 가 일치 판정의 `matched` 에서 파생돼 seed id 또는 none 두 값만
//!     낼 수 있었다 — 가장 유력한 계약 위반 형태를 안 보이게 만드는 결함이었다).
//!     ★마커쌍 자기모순 재발 방지(리뷰어 NOTE FIX)★: "일치 여부와 **무관하게 항상** 첫 배달 레코드 값을
//!     찍는다" 로 짜면, B 가 틀린 id 로 먼저 답하고(then) 맞는 id 로 나중에 답할 때
//!     `REPLY_IN_REPLY_TO=<wrong-id>` 인데 `REPLY_MATCHES_SEED=true` 인 자기모순 쌍이 난다(두 마커가 서로
//!     다른 레코드를 가리킴 — 첫 레코드 vs 일치 레코드). 그래서 이제 매치가 존재하면 그 매치 레코드를
//!     **항상** 우선해 두 마커가 같은 레코드에서 파생되게 한다(첫-레코드 폴백은 매치가 아예 없을 때만).
//!     ★F3 레이스는 폴링으로 닫는다(FIX)★: 판정은 **첫 도착 레코드가 아니라** A 턴 대기까지 끝난 뒤
//!     baseline 이후 B→A 레코드 **전부**를 스캔해 내리되, 그 스캔에 앞서 "배달 + in_reply_to ==
//!     seed_msg_id" 레코드가 나타나길 같은 폴링 간격(200ms)으로 마저 기다린다(seed-request 일 때만 —
//!     plain 씨앗은 in_reply_to 자체가 없어 폴링할 대상이 없으므로 기본 경로를 늦추지 않는다). 첫
//!     레코드가 실제 회신과 무관한 "ack" 한 통이고 진짜 회신이 그 뒤에 늦게 와도, A 턴 대기 뒤 이 폴링이
//!     (잔여 예산이 있을 때만) 마저 잡아 두므로 스캔이 놓치지 않는다(이전엔 스캔만 하고 기다리진 않아 A 턴이 먼저 끝나면 여전히
//!     거짓 negative 가 났다). 배달된(is_delivered) 레코드만 증거로 인정한다 — write 실패 레코드가
//!     우연히 seed id 를 실었어도 실제로 도달 안 했으므로 "회신 성공" 의 증거가 아니다.
//!     ★예산은 wait_for_reply 전용이 아니라 A 턴 대기와 공유하는 하나의 벽시계다(option b, FIX)★: 이
//!     폴링이 쓰는 잔여 예산은 씨앗 주입 직후 잡은 시각부터 `REPLY_WAIT_CAP`(180s)을 잰 나머지다 —
//!     그 사이엔 `wait_for_reply`(B 의 첫 답신 대기)뿐 아니라 그 뒤 이어지는 `wait_turn_end`(A 턴 종료
//!     대기, 최대 `TURN_WAIT_CAP`=180s)까지 **같은 벽시계를 함께 태운다**(이전 주석은 "wait_for_reply 가
//!     소비한 시간만 뺀다" 고 잘못 말했었다 — 실제로는 A 턴 대기가 더 큰 소비자일 수 있다). 즉 A 턴
//!     대기가 길어지면 이 폴링의 잔여 예산이 0 이하로 떨어질 수 있고, 그러면 폴링은 **아예 돌지 않는다**
//!     (조용히 스킵) — 새 타임아웃 상수를 만들지 않는 대신 예산을 공유시킨 트레이드오프다(사용자 결정,
//!     option b: 구조는 그대로 두고 이 사실을 정확히 알리는 쪽). 그래서 결과 블록에
//!     `REPLY_POLL=matched|timeout|skipped-no-budget` 을 항상 찍어(seed-request 일 때만) 예산 고갈로
//!     폴링이 안 돈 경우를 침묵시키지 않는다 — skipped-no-budget = 잔여 예산이 0/음수라 폴링 자체가 안
//!     돎, timeout = 폴링이 돌았으나 예산 소진까지 못 찾음, matched = 폴링이 잡았든(또는 폴링 전에 이미
//!     배달돼 있었든) 최종 스캔이 일치를 확인함.
//!     ★N1★ `--seed-reply-by` 는 B 의 봉투에 `reply-by` 속성을 **렌더만** 한다 — 이 하네스는 데몬의 60초
//!     sweep 을 돌리지 않는 단발 실행이라, 여기서 기한 초과 타임아웃/notice 가 발화하는 일은 없다.
//!     ★N2★ 잘못된 기간 표기(파싱 실패)는 두 에이전트가 모두 스폰된 **뒤**, 씨앗 발송 시점에야 잡힌다
//!     (`validate_contract` 가 그 자리서 반려) — 비용은 있으나 분류는 여전히 SETUP-FAIL 이다(스폰 자체는
//!     헛되지 않았다고 보지 않는다 — 그냥 더 늦게 걸릴 뿐).
//!   - **`--b-task <text|@path>`** — B 의 원과제 프롬프트(기본 auth 모듈 과제)를 대체한다. `@` 접두면
//!     파일 참조(절대/상대 — 상대는 repo 루트 기준, `--priming` 명시 경로 override 와 동일 규약)로 그
//!     내용을 읽는다. 파일 부재/읽기 실패는 SETUP-FAIL(exit 1, 아직 아무것도 스폰하지 않은 시점에
//!     fail-fast — 정리할 리소스 없음). ★F5 오용 가드★ 값 누락·다음 토큰이 `--` 로 시작(플래그로 오인
//!     가능해 `take_flag_value` 가 소비하지 않는 바로 그 경우)·빈 값/공백만인 값은 전부 **인자 오류**
//!     (exit 1, 스폰 전 fail-fast — 조용히 기본값으로 넘기지 않는다). 텍스트를 파일로 좁히는 게 아니다 —
//!     인라인 텍스트는 여전히 허용, 잘못 쓴 값만 막는다(사용자 결정). 결과 블록에
//!     `B_TASK=default|file:<path>|inline(<n> bytes)` 를 항상 찍는다(F5).
//!   둘 다 test-only 노브. 미지정이면 **스폰·프라이밍·발신 동작은 오늘과 바이트 동일**하다 — 다만 결과
//!   블록 stdout 은 두 노브 모두에 대해 항상 한 줄씩 늘어난다(`SEED_KIND=plain`·`B_TASK=default`, F4).
//!   ★F4 — 이 두 줄은 의도적으로 항상 찍는다★: `SEED_KIND=` 는 조용히 무시되는 미지정 토큰들 사이에서
//!   오퍼레이터의 오타(예: `SEED_KIND=plain` 인데 `--seed-request` 를 쳤다고 착각)를 눈에 띄게 하는
//!   신호이기도 하다. 즉 "오늘과 바이트 동일" 은 에이전트 스폰·프라이밍·전송 로직에 대한 주장이지 stdout
//!   전문에 대한 주장이 아니다 — 결과 블록에 두 줄이 늘어나는 것과 모순이 아니다.
//!
//! ## 실행(오케스트레이터가 런타임에 돌린다 — 이 파일은 빌드/컴파일만)
//! ★CLI 입구를 쓰는 실험(운영 B `prompts/agent-priming-cli.md` 또는 `--cli-only`)은 먼저 `engram-send` 를
//!   빌드해야 한다★ — 이 하네스는 자기 exe 형제에서 `engram-send`(Win: `.exe`) 를 찾아 CLI 입구를 켠다.
//!   형제에 없으면 B 가 그 경로로 못 보내 **인프라 부재를 실험적 negative 로 오인**할 수 있다. `cargo run` 은
//!   dep bin 을 안 만들므로 별도로 빌드한다(같은 profile/target 이어야 형제로 co-locate 된다):
//! ```text
//! # 1) CLI 입구 바이너리 먼저 빌드(CLI 경로 실험 필수 — 형제 위치에 놓이게)
//! cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send
//! # 2) 하네스 실행
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke                 # 기본(both, MCP)
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --priming prompts/agent-priming-cli.md --model sonnet
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --cli-only    # provision 강제 비-MCP(false path 전체)
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --seed-request --seed-reply-by 5m   # 씨앗을 회신 계약으로
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --b-task "You are on the billing module. Reply in one line when ready."
//! ```
//! CLI 입구가 필요한 프라이밍(본문이 engram-send/ENGRAM_SEND_EXE 를 언급 — 명시 경로 무관)인데 `engram-send`
//!   가 형제에 없으면, 하네스는 normal negative 가 아니라 **SETUP-SKIP**(engram-send not built) 라벨로 요란히
//!   알리고 종료한다 — 인프라 부재를 "B 가 안 보냄" 으로 오귀속하지 않는다. 판정은 셀렉터·basename 이 아니라
//!   **해석된 프라이밍 파일 본문(content)** 이라 명시 경로 override 와 CLI-지시 프라이밍까지 모두 잡힌다(ADR-0094).
//!
//! ## 핵심 불변식(ADR-0092/0086/0088)
//! - **required-features = ["test-harness"]** — 운영/릴리즈 빌드는 이 bin 을 컴파일하지 않는다.
//! - **프라이밍은 실물 파일에서**(ADR-0092) — 하드코딩 금지. 여기선 케이스→경로 매핑만 하고 `ENGRAM_PRIMING_FILE`
//!   env 로 FilePrimingProvider 에 넘긴다(두 에이전트가 같은 변형을 받게 provider 생성 **전에** set).
//!   ★`--cli-only` 예외★: 그 모드는 override 를 세우지 않고 `ENGRAM_FORCE_CLI_ONLY_SEND` 만 세운다 —
//!   provision 이 CliOnly 파일을 스스로 고르는 걸 관측한다(ADR-0099 FIX 3).
//! - **from 은 토큰 파생**(ADR-0086) — 씨앗 A→B 의 from = A 의 실 발급 신원(BoundIdentity), 본문 문자열 아님.
//! - **B 의 답신은 실 입구로만**(하네스가 handle_send 를 대신 부르지 않는다) — 이게 이 하네스의 핵심 새 검증.
//! - **배달 관측 = ADR-0088 in-proc 싱크** — registry 에 `DeliveryObserver` 를 설치해 relay 레코드를 회수한다
//!   (detached 데몬 로그 스크레이핑 금지). registry 에 read accessor 를 추가하지 않고 이 싱크로만 회수한다.
//!   ★확장(roundtrip-smoke `--seed-request`)★: `DeliveryObservation.in_reply_to`(Option<String>)는 발신
//!   시점에 검증된 구조화 메타(`SendMeta.reply_to`)를 각 wrap 호출부가 `observe_success`/
//!   `observe_failure`(service.rs) 에 **파라미터로 직접 넘겨** 채운다 — 렌더된 봉투 문자열을 다시 파싱하지
//!   않는다(옛 substring 파서는 본문 이스케이프 허점으로 위조 가능해 리뷰 F1 에서 삭제됐다 — ingress.rs
//!   `DeliveryObservation.in_reply_to` 주석이 정본). registry 에 새 read accessor 는 없다(ADR-0088 HARD
//!   CONSTRAINT 준수) — 관측 함수 시그니처에 파라미터 하나가 늘었을 뿐, registry 조회는 없다.
//! - **결과 3분류(FIX round-2 #2/#4/#5)** — ① **valid negative**(setup 성공했으나 B 가 안 보냄) = 구조화
//!   결과 출력 후 exit 0(유효한 실험 결과). ② **SETUP-SKIP**(exit 1) = 케이스가 요구하는 인프라 부재
//!   (CLI-지시 프라이밍인데 engram-send 미빌드 — 판정은 셀렉터·basename 이 아니라 **해석된 프라이밍 파일 본문**)
//!   — normal negative 로 오귀속 금지. ③ **SETUP-FAIL**(exit 1) = 준비 단계 실패(**인자 오류** —
//!   `--b-task`/`--seed-reply-by` 오용 가드·`--cli-only` co-pass 거부 등, 스폰 전 fail-fast / A/B 출력
//!   구독 실패 / B 원과제 턴 실패 / A·B process death / 씨앗 ACK 에러 / priming 파일 부재). valid negative
//!   는 setup 이 온전히 성공했고 **A·B 가 모두 살아 있을 때만** 보고한다(A 死 → B 답신이 도달할 대상 없음).
//! - **skip_no_claude loud-skip** — claude 부재/인증 실패면 요란하게 스킵(silent skip 금지).
// ADR-0092

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot, MessagingSlot};
use engram_dashboard_daemon::control::priming::{FilePrimingProvider, PrimingProvider};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::messaging_host::messaging_for_manager;
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

/// 스폰 후 목록 등장 대기.
const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
/// 턴 종료(MessageDone) 대기 상한.
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);
/// B 답신(outbound relay) 대기 상한 — 초과 시 NEGATIVE(B did not send) 결과.
const REPLY_WAIT_CAP: Duration = Duration::from_secs(180);

/// A(발신자 팀원)의 표시 이름 — B 가 봉투에서 배워 `to=alice` 로 답신한다.
const NAME_A: &str = "alice";
/// B(수신·답신) 표시 이름.
const NAME_B: &str = "bob";

/// B 원과제(일하는 팀원 맥락) — auth 모듈 작업 중. 자연스러운 협업 셋업.
const TASK_PROMPT_B: &str =
    "You are currently working on the auth module (login/session). When you're ready to start, reply in one line.";

/// ★씨앗 A→B(ADR-0092 — 자연 팀원 질문, 기계적 "툴 X 써라" 아님)★: A 가 B 에게 진행 상황을 묻는
///   평범한 협업 질문 → 답을 A 에게 돌려주는 게 자연스러운 반응이 되도록 만든다. 발신 방법(툴/CLI)은
///   본문이 아니라 **프라이밍 변형**이 가르친다(C0/기본 = 프로덕션 both-teaching `prompts/agent-priming.md`).
const SEED_A_TO_B: &str =
    "Can you share the status of the auth module? If you're stuck anywhere on the login path, tell me what you need too.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

/// ★loud skip(priming_smoke 이식)★: claude 스폰 불가면 요란하게 스킵(exit 0 이되 SKIPPED 라벨을
///   stdout+stderr 에 남긴다 — silent skip 금지).
fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [roundtrip-smoke]: {reason} — A→B→A 왕복 실측 불가(claude 부재/인증).");
    println!("{line}");
    eprintln!("{line}");
    0
}

/// ★SETUP-SKIP(FIX round-2 #2)★: 선택 케이스가 요구하는 인프라(예: CLI 입구용 engram-send)가 없어
///   실험을 유효하게 돌릴 수 없을 때. **normal negative 와 구분되는** 라벨로 요란히 알린다 — 인프라
///   부재를 "B 가 안 보냄" 으로 오귀속하지 않는다. exit 1(설정 미비는 실험 결과가 아니라 실행 조건 미충족).
fn setup_skip(reason: &str) -> i32 {
    let line = format!("SETUP-SKIP [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

/// ★SETUP-FAIL(FIX round-2 #4)★: 스폰 후 실험 준비(B 원과제 턴 / 씨앗 ACK / B 생존) 중 하나가 진짜로
///   실패했을 때. valid negative("B did not send")와 **구분되는** 라벨로 알린다 — 유효 negative 는 setup
///   이 온전히 성공했을 때만 보고한다. exit 1(실험 결과가 아니라 setup 실패).
fn setup_fail(reason: &str) -> i32 {
    let line = format!("SETUP-FAIL [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

/// ★프라이밍 본문이 CLI 발신 경로를 지시하는가(순수·단위테스트 대상)★: 텍스트가 `engram-send` 또는
///   `ENGRAM_SEND_EXE` 를 언급하면 CLI 입구(engram-send)로 보내라는 프라이밍이다. 둘 중 하나만 있어도 true.
///   ★대소문자 무시(FIX)★: 본문 산문이 `ENGRAM-SEND`/`Engram-Send` 처럼 대소문자를 섞어 써도 잡아야 한다 —
///   놓치면 false negative(CLI 지시인데 미검출) → 인프라 부재를 정상 negative 로 오귀속. 본문을 한 번
///   lowercase 로 복사(단일 할당)해 소문자 리터럴과 대조한다.
///   ★basename 이 아니라 본문(content)인 이유★: 이전 판본은 하드코딩된 basename 리스트
///   (`agent-priming-send-cli.md`/`-send-both.md`)만 봤다. 그런 리스트는 rot 한다 — 새 CLI-지시 프라이밍
///   (v3-en-cli 등)이 리스트에서 누락돼 가드가 조용히 우회됐고, engram-send 부재(인프라 부재)가 SETUP-SKIP
///   대신 정상 negative(B_SENT=false)로 오귀속됐다. 그래서 파일명이 아니라 실제 본문을 진실의 출처로 본다 —
///   어느 프라이밍이든 CLI 발신을 지시하면 basename 과 무관하게 잡힌다.
///   ★의도적으로 보수적(부정문 false positive 는 수용)★: "engram-send 를 쓰지 마라" 같은 부정문도 substring
///   존재만으로 true → 헛된 SETUP-SKIP 이 될 수 있다. 그러나 SETUP-SKIP 은 요란한 exit-1 로, 틀릴 수 있는
///   데이터 발화를 거부하는 안전한 방향이다(실 프라이밍에 그런 부정문은 없다). 부정 파싱은 넣지 않는다 —
///   substring 존재 ⇒ CLI-지시로 취급, 헛된 skip 이 안전한 쪽.
fn priming_text_directs_cli(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("engram-send") || lower.contains("engram_send_exe")
}

/// ★--cli-only 가 상속된 ENGRAM_PRIMING_FILE override 와 충돌하는가(순수·단위테스트 대상, ADR-0099)★:
///   cli-only 모드는 provision 이 CliOnly 파일을 스스로 고르는 걸 관측하는 게 목적이라, 부모 env 에 미리
///   깔린 비어 있지 않은 override 는 그 auto-select 를 덮어써 관측을 무의미하게 만든다 → 충돌(true)로 본다.
///   `--priming` co-pass 거부와 대칭인 순수 판정자다. cli_only=false 면 env 값과 무관하게 충돌 아님(false)
///   — 운영/일반 모드는 override 를 정당히 쓴다. env 값이 비어 있으면(미설정 취급) 충돌 아님.
fn cli_only_env_override_conflicts(cli_only: bool, env_value: Option<&std::ffi::OsStr>) -> bool {
    cli_only && matches!(env_value, Some(v) if !v.is_empty())
}

/// ★--cli-only 성공 판정(순수·단위테스트 대상, ADR-0099)★: cli-only 모드에서 이 실측이 **성공(pass)** 인가.
///   이 모드는 provision 을 비-MCP 로 강제해 false path 전체가 정합하게 도는지를 실측하는 게 목적이므로,
///   B 가 실제로 발신했고(b_sent) 그 입구가 반드시 `cli` 여야만 성공이다 — 아무것도 안 보낸(b_sent=false,
///   entrance="none") 경우는 이 모드에선 **실패**로 본다(일반 모드의 valid-negative 와 다르다: 강제 false
///   path 가 도는 걸 못 봤으니 실측 목적 미달). entrance="mcp"(강제 seam 이 MCP 를 못 지움)는 앞선
///   SETUP-FAIL 이 이미 잡지만, 순수 판정자 수준에서도 cli 아닌 건 전부 실패로 매핑해 이중 안전망을 둔다.
fn cli_only_run_passed(b_sent: bool, entrance_label: &str) -> bool {
    b_sent && entrance_label == "cli"
}

/// ★REPLY_POLL 라벨 판정(순수·단위테스트 대상)★: `--seed-request` 결과 블록의 `REPLY_POLL=` 값을
///   결정하는 우선순위 판정자 — matched > timeout > skipped-no-budget. `matched` 는 `wait_for_matching_reply`
///   가 실제로 돌아 잡았든(poll_ran=true) 폴링이 예산 부족으로 스킵됐지만 이미 그 전에 배달돼 있었든
///   (poll_ran=false) 최종 스캔(`records_after`)이 seed 와 일치하는 회신을 확인하기만 하면 최우선이다 —
///   poll_ran 값과 무관하게 이긴다. matched 가 아닐 때만 poll_ran 을 본다: poll_ran=true(폴링이 예산을
///   받아 실제로 돌았지만 예산 소진까지 못 찾음) = timeout, poll_ran=false(잔여 REPLY_WAIT_CAP 예산이
///   씨앗 주입 이후 A 턴 대기까지 소비돼 0/음수라 폴링 자체가 안 돎) = skipped-no-budget. 이전엔 이 로직이
///   호출부(print 시점)에 인라인 if/else 로만 있어 단위테스트가 없었다 — 여기로 뽑아 세 갈래를 각각 검증한다.
fn reply_poll_label(matched: bool, poll_ran: bool) -> &'static str {
    if matched {
        "matched"
    } else if poll_ran {
        "timeout"
    } else {
        "skipped-no-budget"
    }
}

/// --seed-reply-by 가 --seed-request 없이 단독 지정됐는가(순수·단위테스트 대상): true 면 인자 오류다.
///   reply_by 는 request 계약의 회신 기한이라 request 자체가 없으면 추적할 계약이 없다 - 조용히 무시하지
///   않고 반려한다(ingress.rs validate_contract 의 "reply_by 는 request 전용" 규칙과 같은 정신을 CLI 인자
///   레벨에서 fail-fast 로 앞당긴다 - 실 에이전트를 스폰하기 전에 걸러야 헛된 스폰을 막는다).
fn seed_reply_by_without_request_is_invalid(
    seed_request: bool,
    seed_reply_by: &Option<String>,
) -> bool {
    !seed_request && seed_reply_by.is_some()
}

/// --b-task 값이 파일 참조(`@path`)인가(순수·단위테스트 대상) - 그렇다면 repo 루트 기준으로 절대화한
///   경로를 돌려준다(절대 경로면 그대로, 상대면 join - `--priming` 명시 경로 override 와 동일 규약).
///   `@` 접두가 아니면 None(호출자는 값을 인라인 텍스트 그대로 쓴다). 존재 검사는 하지 않는다(호출자가
///   읽기 시도로 판정 - `resolve_priming_path` 와 같은 분업).
fn resolve_b_task_file_path(value: &str, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel = value.strip_prefix('@')?;
    let p = PathBuf::from(rel);
    Some(if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    })
}

/// CLI 인자 파싱 결과(순수) — priming 셀렉터 + 모델. `run` 이 이걸로 env·스폰을 배선한다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    /// `--priming` 값(프라이밍 파일 경로 — 절대/상대, 또는 편의 별칭 `C0`). 미지정이면 None(= 기본 both
    ///   프라이밍 `prompts/agent-priming.md`, `C0` 별칭과 동일). C1~C3 는 ADR-0099 로 별칭이 제거돼 이제 그냥
    ///   파일 경로로 해석된다(특수 매핑 없음 — 부재로 걸림).
    priming: Option<String>,
    /// `--model` 값(기본 sonnet).
    model: String,
    /// `--disallow-mcp` 플래그(ADR-0094 CLI-only 측정): 켜지면 `ENGRAM_DISALLOW_MCP_SEND` env 를 세워
    ///   두 에이전트가 MCP send_message grant **없이** 스폰 → engram-send CLI 로만 발신하게 강제한다.
    ///   test-only 측정 노브(운영 스위치 아님). 미지정이면 오늘 동작(MCP grant 포함).
    disallow_mcp: bool,
    /// `--cli-only` 플래그(ADR-0099 FIX 3): 켜지면 `ENGRAM_FORCE_CLI_ONLY_SEND` env 를 세워 provision 이
    ///   실 claude 스폰을 **비-MCP 백엔드로 강제**한다 → false path 전체(no mcp-config + CliOnly 프라이밍 +
    ///   [Cli] grant)가 돈다. ★`--disallow-mcp` 와 다른 점★: 후자는 MCP grant 만 빼고 MCP 서버는 여전히
    ///   부착·both-teaching 프라이밍이라 물리/교육 채널이 갈린다(측정용 불일치). `--cli-only` 는 provision
    ///   자체를 CLI-only 로 정렬해 정합 불변식을 보존한 채 false path 를 실측한다. ★이 모드는 `--priming`
    ///   override 를 주지 않아야 한다★ — provision 이 자동으로 `prompts/agent-priming-cli.md` 를 고르는 걸
    ///   보는 게 목적이다(entrance=cli 기대). test-only 노브(운영 스위치 아님).
    cli_only: bool,
    /// `--seed-request` 플래그: 켜지면 씨앗 A->B 를 회신 계약(SendContract{request:true,..})으로 보낸다
    ///   — 같은 실 control 경로(handle_send, Entrance::Cli) 그대로, 계약 축만 얹는다. 미지정이면 오늘
    ///   동작(plain 통보, SendContract::default(), 바이트 동일).
    seed_request: bool,
    /// `--seed-reply-by <dur>` 값(기간 표기 - "5m"/"10m"/"1h", validate_contract 가 최종 검증). 반드시
    ///   --seed-request 와 함께 써야 한다(단독 지정은 인자 오류) - reply_by 는 request 계약의 기한이라
    ///   추적할 계약이 없으면 조용한 무시가 된다(ingress.rs validate_contract 규칙 1 과 같은 정신).
    seed_reply_by: Option<String>,
    /// ★값 자체의 오용(F5 와 같은 규율)★ `--seed-reply-by` 가 값 누락 / 다음 토큰이 `--` 로 시작(플래그로
    ///   오인 가능해 `take_flag_value` 라면 조용히 소비 안 했을 경우) / 빈 값·공백만인 값으로 잘못 쓰였는가.
    ///   Some(reason) 이면 `run()` 이 실 claude 를 스폰하기 **전에** SETUP-FAIL 로 fail-fast 한다(`--b-task`
    ///   오용 가드와 같은 분업 — parse_args 는 사실만 순수하게 기록하고, 반려 여부·시점은 호출자가 정한다).
    seed_reply_by_error: Option<String>,
    /// `--b-task <text|@path>` 값 - 지정 시 B 원과제 프롬프트(TASK_PROMPT_B)를 대체한다. `@` 접두면
    ///   파일 참조(절대/상대 - 상대는 repo 루트 기준, --priming 상대경로와 동일 규약)로 해석해 그 내용을
    ///   읽는다. 미지정이면 기본 auth 모듈 과제(오늘 동작, 바이트 동일).
    b_task: Option<String>,
    /// ★F5 오용 가드★ `--b-task` 가 잘못 쓰였는가 — 값 누락 / 다음 토큰이 `--` 로 시작(플래그로 오인돼
    ///   `take_flag_value` 라면 조용히 값 없음으로 넘겼을 경우) / 빈 값·공백만인 값. Some(reason) 이면
    ///   `run()` 이 실 claude 를 스폰하기 **전에** SETUP-FAIL 로 fail-fast 한다(`--seed-reply-by` 단독
    ///   지정 검사와 같은 분업 — parse_args 는 사실만 순수하게 기록하고, 반려 여부·시점은 호출자가 정한다).
    ///   텍스트 값을 막는 가드가 아니다 — 유효한 인라인 텍스트는 그대로 통과(사용자 결정, 파일로 좁히지
    ///   않는다).
    b_task_error: Option<String>,
}

/// 배달 관측 싱크(ADR-0088) — relay 레코드를 스레드 안전 Vec 에 모은다. 하네스가 registry 에 설치하고
///   나중에 from=B·to=A 레코드를 조회한다. registry 는 read accessor 를 노출하지 않으므로(write-only
///   observer 슬롯) 이 싱크가 회수 경로다.
struct CapturingObserver {
    records: Mutex<Vec<DeliveryObservation>>,
}

impl CapturingObserver {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
    /// 지금까지 관측된 레코드 총수(도착 순서 = Vec push 순서). 씨앗 주입 **직전**에 이 값을 baseline 으로
    ///   잡아, 그 이후에 도착한 레코드만 B 의 답신 후보로 본다(FIX round-2 #1 — pre-seed 오탐 차단).
    fn record_count(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// baseline **이후**에 도착한 레코드 중 from=`from_id`·to=`to_id` 인 첫 배달 스냅샷(있으면).
    ///   ★왜 baseline 절단인가(FIX round-2 #1)★: observer 는 B 의 원과제 턴 **전에** 설치된다. 만약 B 가
    ///   task-establishing 턴에서 A 에게 메시지를 하나 흘리면 그 pre-seed 레코드가 "답신" 으로 오인돼
    ///   거짓 B_SENT=true 를 내 실험을 오염시킨다. 그래서 씨앗 주입 직전 record_count 를 baseline 으로 잡고
    ///   `records[baseline..]` 만 훑어 씨앗에 인과적으로 뒤따르는 outbound 만 답신으로 본다.
    fn find_delivery_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Option<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)?
            .iter()
            .find(|r| r.from.peer_id == from_id && r.to_id == to_id)
            .cloned()
    }

    /// ★F3★ baseline **이후** 도착한 from=`from_id`·to=`to_id` 레코드 **전부**(순서 보존) — `find_delivery_after`
    ///   와 달리 첫 매치에서 멈추지 않는다. b_sent/entrance 판정("B 가 뭐라도 보냈나")은 여전히 첫 도착
    ///   (`find_delivery_after`)으로 충분하지만, 회신 **내용** 검증(REPLY_IN_REPLY_TO/REPLY_MATCHES_SEED)은
    ///   그래선 안 된다 — 첫 레코드가 회신과 무관한 "ack" 한 통이고 진짜 회신이 그 뒤에 오면 first-wins 는
    ///   거짓 negative(REPLY_MATCHES_SEED=false)를 낸다. ★이 메서드 자체는 호출 시점까지 쌓인 레코드의
    ///   스냅샷 스캔일 뿐이다★ — 레이스를 실제로 닫는 건 `run()` 이 A 턴 대기 직후 호출하는
    ///   `wait_for_matching_reply` 다. ★그런데 그 호출 자체가 조건부다(정정 — 이전 판본은 여기서 "잔여
    ///   REPLY_WAIT_CAP 예산 = wait_for_reply 가 쓰고 남긴 것" 이라 잘못 말했다)★: REPLY_WAIT_CAP 벽시계는
    ///   **씨앗 주입 시점부터** 재고, 그 사이엔 `wait_for_reply`(B 첫 답신 대기)뿐 아니라 그 뒤 이어지는
    ///   A 턴 대기(`wait_turn_end`, 최대 TURN_WAIT_CAP=180s)까지 **함께** 그 벽시계를 소비한다 — A 턴
    ///   대기가 길면 잔여가 0/음수로 떨어져 `wait_for_matching_reply` 가 **아예 호출되지 않을 수 있다**
    ///   (폴링 자체가 안 돎). 그래서 이 메서드가 print 시점에 뽑아내는 최종 값은 세 갈래로 갈린다: 폴링이
    ///   돌아 매치를 찾음(matched) / 폴링이 돌았으나 예산 소진까지 못 찾음(timeout) / 잔여 예산이 없어
    ///   폴링 자체가 안 돎(skipped-no-budget) — 우선순위 판정은 `reply_poll_label` 참조.
    fn records_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Vec<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)
            .map(|slice| {
                slice
                    .iter()
                    .filter(|r| r.from.peer_id == from_id && r.to_id == to_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl DeliveryObserver for CapturingObserver {
    fn observe(&self, obs: DeliveryObservation) {
        self.records.lock().unwrap().push(obs);
    }
}

async fn run() -> i32 {
    let args = parse_args(std::env::args().skip(1));
    // ★F5-style 오용 가드(신규)★: --seed-reply-by 값 자체(문법)가 잘못 쓰였으면 — "--seed-request 없이
    //   단독 지정" 같은 의미 검사보다 먼저 걸러야 한다(값이 애초에 안 먹혔다면 그 의미를 논해도 무의미) —
    //   --b-task 오용 가드와 같은 분업으로 스폰 전에 fail-fast(인자 오류, exit 1).
    if let Some(reason) = &args.seed_reply_by_error {
        return setup_fail(reason);
    }
    // --seed-reply-by 는 --seed-request 없이 의미가 없다 - 다른 setup(priming 해석·MCP 서버 기동 등)보다
    //   먼저, 실 claude 를 스폰하기 전에 걸러 헛된 스폰을 막는다(인자 오류, exit 1).
    if seed_reply_by_without_request_is_invalid(args.seed_request, &args.seed_reply_by) {
        return setup_fail(
            "--seed-reply-by requires --seed-request (reply_by is only meaningful as the deadline of a request contract) — add --seed-request or drop --seed-reply-by",
        );
    }
    // ★F5 오용 가드★ --b-task 가 값 누락/플래그처럼 보이는 값/빈·공백 값으로 잘못 쓰였으면 다른 setup 보다
    //   먼저, 실 claude 를 스폰하기 전에 걸러 헛된 스폰을 막는다(인자 오류, exit 1 — reply_by 단독 지정
    //   검사와 같은 분업: parse_args 는 사실만 기록, 반려는 여기서).
    if let Some(reason) = &args.b_task_error {
        return setup_fail(reason);
    }

    let repo_root = repo_root_from_manifest();
    let priming_selector = args.priming.clone();
    // ★--cli-only 는 priming override 를 주지 않아야 한다(ADR-0099 FIX 3)★: 이 모드의 목적은 provision 이
    //   자동으로 `prompts/agent-priming-cli.md`(CliOnly 변형)를 고르는 걸 보는 것이다. 그래서 여기서는
    //   ENGRAM_PRIMING_FILE override 를 세우지 않고, 보고·CLI-요구 판정용으로 그 CLI-only 운영 파일을
    //   effective priming 으로 해석만 한다. `--priming` 을 함께 주면 목적(auto-select 관측)과 충돌하므로
    //   fail-fast 한다(오해 방지).
    if args.cli_only && priming_selector.is_some() {
        return setup_fail(
            "--cli-only 는 --priming override 와 함께 쓸 수 없다 — 이 모드는 provision 이 자동으로 prompts/agent-priming-cli.md 를 고르는 걸 관측하는 게 목적이다(override 를 주면 그 관측이 무의미)",
        );
    }
    // ★--cli-only 는 **상속된** ENGRAM_PRIMING_FILE 도 거부한다(ADR-0099)★: 부모 env 에 이 override 가 미리
    //   깔려 있으면 provider(priming.rs)가 그걸 최우선으로 읽어 provision 의 CliOnly auto-select 를 조용히
    //   덮어쓴다 — `--priming` co-pass 거부와 같은 구멍이 env 로 들어온다. **조용히 clear 하지 않는다**
    //   (operator 가 일부러 세운 값일 수 있어 지우면 숨은 의도 파괴) — 어느 값이든(비어 있지 않으면) 그 이름을
    //   박아 SETUP-FAIL 로 요란히 거부하고 operator 가 직접 걷어내게 한다. co-pass 거부와 대칭이다.
    if cli_only_env_override_conflicts(
        args.cli_only,
        std::env::var_os("ENGRAM_PRIMING_FILE").as_deref(),
    ) {
        return setup_fail(
            "--cli-only 인데 부모 env 에 ENGRAM_PRIMING_FILE 이 설정돼 있다 — 이 override 가 provision 의 CliOnly auto-select 를 덮어써 관측을 무의미하게 만든다. 조용히 지우지 않으니(숨은 의도 파괴 방지) 실행 전에 직접 unset 하라",
        );
    }
    // effective priming 경로: cli-only 면 CliOnly 운영 파일(provision 이 auto-select 할 그 파일), 아니면
    //   셀렉터 해석 결과. 두 경우 모두 repo 루트 기준 절대화.
    let priming_selector_for_resolve = if args.cli_only {
        Some("prompts/agent-priming-cli.md")
    } else {
        priming_selector.as_deref()
    };
    let resolved_priming = match resolve_priming_path(priming_selector_for_resolve, &repo_root) {
        Some(p) => p,
        None => {
            // 절대화조차 못 함(비정상 셀렉터) — 프라이밍은 실험 필수라 fail-fast.
            return setup_fail(&format!(
                "priming 셀렉터({priming_selector:?})를 절대경로로 못 풂 — 실험 불가"
            ));
        }
    };
    // ★존재 검사 fail-fast(FIX round-2 #5)★: `FilePrimingProvider` 는 존재하지 않는 override 를 조용히
    //   버리고 UNPRIMED 로 스폰한다. 그러면 라벨은 "priming X 로 primed" 라 주장하지만 실제론 unprimed —
    //   케이스가 거짓말한다. 프라이밍은 이 실험의 본질이므로, 실제로 in-effect 가 아닌 경로는 절대 진행·
    //   출력하지 않는다. 여기서 확인해 없으면 SETUP-FAIL.
    if !resolved_priming.is_file() {
        return setup_fail(&format!(
            "priming 파일 없음: {} (case={:?}) — 존재하지 않는 override 는 UNPRIMED 스폰으로 이어져 케이스 라벨을 거짓으로 만든다",
            resolved_priming.display(),
            priming_selector
        ));
    }
    // ★프라이밍 본문 단일 읽기 + fail-closed(FIX)★: 여기서 딱 한 번 읽어 아래 CLI-요구 가드가 재사용한다.
    //   이전 판본은 존재 검사(위)와 가드에서 파일을 두 번 만졌고(TOCTOU 창), 가드 쪽은
    //   `read_to_string(...).unwrap_or(false)` 라 읽기 실패(공유 위반·권한·검사 후 삭제/교체·비-UTF-8)를
    //   전부 "CLI 요구 아님" 으로 삼켜 헛된 정상 negative 를 낼 수 있었다. 프라이밍 파일은 실험의 본질이므로
    //   읽을 수 없으면 조용히 진행하지 않고 SETUP-FAIL(exit 1). is_file 통과 후 여기서 즉시 읽어 그 창을 좁힌다.
    let priming_content = match std::fs::read_to_string(&resolved_priming) {
        Ok(c) => c,
        Err(e) => {
            return setup_fail(&format!(
                "priming 파일 읽기 실패: {} (case={:?}): {e} — 프라이밍은 실험 필수라 읽을 수 없으면 진행 불가",
                resolved_priming.display(),
                priming_selector
            ));
        }
    };
    // --b-task 해석(지정 시에만): `@path` 는 파일 참조, 그 외는 인라인 텍스트 그대로. 파일 부재/읽기
    //   실패는 SETUP-FAIL(인프라 부재를 실험적 negative 로 오인하지 않는다 - priming 파일 읽기와 같은
    //   분업). 실 claude 를 아직 하나도 스폰하지 않은 시점이라 정리(cleanup)할 리소스가 없다.
    // ★F5★ `b_task_kind` 는 결과 블록의 `B_TASK=` 자기서술 줄(항상 찍힘) 재료 — default/file:<path>/
    //   inline(<n> bytes) 셋 중 하나. `<n>` 은 CLI 값의 바이트 길이(String::len — char 수 아님).
    let (task_prompt_b, b_task_kind): (String, String) = match &args.b_task {
        None => (TASK_PROMPT_B.to_string(), "default".to_string()),
        Some(v) => match resolve_b_task_file_path(v, &repo_root) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let kind = format!("file:{}", path.display());
                    (content, kind)
                }
                Err(e) => {
                    return setup_fail(&format!(
                        "--b-task 파일 읽기 실패: {} : {e} — B 원과제 프롬프트를 못 구했으니 진행 불가",
                        path.display()
                    ));
                }
            },
            None => {
                let kind = format!("inline({} bytes)", v.len());
                (v.clone(), kind)
            }
        },
    };
    // ★env 로 넘겨 FilePrimingProvider 생성 전에 set★: provision 마다 priming_file() 이 이 env 를
    //   최우선 override 로 읽어 두 에이전트(A·B) 모두 같은 변형을 받는다.
    //   ★--cli-only 예외(ADR-0099 FIX 3)★: 이 모드는 override 를 **세우지 않는다** — provision 이 강제된
    //     비-MCP 분기에서 CliOnly 변형(prompts/agent-priming-cli.md)을 스스로 고르는 걸 관측하는 게 목적이다.
    //     override 를 세우면 그 auto-select 를 우회하므로 일부러 뺀다.
    if !args.cli_only {
        std::env::set_var("ENGRAM_PRIMING_FILE", &resolved_priming);
    }
    eprintln!(
        "[roundtrip] priming = {} (case={:?}, cli_only={})",
        resolved_priming.display(),
        priming_selector,
        args.cli_only
    );
    // ★ADR-0094 CLI-only 측정 seam★: `--disallow-mcp` 가 켜지면 provision 전에 env 를 세워, 두 에이전트가
    //   MCP send_message grant **없이** 스폰돼 engram-send CLI 로만 발신하게 강제한다. build_grants 가 이
    //   env 를 읽는다(control/mod.rs). 프라이밍 env 와 같은 지점(provider·manager 배선 전)에 세워야 두
    //   에이전트 모두 같은 grant 셋으로 provision 된다. (CLI 입구 활성 = send_exe 존재는 아래에서 가드.)
    if args.disallow_mcp {
        std::env::set_var("ENGRAM_DISALLOW_MCP_SEND", "1");
        eprintln!("[roundtrip] --disallow-mcp → MCP send grant 제거(CLI-only 측정, ENGRAM_DISALLOW_MCP_SEND=1)");
    }
    // ★ADR-0099 FIX 3 CLI-only 강제 seam★: `--cli-only` 가 켜지면 provision 전에 env 를 세워, provision 이
    //   실 claude 스폰을 **비-MCP 로 강제**한다 → false path 전체(no mcp-config + CliOnly 프라이밍 + [Cli]
    //   grant)가 돈다. control/mod.rs::provision 이 이 env 를 분기 맨 위에서 읽어 effective flag 를 false 로
    //   덮는다. --disallow-mcp 와 달리 물리/교육 채널이 정합(둘 다 CLI)이라 실 claude 를 비-MCP 백엔드처럼
    //   굴려 false 분기를 실측한다(CLI 입구 활성 = send_exe 필수 — 아래에서 가드).
    if args.cli_only {
        std::env::set_var("ENGRAM_FORCE_CLI_ONLY_SEND", "1");
        eprintln!("[roundtrip] --cli-only → provision 을 비-MCP 로 강제(false path 전체, ENGRAM_FORCE_CLI_ONLY_SEND=1); entrance=cli 기대");
    }

    // 배선(priming_smoke 미러) — 실 FilePrimingProvider·MCP 서버·AgentManager.
    let registry = Arc::new(ControlRegistry::new());
    // ADR-0088: 배달 관측 싱크 설치 — B→A outbound relay 를 회수한다(로그 스크레이핑 금지).
    let observer = Arc::new(CapturingObserver::new());
    registry.set_delivery_observer(observer.clone());

    let slot = Arc::new(ManagerSlot::new());
    // C1: MessagingService 늦은 주입 슬롯 — send/flush 담당(manager 조립 후 채운다).
    let messaging_slot = Arc::new(MessagingSlot::new());
    let handle =
        match start_mcp_server(registry.clone(), slot.clone(), messaging_slot.clone()).await {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[roundtrip] MCP 서버 기동 실패: {e}");
                return 1;
            }
        };
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-roundtrip-{}", AgentId::new_v4()));
    let ws_a = std::env::temp_dir().join(format!("engram-roundtrip-ws-a-{}", AgentId::new_v4()));
    let ws_b = std::env::temp_dir().join(format!("engram-roundtrip-ws-b-{}", AgentId::new_v4()));
    let _ = std::fs::create_dir_all(&ws_a);
    let _ = std::fs::create_dir_all(&ws_b);

    // ★send_exe 배선(CLI 입구 활성화 — CLI-지시 프라이밍/`--cli-only`/`--disallow-mcp` 에 필수)★: engram-send 는 데몬 exe 형제로 배포된다. 이
    //   하네스는 cargo 가 만든 target 디렉토리(현재 exe 형제)에서 engram-send 를 찾아 endpoint 에 싣는다.
    //   못 찾으면 None(CLI 입구 비활성 — MCP 만).
    let send_exe = sibling_send_exe();
    match &send_exe {
        Some(p) => eprintln!("[roundtrip] engram-send = {}", p.display()),
        None => eprintln!("[roundtrip] engram-send 형제 바이너리 없음 — CLI 입구 비활성(MCP 만)."),
    }
    // ★CLI 요구 프라이밍인데 engram-send 부재 = SETUP-SKIP(ADR-0094)★: CLI 발신을 지시하는 프라이밍은
    //   B 가 CLI 입구로 보내도록 지시한다. send_exe 가 None 이면 B 는 그 경로로 물리적으로 못 보내므로,
    //   결과 B_SENT=false 는 "B 가 안 보내기로 함"(정상 negative)이 아니라 인프라 부재다. 판정은 셀렉터·
    //   basename 이 아니라 **해석된 프라이밍 파일 본문**으로 한다 — 명시 경로 override 도, basename 리스트에서
    //   누락되던 새 CLI-지시 프라이밍도 잡힌다. 위에서 단 한 번 읽어 둔 `priming_content` 를 순수 판정자
    //   `priming_text_directs_cli` 에 넘긴다(재읽기·TOCTOU 없음). 실 claude 2개를 스폰하기 **전에** 요란히
    //   SETUP-SKIP 하고 종료한다 — 헛된 스폰·오귀속 둘 다 막는다.
    if priming_text_directs_cli(&priming_content) && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(&format!(
            "engram-send not built, CLI inlet unavailable (case={:?} requires the CLI send path). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
            priming_selector
        ));
    }
    // ★--disallow-mcp 는 CLI 입구가 반드시 살아 있어야 한다(ADR-0094)★: MCP send grant 를 빼는데 CLI grant
    //   마저 없으면(send_exe=None) 두 에이전트는 발신 경로가 **하나도** 없어, B_SENT=false 는 정상 negative
    //   가 아니라 인프라 부재다. 위 CLI-요구 프라이밍 스킵과 같은 이유로 스폰 **전에** 요란히 SETUP-SKIP.
    if args.disallow_mcp && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--disallow-mcp requires the CLI inlet (engram-send) but it is not built — MCP grant removed AND no CLI grant means agents have no send path. 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
        );
    }
    // ★--cli-only 는 CLI 입구가 반드시 살아 있어야 한다(ADR-0099 FIX 3)★: 이 모드는 provision 을 비-MCP 로
    //   강제하므로 MCP 입구가 물리적으로 없다 — send_exe 마저 없으면 provision 이 fail-closed edge(Err)로
    //   스폰을 막는다(control/mod.rs). 그걸 SETUP-FAIL 로 늦게 만나기 전에 스폰 **전에** 요란히 SETUP-SKIP.
    if args.cli_only && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--cli-only requires the CLI inlet (engram-send) but it is not built — forced non-MCP spawn has no MCP inlet, and no CLI grant means agents have no send path (provision would fail-closed). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
        );
    }

    let priming_provider: Arc<dyn PrimingProvider> = Arc::new(FilePrimingProvider::new(repo_root));
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        data_dir.clone(),
        send_exe,
        priming_provider,
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let profile_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-prof-{}", AgentId::new_v4()));
    let preset_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-preset-{}", AgentId::new_v4()));
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        profile_dir.clone(),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        preset_dir.clone(),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());
    // C1: MessagingService 조립(발송 3분기·flush) — manager 를 DeliveryPort 로 감싼다. 이 하네스는
    //   flush sink 를 배선하지 않으므로(NoopStatus) 파킹 시나리오는 handle_single_send 직접 경로만 탄다.
    //   씨앗 A→B 는 산 수신자라 delivered 경로.
    let messaging = Arc::new(messaging_for_manager(manager.clone(), registry.clone()));
    messaging_slot.set(messaging.clone());

    // ── A·B 스폰(둘 다 실 primed claude, stream-json, Fresh) ─────────────────────────
    // A 는 이름 alice(B 가 봉투에서 배워 to=alice 로 답신), B 는 bob.
    let agent_a = match spawn_named(&manager, NAME_A, &args.model, &ws_a) {
        Some(a) => a,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("A 스폰/등장 실패");
        }
    };
    let agent_b = match spawn_named(&manager, NAME_B, &args.model, &ws_b) {
        Some(b) => b,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[agent_a.id], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("B 스폰/등장 실패");
        }
    };
    eprintln!(
        "[roundtrip] spawned A(alice)={} B(bob)={} model={}",
        agent_a.id, agent_b.id, args.model
    );

    // A·B 각각에 출력 관측 sink 부착.
    let obs_a = Arc::new(TurnObserver::new());
    let obs_b = Arc::new(TurnObserver::new());
    let sink_a = manager.subscribe(agent_a.id, obs_a.clone()).ok();
    let sink_b = manager.subscribe(agent_b.id, obs_b.clone()).ok();

    // ★setup-failure 시 공통 정리(FIX round-2 #4)★: 아래 setup 단계에서 hard-fail 하면 이 클로저로 구독
    //   해제·kill·디렉토리 정리·MCP 종료를 하고 SETUP-FAIL 을 낸다(valid negative 와 구분).
    macro_rules! fail_setup {
        ($reason:expr) => {{
            if let Some(id) = sink_a {
                let _ = manager.unsubscribe(agent_a.id, id);
            }
            if let Some(id) = sink_b {
                let _ = manager.unsubscribe(agent_b.id, id);
            }
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
            handle.shutdown().await;
            return setup_fail($reason);
        }};
    }

    // ★A 구독 실패 = SETUP-FAIL(FIX round-2 #4)★: A 의 `TurnObserver` 를 못 붙이면(sink_a=None) A 가
    //   답신을 처리하며 낸 텍스트(정성 관측)를 아예 볼 수 없다 — 그 상태의 정성 결과는 무의미하므로 valid
    //   negative 로 보고하면 안 된다. B 구독 실패도 같은 이유(B 턴 관측 불가 → 원과제 setup 판정 불가).
    if sink_a.is_none() {
        fail_setup!("A 출력 구독 실패(sink_a=None) — A 턴 관측 불가, 정성 결과 무의미(setup 실패)");
    }
    if sink_b.is_none() {
        fail_setup!("B 출력 구독 실패(sink_b=None) — B 턴 관측 불가, setup 판정 불가(setup 실패)");
    }

    // ── 1) B 원과제 턴(일하는 팀원 맥락) ────────────────────────────────────────────
    // ★turn 실패 = setup 실패(FIX round-2 #4)★: 이전엔 warn 후 계속했다 — 그러면 "일하는 팀원 맥락" 이
    //   서지 않은 채 B_SENT=false 를 정상 negative 로 보고해 setup 실패를 실험 결과로 오인한다. B 가 원과제를
    //   수용(턴 종료)하지 못하거나 그 사이 죽으면 valid negative 가 아니라 SETUP-FAIL 이다.
    if !send_and_wait(&manager, agent_b.id, &obs_b, &task_prompt_b) {
        if !is_agent_alive(&manager, agent_b.id) {
            fail_setup!("B 가 원과제 턴 도중 종료됨(process death) — 팀원 맥락 setup 실패");
        }
        fail_setup!(
            "B 원과제 턴이 cap 내 종료 신호 없음 — 팀원 맥락 setup 실패(valid negative 아님)"
        );
    }
    eprintln!(
        "[roundtrip] --- B task turn ---\n{}\n--- end ---",
        obs_b.response_text().trim()
    );
    // 씨앗 주입 직전 B 생존 재확인 — task 턴은 끝났지만 그 뒤 죽었을 수 있다.
    if !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B 가 씨앗 주입 전 종료됨 — setup 실패");
    }

    // ── 2) 씨앗 A→B(실 control 경로, from = A 의 실 발급 신원) ────────────────────────
    // ★from = 토큰 파생(ADR-0086)★: A 는 Fresh 스폰이라 epoch 0 — provision 이 그 (id,0)에 토큰을 이미
    //   발급했다(registry 에 산 신원). 본문 문자열이 아니라 이 BoundIdentity 가 발신자다.
    let from_a = BoundIdentity {
        agent_id: agent_a.id,
        epoch: 0,
    };
    // A 의 답신 관측 baseline 을 씨앗 주입 **전에** 잡는다(B 답신이 A 턴을 밀어 올리는 걸 본다).
    obs_a.begin_turn();
    let baseline_a = obs_a.done_snapshot();
    // ★B→A relay baseline(FIX round-2 #1)★: 씨앗 주입 **직전**에 관측 레코드 수를 잡는다. B 가 원과제 턴에서
    //   A 에게 흘린 pre-seed 레코드가 답신으로 오인되는 걸 막는다 — 이후 도착분만 답신 후보.
    let reply_baseline = observer.record_count();
    // ★진단(탐색)★: B 가 씨앗을 받고 자기 턴에 응답은 하는데 send 로 라우팅만 안 하는지 보려고 B 의 씨앗-후
    //   턴 텍스트를 캡처한다. begin_turn 은 text 만 비우고 done_count 는 누적이라, reply 대기 동안 B 턴이
    //   끝나면 response_text 에 씨앗-후 출력이 담긴다.
    obs_b.begin_turn();
    let baseline_b = obs_b.done_snapshot();

    // --seed-request(지정 시): 씨앗을 회신 계약(SendContract{request:true,..})으로 보낸다 - 같은 실
    //   control 경로(handle_send, Entrance::Cli) 그대로, 계약 축만 얹는다. 미지정이면 오늘 동작
    //   (SendContract::default() = plain 통보, 바이트 동일).
    let seed_contract = if args.seed_request {
        SendContract {
            request: true,
            reply_by: args.seed_reply_by.clone(),
            reply_to: None,
        }
    } else {
        SendContract::default()
    };
    let seed = ControlCommand {
        from: from_a,
        to: NAME_B.to_string(), // 이름으로 지목(alice→bob).
        body: SEED_A_TO_B.to_string(),
        contract: seed_contract,
    };
    let ack = handle_send(&manager, &registry, &messaging, Entrance::Cli, seed);
    eprintln!("[roundtrip] seed A→B ACK = {}", ack.to_json());
    // ★씨앗 ACK 에러 = setup 실패(FIX round-2 #4)★: ACK 가 error(수신자 미해석·write 실패 등)면 B 는 애초에
    //   씨앗을 못 받았다 — 그 뒤 B_SENT=false 는 "B 가 답 안 함" 이 아니라 씨앗 배달 실패다. B 는 산 수신자라
    //   접수(delivered) 되어야 한다(파킹이 아님 — is_accepted 로 반려만 거른다).
    if !ack.is_accepted() {
        fail_setup!(&format!(
            "씨앗 A→B ACK 가 접수 실패(반려): {}",
            ack.to_json()
        ));
    }
    // 씨앗 논리 메시지 id(ADR-0088 확장 - --seed-request 판정용): ACK JSON 의 `id` 가 곧 이 씨앗의
    //   msg_id 다(handle_send 성공 응답 shape, spec §6). B 의 답신이 이 id 로 회신했는지
    //   (REPLY_MATCHES_SEED) 비교할 기준값을 여기서 한 번 뽑아 둔다.
    let seed_msg_id: Option<String> = ack
        .to_json()
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // ★ACK shape-drift 가드(신규)★: 현재 `ControlResult::Ok { id: String, .. }` shape(ingress.rs) 상
    //   접수된(accepted) ACK 는 `id` 를 항상 싣는다 — 위 파싱이 None 을 내는 건 오늘은 불가능하다. 그런데
    //   이 가드가 없으면, 그 shape 이 미래에 바뀌어(예: id 가 optional 로) 조용히 None 이 나올 때 --seed-request
    //   경로가 그걸 알아채지 못하고 REPLY_POLL=skipped-no-budget(예산 고갈)로 **오분류**한다 — 실제 원인은
    //   예산 부족이 아니라 판정 기준값(seed_msg_id) 자체가 없어 애초에 회신 일치를 판정할 수 없는 것인데,
    //   같은 라벨 뒤에 숨어 버린다. --seed-request 일 때만 유의미하므로(plain 씨앗은 이 id 를 안 쓴다) 그
    //   경우로 한정해 스폰 이후 첫 발견 즉시 SETUP-FAIL 로 fail-fast 한다(조용히 진행해 오분류를 내지 않는다).
    if args.seed_request && seed_msg_id.is_none() {
        fail_setup!(&format!(
            "seed ACK missing id — request judgment impossible (ACK={})",
            ack.to_json()
        ));
    }

    // ── 3) B 의 답신을 **B 자신의 발신 경로**로 대기(하네스는 handle_send 를 부르지 않는다) ──────
    //    B(실 claude)가 MCP send_message 또는 engram-send CLI 를 스스로 호출 → 실 입구 → handle_send →
    //    wrap → A stdin. 그 relay 가 관측 싱크에 baseline 이후 from=B·to=A 레코드로 남는지 폴링한다.
    // ★F3 잔여 예산 계산용 시작점(option b — 예산은 A 턴 대기와 공유)★: 이 시각부터 REPLY_WAIT_CAP(180s)
    //   을 재는데, 그 사이엔 바로 아래 wait_for_reply 뿐 아니라 그 뒤 이어지는 A 턴 대기(wait_turn_end,
    //   최대 TURN_WAIT_CAP=180s)까지 **같은 벽시계를 함께 소비한다** — 새 타임아웃 상수를 만들지 않고
    //   하나의 창을 공유시킨 트레이드오프다(사용자 결정, option b — "wait_for_reply 소비분만 뺀다" 던
    //   이전 주석은 틀렸다). A 턴 대기가 길면 아래 폴링(wait_for_matching_reply)의 잔여 예산이 0/음수가
    //   될 수 있고, 그러면 폴링은 아예 돌지 않는다 — 그 경우를 결과 블록의 REPLY_POLL=skipped-no-budget
    //   으로 반드시 알린다(침묵 금지).
    let reply_wait_started = Instant::now();
    let reply_obs = wait_for_reply(
        &observer,
        reply_baseline,
        agent_b.id,
        agent_a.id,
        REPLY_WAIT_CAP,
    );
    let b_sent = reply_obs.is_some();
    // ★valid negative 게이트(FIX round-2 #4)★: B 가 안 보냈는데(reply_obs=None) 그 사이 **A 또는 B** 가
    //   죽었다면 그건 "B 가 안 보내기로 함"(정상 negative)이 아니라 process death setup 실패다.
    //   - B 사망: B 가 답신을 만들 주체를 잃음.
    //   - A 사망: A 가 죽으면(스폰 후 / 씨앗 ACK 후) B 의 답신이 A 에 도달할 대상이 없어 관측이 안 뜨고,
    //     B 는 살아 있어 기존 B-only 게이트는 이를 정상 negative 로 오분류한다 — 이게 원 blocker 와 같은
    //     방식으로 실험 데이터를 오염시킨다. 그래서 valid negative 판정 지점에서 A 생존도 함께 확인한다.
    //   A·B 모두 살아 있는데도 안 보낸 경우만 유효한 실험 negative 로 아래에서 보고한다.
    if !b_sent && !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B 가 답신 대기 중 종료됨(process death) — valid negative 아님(setup 실패)");
    }
    if !b_sent && !is_agent_alive(&manager, agent_a.id) {
        fail_setup!(
            "A 가 답신 대기 중 종료됨(process death) — B 답신이 도달할 대상 없음, valid negative 아님(setup 실패)"
        );
    }
    let entrance_label = match &reply_obs {
        Some(o) => entrance_str(o.entrance),
        None => "none",
    };
    // ★--cli-only 판정(ADR-0099 FIX 3)★: 이 모드는 provision 을 비-MCP 로 강제해 MCP 입구가 물리적으로
    //   없다 — B 가 보냈다면(b_sent) entrance 는 반드시 `cli` 여야 한다. `mcp` 가 관측되면 강제 seam 이
    //   실제로 MCP 를 제거하지 못한 것(배관 결함)이므로 SETUP-FAIL(setup 결함)로 요란히 알린다.
    //   ★entrance=none(B 미발신)은 여기서 안 잡고 끝의 엄격 VERDICT 가 FAIL(exit 1)로 처리한다★ —
    //   여기 SETUP-FAIL 은 "seam 배관 결함"(mcp 새어나옴) 전용이고, "강제 false path 미실증"(아무도 안 보냄)은
    //   결과 판정이라 최종 VERDICT 로 분리한다(라벨이 서로 다른 실패 원인을 섞지 않게).
    if args.cli_only && b_sent && entrance_label != "cli" {
        fail_setup!(&format!(
            "--cli-only 인데 B 가 entrance={entrance_label} 로 발신 — 강제 seam 이 MCP 입구를 제거 못 함(배관 결함, 정상 negative 아님)"
        ));
    }

    // ── 4) A 가 B 답신을 처리하며 낸 텍스트 대기(정성 관측) ───────────────────────────
    //    B 가 보냈으면 그 relay 가 A stdin 에 꽂혀 A 턴이 돈다. 남은 시간만큼 A 턴 종료를 기다린다.
    let a_responded = if b_sent {
        obs_a.wait_turn_end(baseline_a, TURN_WAIT_CAP)
    } else {
        // B 가 안 보냈으면 A 턴이 돌 이유가 없다 — 짧게만 확인(이미 REPLY_WAIT_CAP 동안 아무것도 없었음).
        obs_a.done_snapshot() > baseline_a
    };
    let a_response = obs_a.response_text();

    // ★F3 레이스를 여기서 실제로 닫는다★: wait_for_reply 는 baseline 이후 첫 B→A 레코드(무관한 "ack" 일
    //   수 있음)에서 멈춘다 — A 턴 대기까지 끝나도 진짜 계약 회신(in_reply_to == seed_msg_id)이 아직
    //   안 왔을 수 있어, 그 상태로 바로 print 시점 스캔을 하면 거짓 negative 가 난다. 그래서 A 턴 대기가
    //   끝난 지금, 그 레코드가 나타나길 잔여 REPLY_WAIT_CAP 예산으로 마저 폴링한다(같은 루프/간격 재사용
    //   — 새 타임아웃 상수 없음). ★그 잔여 예산은 wait_for_reply 만의 소비가 아니라 방금 끝난 A 턴 대기
    //   (wait_turn_end, 최대 TURN_WAIT_CAP=180s)까지 같은 REPLY_WAIT_CAP 벽시계를 함께 태운 뒤 남은
    //   값이다(option b) — A 턴이 길게 돌면 이 잔여가 0/음수일 수 있고, 그럴 땐 바로 아래
    //   `remaining.is_zero()` 가드가 폴링을 아예 건너뛴다. 그 스킵은 조용히 넘어가지 않고 결과 블록의
    //   REPLY_POLL=skipped-no-budget 로 반드시 드러낸다(항목1, option b — 아래 reply_poll_ran 이 그
    //   판정용 신호). seed-request 일 때만 한다 — plain 씨앗은 in_reply_to 자체가 없어 폴링할 대상이
    //   없고, 기본 경로(SEED_KIND=plain)를 늦추지 않아야 하기 때문이다. 예산이 소진되도록 못 찾으면
    //   아래 스캔이 정직하게 none/false 로 남긴다.
    // ★REPLY_POLL 마커용 신호(항목1, option b)★: 이 폴링이 실제로 "돌았는가"(예산이 있어 루프에
    //   진입했는가)만 기록한다 — 최종 라벨(matched/timeout/skipped-no-budget)은 5) 절에서 이 값과
    //   reply_matches_seed(최종 스캔 결과)를 함께 봐서 정한다. "matched" 는 폴링이 잡았든 폴링이
    //   스킵됐지만 이미 배달돼 있었든 상관없이 최우선이므로, 여기서는 예산 유무만 정직하게 담아 둔다.
    let mut reply_poll_ran = false;
    // ★REPLY_POLL_BUDGET_MS 계측(신규)★: 폴링 돌지/스킵 여부를 가른 바로 그 결정 지점에서 잔여 예산(ms)을
    //   그대로 남긴다 — 0 = 예산 없음(스킵) 또는 1ms 미만 잔여(as_millis 절사로 poll_ran=true 인데 0 이
    //   찍힐 수 있다). 스킵 여부의 정본은 REPLY_POLL 라벨이지 이 수치가 아니다. 이게 있어야 "폴링이 100ms 짜리 굶주린 예산만 받고 timeout 이 났다" 는
    //   기아(starvation) 상황과 "폴링이 정상적인(수십 초) 예산을 다 쓰고도 못 찾은" negative 를 같은
    //   REPLY_POLL=timeout 라벨 뒤에서 구분할 수 있다(라벨만으로는 두 상황이 안 갈린다).
    let mut reply_poll_budget_ms: u64 = 0;
    if args.seed_request {
        if let Some(sid) = seed_msg_id.as_deref() {
            let remaining = REPLY_WAIT_CAP.saturating_sub(reply_wait_started.elapsed());
            reply_poll_budget_ms = remaining.as_millis() as u64;
            if !remaining.is_zero() {
                reply_poll_ran = true;
                wait_for_matching_reply(
                    &observer,
                    reply_baseline,
                    agent_b.id,
                    agent_a.id,
                    sid,
                    remaining,
                );
            }
        }
    }

    // ── 5) 구조화 stdout 마커(오케스트레이터 판정용) ────────────────────────────────
    // cli-only 모드는 셀렉터가 없으므로(override 금지) 전용 라벨을 단다 — 오케스트레이터가 이 실측이
    //   false-path(provision 강제 비-MCP) 임을 구분하게.
    let case_label = if args.cli_only {
        "CLI-ONLY(forced non-MCP)"
    } else {
        priming_selector.as_deref().unwrap_or("C0")
    };
    println!("\n===== ROUNDTRIP CASE={case_label} B_SENT={b_sent} ENTRANCE={entrance_label} =====");
    println!("[model] {}", args.model);
    // 존재 검사를 통과한 실제 in-effect 경로만 출력한다(FIX round-2 #5 — 거짓 라벨 금지).
    println!("[priming] {}", resolved_priming.display());
    println!("[seed A->B body] {SEED_A_TO_B}");
    // --seed-request(지정 시) 결과 마커 - 미지정이면 SEED_KIND=plain 뿐(오늘 동작, 새 줄 하나만 추가).
    let seed_kind = if args.seed_request {
        "request"
    } else {
        "plain"
    };
    println!("SEED_KIND={seed_kind}");
    // ★F5★ B_TASK 자기서술 줄 — SEED_KIND 와 같은 규율로 항상 찍는다(default 여도).
    println!("B_TASK={b_task_kind}");
    if args.seed_request {
        // ★F5 규율★ SEED_REPLY_BY 자기서술 줄 — B_TASK/SEED_KIND 와 같은 규율로, request 일 땐 항상 찍는다
        //   (미지정이면 "none" — 조용히 빠지지 않는다).
        println!(
            "SEED_REPLY_BY={}",
            args.seed_reply_by.as_deref().unwrap_or("none")
        );
        // ★F3★ "첫 레코드 승" 이 아니라, wait_for_matching_reply(위, A 턴 대기 직후)가 — **잔여
        //   REPLY_WAIT_CAP 예산이 남아 있을 때만** — 실제 계약 회신을 폴링해 마저 기다린다. 그 예산은
        //   씨앗 주입 시점부터 A 턴 대기(wait_turn_end)까지 함께 소비한 벽시계라, A 턴이 길면 잔여가
        //   0/음수가 돼 이 폴링이 **아예 호출되지 않을 수 있다**(무조건 기다리는 게 아니다 — reply_poll_ran
        //   이 그 실행 여부를 담는다). 폴링이 돌았든 안 돌았든(=스킵), 그 **뒤** baseline 이후 B→A 레코드
        //   **전부**를 스캔한다 — 첫 레코드가 실제 회신과 무관한 "ack" 한 통이어도, 폴링이 돌아 진짜 회신을
        //   이미 잡아 뒀다면(또는 폴링 전에 이미 배달돼 있었다면) 여기 스캔이 그걸 찾는다(레이스가 폴링으로
        //   닫혔으므로 이 스캔은 이미 채워진 결과를 읽는 역할). 폴링이 예산 부족으로 아예 안 돈 경우엔
        //   스캔이 찾을 게 새로 생기지 않으므로 정직하게 none/false 로 남고, 그 스킵은 아래 REPLY_POLL=
        //   skipped-no-budget 라벨로 timeout(폴링은 돌았으나 못 찾음)과 구분된다. **배달된(is_delivered)
        //   레코드만** 증거로 인정한다 — write 가 실패한 레코드는 실제로 도달하지 않았으므로, 그
        //   in_reply_to 가 우연히 seed id 와 같아도 "B 가 성공적으로 회신했다" 의 증거가 아니다(그래서
        //   폴링 예산이 소진되도록 못 찾은 "only failed records carry it" 케이스는 정직하게 none/false 로
        //   남긴다).
        let after_seed = observer.records_after(reply_baseline, agent_b.id, agent_a.id);
        // REPLY_MATCHES_SEED: 엄격 일치 판정(변경 없음) — 배달된 레코드 중 seed_msg_id 와 정확히 같은
        //   in_reply_to 를 가진 것이 있는가.
        let matched = after_seed.iter().find(|r| {
            r.is_delivered()
                && seed_msg_id
                    .as_deref()
                    .is_some_and(|s| r.in_reply_to.as_deref() == Some(s))
        });
        let reply_matches_seed = matched.is_some();
        // ★finding 1 FIX★ REPLY_IN_REPLY_TO 는 first-wins 만으로 정하지 않는다 — 아래 항목2 FIX 참조.
        //   baseline 이후 첫 **배달된** B→A 레코드 중 in_reply_to.is_some() 인 것을 폴백용으로 찾아 둔다
        //   (매치가 없을 때만 이 값을 쓴다 — "B 가 in-reply-to 를 아예 안 실음" 과 구분하는 용도는 유지).
        let first_reply_with_id = after_seed
            .iter()
            .find(|r| r.is_delivered() && r.in_reply_to.is_some());
        // ★항목2 마커쌍 자기모순 FIX(reviewer NOTE)★: 이전엔 REPLY_IN_REPLY_TO 를 **항상 무조건**
        //   first-wins(`first_reply_with_id`)로 찍었다 — B 가 틀린 id 로 먼저 답하고 맞는 id 로 나중에
        //   답하면 REPLY_IN_REPLY_TO=<wrong-id> 인데 REPLY_MATCHES_SEED=true 인 자기모순 쌍이 났다(두
        //   마커가 서로 다른 레코드를 가리킴 — 하나는 첫 레코드, 하나는 일치 레코드). 그래서 이제: 일치
        //   레코드(`matched`)가 있으면 **그 레코드의 값**(= seed id 와 동일)을 우선해서 찍고, 일치가
        //   아예 없을 때만 first-wins 로 폴백한다 — 두 마커가 항상 같은 레코드에서 파생되어 모순이 없다.
        let reply_in_reply_to: Option<String> = match matched {
            Some(m) => m.in_reply_to.clone(),
            None => first_reply_with_id.and_then(|r| r.in_reply_to.clone()),
        };
        println!(
            "REPLY_IN_REPLY_TO={}",
            reply_in_reply_to.as_deref().unwrap_or("none")
        );
        println!("REPLY_MATCHES_SEED={reply_matches_seed}");
        // ★항목1 REPLY_POLL 마커(option b)★: 예산이 A 턴 대기에 다 먹혀 폴링이 조용히 스킵되는 경우를
        //   절대 침묵시키지 않는다. 우선순위 판정은 순수 함수 `reply_poll_label`(단위테스트 대상)로 뽑아
        //   뒀다 — matched(폴링이 잡았든 폴링 전에 이미 배달돼 있었든 최종 스캔이 일치를 확인하면 최우선)
        //   > timeout(폴링이 실제로 돌았는데 예산 소진까지 못 찾음) > skipped-no-budget(잔여 예산이
        //   0/음수라 폴링 자체가 안 돎 — reply_poll_ran=false).
        let reply_poll = reply_poll_label(reply_matches_seed, reply_poll_ran);
        println!("REPLY_POLL={reply_poll}");
        // ★REPLY_POLL_BUDGET_MS(신규)★: 위에서 폴링 여부를 가른 그 결정 지점의 잔여 예산(ms)을 그대로
        //   찍는다(스킵이면 0) — REPLY_POLL 라벨만으로는 "100ms 짜리 굶주린 예산 끝에 난 timeout" 과
        //   "정상 예산을 다 쓰고도 못 찾은 negative" 가 구분되지 않는다. 이 값이 있어야 그 둘을 가른다.
        println!("REPLY_POLL_BUDGET_MS={reply_poll_budget_ms}");
    }
    println!("[B sent reply to A] {b_sent}");
    println!("[B chosen entrance] {entrance_label}");
    if let Some(o) = &reply_obs {
        // 봉투 배달 레코드는 body 텍스트를 담지 않는다(보안) — 바이트 수·msg_id 만.
        println!(
            "[B->A delivery] msg_id={} bytes={} to_epoch={:?}",
            o.msg_id, o.bytes_requested, o.to_epoch
        );
    }
    println!("[A responded within cap] {a_responded}");
    println!("[A full response text]\n{}", a_response.trim());
    // 진단: B 가 씨앗을 받고 자기 턴에 낸 응답(send 라우팅과 무관 — B_SENT=false 여도 여기 텍스트가 있으면
    //   "B 는 답했으나 send 로 안 보냄"이고, 비면 "B 가 씨앗에 반응 안 함").
    let b_turn_ended = obs_b.done_snapshot() > baseline_b;
    let b_seed_response = obs_b.response_text();
    println!("[B post-seed turn ended] {b_turn_ended}");
    println!("[B post-seed turn text]\n{}", b_seed_response.trim());
    println!("===== END ROUNDTRIP (orchestrator judges qualitatively) =====\n");

    // ── 정리 ──────────────────────────────────────────────────────────────────────
    if let Some(id) = sink_a {
        let _ = manager.unsubscribe(agent_a.id, id);
    }
    if let Some(id) = sink_b {
        let _ = manager.unsubscribe(agent_b.id, id);
    }
    let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
    cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
    handle.shutdown().await;
    // ★--cli-only 는 엄격 판정(ADR-0099)★: 이 모드는 provision 을 비-MCP 로 강제해 false path 전체가
    //   정합하게 도는지를 실측하는 게 목적이라, B 가 실제로 CLI 입구로 보냈을 때만(b_sent && entrance=cli)
    //   성공이다. 아무것도 안 보낸 경우(B_SENT=false/ENTRANCE=none)는 일반 모드의 valid-negative 와 달리
    //   **실패**로 본다(강제 false path 가 도는 걸 못 봤으니 목적 미달). 일반 모드는 종전대로 negative 도 exit 0.
    if args.cli_only {
        if cli_only_run_passed(b_sent, entrance_label) {
            println!("VERDICT [roundtrip-smoke --cli-only]: PASS — B 가 CLI 입구로 발신(b_sent=true, entrance=cli)");
            return 0;
        }
        let line = format!(
            "VERDICT [roundtrip-smoke --cli-only]: FAIL — 강제 false path 미실증(b_sent={b_sent}, entrance={entrance_label}); cli-only 는 b_sent=true AND entrance=cli 여야 pass"
        );
        println!("{line}");
        eprintln!("{line}");
        return 1;
    }
    // ★negative(B did not send)도 정상 exit 0★: 유효한 실험 결과지 하네스 실패가 아니다(ADR-0092).
    0
}

/// 인자 파싱(순수·단위테스트 대상): `--priming <값>`·`--model <값>`·불리언 `--disallow-mcp`/`--cli-only`
///   /`--seed-request`·`--seed-reply-by <값>`·`--b-task <값>` 를 인식한다. 미지정 model=sonnet, 미지정
///   priming=None(= 기본 both 프라이밍), 미지정 seed_request=false/seed_reply_by=None/b_task=None(= 오늘
///   동작). 알 수 없는 토큰은 무시(하네스라 관대). `iter` 로 받아 std::env 의존을 뺀다.
/// 플래그를 값으로 삼키지 않는다(FIX round-2 #7): `--priming --model opus` 처럼 다음 토큰이 또 플래그
///   (`--` 로 시작)면 그건 값이 아니라 새 플래그다 — peek 해서 값으로 소비하지 않고 넘긴다(그 플래그는
///   다음 루프에서 제대로 처리, priming 은 미지정 유지). 이렇게 안 하면 `--model` 이 priming 값으로 먹혀
///   model 이 조용히 기본값에 남는다.
/// ★F5 — `--b-task`·`--seed-reply-by` 는 예외(더 엄격)★: 다른 값-플래그(`--priming`/`--model`)는 값이
///   없거나 플래그처럼 보이면 `take_flag_value` 가 조용히 None 을 돌려 호출자가 기본값을 유지하지만,
///   `--b-task` 와 `--seed-reply-by` 는 그 세 오용(값 누락·다음이 `--` 로 시작·빈/공백뿐인 값)을 전부
///   각각 `b_task_error`/`seed_reply_by_error` 에 사유로 기록한다 — B 원과제 프롬프트나 회신 기한이
///   통째로 조용히 기본값(auth 모듈 과제 / 기한 없음)으로 미끄러지면 오퍼레이터가 눈치채기 어렵기
///   때문이다(다른 노브는 미지정=오늘 동작이 곧 안전한 기본이라 관대해도 되지만, 이 둘은 "쓰려던 값이
///   안 먹혔다" 는 신호를 죽인다 — 특히 `--seed-reply-by` 가 조용히 안 먹히면 request 계약이 기한 없이
///   돈다).
fn parse_args(iter: impl Iterator<Item = String>) -> Args {
    let mut priming = None;
    let mut model = "sonnet".to_string();
    // ADR-0094: `--disallow-mcp` 는 값 없는 불리언 플래그(존재 = 켜짐) — take_flag_value 로 다음 토큰을
    //   삼키지 않는다(그 자체로 완결).
    let mut disallow_mcp = false;
    // ADR-0099 FIX 3: `--cli-only` 도 값 없는 불리언 플래그(존재 = 켜짐).
    let mut cli_only = false;
    // `--seed-request` 도 값 없는 불리언 플래그(존재 = 켜짐).
    let mut seed_request = false;
    let mut seed_reply_by = None;
    // ★F5 오용 가드(신규)★ `--seed-reply-by` 도 `--b-task` 와 같은 강화를 받는다 — `take_flag_value` 의
    //   "조용히 None" 을 받지 않고 잘못 쓴 값(누락/플래그로 오인/빈 값)을 사유째 기록해 `run()` 이 스폰 전에
    //   fail-fast 하게 한다(조용히 넘기면 request 계약이 기한 없이 돌아 오퍼레이터가 눈치채기 어렵다).
    let mut seed_reply_by_error: Option<String> = None;
    let mut b_task = None;
    // ★F5 오용 가드★ `--b-task`·`--seed-reply-by` 는 다른 값-플래그(`--priming`/`--model`)와 다르게
    //   `take_flag_value` 의 "조용히 None" 을 그대로 받지 않는다 — 잘못 쓴 값(누락/플래그로 오인/빈 값)을
    //   여기서 사유째 기록해 두면 `run()` 이 스폰 전에 fail-fast 한다(다른 플래그는 관대하게 무시하는
    //   기존 규율을 그대로 유지 — 이건 이 둘의 전용 강화다).
    let mut b_task_error: Option<String> = None;
    let mut it = iter.peekable();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--priming" => {
                if let Some(v) = take_flag_value(&mut it) {
                    priming = Some(v);
                }
            }
            "--model" => {
                if let Some(v) = take_flag_value(&mut it) {
                    model = v;
                }
            }
            "--disallow-mcp" => disallow_mcp = true,
            "--cli-only" => cli_only = true,
            "--seed-request" => seed_request = true,
            // ★F5 오용 가드(신규)★ 값 누락 / 다음 토큰이 `--` 로 시작(플래그로 오인 가능) / 빈·공백뿐인
            //   값은 전부 seed_reply_by_error 에 사유째 기록한다(--b-task 와 같은 peek 패턴 — 조용히
            //   None 을 돌리는 take_flag_value 를 여기선 안 쓴다).
            "--seed-reply-by" => match it.peek() {
                None => {
                    seed_reply_by_error = Some(
                        "--seed-reply-by requires a value (duration text like '5m') but none was given."
                            .to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    seed_reply_by_error = Some(format!(
                        "--seed-reply-by requires a value (duration text like '5m'); '{next}' looks like another flag, not a duration value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        seed_reply_by_error = Some(
                            "--seed-reply-by value is empty or whitespace-only; provide a duration like '5m'."
                                .to_string(),
                        );
                    } else {
                        seed_reply_by = Some(v);
                    }
                }
            },
            "--b-task" => match it.peek() {
                None => {
                    b_task_error = Some(
                        "--b-task requires a value (text or @path) but none was given.".to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    b_task_error = Some(format!(
                        "--b-task requires a value (text or @path); '{next}' looks like another flag, not a task value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        b_task_error = Some(
                            "--b-task value is empty or whitespace-only; provide task text or an @path reference."
                                .to_string(),
                        );
                    } else {
                        b_task = Some(v);
                    }
                }
            },
            _ => {}
        }
    }
    Args {
        priming,
        model,
        disallow_mcp,
        cli_only,
        seed_request,
        seed_reply_by,
        seed_reply_by_error,
        b_task,
        b_task_error,
    }
}

/// 플래그 값 하나를 소비하되, 다음 토큰이 또 다른 플래그(`--`)면 소비하지 않는다(FIX round-2 #7).
///   반환 None = 값 없음(플래그가 값 없이 끝났거나 다음이 또 플래그) → 호출자는 기본값 유지.
fn take_flag_value<I: Iterator<Item = String>>(it: &mut std::iter::Peekable<I>) -> Option<String> {
    match it.peek() {
        Some(next) if next.starts_with("--") => None, // 다음이 플래그 → 값 아님(넘김, 소비 X).
        Some(_) => it.next(),                         // 정상 값 → 소비.
        None => None,                                 // 값 없이 끝.
    }
}

/// ★셀렉터→priming 파일 경로(순수·단위테스트 대상, ADR-0099)★: repo 루트 기준 경로로 매핑한다.
///   - C0(또는 None) → `prompts/agent-priming.md`(운영 A = both-teaching).
///   - 그 외 = **파일 경로로 간주**(절대면 그대로, 상대면 repo 루트 기준 join) — 명시 override. 운영 B
///     (`prompts/agent-priming-cli.md`)나 임시 실험 파일을 이 경로로 직접 지정한다.
/// 반환은 항상 절대경로(존재 검사는 하지 않는다 — FilePrimingProvider 가 최종 존재/CLI-안전 검사).
///   절대화조차 못 하면 None.
///   ※ 옛 C1~C3 실험 별칭은 ADR-0099 로 제거됐다(실험 변형 파일 정리 — git 이력 보존). C1~C3 문자열을
///     넘기면 이제 "그 이름의 파일 경로"로 해석돼 repo 루트 기준 join 되고(존재하지 않아 하류에서 None),
///     별도 특수 매핑은 없다.
fn resolve_priming_path(selector: Option<&str>, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel: &str = match selector {
        None | Some("C0") | Some("c0") => "prompts/agent-priming.md",
        Some(path) => {
            // 명시 경로 override. 절대면 그대로, 상대면 repo 루트 기준.
            let p = PathBuf::from(path);
            let joined = if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            };
            return joined.is_absolute().then_some(joined);
        }
    };
    let joined = repo_root.join(rel);
    joined.is_absolute().then_some(joined)
}

/// 어느 입구 라벨인가(관측 레코드 → 문자열). Entrance 는 daemon crate 내부 as_str 이 private 이라
///   여기서 매핑한다(하네스 표시 전용).
fn entrance_str(e: Entrance) -> &'static str {
    match e {
        Entrance::Mcp => "mcp",
        Entrance::Cli => "cli",
        // C3: 데몬 자가 발신(`<notice>` 타임아웃 통지) — 이 하네스는 A→B 발송만 돌리므로 표시용으로만 존재.
        Entrance::Daemon => "daemon",
    }
}

/// 이 크레이트 매니페스트에서 두 단계 위로 올라간 repo 루트(`prompts/` 가 그 아래).
///   ★discovery(FIX round-2 #6)★: `priming_smoke.rs` 와 **같은** 컴파일타임 `CARGO_MANIFEST_DIR` 기반
///   방식이다(둘 다 동일 — 확인함). 운영 데몬의 exe-walk-up(`discovery::find_install_root`, ADR-0092:
///   WMI 스폰이라 cwd 불신)과는 다르지만, 이 실험 하네스는 항상 `cargo run` 으로 도는 컴파일타임 소스
///   트리 안이므로 MANIFEST_DIR 이 신뢰 가능하다(빌드된 bin 을 다른 곳으로 옮겨 실행하는 경로는 없다).
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent() // .../crates
        .and_then(|p| p.parent()) // .../engram-dashboard (repo 루트)
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

/// 현재 exe 형제에서 `engram-send`(Windows 는 .exe) 를 찾는다 — CLI 입구를 켜려면 필요. 못 찾으면 None
///   (CLI 입구 비활성, MCP 만). cargo run 시 exe 는 target/<profile>/ 아래라 engram-send 도 그 형제다.
fn sibling_send_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "engram-send.exe"
    } else {
        "engram-send"
    };
    let cand = dir.join(name);
    cand.is_file().then_some(cand)
}

/// 에이전트가 아직 살아 있나(비-terminal 상태) — setup 실패 vs valid negative 판별(FIX round-2 #4).
///   목록에서 사라졌거나 terminal(Exited/Failed/Killed)이면 false. Running/Exiting 은 alive.
fn is_agent_alive(manager: &Arc<AgentManager>, id: AgentId) -> bool {
    manager
        .list_agents()
        .iter()
        .find(|a| a.id == id)
        .map(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Exiting))
        .unwrap_or(false)
}

/// 이름 붙인 primed claude(stream-json, Fresh) 1개 스폰 + 목록 등장 대기. 실패/미등장이면 None.
fn spawn_named(
    manager: &Arc<AgentManager>,
    name: &str,
    model: &str,
    workspace: &std::path::Path,
) -> Option<AgentInfo> {
    // ★canonical name = display_name(ADR-0101 WYSIWYA)★: 라우팅·로스터·봉투 sender 가 쓰는 이름 =
    //   display_name ?? basename(session.cwd) 다(profile.name 은 더 이상 주소축 아님). 두 에이전트는
    //   같은 workspace cwd 를 공유해 basename 이 동일 → cwd 파생이면 alice/bob 이 같은 이름으로 충돌·
    //   오라우팅(bob 로 답신)한다. 그래서 이름을 display_name 에 심어 **결정적으로** 구분한다.
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec!["--model".to_string(), model.to_string()],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        workspace.to_path_buf(),
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    let info = manager.spawn_agent(&profile, SpawnMode::Fresh).ok()?;
    let deadline = Instant::now() + SPAWN_APPEAR_TIMEOUT;
    while Instant::now() < deadline {
        if manager.list_agents().iter().any(|a| a.id == info.id) {
            return Some(info);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    None
}

/// baseline **이후** 도착한 from=B·to=A outbound relay 레코드를 상한까지 폴링. 나타나면 그 레코드,
///   상한 초과면 None(negative — B 가 안 보냄). `baseline` = 씨앗 주입 직전 record_count(FIX round-2 #1
///   — pre-seed 오탐 차단). 폴링인 이유: relay 는 B 의 실 claude 판단에 달려 비결정적 지연을 가진다
///   (cv 신호원이 없어 짧은 sleep 폴링이 단순·충분).
fn wait_for_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer.find_delivery_after(baseline, from_b, to_a) {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// ★F3 레이스를 실제로 닫는 폴링(--seed-request 전용)★: `wait_for_reply` 는 baseline 이후 첫 B→A
///   레코드(회신과 무관한 "ack" 일 수 있음)에서 멈춘다 — 진짜 계약 회신(배달 + in_reply_to == seed_msg_id)
///   이 그 뒤에 도착하면 그 시점의 관측만으론 놓친다. 그래서 A 턴 대기까지 끝난 뒤, **잔여** 예산
///   (`cap` — 호출부가 `REPLY_WAIT_CAP.saturating_sub(elapsed)` 로 계산해 넘긴다, 새 타임아웃 상수 없음)
///   만큼, 같은 루프 구조·폴링 간격(200ms)으로 그 레코드가 나타나길 마저 기다린다.
///   ★그 `elapsed` 는 wait_for_reply 만의 소비가 아니다(option b, FIX)★: 호출부가 재는 시각은 씨앗 주입
///   직후부터라, wait_for_reply 뒤에 이어지는 A 턴 대기(wait_turn_end, 최대 TURN_WAIT_CAP=180s)까지
///   **같은 REPLY_WAIT_CAP 벽시계를 함께 태운다**(이전 주석은 "wait_for_reply 소비분만 뺀다" 고 잘못
///   말했었다). 그래서 `cap` 이 0(또는 음수, `saturating_sub` 이 0 으로 바닥침)이면 호출부는 이 함수를
///   아예 부르지 않는다(폴링 스킵) — 그 스킵은 결과 블록의 REPLY_POLL=skipped-no-budget 으로 반드시
///   라벨링되며 침묵하지 않는다(항목1, option b). 매치를 찾으면 그 값을 돌려주지만(호출자는 부수효과로만
///   쓴다 — 실제 사용은 그 뒤 `records_after` 스캔), 예산이 소진되도록 못 찾으면 None(정직한 negative —
///   호출자가 그대로 스캔해도 같은 결과).
fn wait_for_matching_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    seed_msg_id: &str,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer
            .records_after(baseline, from_b, to_a)
            .into_iter()
            .find(|r| r.is_delivered() && r.in_reply_to.as_deref() == Some(seed_msg_id))
        {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 프롬프트를 유저 턴으로 보내고 이번 턴 종료(MessageDone)까지 대기. priming_smoke 와 동일.
fn send_and_wait(
    manager: &Arc<AgentManager>,
    id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
) -> bool {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    if manager.write_stdin_observed(id, prompt.as_bytes()).is_err() {
        return false;
    }
    obs.wait_turn_end(baseline, TURN_WAIT_CAP)
}

async fn cleanup(manager: &Arc<AgentManager>, agent_ids: &[AgentId], dirs: &[&PathBuf]) {
    for id in agent_ids {
        let _ = manager.kill_agent(*id);
    }
    if !agent_ids.is_empty() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.list_agents().is_empty() {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// 턴 관측기 — MessageDone 카운트(턴 종료 신호) + TextDelta 누적(응답 텍스트).
///
/// ★lost-wakeup 방지(FIX round-2 #3)★: 이전 판본은 `done_count` 를 `AtomicU64` 로 두고 mutex 밖에서
///   증가·notify 했다. 그러면 waiter 가 [원자 predicate 체크] 와 [`wait_timeout` 등록] 사이에 완료 신호가
///   끼면 그 wakeup 을 잃고(cv 는 등록 전 notify 를 기억하지 않는다) 이미 끝난 턴을 상한(cap)까지 헛대기해
///   **거짓 타임아웃**(A 무응답/ B task 타임아웃)을 낸다. 그래서 표준 condvar 규율로 바꾼다:
///   predicate 상태(`done_count`)를 **cv 가 쓰는 바로 그 mutex 안**에 넣고, 발신·대기 모두 그 락을 잡은 채
///   갱신/재확인한다 → notify 는 락 해제 후 관측되므로 wakeup 손실이 원천적으로 없다.
///   `inner`(응답 텍스트)와 `done_count` 를 한 구조체(`State`)로 묶어 단일 mutex 로 보호한다.
struct TurnState {
    /// 이번 턴 누적 응답 텍스트.
    text: String,
    /// 관측된 MessageDone 누계(턴 종료 신호). cv predicate 의 단일 출처 — 이 mutex 로만 접근.
    done_count: u64,
}

struct TurnObserver {
    id: SinkId,
    state: Mutex<TurnState>,
    cv: Condvar,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            state: Mutex::new(TurnState {
                text: String::new(),
                done_count: 0,
            }),
            cv: Condvar::new(),
        }
    }
    fn begin_turn(&self) {
        self.state.lock().unwrap().text.clear();
    }
    fn done_snapshot(&self) -> u64 {
        self.state.lock().unwrap().done_count
    }
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.state.lock().unwrap();
        loop {
            // predicate 를 mutex 보유 중 재확인(표준 condvar 루프) — notify 는 이 락 안에서만 반영된다.
            if g.done_count > baseline {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (ng, _to) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }
    fn response_text(&self) -> String {
        self.state.lock().unwrap().text.clone()
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            return Ok(());
        };
        match ev {
            OutputEvent::TextDelta { text, .. } => {
                self.state.lock().unwrap().text.push_str(text);
            }
            OutputEvent::MessageDone { .. } => {
                // ★락 보유 중 상태 변경 후 notify(wakeup 손실 방지)★: predicate(done_count)를 cv 의 mutex
                //   안에서 올린다. guard 를 notify 후 drop 해도 되고 전에 drop 해도 되지만, 표준 규율대로
                //   보유 중 변경만 지키면 [체크↔등록] 갭에 낀 완료가 사라지지 않는다.
                let mut g = self.state.lock().unwrap();
                g.done_count += 1;
                drop(g);
                self.cv.notify_all();
            }
            _ => {}
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(s(&[]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
        assert!(!a.disallow_mcp, "기본은 MCP 허용(오늘 동작)");
        assert!(!a.cli_only, "기본은 cli-only 강제 없음(오늘 동작)");
        assert!(
            !a.seed_request,
            "기본은 씨앗 계약 없음(plain 통보, 오늘 동작)"
        );
        assert_eq!(a.seed_reply_by, None);
        assert_eq!(
            a.seed_reply_by_error, None,
            "기본은 --seed-reply-by 오용 없음"
        );
        assert_eq!(a.b_task, None, "기본은 B 원과제 override 없음(오늘 동작)");
        assert_eq!(a.b_task_error, None, "기본은 --b-task 오용 없음");
    }

    #[test]
    fn parse_args_cli_only_flag_is_boolean() {
        // ★ADR-0099 FIX 3★: `--cli-only` 는 값 없는 불리언 플래그(존재 = 켜짐). 뒤 토큰(--model)을 값으로
        //   삼키지 않고, model 은 정상 파싱돼야 한다.
        let a = parse_args(s(&["--cli-only", "--model", "opus"]));
        assert!(a.cli_only, "--cli-only 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--cli-only 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_cli_only_absent_is_false() {
        // 플래그 미지정이면 오늘 동작(강제 없음) 유지 — 운영 회귀 0.
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
        assert!(!a.cli_only);
    }

    #[test]
    fn parse_args_disallow_mcp_flag_is_boolean() {
        // ★ADR-0094★: `--disallow-mcp` 는 값 없는 불리언 플래그(존재 = 켜짐). 뒤 토큰(--model)을 값으로
        //   삼키지 않고, model 은 정상 파싱돼야 한다.
        let a = parse_args(s(&["--disallow-mcp", "--model", "opus"]));
        assert!(a.disallow_mcp, "--disallow-mcp 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--disallow-mcp 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_disallow_mcp_absent_is_false() {
        // 플래그 미지정이면 오늘 동작(MCP 허용) 유지 — 운영 회귀 0.
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "haiku"]));
        assert!(!a.disallow_mcp);
    }

    #[test]
    fn parse_args_priming_and_model() {
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "opus"]));
        assert_eq!(a.priming.as_deref(), Some("some/priming.md"));
        assert_eq!(a.model, "opus");
    }

    #[test]
    fn parse_args_order_independent_and_ignores_unknown() {
        let a = parse_args(s(&[
            "--model",
            "haiku",
            "junk",
            "--priming",
            "other/priming.md",
        ]));
        assert_eq!(a.priming.as_deref(), Some("other/priming.md"));
        assert_eq!(a.model, "haiku");
    }

    #[test]
    fn parse_args_flag_without_value_is_ignored() {
        // --priming 뒤에 값이 없으면 priming 은 None 유지(패닉 없이 관대).
        let a = parse_args(s(&["--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
    }

    #[test]
    fn parse_args_flag_does_not_consume_next_flag_as_value() {
        // ★FIX round-2 #7★: `--priming --model opus` — --priming 은 값이 없고(다음이 플래그), --model 은
        //   제대로 opus 로 파싱돼야 한다(이전엔 --model 이 priming 값으로 먹혀 model 이 sonnet 에 남았다).
        let a = parse_args(s(&["--priming", "--model", "opus"]));
        assert_eq!(
            a.priming, None,
            "다음 토큰이 플래그면 priming 값으로 삼키지 않는다"
        );
        assert_eq!(a.model, "opus", "--model 은 정상 파싱돼야");
    }

    #[test]
    fn parse_args_trailing_flag_flags_both_ignored_cleanly() {
        // 둘 다 값 없이 끝나는 malformed — 패닉 없이 기본값 유지.
        let a = parse_args(s(&["--model", "--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(
            a.model, "sonnet",
            "--model 뒤가 플래그라 값 없음 → 기본 유지"
        );
    }

    #[test]
    fn resolve_case_c0_maps_to_current_priming() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(None, &root).expect("C0 경로");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md"),
            "C0 은 현행 priming: {got:?}"
        );
        // 명시 "C0" 셀렉터도 같은 경로.
        let got2 = resolve_priming_path(Some("C0"), &root).expect("C0 경로");
        assert_eq!(got, got2);
    }

    // ADR-0099: 옛 C1~C3 실험 별칭 매핑 테스트는 별칭 제거와 함께 삭제됐다. C1~C3 는 이제 파일 경로로
    //   해석돼 repo 루트 기준 join 될 뿐 특수 매핑이 없다(아래 명시 경로 override 테스트가 그 동작을 커버).

    #[test]
    fn resolve_explicit_absolute_path_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "C:\\custom\\my-priming.md"
        } else {
            "/custom/my-priming.md"
        };
        let got = resolve_priming_path(Some(abs), &root).expect("절대 override");
        assert_eq!(got, PathBuf::from(abs), "절대 경로는 그대로 통과");
    }

    #[test]
    fn resolve_explicit_relative_path_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(Some("sub/custom.md"), &root).expect("상대 override");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/custom.md") || got.ends_with("sub\\custom.md"),
            "상대 경로는 repo 루트 기준 join: {got:?}"
        );
    }

    // ★ADR-0094★: CLI-요구 판정은 basename 리스트가 아니라 프라이밍 **본문(content)** 으로 한다 —
    //   `engram-send` 또는 `ENGRAM_SEND_EXE` 를 언급하면 CLI 발신 지시. basename 리스트는 rot 하므로
    //   (새 CLI-지시 프라이밍이 누락돼 가드 우회 → 인프라 부재 오귀속) 본문을 진실의 출처로 삼는다.
    #[test]
    fn priming_text_directs_cli_true_for_engram_send_mention() {
        // engram-send CLI 를 언급하는 본문 → true(ENGRAM_SEND_EXE 와 engram-send 둘 다 등장).
        let text = "To reply, run in your shell: `$ENGRAM_SEND_EXE --to alice --body ...`\n\
                    i.e. run the engram-send command with the recipient name.";
        assert!(
            priming_text_directs_cli(text),
            "engram-send 언급 → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_true_for_env_var_only() {
        // ENGRAM_SEND_EXE 만 있어도(engram-send 리터럴 없이) CLI 지시.
        let text = "Invoke the binary referenced by ENGRAM_SEND_EXE to deliver your message.";
        assert!(
            priming_text_directs_cli(text),
            "ENGRAM_SEND_EXE 언급 → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_false_for_mcp_only() {
        // MCP send_message 만 언급하고 CLI 경로는 없음 → false(CLI 없이도 유효한 실험).
        let text = "To reply, call the MCP tool `send_message` with the recipient and body.";
        assert!(
            !priming_text_directs_cli(text),
            "MCP send_message 만 → CLI 지시 아님"
        );
    }

    #[test]
    fn priming_text_directs_cli_false_for_empty() {
        // 빈 본문(발신 지시 없음) → false.
        assert!(!priming_text_directs_cli(""), "빈 본문 → CLI 지시 아님");
    }

    #[test]
    fn priming_text_directs_cli_case_insensitive() {
        // ★FIX★: 대소문자 무시 — 산문이 대문자/혼합으로 써도 CLI 지시로 잡아야 한다(놓치면 false negative).
        assert!(
            priming_text_directs_cli("Reply via ENGRAM-SEND right away."),
            "대문자 ENGRAM-SEND → CLI 지시"
        );
        assert!(
            priming_text_directs_cli("Use the Engram-Send helper to deliver."),
            "혼합 Engram-Send → CLI 지시"
        );
        assert!(
            priming_text_directs_cli("The var Engram_Send_Exe points to the binary."),
            "혼합 Engram_Send_Exe → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_negation_is_intentionally_true() {
        // ★수용된 false positive(문서화)★: "engram-send 를 쓰지 마라" 같은 부정문도 substring 존재만으로
        //   true → 헛된 SETUP-SKIP. 이는 의도된 보수적 방향이다(요란한 exit-1 로 틀릴 수 있는 발화 거부).
        //   실 프라이밍엔 그런 부정문이 없고, 부정 파싱은 넣지 않는다. 순수 레벨의 현 동작을 못박아 둔다.
        assert!(
            priming_text_directs_cli("Do NOT use engram-send; use MCP instead."),
            "부정문도 substring 존재로 true — 의도된 보수적 skip 방향"
        );
    }

    // ── ADR-0099: --cli-only 가 상속된 ENGRAM_PRIMING_FILE override 를 거부하는가(순수 판정) ──────────
    #[test]
    fn cli_only_rejects_inherited_priming_env() {
        use std::ffi::OsStr;
        // cli-only + 비어 있지 않은 env override → 충돌(true, SETUP-FAIL 유발). `--priming` co-pass 거부와 대칭.
        assert!(
            cli_only_env_override_conflicts(true, Some(OsStr::new("prompts/agent-priming.md"))),
            "cli-only 인데 상속 env override 있음 → 거부(충돌)"
        );
    }

    #[test]
    fn cli_only_ignores_empty_or_absent_priming_env() {
        use std::ffi::OsStr;
        // env 미설정(None) 또는 빈 값이면(미설정 취급) 충돌 아님 — 정상 진행.
        assert!(
            !cli_only_env_override_conflicts(true, None),
            "cli-only 인데 env 미설정 → 충돌 아님"
        );
        assert!(
            !cli_only_env_override_conflicts(true, Some(OsStr::new(""))),
            "cli-only 인데 env 빈 값(미설정 취급) → 충돌 아님"
        );
    }

    #[test]
    fn non_cli_only_never_conflicts_with_priming_env() {
        use std::ffi::OsStr;
        // 일반 모드(cli_only=false)는 env override 를 정당히 쓴다 — 값이 있어도 충돌 아님.
        assert!(
            !cli_only_env_override_conflicts(false, Some(OsStr::new("prompts/agent-priming.md"))),
            "일반 모드는 env override 정당 → 충돌 아님(cli_only=false)"
        );
    }

    // ── ADR-0099: --cli-only 엄격 성공 판정(순수) — b_sent && entrance=cli 여야 pass ────────────────
    #[test]
    fn cli_only_pass_only_when_sent_via_cli() {
        // 유일한 pass 조합: 실제 발신 + CLI 입구.
        assert!(
            cli_only_run_passed(true, "cli"),
            "b_sent=true & entrance=cli → PASS"
        );
    }

    #[test]
    fn cli_only_fail_when_nothing_sent() {
        // ★핵심(FIX 4)★: 아무것도 안 보낸 경우(b_sent=false/entrance=none)는 일반 모드의 valid-negative 와
        //   달리 cli-only 에선 FAIL(강제 false path 미실증) — pass 아님.
        assert!(
            !cli_only_run_passed(false, "none"),
            "b_sent=false/entrance=none → FAIL(pass 아님)"
        );
    }

    #[test]
    fn cli_only_fail_when_sent_via_non_cli_entrance() {
        // entrance=mcp(강제 seam 이 MCP 를 못 지움)나 그 밖의 입구는 pass 아님(이중 안전망 — 앞선 SETUP-FAIL 과
        //   별개로 순수 판정자도 cli 아닌 건 전부 실패로).
        assert!(
            !cli_only_run_passed(true, "mcp"),
            "entrance=mcp → pass 아님"
        );
        assert!(
            !cli_only_run_passed(true, "none"),
            "b_sent=true 라도 entrance=none 이면 pass 아님"
        );
    }

    // ── REPLY_POLL 라벨 판정(순수) — 우선순위 matched > timeout > skipped-no-budget ──────────────────
    #[test]
    fn reply_poll_label_matched_wins_regardless_of_poll_ran() {
        // matched 는 폴링이 실제로 돌아 잡았든(poll_ran=true), 폴링 전에 이미 배달돼 있어 스킵됐든
        // (poll_ran=false) 최우선이다 — 두 조합 모두 "matched".
        assert_eq!(reply_poll_label(true, true), "matched");
        assert_eq!(
            reply_poll_label(true, false),
            "matched",
            "matched 는 poll_ran 과 무관하게 최우선(skipped-no-budget 보다 이긴다)"
        );
    }

    #[test]
    fn reply_poll_label_timeout_when_ran_but_not_matched() {
        // 폴링이 예산을 받아 실제로 돌았지만 예산 소진까지 매치를 못 찾은 경우 = timeout.
        assert_eq!(reply_poll_label(false, true), "timeout");
    }

    #[test]
    fn reply_poll_label_skipped_no_budget_when_not_ran_and_not_matched() {
        // 잔여 예산이 0/음수라 폴링 자체가 안 돌았고, 매치도 없는 경우 = skipped-no-budget.
        assert_eq!(reply_poll_label(false, false), "skipped-no-budget");
    }

    // ── --seed-request/--seed-reply-by(순수 파싱) ─────────────────────────────────────────────────
    #[test]
    fn parse_args_seed_request_flag_is_boolean() {
        // --seed-request 는 값 없는 불리언 플래그(존재 = 켜짐) — 뒤 토큰(--model)을 값으로 삼키지 않는다.
        let a = parse_args(s(&["--seed-request", "--model", "opus"]));
        assert!(a.seed_request, "--seed-request 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--seed-request 뒤 --model 은 정상 파싱");
        assert_eq!(a.seed_reply_by, None);
    }

    #[test]
    fn parse_args_seed_request_absent_is_false() {
        // 플래그 미지정이면 오늘 동작(plain 통보) 유지 — 운영 회귀 0.
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
        assert!(!a.seed_request);
    }

    #[test]
    fn parse_args_seed_reply_by_value() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert!(a.seed_request);
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
    }

    #[test]
    fn parse_args_seed_reply_by_does_not_consume_next_flag_as_value() {
        // FIX round-2 #7 과 동일 규율: --seed-reply-by 다음이 또 플래그면 값으로 삼키지 않는다.
        // ★F5 오용 가드(신규)★: 이젠 조용히 넘기지 않고 seed_reply_by_error 에 사유를 남긴다(--b-task
        //   의 parse_args_b_task_next_looks_like_flag_is_an_error 와 대칭).
        let a = parse_args(s(&["--seed-reply-by", "--model", "opus"]));
        assert_eq!(
            a.seed_reply_by, None,
            "다음 토큰이 플래그면 seed_reply_by 값으로 삼키지 않는다"
        );
        assert!(
            a.seed_reply_by_error.is_some(),
            "다음 토큰이 플래그처럼 보이면 오용 에러를 기록해야(F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model 은 정상 파싱돼야(에러 기록이 뒤 파싱을 막지 않는다)"
        );
    }

    // ★F5 오용 가드(신규)★ --seed-reply-by 값 누락/빈 값/공백뿐인 값 — --b-task 오용 가드와 같은 규율.
    #[test]
    fn parse_args_seed_reply_by_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by"]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "값 없이 끝나는 --seed-reply-by 는 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_empty_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", ""]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "빈 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "   "]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "공백뿐인 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_valid_value_has_no_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
        assert_eq!(
            a.seed_reply_by_error, None,
            "유효한 값은 에러를 기록하지 않는다"
        );
    }

    #[test]
    fn seed_reply_by_without_request_is_rejected() {
        // --seed-reply-by 단독 지정(= --seed-request 없이)은 인자 오류다.
        assert!(
            seed_reply_by_without_request_is_invalid(false, &Some("5m".to_string())),
            "seed_request=false 인데 seed_reply_by=Some → 반려"
        );
    }

    #[test]
    fn seed_reply_by_with_request_is_valid() {
        // --seed-request 와 함께면 유효(반려 아님).
        assert!(!seed_reply_by_without_request_is_invalid(
            true,
            &Some("5m".to_string())
        ));
    }

    #[test]
    fn seed_reply_by_absent_is_always_valid() {
        // seed_reply_by=None 이면 seed_request 값과 무관하게 반려 아님(단독 지정이 아니므로).
        assert!(!seed_reply_by_without_request_is_invalid(false, &None));
        assert!(!seed_reply_by_without_request_is_invalid(true, &None));
    }

    // ── --b-task(순수 파싱 + 파일참조 해석) ───────────────────────────────────────────────────────
    #[test]
    fn parse_args_b_task_inline_text() {
        let a = parse_args(s(&["--b-task", "You are on billing. Reply in one line."]));
        assert_eq!(
            a.b_task.as_deref(),
            Some("You are on billing. Reply in one line.")
        );
    }

    #[test]
    fn parse_args_b_task_file_reference_kept_raw() {
        // parse_args 는 순수 토큰화만 한다 — `@` 접두 해석(파일 읽기)은 run() 이 나중에 한다.
        let a = parse_args(s(&["--b-task", "@prompts/experiments/b-task.md"]));
        assert_eq!(a.b_task.as_deref(), Some("@prompts/experiments/b-task.md"));
    }

    // ★F5 오용 가드★ --b-task 는 다른 값-플래그와 달리 "다음이 플래그처럼 보임" 을 조용히 넘기지 않고
    //   인자 오류로 기록한다(parse_args_defaults 의 b_task_error=None 과 대칭 — 여기선 Some 이어야 한다).
    #[test]
    fn parse_args_b_task_next_looks_like_flag_is_an_error() {
        let a = parse_args(s(&["--b-task", "--model", "opus"]));
        assert_eq!(
            a.b_task, None,
            "다음 토큰이 플래그면 b_task 값으로 삼키지 않는다(값은 여전히 미설정)"
        );
        assert!(
            a.b_task_error.is_some(),
            "다음 토큰이 플래그처럼 보이면 오용 에러를 기록해야(F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model 은 정상 파싱돼야(에러 기록이 뒤 파싱을 막지 않는다)"
        );
    }

    #[test]
    fn parse_args_b_task_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--b-task"]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "값 없이 끝나는 --b-task 는 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_b_task_empty_value_is_an_error() {
        let a = parse_args(s(&["--b-task", ""]));
        assert_eq!(a.b_task, None);
        assert!(a.b_task_error.is_some(), "빈 값은 오용 에러를 기록해야(F5)");
    }

    #[test]
    fn parse_args_b_task_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--b-task", "   "]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "공백뿐인 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_b_task_valid_value_has_no_error() {
        let a = parse_args(s(&["--b-task", "You are on billing."]));
        assert_eq!(a.b_task.as_deref(), Some("You are on billing."));
        assert_eq!(a.b_task_error, None, "유효한 값은 에러를 기록하지 않는다");
    }

    #[test]
    fn resolve_b_task_file_path_absolute_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "@C:\\custom\\task.md"
        } else {
            "@/custom/task.md"
        };
        let got = resolve_b_task_file_path(abs, &root).expect("@ 접두 → 파일 참조");
        let expected = if cfg!(windows) {
            "C:\\custom\\task.md"
        } else {
            "/custom/task.md"
        };
        assert_eq!(got, PathBuf::from(expected), "절대 경로는 그대로 통과");
    }

    #[test]
    fn resolve_b_task_file_path_relative_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_b_task_file_path("@sub/task.md", &root).expect("@ 접두 → 상대 파일 참조");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/task.md") || got.ends_with("sub\\task.md"),
            "상대 경로는 repo 루트 기준 join: {got:?}"
        );
    }

    #[test]
    fn resolve_b_task_file_path_none_for_inline_text() {
        // `@` 접두가 아니면 파일 참조가 아니다 — 호출자는 값을 인라인 텍스트 그대로 쓴다.
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        assert_eq!(
            resolve_b_task_file_path("plain inline text", &root),
            None,
            "@ 접두 없으면 None(인라인 텍스트로 처리)"
        );
    }
}
