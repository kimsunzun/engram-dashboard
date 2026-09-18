//! CodexAppServerTransport — `codex app-server --stdio` 자식 프로세스용 [`AgentTransport`] 구현.
//!
//! ★이 파일이 소유하는 것★: 자식 프로세스 + Job Object · 단일 stdin writer · 리더(pump) · 나간 요청의
//!   대기표와 id 계수기 · thread id 와 준비 상태 · 게이트되는 입력 큐. 나가는 봉투의 id·메서드·순서를
//!   전부 여기서 쥔다 — 그래서 [`AgentTransport::send_input`] 이 받는 바이트는 와이어 프레임이 아니라
//!   **메시지 본문**이다([`crate::backend::InputEncoder::TransportFramed`]).
//!
//! ★스레드 셋과 그들 사이의 벽★:
//!   - **리더**(= pump, [`AgentTransport::start`] 가 띄운다) — decoder 를 `&mut` 로 배타 소유한다.
//!     ★stdin 락을 절대 잡지 않고 대기표를 기다리지도 않는다★. 답해야 할 줄이 오면 **outbox 에 넣고
//!     즉시 돌아간다.** 어기면 읽기가 멈추고, 읽기가 멈추면 상대 큐가 차고, 상대가 stdout write 핸들을
//!     안 닫아 **EOF 가 영영 안 온다** — 프로세스는 살아 있고 신호는 하나도 없다.
//!   - **라이터**(자기 자신) — stdin 락을 블로킹 write 내내 쥐는 **유일한** 스레드이고, 제어 큐를 비우는
//!     **유일한** 스레드이기도 하다. 핸드셰이크도 여기서 돈다(그동안 유저 입력은 큐가 붙든다). ★그
//!     기다림 중에도 제어 큐는 계속 비운다★ — 통째로 park 하면 리더가 넣은 거절이 못 나가고, 상대가 그
//!     답을 기다리는 중이었다면 양쪽이 시한까지 서로를 기다린다.
//!   - **stderr drain** — 파이프가 차서 자식이 멈추는 것을 막는다.
//!
//! ★락 순서 = 상태 → 대기표★(ADR-0006). 역방향은 없다: [`Pending`] 은 자기 락을 쥔 채 밖을 부르지
//!   않는다. 상태 락을 쥔 채 stdin 락을 잡는 자리도 없다.
//!
//! ★teardown 은 오늘 인과 그대로다(ADR-0001 2 동사)★ — [`AgentTransport::shutdown`] 안에 기다림을
//!   더하지 않고, ★**그 경로에서는** stdin 을 kill 보다 먼저 닫지 않는다★(사유 정본 = `transport/stdio.rs`
//!   의 같은 자리). ★**이것은 파일 전체 규칙이 아니라 teardown 경로의 규칙이다 — 양방향으로 오해하지 말 것**★:
//!   ① 핸드셰이크가 실패한 갈래에서는 [`writer_loop`] 이 **kill 없이** stdin 을 닫는다 — ★기록 실패뿐
//!   아니라 **왕복 실패(거절 포함)에서도** 닫는다★(그 자리 주석이 왜 거기서 안전한지의 정본). 그 존재를
//!   이 문장으로 되돌리지 말 것. ② 반대로 그 새 자리를 근거로
//!   `shutdown()` 의 순서를 뒤집지도 말 것 — 두 자리가 안전한 이유가 **다르고**, 그쪽 순서는
//!   [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`] 가 지키는 실제 회귀다.
//!   라이터 스레드는 아무도 join 하지 않고, ★닫힘 표식(`State::closed`)을 보고 스스로 끝난다 — 그 표식을
//!   세우는 자리가 **둘**이다★: `shutdown()`(우리가 죽였다)과 [`ReaderExit`] 의 `Drop`(상대가 스스로
//!   끝났거나 리더가 panic 했다 — `Drop` 이라 unwind 도 반드시 지난다). 앞의 경우 자식을 죽이면 파이프가
//!   깨져 블록된 write 가 에러로 풀린다. ★그 대가 = 큐에 남은 미전송 입력이 조용히 사라진다★.
//!
//! ★알려진 한계 — 이 목록을 줄이지 말고, 고칠 때 같이 지울 것★:
//!   - **턴 자체에는 시한이 없다.** 시한이 걸리는 것은 `turn/start` **응답**뿐이라, 턴이
//!     `turn/completed` 없이 `error` 알림만 남기고 끝나면 게이트가 영영 안 풀린다. 그 뒤의
//!     [`AgentTransport::send_input`] 은 그것을 "큐가 찼다" 로 신고한다 — 사유가 어긋난 신고다.
//!   - **비-Windows 에는 손자를 거두는 수단이 없다.** Job Object 도 프로세스 그룹도 안 쓴다.
//!   - [`AgentTransport::interrupt`] 는 닫힘 표식을 안 본다 — 닫히는 찰나에 버려질 줄 하나에 `Ok` 를
//!     돌려주는 창이 있다.
//!   - **이력 복원은 통째로 best-effort 다** — 거절·시한·줄 상한 어느 것이든 결말은 「거기까지만
//!     복원한다」이고 연결은 그대로 선다([`hydrate_history`]). 그래서 **얼마나 복원됐는지는 로그에만
//!     남고 화면에는 표시가 없다** — 사람은 짧아진 화면을 「원래 이만큼이었다」로 읽는다.
//!   - ★**복원된 이력과 라이브 줄이 섞일 수 있다 — 그것을 막는 것은 구조가 아니라 실측이다**★:
//!     리더는 핸드셰이크 중에도 돌고 있어서, 상대가 그 창에서 알림을 흘리면 그 줄이 이력과 섞인다.
//!     ★★섞이는 축이 **둘**이고 닫힌 것은 하나뿐이다 — 「쪼개짐은 닫혔다」로만 적으면 거짓이다★★:
//!     ① **링 안의 순서**는 닫혔다 — [`OutputCore::emit_batch_without_turn_observation`] 이 이력 전량을
//!        한 락 구간에서 싣는다. 그래서 **나중에 붙는** 구독자는 언제나 온전한 이력을 replay 로 받는다.
//!     ② **이미 붙어 있는 구독자에게 가는 배달 순서는 닫히지 않았다** — 그 문은 링 락을 놓고 나서
//!        보내므로, 그 사이의 라이브 emit 이 [배치 seq N] → [라이브 seq N+k] → [배치 seq N+1] 로 끼어들
//!        수 있다. ★그리고 프론트는 그것을 **재정렬이 아니라 폐기**로 처리한다★ — `RichSlot` 의 구독
//!        콜백이 `chunk.seq <= lastSeq` 를 버리므로, 끼어든 뒤의 이력 프레임이 **화면에서 사라진다**.
//!     즉 오늘 「앞에 선다」와 「배달 중 쪼개진다」 둘 다 열려 있고, 실제로 그것을 막고 있는 것은
//!     아래 실측뿐이다. 두 축을 함께 닫는 길은 하나다 — 게이트가 열릴 때까지 **리더가 emit 을 유계로
//!     붙드는 것**. 그 큐의 상한과 넘쳤을 때의 처분(조용한 유실)을 정하는 것이 선결이라 별건이다.
//!     ★봉쇄가 무엇인지 정확히 적는다 — 그것이 이 항목의 요점이다★: ADR-0203 이 이어받기 뒤 `item/*`
//!     알림을 **0 건**으로 실측했고(0.154.0), 턴을 여는 자리가 [`Link::Ready`] 를 요구해 우리 쪽 턴도
//!     아직 없다. ★즉 **벤더 행동 관측**이지 구조적 보장이 아니다★ — codex 는 스스로 업데이트하고 이
//!     프로토콜에는 버전 칸이 없어(이 헤더의 다른 항목이 그 사실을 이미 말한다) 그 0 건이 조용히 바뀔 수
//!     있다. 바뀌면 증상은 「복원된 대화 위에 낯선 줄 하나」이고, 닫으려면 리더가 게이트 전까지 emit 을
//!     **유계로 붙들어야** 한다 — 그 큐의 상한과 넘쳤을 때의 처분(조용한 유실)을 먼저 정해야 하므로
//!     별건이다.
//!     ★그 별건은 **안 만들기로 닫혔다**(ADR-0205) — 이 한계는 열린 채로 남고, 재론 방아쇠도 그 ADR 이 쥔다★.
//!   - 연결이 서지 못하면 큐에 선 입력이 사라진다. **몇 건이 사라졌는지는 로그와 화면 둘 다에 남는다** —
//!     실패 결말의 `detail` 이 그 건수를 싣는다([`writer_loop`] 의 그 갈래). ★한때 여기 「로그에만 남는다
//!     (화면에 오르는 것은 "핸드셰이크 실패" 뿐이다)」로 적혀 있던 것은 낡은 서술이다★ — 건수는 실려 있고,
//!     머리말도 하나가 아니라 둘이다([`HandshakeFailure::headline`] — 프로토콜 왕복 실패와 세션 id 기록
//!     실패를 갈라 적는다).
//!   - pump 는 panic 하는 `thread::spawn` 으로 띄운다(나머지 둘은 실패를 로그로 흡수한다).
//!   - **한 번의 블로킹 쓰기는 무한히 매달릴 수 있다** — 상대가 우리 stdin 을 안 읽으면 `write_all` 이
//!     파이프 backpressure 로 멈추고, 거기서 빠져나오는 길은 `shutdown()` 의 kill 뿐이다. 시한도
//!     조건변수도 그 한 번의 쓰기 안쪽에는 닿지 않는다. ★이것이 이 파일 전체에 걸리는 한계이지
//!     핸드셰이크만의 것이 아니다★.
//!   - **라이터의 panic 봉쇄는 한 자리뿐이다** — 세션 id 기록 호출을 감싼 [`record_session_id`] 의
//!     `catch_unwind` 가 그것이고(pump 에도 따로 있다), **그 밖의 라이터 코드에는 없다.** 봉쇄 밖에서
//!     라이터가 panic 하면 그 시점에 이미 선 상태가 남는다: 핸드셰이크 **전**이면 link 는 `Connecting`
//!     이고 닫힘 표식도 안 서서 [`AgentTransport::send_input`] 이 계속 `Ok` 를 돌려주며 큐를 상한까지
//!     채우고 그 정지를 "큐가 찼다" 로 신고한다(위 첫 항목과 같은 사유 어긋남). 핸드셰이크 **뒤**면 link
//!     는 이미 `Ready` 라 같은 증상이 턴 경로에서 난다. ★어느 쪽이든 닫힘 표식은 안 서므로 pump 도
//!     reaper 도 움직이지 않는다★.
//!   - ★**위 봉쇄는 unwind 빌드에서만 선다**★ — 워크스페이스 루트 `Cargo.toml` 의 `[profile.release]` 가
//!     `panic = "abort"` 라, 릴리스에서는 `catch_unwind` 가 아무것도 잡지 않고 프로세스가 죽는다.
//!   - **응답보다 먼저 온 종료를 붙들어 두는 칸은 [`EARLY_COMPLETION_SLOTS`] 개다**(그 칸에는 turn id
//!     와 **그 줄이 만든 턴 경계**가 함께 실린다 — [`EarlyCompletion`]). 한 응답을 기다리는
//!     동안 그보다 많은 종료가 흘러오면 가장 오래된 것부터 버려지고, 버려진 것이 우리 턴의 것이었다면 그
//!     턴은 다시 끝낼 것이 없어진다. 열려 있는 턴이 하나뿐이라 정상 운용에서는 칸 하나로 충분하지만,
//!     상대가 우리가 연 적 없는 턴의 종료를 흘리면 그만큼 잠식된다. ★그 칸은 턴이 `Idle` 로 돌아가는
//!     자리마다 통째로 비워진다★ — 사유 정본 = [`end_turn_if`] 의 그 줄.
//!   - **Job Object 편입은 spawn **뒤**라, 그 사이에 만들어진 손자는 Job 밖이다.** 편입된 뒤로는
//!     breakaway 가 막혀 있어(`BREAKAWAY_OK`·`SILENT_BREAKAWAY_OK` 둘 다 안 켠다) 트리가 통째로 내려가지만,
//!     그 창에서 태어난 자손은 그 보장 밖이다. ★이 창은 이 통로만의 것이 아니다★ — `pty.rs`·`stdio.rs` 가
//!     같은 모양이고 이 저장소에 `CREATE_SUSPENDED` 는 한 줄도 없다. 고치는 것은 세 통로를 함께 건드리는
//!     별건이다.
//!   - **핸드셰이크가 실패하면 [`writer_loop`] 이 우리 쪽 stdin 을 놓는다 — 갈래를 가리지 않는다.**
//!     ★한때 여기 「자식·리더·라이터는 그대로 남고 매니저가 거둘 때까지 상주한다」로 적혀 있었다. 그것은
//!     낡은 서술을 넘어 **거짓이었다 — 아무도 그 세션을 거두지 않는다**★: 수거를 여는 것은 pump 의
//!     [`OutputCore::finish`] 가 내는 한 메시지이고 reaper 는 그 단독 소비자라(ADR-0019), 리더가 EOF 를
//!     못 보면 그 메시지가 **아예 만들어지지 않는다.** 그래서 그 모양은 「거둬질 때까지 상주」가 아니라
//!     **영구 wedge** 였다 — 화면은 `Running` 인데 입력은 전부 거절, 자식 + Job Object + 스레드 셋이
//!     데몬 수명 내내 붙들린다. 부팅 복원이면 이어받을 수 있는 codex 프로필 수만큼 한꺼번에 선다.
//!     ★그래도 이 통로가 kill 을 부르는 것은 아니다★ — 쓰기 끝을 놓을 뿐이고, 종료 전이는 여전히 pump
//!     단독이다(ADR-0005 그대로, ADR-0001 의 2 동사는 `kill` 핸들러의 것). ★그 wedge 를 실제로 끊는
//!     것은 매니저의 teardown 이다★(아래 「회수」 항목).
//!   - ★**상대가 stdout 만 닫고 계속 사는 경우는 아직 주인이 없다**★ — 리더가 EOF 를 보고 `closed` 를
//!     세워 세션은 수거되는데 자식은 살아 있다. Windows 는 통로가 drop 될 때 Job 핸들이 닫히며
//!     `KILL_ON_JOB_CLOSE` 가 트리를 거두지만, **비-Windows 에는 그 backstop 이 없어 고아가 남는다**
//!     (이 헤더가 이미 적는 「비-Windows 에는 손자를 거두는 수단이 없다」의 부모 판이다).
//!     ★**릴리스 도달성이 갈래마다 다르다 — 「전부 죽은 코드」로도 「전부 돈다」로도 읽지 말 것**★:
//!     세션 id **기록** 실패는 [`record_session_id`] 가 잡은 패닉에서만 나오는데 워크스페이스 루트의
//!     `[profile.release]` 가 `panic = "abort"` 라 릴리스에서는 그 자리에서 프로세스가 죽는다 — 그래서
//!     [`HandshakeFailure::headline`] 의 「세션 id 기록 실패」 머리말과 「이 화신을 쓰지 않는다」 `detail`
//!     은 unwind 빌드에서만 관측된다. **stdin 닫기는 다르다** — 왕복 실패(시한·해독 실패·그리고 상대의
//!     **거절**)로 릴리스에서도 닿는다. ★**서수로 가리키지 말 것**★ — 갈래 순서는 바뀐다.
//!   - **핸드셰이크 실패 뒤의 회수는 첫 고리가 *상대의* 행동이고, 그것이 안 오면 끊는 것은 매니저다** —
//!     stdin 을 놓은 뒤 실제로 세션을 끝내는 것은 「상대가 EOF 를 보고 스스로 exit 한다」이며(실측
//!     41–51ms — ★그 수치는 **기록 실패 갈래**에서 잰 것이고 거절 갈래는 미측정★), 그것이 일어나야
//!     리더 EOF → pump → reaper 가 이어진다.
//!     ★그 전제가 깨져도 이 통로는 **아무것도 죽이지 않는다**★ — 대신 [`Link::Down`] 이
//!     [`AgentTransport::link_state`] 로 즉시 밖에 보이고, 활성화 판정이 그것을 실패로 확정한 뒤 이미
//!     있는 teardown(ADR-0001 2 동사)을 돌린다. ★그 처분을 이 파일로 되가져오지 말 것 — ADR-0199 가
//!     라이터의 `child.kill()`·`TerminateJobObject` 를 명시적으로 금지한다★(한 번 들어왔다가 걷혔다).
//!     ★**남는 구멍**: 그 판정이 **걸려 있지 않은** spawn(Fresh 로 띄운 codex)에서는 아무도 그 `Down` 을
//!     보지 않는다 — 그 갈래는 여전히 주인이 없다.★ codex 는 스스로 업데이트하고 이
//!     프로토콜에는 버전 칸이 없어(이 헤더의 다른 항목이 그 사실을 이미 말한다) 그 행동이 조용히 바뀔 수
//!     있다. **이 항목을 줄이지 말 것** — 고치려면 유계 대기와 그 뒤의 처분을 누가 지는지부터 정해야 한다.
//!   - **세션 id 기록 포트는 라이터 스레드 위에서 동기로 불린다** — 그 콜백이 블록하면 그 스레드가 갇혀
//!     입력도 제어 줄도 나가지 않는다. ★**이것은 잠재적이 아니라 실재한다**★: 조립점(`manager.rs`)이 모든
//!     spawn 에 포트를 넘기고, 그 구현은 프로필 레지스트리 락을 잡아 `agents.json` 을 통째로 다시 쓴다.
//!     즉 이 통로는 핸드셰이크 끝에서 **남의 디스크 쓰기만큼** 멈출 수 있다. 그 동안 제어 줄이 슬라이스당
//!     하나씩만 나가는 위 항목과 곱해져 [`OUTBOX_LIMIT`] 거절 떨굼을 **길게** 만든다.
//!   - **나간 요청의 실제 상한은 `budget + SWEEP_INTERVAL` 이고, 그 시계는 첫 쓰기 *뒤에* 시작한다.**
//!     첫 `recv_timeout` 한 슬라이스가 지나야 시한을 처음 읽고, 그 앞의 요청 쓰기 자체는 유계가 아니다.
//!   - **핸드셰이크 중에는 제어 줄이 슬라이스당 하나씩만 나간다.** 서버 요청이 그보다 빨리 쌓이면
//!     [`OUTBOX_LIMIT`] 에 닿아 거절을 떨구기 시작한다 — 화면에는 보이고 핸드셰이크 시한으로 끝나지만,
//!     그 동안의 의무 누락은 실재한다.
//!   - `turn/completed` 의 귀속은 **상대가 준 turn id 문자열**에 기댄다. 상대가 지금 쓰는 id 를 그대로
//!     되보내면 엉뚱한 턴이 닫힌다 — 우리가 발급한 값이 아니므로 이 층에서 더 셀 수 있는 것이 없다.
//!   - **귀속 못 한 `turn/completed` 는 턴 경계를 못 낸다**([`Reader::note_turn`]). 그래서 상대가 turn id
//!     없는 종료만 보내는 조합에서는 그 턴이 이 통로에서도 사실 계층에서도 열린 채 남는다 — 응답이 이미
//!     왔다면 시한 backstop 도 없다. ★그래도 두 축이 **같은** 판정을 받는 것이 이 모양의 요점이다★: 큐를
//!     쥔 이 층이 다음 입력을 안 받는 동안 화면만 「끝났다」로 그리면 그 불일치가 더 나쁘다.
//!     ★그 「같은 판정」은 **진행 쪽에도 걸려야** 성립한다★ — 종료만 게이트하고 진행을 열어 두면, 열린
//!     턴이 없는 동안 온 `item/*` 한 줄이 사실 계층만 켜 놓고 그 종료는 위 문장대로 막혀 아무도 못 끈다
//!     (통로는 `Idle` = 큐 열림인데 사실 계층은 바쁨). 그 가름을 [`Reader::claims_our_turn`] 이 진다 —
//!     본문은 언제나 화면에 올리되 「턴 중」의 근거로 셀지만 같은 축으로 가른다.
//!
//! tauri import 0.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::decoder::{history_turn_boundary, CodexAppServerDecoder};
use super::protocol::{
    self, method, ClientInfo, Inbound, InitializeParams, InitializeResponse, RequestId,
    SortDirection, Thread, ThreadItemsListParams, ThreadItemsListResponse, ThreadOpen,
    ThreadResumeResponse, ThreadStartResponse, TurnInterruptParams, TurnStartParams,
    TurnStartResponse, UserInput, METHOD_NOT_FOUND,
};
use crate::backend::SessionIdSink;
use crate::output_core::{estimate_cost_bytes, OutputCore, REPLAY_MAX_BYTES, REPLAY_MAX_EVENTS};
use crate::transport::{AgentTransport, LinkResolution, LinkSink, OutputDecoder};
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps, TurnOutcome,
};

#[cfg(windows)]
use crate::platform::JobObjectHandle;

// ── 상수 ──────────────────────────────────────────────────────────────────────
//
// ★아래 값들은 전부 지어낸 것이다 — 이 축을 실측한 적이 없다★. 각 항목이 자기 근거를 진다.

/// 나간 요청 하나가 답을 기다리는 상한.
///
/// ★모든 요청에 예외 없이 건다★(실측 0.154.0): 서버는 자기가 **해독하지 못한 봉투에는 `id` 가 실려
/// 있어도 아무 답도 하지 않는다** — 답이 오는 것은 봉투가 읽힌 뒤의 파라미터 오류뿐이다. 시한이 없는
/// 대기표는 그래서 "언젠가" 가 아니라 **정상 운용 중에** 영구히 매달린다.
/// ★값의 근거★: 관측된 왕복은 `initialize` ≈130ms · `turn/interrupt` ≈18ms 다. 30 초는 그 중 가장 느린
/// 것의 200 배가 넘어 정상 왕복을 끊을 여지가 없고, `thread/start` 가 자기 MCP 자식들을 띄우느라
/// 늦어지는 경우(실측 — 그 자식들은 우리 손자라 Job Object 가 함께 거둔다)도 넉넉히 덮는다. 동시에
/// 사람이 화면 앞에서 "멈췄다" 고 판단하기 전에 오류가 뜬다.
/// ★턴의 길이와 무관하다★ — `turn/start` 의 **응답**은 턴이 끝날 때가 아니라 턴이 열릴 때 온다.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// **연결이 서기까지 전체**의 상한 — 두 왕복 + 기록 호출 + 이력 페이징. ★요청 하나가 아니라 구획
/// 하나에 건다★.
///
/// ★★이력 페이징이 이 예산 **안에서** 도는 것이 계약이다(ADR-0203)★★: 그쪽에 따로 예산을 주면
///   `Ready` 가 최대 두 예산만큼 늦어져 아래 백스톱 계약이 깨지고, 그러면 **화면을 받아 오느라 성공한
///   이어받기가 실패로 판정된다.** 시계를 잡는 자리가 [`writer_loop`] 하나인 것이 그 실물이고,
///   [`tests::the_handshake_and_the_history_share_one_budget`] 가 그것을 잰다.
///   ★대가 = 핸드셰이크가 느렸으면 복원되는 화면이 짧아진다★ — 그 방향으로 기운 것은 의도다.
///
/// ★이 값은 이제 **아무 창에도 맞출 필요가 없다**★ — 활성화 판정은 창을 지켜보지 않고 이 통로가 내는
///   연결 결말을 **그 자리에서** 받는다(사용자 결정 — [`AgentTransport::link_state`]). 즉 상한의 목적이
///   「판정보다 먼저 결말을 내라」에서 **「답하지 않는 상대를 언젠가는 포기하라」** 로 좁아졌다.
///   ★한때 여기 「매니저의 판정 창보다 작을 것」이 계약으로 적혀 있었다 — 그 창이 없어졌으므로 낡았다★.
/// ★그래도 값을 안 올린다★: [`REQUEST_DEADLINE`](30 초)로 되돌리면 두 왕복이 최대 60 초가 되는데,
///   그만큼 기다려서 얻는 것이 없다 — 관측된 왕복은 `initialize` ≈130ms 이고 거절은 **즉시** 온다
///   (실측 — 하위 스레드 `thread/resume` 이 `-32600` 을 곧바로 돌려줬다). 10 초는 그 정상 왕복의 70 배가
///   넘어 느린 기동(npm shim → node)까지 덮으면서, 사람이 「멈췄다」고 느끼기 전에 결말이 난다.
/// ★남은 계약은 하나뿐이다: 매니저의 liveness 백스톱보다 **작을 것**★
///   ([`crate::manager::LINK_RESOLUTION_BACKSTOP`]). 그쪽은 판정이 아니라 **포기**라, 이 값이 더 크면
///   사유 있는 거절이 사유 없는 포기에 덮인다. 그 관계는 같은 crate 안이라 시험대가 직접 잰다.
/// ★★**이 상한은 「응답 대기」에만 걸린다 — 쓰기에는 안 걸린다**★★: 상대가 우리 stdin 을 읽지 않으면
///   [`write_line`] 의 `write_all` 이 파이프 backpressure 로 무한히 매달리고, 그 사이 이 예산은 한 번도
///   평가되지 않는다(핸드셰이크 사이의 `initialized` 알림 쓰기도 같은 자리다). 그 경로에서 링크는
///   `Connecting` 에 영영 멈추고, **그것을 끊는 것은 이 통로가 아니라 위 백스톱과 그 뒤의 teardown 이다.**
///   그 사실이 백스톱이 아직 존재하는 유일한 이유다 — 지우려면 쓰기를 유계로 만드는 것이 먼저다.
/// ★턴 요청은 이 값을 쓰지 않는다★ — [`REQUEST_DEADLINE`] 그대로다.
const HANDSHAKE_BUDGET: Duration = Duration::from_secs(10);

/// 라이터가 **주위를 둘러보는** 주기. 두 곳이 이 값을 쓴다 — 시한 만료 훑기, 그리고 핸드셰이크가 답을
/// 기다리는 동안 제어 줄을 **한 줄씩** 내보내는 것([`request_blocking`]).
///
/// ★둘의 성질이 다르다★: 훑기는 맵 스캔 하나이고, 제어 줄 배수는 **블로킹 쓰기**다. 그래서 이 값은
/// 「한 슬라이스가 이만큼 걸린다」가 아니라 「**막히지 않는 한** 이 간격으로 다시 판단한다」를 뜻한다.
/// 한 번의 블로킹 쓰기가 무한히 매달릴 수 있다는 사실은 이 상수가 아니라 모듈 헤더의 「알려진 한계」가 진다.
/// ★값의 근거★: [`REQUEST_DEADLINE`] 대비 60 분의 1 이라 시한 오차가 판정을 바꾸지 않고, 핸드셰이크 중
/// 서버 요청 거절이 나가기까지의 지연도 그만큼으로 묶인다.
const SWEEP_INTERVAL: Duration = Duration::from_millis(500);

/// 게이트 없이 나가는 제어 줄(서버 요청 거절 · `turn/interrupt`)의 대기 상한.
///
/// ★근거★: 이 줄들은 턴 상태와 무관하게 바로 나가므로 상대가 stdin 을 읽는 한 쌓이지 않는다. 쌓인다면
/// 상대가 **읽기를 멈춘 것**이고, 그때 이 상한이 "신호 없는 정지" 를 관측 가능한 거절로 바꾼다. 64 는
/// 그 정지를 짧은 시간 안에 드러낼 만큼 작고, 승인 요청이 몰리는 순간을 흡수할 만큼 크다.
const OUTBOX_LIMIT: usize = 64;

/// 이력 한 페이지에 요청하는 item 수.
///
/// ★★이 값은 **줄 크기를 못 잡는다 — 그렇게 읽지 말 것**★★(실측): `limit` 이 자르는 것은 개수이고
///   한 item 의 바이트는 상대가 정한다. 실측 표본에서 `limit:50` 한 페이지가 1,823,509B 였고,
///   `limit:1` 에서도 가장 큰 페이지가 437,196B 였다(`commandExecution.aggregatedOutput` 하나).
///   즉 어떤 `limit` 도 [`MAX_LINE_BYTES`] 아래를 보장하지 못한다.
/// ★그래서 이 값이 재는 것은 **그 상한을 넘길 확률과 왕복 수의 맞바꿈**이다★ — 25 는 관측된 최대
///   페이지(1.8MB@50)의 절반쯤이라 줄 상한까지 4 배 남고, 2MiB 링을 몇 왕복 안에 채운다.
/// ★넘겨서 줄이 버려지면 그 요청은 시한까지 답을 못 받고, 복원은 **거기까지만** 하고 끝난다★ —
///   조용하고 무해하다(그 갈래의 처분은 [`hydrate_history`] 가 진다).
const HISTORY_PAGE_LIMIT: u32 = 25;

/// 이력 페이징이 도는 최대 왕복 수 — **폭주 방지 backstop 이지 정책이 아니다**.
///
/// ★정상 종료는 셋이다★: 커서 소진 · 링을 채움 · 핸드셰이크 예산 소진. 이 값이 발화하는 것은 상대가
///   끝나지 않는 커서를 계속 주는 경우뿐이고, 그때도 예산이 먼저 끊는 것이 보통이다.
/// ★값의 근거★: [`HISTORY_PAGE_LIMIT`] 와 곱하면 6400 item 으로 링의 건수 천장(4096)을 넘는다 —
///   즉 이 상한 때문에 실을 수 있는 것을 못 싣는 일이 없다.
const MAX_HISTORY_PAGES: usize = 256;

/// 아직 못 보낸 유저 턴의 대기 상한. 넘으면 그 호출이 `Err` 로 돌아간다(ADR-0190 — 버리지 않는다).
///
/// ★근거★: 한 칸이 완결된 유저 턴 하나다. 큐가 서는 창은 핸드셰이크(≈130ms)와 진행 중인 턴 하나뿐이라,
/// 32 칸이 차 있다는 것은 사람이 답을 하나도 못 본 채 32 턴을 밀어 넣었다는 뜻이다 — 그 지점에서는
/// 더 받는 것보다 거절하는 쪽이 정직하다.
const INPUT_QUEUE_LIMIT: usize = 32;

/// 리더가 한 번에 읽는 바이트.
const READ_BUF_BYTES: usize = 4096;

/// 한 줄이 쓸 수 있는 최대 바이트. 넘긴 줄은 **그 줄만** 버리고 다음 개행부터 복구한다.
///
/// ★이 값은 관측에서 유도한 것이 아니다★ — 관측된 최대치(4KB 대의 오류 본문)는 **하한이 무엇이면
/// 안 되는지**만 말해 주고 상한을 정해 주지 않는다. 이것이 재는 것은 정상 줄의 크기가 아니라
/// **개행을 안 보내는 상대가 이 버퍼 하나로 가져갈 수 있는 메모리**이고, 그 축에서 4MiB 는 세션 하나가
/// 실수로 물 수 있는 양으로는 눈에 띄지 않고 상대가 메모리를 고갈시키기에는 턱없이 작다는 자리다.
/// ★어긋나면 어느 쪽으로 틀리나★: 너무 크면 그 한 줄이 늦게 잡히고, 너무 작으면 정상 줄이 버려져
/// 화면이 빈다 — 그래서 관측된 최대치의 천 배 쪽으로 기울였다.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 로그·화면으로 옮기는 상대 문자열 상한(문자 수).
///
/// ★근거★: 위 4KB 오류 본문이 자르지 않으면 로그 한 줄을 통째로 덮는다. 512 자면 메서드 이름과 사유
/// 첫 문장이 남는다.
const LOG_STRING_LIMIT: usize = 512;

/// `initialize` 에 싣는 클라이언트 이름. 상대는 이 값을 자기 로그·`user_agent` 에 적는다.
const CLIENT_NAME: &str = "engram-dashboard";

/// 응답보다 먼저 온 종료 알림의 turn id 를 붙들어 두는 칸 수.
///
/// ★근거★: 한 번에 열려 있는 턴은 하나뿐이라 **정상 운용에서 필요한 칸은 하나**다. 나머지는 상대가
/// 우리가 연 적 없는 턴의 종료를 흘릴 때를 위한 여유이고, 이 값이 그 여유의 상한이다. 넘으면 가장 오래된
/// 것부터 버린다 — 늦게 온 것일수록 지금 기다리는 턴의 것일 가능성이 높다.
const EARLY_COMPLETION_SLOTS: usize = 8;

/// 붙들어 둘 turn id 의 최대 바이트. ★자르지 않고 **거른다**★ — 이 값은 사람이 읽는 관측 키가 아니라
/// 나중에 **같은지 대조할 토큰**이라, 잘라 보관하면 서로 다른 긴 id 둘이 같은 것으로 읽힐 수 있다.
/// 관측된 id 는 UUIDv7 문자열(36 바이트)이고 128 은 그 여유분이다.
const MAX_TURN_ID_BYTES: usize = 128;

// ★오류 코드로 분기하는 자리가 없는 것은 의도다 — 재시도 백오프 상수도 그래서 없다★.
//   오류 응답은 코드가 무엇이든 그 요청의 대기자를 깨우므로 조용히 멎지 않는다. 그 위에 「특정 코드면
//   같은 요청을 다시 보낸다」를 얹지 않은 이유는 셋이다: ① 과부하 코드가 실제로 이 봉투로 오는지
//   미검증이다(스키마에 `-32xxx` 대역이 0 회이고, 과부하·레이트리밋은 `error` **알림**의
//   `codexErrorInfo` 로 온다) ② 우리가 내는 세 요청은 재시도가 위험하다 — `initialize` 는 한 번만
//   보낼 수 있고(실측: 두 번째는 오류), `thread/start` 재시도는 스레드를 둘 만들고 `thread/resume`
//   재시도도 같은 스레드를 두 번 여는 요청이며, `turn/start`
//   재시도는 상대가 이미 받은 턴을 한 번 더 연다 ③ 그래서 값을 고르려면 실 서버에서 그 코드를 보는 것이
//   먼저다. ★다시 열 때 필요한 것★ = 어느 요청에 어떤 코드가 언제 오는지의 관측.

/// 턴이 끝났다는 알림 — ★이 통로의 상태 기계 입력이다★. 큐 해제는 턴이 끝났다는 사실을 알아야 하므로
/// 봉투를 분류하는 이 층이 자기 몫으로 읽는다.
///
/// ★같은 줄을 번역기도 읽어 턴 경계(`TurnEnd`)로 옮긴다 — 두 독자는 하는 일이 다르다★: 그쪽은
/// 「이 줄이 무슨 결말을 말하나」를 번역하고(codex 어휘 → 중립 어휘), 이쪽은 「그 결말이 **우리 턴의
/// 것인가**」를 판정한다. ★번역기에는 그 판정에 쓸 재료가 없다★ — 우리 thread·turn id 는 이 층에만
/// 있다. 그래서 번역은 무조건 하되 **그 산출을 화면으로 올릴지는 이 층이 정한다**([`Reader::note_turn`]
/// 의 `boundary` 인자). 그 게이트를 걷으면 남의 턴 종료가 우리 턴을 닫아 이후 출력이 가짜 경계로
/// 쪼개지고, 사실 계층은 한가함을 관측해 턴 도중에 우편을 꽂는다.
///
/// ★`turn/started` 는 **일부러** 읽지 않는다 — 되살리지 말 것★: 그 알림에는 **우리가 발급한 식별자가
/// 하나도 없어서** 어느 턴의 것인지 원리상 귀속시킬 수 없다. 「지금 턴이 답을 기다리는 중인가」 같은
/// 정황으로 대신 가르려 하면, 앞 턴의 늦은 알림이 새 턴의 칸에 자기 id 를 적고 그 뒤로는 회복되지
/// 않는다(그 id 로 온 종료 알림이 살아 있는 턴을 닫고, 큐가 풀려 턴이 겹치고, `interrupt` 가 죽은 턴을
/// 겨눈다). 턴 id 를 정하는 것은 **우리 요청 id 로 짝지어지는 `turn/start` 응답 하나뿐**이다.
///
const TURN_COMPLETED: &str = method::TURN_COMPLETED;

// ── 상태 기계 ─────────────────────────────────────────────────────────────────

/// 연결 축. ★끊기면 이 화신에서 되살아나지 않는다(ADR-0192)★ — 재연결도 재`initialize` 도 없다.
#[derive(Debug)]
enum Link {
    /// 핸드셰이크 전 또는 진행 중. 입력은 거절이 아니라 **큐에 선다**.
    Connecting,
    /// 핸드셰이크 둘째 요청(`thread/start` 또는 `thread/resume`)의 응답을 받고 기록 호출이 돌아왔다 —
    /// `turn/start` 가 여기서부터 허용된다.
    Ready,
    /// 끊겼거나 핸드셰이크가 실패했다. 사유는 새 입력을 거절할 때 그대로 인용한다.
    Down(String),
}

/// 턴 축.
///
/// ★`turn_id` 를 채우는 것은 `turn/start` **응답** 하나뿐이다★ — 그 응답만이 우리 요청 id 로 짝지어져
/// 모호함 없이 귀속된다(알림은 안 쓰는 이유 = [`TURN_COMPLETED`] doc).
/// `seq` = 우리가 턴을 열 때마다 올리는 표식. ★비교는 일치/불일치만★이고, 하는 일은 하나다 — **늦게 온
/// 것이 그 사이에 열린 다음 턴을 건드리는 것을 막는다.** 이 표식이 없으면 한 스레드 위에서 턴이 겹치고,
/// 그것을 막으려고 입력 큐가 존재한다(ADR-0193).
#[derive(Debug)]
enum TurnState {
    Idle,
    Active { seq: u64, turn_id: Option<String> },
}

/// 한 락 아래 사는 것들. ★"보낼 수 있나" 검사와 "진행 중으로 전이" 가 **같은 임계 구역**이어야 한다★
/// (ADR-0193): [`AgentTransport::send_input`] 은 `&self` 라 동시 호출이 구조적으로 가능하므로, 둘을
/// 나누면 두 호출자가 모두 idle 을 보고 각자 턴을 연다. 그 짝짓기가 [`take_turn_locked`] 한 곳에만 있다.
struct State {
    link: Link,
    turn: TurnState,
    thread_id: Option<String>,
    /// 턴 게이트를 안 타는 제어 줄. 라이터가 입력보다 **먼저** 집는다 — 거절 응답이 큐에 선 유저 턴
    /// 뒤에서 기다리면 상대가 그 요청의 답을 영영 못 받는다.
    outbox: VecDeque<String>,
    /// 아직 봉투가 안 된 유저 턴 본문.
    input: VecDeque<Vec<u8>>,
    /// 다음에 열 턴의 표식. 단조 증가만 하고 되감지 않는다.
    next_turn_seq: u64,
    /// ★응답보다 먼저 온 종료 알림★ — 그 시점에는 귀속할 수 없어 붙들어 두고, 뒤이어 오는
    /// `turn/start` 응답이 **우리 턴의 id 를 권위 있게** 알려 줄 때 대조해 끝낸다.
    ///
    /// ★붙드는 것은 「종료가 있었다」가 아니라 **그 id** 다★ — 그 구분이 이 칸이 오귀속이 아닌 이유다.
    ///   끝내는 판정은 여전히 id 일치 하나뿐이고, 여기 있다는 사실만으로 끝나는 턴은 없다.
    early_completions: VecDeque<EarlyCompletion>,
    /// 이 통로는 더 보낼 것이 없다 — 라이터가 이것을 보고 루프를 끝낸다.
    ///
    /// ★세우는 자리가 둘이다★: [`AgentTransport::shutdown`](우리가 죽였다)과 [`ReaderExit`] 의 `Drop`
    /// (상대가 스스로 끝났거나 **리더가 panic 했다** — `Drop` 이라 unwind 도 반드시 지난다). ★둘째를
    /// 빠뜨리면 자연 종료한 에이전트마다 라이터 스레드가 영영 남는다★ — 그 스레드가 core·stdin·대기표의
    /// `Arc` 를 들고 있어 세션 하나치 메모리가 함께 남고, `shutdown()` 은 reaper 경로에서 불리지 않아
    /// 아무도 그것을 깨우지 않는다.
    closed: bool,
}

/// 응답보다 먼저 온 `turn/completed` 한 건 — id 와 **그 줄이 만든 턴 경계**를 함께 붙든다.
///
/// ★경계까지 붙드는 이유★: 이 시점에 그 경계를 화면으로 올리면 아직 귀속되지 않은 종료가 살아 있는
///   턴을 닫는다([`Boundary`]). 그렇다고 버리면 대조가 성립한 뒤에 올릴 것이 없어져 **그 대화의 대기
///   표시가 영영 돈다** — 결말을 알고 있었는데도 그렇다. 그래서 미루기만 한다.
struct EarlyCompletion {
    /// 대조용 토큰. ★마스킹하지 않고 담는 것은 의도다★ — 변형하면 대조가 깨진다. 대신 길이로 거르고
    /// ([`MAX_TURN_ID_BYTES`]), 로그·화면으로 나갈 때 [`sanitize`] 를 지난다.
    turn_id: String,
    /// 그 줄에서 번역기가 낸 턴 경계. `None` = 번역기가 경계를 못 냈다(모양이 깨진 줄·번역기 없음).
    boundary: Option<OutputEvent>,
}

impl State {
    fn new() -> Self {
        State {
            link: Link::Connecting,
            turn: TurnState::Idle,
            thread_id: None,
            outbox: VecDeque::new(),
            input: VecDeque::new(),
            next_turn_seq: 0,
            early_completions: VecDeque::new(),
            closed: false,
        }
    }
}

/// 상태 + 그것을 기다리는 조건변수. 라이터가 여기서 잔다.
///
/// ★조건변수의 `notify` 는 **지연**을 줄이지 정확성을 지지 않는다★ — 라이터는 [`SWEEP_INTERVAL`] 짜리
/// `wait_timeout` 으로 깨어 같은 조건을 다시 보므로, 모든 `notify_all` 을 지워도 동작은 같고 큐가
/// 풀리기까지 그만큼 늦어질 뿐이다. 정확성을 지는 것은 그 타임아웃이다. ★그래서 "알림이 없으면 멈춘다"
/// 로 읽지 말 것★ — 그렇게 읽으면 있지도 않은 보장을 인용하게 된다.
type SharedState = Arc<(Mutex<State>, Condvar)>;

// ── 대기표 ────────────────────────────────────────────────────────────────────

/// 나간 요청 하나의 답을 누가 어떻게 받나.
enum Waiter {
    /// 보낸 스레드가 채널에서 기다린다(핸드셰이크 전용 — 그 스레드는 리더가 아니다).
    Handshake(mpsc::Sender<Result<Value, String>>),
    /// 기다리는 스레드가 없고 **응답이 턴 id 를 준다** — 리더가 받은 자리에서 반영한다.
    ///
    /// ★`seq` 는 이 요청이 연 턴의 표식이다★ — 없으면 늦게 온 답이 **그 사이에 열린 다른 턴**의 칸에
    /// 자기 id 를 적고, 그 뒤의 `interrupt` 가 엉뚱한 턴을 겨눈다.
    TurnStart { seq: u64 },
    /// 실패만 로그로 본다.
    Fire,
}

struct PendingEntry {
    waiter: Waiter,
    deadline: Instant,
    method: &'static str,
}

/// 우리가 낸 요청의 대기표. ★서버가 낸 id 는 여기 절대 들어오지 않는다★(TRD §4-4) — 두 id 공간은
/// 겹칠 수밖에 없고(서버 id 를 그대로 되돌려 줘야 한다), 가르는 것은 id 값이 아니라 **봉투 모양**이다.
/// 조회는 `result`/`error` 봉투에서만 한다.
#[derive(Default)]
struct Pending {
    entries: Mutex<HashMap<i64, PendingEntry>>,
    /// 더는 답이 올 수 없다 — 새 대기표를 받지 않는다. ★이것이 없으면 통로가 닫힌 **뒤에** 걸린 대기표
    /// 하나가 시한이 다 찰 때까지 자기 스레드를 붙든다★(핸드셰이크가 두 요청을 잇달아 내므로 그 사이에
    /// 닫히는 창이 실재한다).
    closed: AtomicBool,
}

impl Pending {
    /// `false` = 이미 닫혀 걸지 못했다.
    fn register(&self, id: i64, waiter: Waiter, method: &'static str) -> bool {
        self.register_at(id, waiter, method, Instant::now() + REQUEST_DEADLINE)
    }

    /// 시한을 인자로 받는 갈래 — ★시험대가 [`REQUEST_DEADLINE`] 만큼 실제로 자지 않고 만료를 재기 위한
    /// seam 이다★(ADR-0012). 운영 호출자는 위 [`Pending::register`] 뿐이다.
    fn register_at(
        &self,
        id: i64,
        waiter: Waiter,
        method: &'static str,
        deadline: Instant,
    ) -> bool {
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        // ★락을 쥔 채 읽는다★ — 밖에서 읽으면 그 사이에 닫힌 표에 대기표가 들어간다.
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        g.insert(
            id,
            PendingEntry {
                waiter,
                deadline,
                method,
            },
        );
        true
    }

    /// 닫고 남은 것을 전부 돌려준다 — 이후 [`Pending::register`] 는 전부 실패한다.
    fn close(&self) -> Vec<PendingEntry> {
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        self.closed.store(true, Ordering::Release);
        g.drain().map(|(_, e)| e).collect()
    }

    /// ★문자열 id 는 우리 것일 수 없다★ — 우리 계수기는 i64 만 낸다. 그래서 여기서 `None` 이 되고
    /// 호출자가 "모르는 id" 로 버린다.
    fn take(&self, id: &RequestId) -> Option<PendingEntry> {
        let key = match id {
            RequestId::Num(n) => *n,
            RequestId::Str(_) => return None,
        };
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        g.remove(&key)
    }

    fn forget(&self, id: i64) {
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        g.remove(&id);
    }

    fn expired(&self, now: Instant) -> Vec<PendingEntry> {
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let ids: Vec<i64> = g
            .iter()
            .filter(|(_, e)| e.deadline <= now)
            .map(|(k, _)| *k)
            .collect();
        ids.into_iter().filter_map(|k| g.remove(&k)).collect()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

// ── 통로 ──────────────────────────────────────────────────────────────────────

pub(crate) struct CodexAppServerTransport {
    /// pump(try_wait)와 shutdown(kill+wait)이 공유. std Child 는 wait 후 status 를 캐시하므로 이중 wait
    /// 이 무해하다.
    child: Arc<Mutex<Child>>,
    /// ★라이터 스레드만 잡는다★ — 리더가 이 락을 잡으면 모듈 헤더의 데드락이 선다.
    /// [`AgentTransport::shutdown`] 의 마지막 단계만 `try_lock`(블로킹 금지)으로 만진다.
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// `start()` 에서 take 해 리더로 move. None 이면 이미 시작됐다.
    stdout: Mutex<Option<ChildStdout>>,
    stderr: Mutex<Option<ChildStderr>>,
    decoder: Mutex<Option<Box<dyn OutputDecoder>>>,
    /// `start()` 에서 take 해 라이터로 move — 핸드셰이크의 둘째 요청이 이 값 그대로 나간다.
    open_params: Mutex<Option<ThreadOpen>>,
    shutdown: Arc<AtomicBool>,
    state: SharedState,
    pending: Arc<Pending>,
    /// 우리 요청 id 계수기. ★서버 id 는 여기 안 들어온다★ — 두 공간은 따로다.
    next_id: Arc<AtomicI64>,
    sid_sink: Option<SessionIdSink>,
    /// 연결의 결말을 **한 번** 배달하는 포트(`None` = 조립점이 안 줬다 = 이 축이 없다).
    /// ★`start()` 에서 take 해 라이터로 move 한다 — 배달은 그 스레드에서만, 정확히 한 번 일어난다★.
    link_sink: Mutex<Option<LinkSink>>,
    /// 라이터 스레드 핸들. ★아무도 join 하지 않는다 — `shutdown()` 안에서 기다리는 것은 계약 위반이다★.
    ///
    /// 이 핸들을 드는 이유는 하나다: **그 스레드가 실제로 끝나는지를 밖에서 볼 수 있어야 한다.** 안 끝나면
    /// 세션 하나치 메모리가 함께 남는데, 그 사실은 관측할 수단이 없으면 어디에도 안 나타난다.
    writer_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 이 통로가 나르는 출력이 구조화 스트림인가. ★주입값이다(ADR-0044/0030/0191)★ — 통로는 자기가
    /// 무엇을 나르는지 모르고, 아는 쪽은 이 모드를 고른 backend 다.
    structured: bool,
    #[cfg(windows)]
    job_handle: Arc<JobObjectHandle>,
}

/// spawn 뒤 실패 경로에서 자식을 확실히 거두는 가드.
///
/// ★왜 필요한가★: `Child` 는 drop 으로 자식을 죽이지 않는다. spawn 뒤의 `?` 하나가 **이미 돌고 있는**
/// 자식을 남긴 채 돌아가면, 그 자식은 아직 Job 에 들어가지도 않아 나중에 아무도 닿을 수 없다.
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn into_inner(mut self) -> Child {
        self.0.take().expect("ChildGuard 는 한 번만 회수된다")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl CodexAppServerTransport {
    /// **pump 는 아직 안 띄운다**(`start` 에서). `child_pid` 를 함께 돌려준다.
    ///
    /// `open_params` = 핸드셰이크의 둘째 요청으로 그대로 나갈 값([`ThreadOpen`]). ★정책도 이어받기
    ///   여부도 통로가 하드코딩하지 않는다★ — 어느 폴더를 워크스페이스로 믿고 어떤 샌드박스·승인
    ///   정책으로 돌 것인가도, 저장된 스레드를 이어받을 것인가도 backend 지식이라 주입받는다.
    /// `sid_sink` = codex 가 발급한 thread id 를 기록할 곳. `None` = 기록할 곳이 없다.
    pub(crate) fn open(
        spec: &CommandSpec,
        structured: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
        open_params: ThreadOpen,
        sid_sink: Option<SessionIdSink>,
        link_sink: Option<LinkSink>,
    ) -> Result<(CodexAppServerTransport, Option<u32>), PtyError> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        cmd.current_dir(&spec.cwd);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd
            .spawn()
            .map_err(|e| PtyError::SpawnFailed(format!("codex app-server spawn: {e}")))?;

        // ★여기부터 모든 조기 반환은 자식을 거두고 나간다★ — 가드가 그것을 진다.
        let mut guard = ChildGuard(Some(child));
        let child_ref = guard.0.as_mut().expect("방금 담았다");
        let child_pid = Some(child_ref.id());
        let stdin = child_ref.stdin.take();
        let stdout = child_ref.stdout.take();
        let stderr = child_ref.stderr.take();

        #[cfg(windows)]
        let job_handle = {
            let job = JobObjectHandle::new()?;
            if let Some(pid) = child_pid {
                job.assign(pid)?;
            }
            Arc::new(job)
        };

        let transport = CodexAppServerTransport {
            child: Arc::new(Mutex::new(guard.into_inner())),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            decoder: Mutex::new(decoder),
            open_params: Mutex::new(Some(open_params)),
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new((Mutex::new(State::new()), Condvar::new())),
            pending: Arc::new(Pending::default()),
            next_id: Arc::new(AtomicI64::new(0)),
            sid_sink,
            link_sink: Mutex::new(link_sink),
            writer_handle: Mutex::new(None),
            structured,
            #[cfg(windows)]
            job_handle,
        };

        Ok((transport, child_pid))
    }
}

/// pump 스레드가 어디서든 panic 하면 그 agent 가 영구 silent 정지하므로 Failed 로 가시화한다.
fn resolve_pump_reason(result: std::thread::Result<TerminalReason>) -> TerminalReason {
    match result {
        Ok(reason) => reason,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            TerminalReason::Error(format!("pump panicked: {msg}"))
        }
    }
}

/// 문자 경계를 지켜 자른다. 잘렸으면 그 사실을 남긴다.
///
/// ★[`sanitize`] 말고 이것을 직접 부르지 말 것★ — 상대 문자열이 마스킹을 건너뛰고 로그·화면으로 나간다.
/// 문이 둘이면 그중 하나는 반드시 잊힌다.
fn clip(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit).collect();
    format!("{head}…(잘림)")
}

/// 외부 프로세스 문자열을 로그·화면으로 옮기기 전 거치는 문 — ★마스킹이 절단보다 먼저다★:
/// `mask_secrets` 의 패턴은 접두 뒤 일정 길이를 요구하므로, 먼저 자르면 경계에 걸친 자격증명이 그
/// 수량자 밑으로 잘려 마스킹을 빠져나간다.
fn sanitize(s: &str, limit: usize) -> String {
    clip(&mask_secrets(s), limit)
}

// ── 라이터 쪽 ─────────────────────────────────────────────────────────────────

fn write_line(stdin: &Mutex<Option<ChildStdin>>, line: &str) -> Result<(), PtyError> {
    let mut guard = stdin.lock().unwrap_or_else(|p| p.into_inner());
    let w = guard
        .as_mut()
        .ok_or_else(|| PtyError::WriteFailed("stdin closed".into()))?;
    w.write_all(line.as_bytes())
        .map_err(|e| PtyError::WriteFailed(e.to_string()))?;
    w.flush().map_err(|e| PtyError::WriteFailed(e.to_string()))
}

/// 제어 큐에 선 줄을 **한 줄만** 내보낸다. 돌려주는 값 = 내보낼 것이 있었나.
///
/// ★한 줄씩인 것이 계약이다★ — 이 함수를 부르는 [`request_blocking`] 은 그 사이사이에 시한을 다시
/// 재야 한다. 전부 비우게 두면 그 한 번의 호출이 무한정 길어져 시한이 **평가되지 않는다**.
/// ★락을 쥔 채 쓰지 않는다★ — 꺼낸 뒤 놓고 쓴다(ADR-0006).
fn drain_one_control_line(stdin: &Mutex<Option<ChildStdin>>, state: &SharedState) -> bool {
    let line = {
        let (lock, _) = &**state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        s.outbox.pop_front()
    };
    match line {
        Some(l) => {
            if let Err(e) = write_line(stdin, &l) {
                tracing::debug!("codex app-server 제어 줄 쓰기 실패: {e}");
            }
            true
        }
        None => false,
    }
}

/// 요청 하나를 내고 답을 **기다린다**. ★리더 스레드에서 부르면 안 된다★ — 기다리는 동안 읽기가 멈춘다.
///
/// ★기다리는 동안에도 제어 줄을 내보낸다★: 이 함수를 부르는 것은 라이터이고, 라이터는 그 큐를 비우는
///   **유일한** 스레드다. 통째로 park 하면 리더가 넣어 둔 서버 요청 거절이 이 대기가 끝날 때까지 못
///   나가고, 상대가 그 답을 기다리는 중이었다면 양쪽이 시한까지 서로를 기다린다(그 뒤 연결은 이 화신에서
///   되살아나지 않는다 — ADR-0192). ★0.154.0 이 실제로 `thread/start` 를 승인 요청 뒤에 두는지는
///   미확인★ — 막는 것은 그 구조적 창이다.
/// ★순서가 계약이다 — **시한을 먼저 재고 그 다음에 한 줄을 쓴다**★: 쓰기는 블로킹이라 먼저 쓰면 그
///   한 번이 매달리는 동안 시한이 평가되지 않는다. 그 순서를 뒤집으면 이 함수는 유계이기를 그만두고,
///   핸드셰이크가 실패하지도 화면에 오르지도 않은 채 입력만 계속 받아들인다.
/// ★그래도 남는 한계★: 한 번의 블로킹 쓰기는 여전히 무한히 매달릴 수 있고, 거기서 빠져나오는 길은
///   `shutdown()` 의 kill 뿐이다(모듈 헤더 「알려진 한계」).
/// `budget` = 이 요청의 시한. 운영 호출자는 전부 [`REQUEST_DEADLINE`] 을 넘긴다 — 인자로 받는 것은
///   ★시험대가 그 상한만큼 실제로 자지 않고 유계성을 재기 위한 seam★이다([`Pending::register_at`] 과
///   같은 사유, ADR-0012).
fn request_blocking<P: Serialize, R: DeserializeOwned>(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    method_name: &'static str,
    params: &P,
    budget: Duration,
) -> Result<R, String> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    if !pending.register(id, Waiter::Handshake(tx), method_name) {
        return Err(format!("{method_name}: 통로가 닫혔다"));
    }

    let line = match protocol::request_line(&RequestId::Num(id), method_name, params) {
        Ok(l) => l,
        Err(e) => {
            pending.forget(id);
            return Err(format!("{method_name} 직렬화 실패: {e}"));
        }
    };
    if let Err(e) = write_line(stdin, &line) {
        pending.forget(id);
        return Err(format!("{method_name} 쓰기 실패: {e}"));
    }

    let deadline = Instant::now() + budget;
    loop {
        match rx.recv_timeout(SWEEP_INTERVAL) {
            Ok(Ok(v)) => {
                return serde_json::from_value(v).map_err(|e| {
                    format!(
                        "{method_name} 응답 해독 실패: {}",
                        sanitize(&e.to_string(), LOG_STRING_LIMIT)
                    )
                })
            }
            Ok(Err(msg)) => return Err(msg),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                pending.forget(id);
                return Err(format!("{method_name}: 대기표가 닫혔다"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // ★쓰기보다 먼저 잰다★ — 위 doc 의 순서 계약.
                if Instant::now() >= deadline {
                    pending.forget(id);
                    return Err(format!(
                        "{method_name}: {}초 안에 답이 없다",
                        budget.as_secs()
                    ));
                }
                drain_one_control_line(stdin, state);
            }
        }
    }
}

/// 핸드셰이크. 순서가 계약이다 — `initialize` → (`initialized`) → **`open` 이 고른 둘째 요청**
/// (`thread/start` 또는 `thread/resume`).
///
/// ★`initialize` 는 한 번만 보낼 수 있다(실측 0.154.0)★ — 두 번째는 오류로 돌아온다. 그래서 이 함수는
/// 화신마다 정확히 한 번 돌고, 실패해도 다시 부르지 않는다(ADR-0192).
/// ★둘째 요청이 거절되면 그대로 `Err` 다 — 새 스레드로 되돌아가지 않는다(ADR-0082)★. codex 쪽 사유가
/// 하나 더 있다: 이 프로토콜의 `-32600` 은 뜻이 하나가 아니다 — 모르는 메서드도, 두 번째 `initialize`
/// 도, 설정 오류도 전부 그 코드로 온다(실측 0.154.0 — 앞 둘의 정본은 이 폴더 `protocol`, 셋째는
/// [`tests::a_peer_error_message_is_masked_before_it_leaves_this_module`] 이 든 실제 응답). 그래서 오류 코드로
/// 「모르는 스레드」를 갈라 새 스레드로 폴백하면, 아직 멀쩡한 손잡이를 무관한 실패에서 덮어쓴다.
fn handshake(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    open: &ThreadOpen,
    deadline: Instant,
) -> Result<Opened, String> {
    // ★시한은 요청마다가 아니라 **구획 전체**에 건다★ — 요청별로 주면 왕복의 합이 상한의 배수가 되고,
    //   그 합이 매니저의 백스톱을 넘기는 순간 이 통로의 실패는 판정에 참여하지 못한다
    //   ([`HANDSHAKE_BUDGET`] 의 doc 이 그 인과의 정본).
    // ★그래서 시한을 **밖에서 받는다**★ — 이 구획 뒤에 이어 도는 이력 페이징([`hydrate_history`])이
    //   같은 시한을 나눠 쓴다. 여기서 `Instant::now()` 를 다시 잡으면 그 둘이 각각 10 초를 갖게 되어
    //   합이 백스톱을 넘고, 이력을 받느라 **성공한 이어받기가 실패로 판정된다.**
    // ★남은 몫이 0 이면 `request_blocking` 이 곧바로 시한 만료로 돌아온다★ — 거기가 유일한 판정 자리라
    //   여기서 미리 갈라 두 곳에서 같은 결론을 내지 않는다.
    let remaining = || deadline.saturating_duration_since(Instant::now());

    let init: InitializeResponse = request_blocking(
        stdin,
        state,
        pending,
        next_id,
        method::INITIALIZE,
        &InitializeParams {
            client_info: ClientInfo {
                name: CLIENT_NAME.to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        },
        remaining(),
    )?;
    // ★이 줄이 상류 드리프트를 사후에 가르는 유일한 기록이다★ — 이 프로토콜에는 버전 칸이 없고
    //   (스키마 전체에 `protocolVersion` 0 회), CLI 는 스스로 업데이트한다.
    tracing::info!(
        user_agent = %sanitize(&init.user_agent, LOG_STRING_LIMIT),
        platform = %sanitize(&init.platform_os, LOG_STRING_LIMIT),
        "codex app-server 연결"
    );

    // 보내는 것이 의무는 아니다(실측 0.154.0 — 안 보내도 `thread/start` 가 성공한다). 해롭지 않아
    //   핸드셰이크 모양을 맞추는 쪽으로 둔다.
    write_line(stdin, &protocol::notification_line(method::INITIALIZED))
        .map_err(|e| format!("initialized 쓰기 실패: {e}"))?;

    // ★두 갈래가 같은 `thread` 를 돌려주고, 그래서 이 함수의 반환은 하나다★ — 이어받기에서도 **상대가
    //   준 id 를 그대로** 올린다. 우리가 보낸 것과 같을 것이라 가정해 되쓰지 않는다: 다르면 그 다른
    //   값이 이 화신이 실제로 말하는 스레드이고, 기록 동사가 그것을 받아야 다음 이어받기가 맞는 곳을 연다.
    let (thread, items_backwards_cursor): (Thread, Option<String>) = match open {
        ThreadOpen::Start(params) => {
            let started: ThreadStartResponse = request_blocking(
                stdin,
                state,
                pending,
                next_id,
                method::THREAD_START,
                params,
                remaining(),
            )?;
            // ★새 대화에는 되돌아갈 이력이 없다 — 응답에 그 칸 자체가 없다★(스키마 0.154.0).
            (started.thread, None)
        }
        ThreadOpen::Resume(params) => {
            let resumed: ThreadResumeResponse = request_blocking(
                stdin,
                state,
                pending,
                next_id,
                method::THREAD_RESUME,
                params,
                remaining(),
            )?;
            (resumed.thread, resumed.items_backwards_cursor)
        }
    };
    tracing::info!(
        cli_version = ?thread.cli_version.as_deref().map(|v| sanitize(v, LOG_STRING_LIMIT)),
        resumed = matches!(open, ThreadOpen::Resume(_)),
        history_cursor = items_backwards_cursor.is_some(),
        "codex thread 개시"
    );
    Ok(Opened {
        thread_id: thread.id,
        items_backwards_cursor,
    })
}

/// 핸드셰이크가 세운 것 — 이 화신이 말하는 스레드와, 그 스레드의 지난 화면으로 들어가는 문.
///
/// ★두 칸이 **다른 등급**이다★: `thread_id` 가 없으면 이 화신은 아무것도 못 한다(핸드셰이크 실패).
///   `items_backwards_cursor` 는 없어도 아무것도 안 깨진다 — 복원할 것이 없거나 새 대화라는 뜻이고,
///   그 부재는 빈 화면으로 시작하는 오늘의 동작 그대로다(ADR-0203: 이력 실패는 조용하고 무해하다).
struct Opened {
    thread_id: String,
    items_backwards_cursor: Option<String>,
}

/// 이어받은 스레드의 지난 화면을 **끝에서부터** 받아 중립 이벤트로 옮긴다(ADR-0203).
///
/// ★★실패가 실패를 만들지 않는다★★: 이 함수는 **어떤 갈래에서도 `Err` 를 내지 않는다.** 커서가 없어도,
///   상대가 거절해도, 줄이 상한을 넘어 답이 안 와도, 예산이 떨어져도 — 결말은 언제나 「거기까지의
///   이벤트」다. 받아 오지 못한 이력은 **빈 화면**이지 실패한 활성화가 아니다(claude 쪽 `.jsonl` 읽기가
///   파일이 없으면 빈 `Vec` 을 돌리는 것과 같은 등급 — ADR-0079).
/// ★★그래서 이 호출은 [`HANDSHAKE_BUDGET`] 을 핸드셰이크와 **나눠 쓴다 — 새 시계를 잡지 않는다**★★:
///   따로 잡으면 `Ready` 가 최대 두 예산만큼 늦어져 매니저의
///   [`crate::manager::LINK_RESOLUTION_BACKSTOP`] 을 넘고, **성공한 이어받기가 「통로가 결말을 못 냈다」로
///   판정된다.** 그 회귀는 화면이 조금 짧아지는 것과 등급이 다르다.
/// ★어디서 멈추나 = 넷★: ① 커서 소진(정상) ② 링을 채웠다 ③ 예산 소진 ④ [`MAX_HISTORY_PAGES`].
///   ②가 이 함수의 천장인 이유는 [`REPLAY_MAX_BYTES`] 가 진다 — 링이 버릴 것을 더 받아 오는 것은 순
///   비용이고, 그 판정 축은 링이 실제로 쓰는 [`estimate_cost_bytes`] 여야 한다(다른 축으로 어림잡으면
///   덜 받아 화면을 잃거나 더 받아 왕복만 버린다).
/// ★방향★: `sortDirection: desc` + resume 이 준 역방향 커서로 **최신부터** 걷는다(스키마 0.154.0 이
///   그 칸의 설명에 그 조합을 직접 적는다). 페이지 안의 `data` 도 최신이 앞이라, 모으는 동안은 역순이고
///   마지막에 한 번 뒤집어 시간순으로 되돌린다(실측 2026-09-16: 첫 페이지 `data[0]` 이 마지막 어시스턴트
///   응답, 마지막 페이지 끝이 최초 `userMessage`).
/// ★번역기를 **이 호출 전용으로 하나 만든다**★ — pump 가 든 것은 `Box<dyn OutputDecoder>` 라 trait 밖
///   문에 닿지 못한다. 갈리는 것은 진단 계수 맵과 되울린 유저 item 의 「먼저 온 쪽」 기억 둘인데,
///   이력에서는 한 item 이 **정확히 한 번** 오므로 그 기억이 할 일이 없고, 계수는 진단 전용이다.
///   ★대가 = 이력에서 본 드리프트와 라이브에서 본 드리프트가 서로 다른 맵에 쌓인다★(둘 다 로그로는
///   나간다 — 관측 키에 메서드 이름이 들어가 어느 쪽인지 구별된다).
// ADR-0203
fn hydrate_history(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    opened: &Opened,
    deadline: Instant,
) -> Vec<OutputEvent> {
    let Some(cursor) = opened.items_backwards_cursor.clone() else {
        // 새 대화이거나 상대가 진입점을 주지 않았다 — 오늘까지의 동작(빈 화면) 그대로다.
        return Vec::new();
    };
    collect_history(&opened.thread_id, cursor, deadline, |params, budget| {
        request_blocking(
            stdin,
            state,
            pending,
            next_id,
            method::THREAD_ITEMS_LIST,
            params,
            budget,
        )
    })
}

/// 위 함수의 **왕복 없는 알맹이** — 페이지를 어떻게 걷고 어디서 멈추고 어떤 순서로 내놓나.
///
/// ★`fetch` 를 주입받는 이유는 하나다: 이 규칙을 자식 프로세스 없이 단독으로 잴 수 있어야 한다★
/// (ADR-0012). 운영에서 그 자리에 들어가는 것은 [`request_blocking`] 하나이고, 시험대는 페이지를
/// 손으로 만들어 넣는다. ★`fetch` 의 `Err` 는 **비-치명**이다★ — 그 지점까지 모은 것을 그대로 낸다.
fn collect_history(
    thread_id: &str,
    entry_cursor: String,
    deadline: Instant,
    mut fetch: impl FnMut(&ThreadItemsListParams, Duration) -> Result<ThreadItemsListResponse, String>,
) -> Vec<OutputEvent> {
    let mut cursor = entry_cursor;
    let mut decoder = CodexAppServerDecoder::new();
    // 최신 → 과거 순으로 쌓는다. 칸 하나 = item 하나가 낸 이벤트들 + 그 item 이 속한 **묶음 번호**.
    // ★아무것도 못 옮긴 item 은 칸을 차지하지 않는다★ — 그래야 턴 경계가 **내용이 있는 턴 사이**에만
    //   서고, 전부 버려진 이력이 경계만 남은 목록이 되지 않는다.
    // ★★키가 원본 id 가 아니라 **번호**인 것은 메모리 상한이다 — 그런데 거르기만으로는 부족했다★★:
    //   `turnId` 에는 와이어 상한이 없어서, 긴 id 를 단 item 이 페이지마다 오면 쌓인 원본 문자열만으로
    //   수백 MiB 가 된다(줄 상한은 페이지 **한 장**에만 걸리고 이 누적에는 안 걸린다). 그렇다고 상한을
    //   넘긴 id 를 전부 `None` 한 칸으로 접으면 **서로 다른 대화 둘이 한 턴으로 렌더된다** — 잃는 것이
    //   「대조가 헐거워진다」가 아니라 「화면이 거짓말을 한다」로 등급이 올라간다.
    //   그래서 축을 둘로 가른다: **구별**은 번호가 지고, **표시**는 거른 id 가 진다.
    //   - 구별: 직전 항목의 원본 id **하나**하고만 비교한다(`last_raw` — 새 값이 들어오면 앞 것은 버려져
    //     언제나 O(1) 이다). 그래서 상한을 넘긴 id 들도 서로 다르면 다른 묶음이 된다.
    //   - 표시: 묶음마다 [`MAX_TURN_ID_BYTES`] 로 **자르지 않고 거른** id 를 하나씩만 든다(`groups`).
    //     자른 둘이 같아지면 서로 다른 턴이 한 턴으로 접히므로 자르지 않는다(`decoder` 의
    //     `MAX_ID_BYTES` 와 같은 판정). 묶음 수는 항목 수 이하라 이쪽도 유계다.
    //   ★남는 대가 = 상한을 넘긴 turn 의 경계에는 **id 가 실리지 않는다**★(경계 자체는 선다).
    let mut newest_first: Vec<(u32, Vec<OutputEvent>)> = Vec::new();
    let mut groups: Vec<Option<String>> = Vec::new();
    let mut last_raw: Option<String> = None;
    let mut cost = 0usize;
    let mut count = 0usize;
    let mut pages = 0usize;
    let mut hit_ceiling = false;

    while pages < MAX_HISTORY_PAGES {
        let budget = deadline.saturating_duration_since(Instant::now());
        if budget.is_zero() {
            tracing::info!("codex 이력 복원: 연결 예산을 다 썼다 — 여기까지만 싣는다");
            break;
        }
        pages += 1;
        let params = ThreadItemsListParams {
            thread_id: thread_id.to_string(),
            cursor: Some(cursor.clone()),
            limit: Some(HISTORY_PAGE_LIMIT),
            sort_direction: Some(SortDirection::Desc),
        };
        let page = match fetch(&params, budget) {
            Ok(page) => page,
            Err(reason) => {
                // ★`warn` 이 아니라 `info` 다★ — 이것은 결함이 아니라 「여기까지만 복원했다」는 사실이고,
                //   연결은 이 뒤로 정상으로 선다. 사유는 그래도 남긴다(상류 드리프트의 첫 신호가 여기다).
                // ★★이 갈래가 「연결이 죽었다」를 **삼킨다는 것을 알고 둔다**★★ — 리더가 EOF 를 보면
                //   대기표가 오류로 깨어나 여기로 오는데, 그 둘을 문자열로 가를 수는 없다. 그래서 그
                //   판정은 여기가 아니라 게이트를 여는 자리가 진다([`open_gate`]) — 거기서 상태를 직접
                //   보므로 문자열 추측이 필요 없다.
                tracing::info!(
                    page = pages,
                    "codex 이력 복원을 여기서 멈춘다 — 화면만 짧아지고 연결은 그대로다: {}",
                    sanitize(&reason, LOG_STRING_LIMIT)
                );
                break;
            }
        };
        let next = page.next_cursor;
        for entry in page.data {
            let translated = decoder.history_item(&entry.turn_id, &entry.item);
            if translated.is_empty() {
                continue;
            }
            let new_run = last_raw.as_deref() != Some(entry.turn_id.as_str());
            let shown =
                (entry.turn_id.len() <= MAX_TURN_ID_BYTES).then_some(entry.turn_id.as_str());
            // ★★천장 판정은 **넣기 전에**, 그리고 **합성할 경계까지 세어서** 한다★★:
            //   ① 넣고 나서 재면 그 한 건만큼 늘 넘긴다. ② 경계를 안 세면 턴이 많은 이력에서 실제 이벤트
            //   수가 여기서 센 것의 최대 두 배가 되어(항목마다 턴이 갈리는 극단) 링이 착지하자마자 절반을
            //   버린다 — 천장을 링 상수에 묶어 둔 이유가 바로 그 어긋남을 없애려던 것이었다.
            // ★경계는 **새 run 마다 하나**다★ — 시간순으로 되돌렸을 때 run 하나당 닫는 경계가 하나이므로,
            //   역순으로 걸으며 run 이 바뀔 때마다 하나씩 세면 총수가 정확히 맞는다.
            let boundary = new_run.then(|| history_turn_boundary(shown));
            let add_events = translated.len() + usize::from(boundary.is_some());
            let add_cost = translated.iter().map(estimate_cost_bytes).sum::<usize>()
                + boundary.as_ref().map(estimate_cost_bytes).unwrap_or(0);
            // ★첫 항목은 무조건 받는다★ — 한 건이 홀로 천장을 넘어도 빈 화면보다는 그 한 건이 낫고,
            //   링도 같은 규칙으로 최신 1 건을 지킨다(`Ring::push` 의 `len() > 1` 가드).
            if !newest_first.is_empty()
                && (cost + add_cost > REPLAY_MAX_BYTES || count + add_events > REPLAY_MAX_EVENTS)
            {
                hit_ceiling = true;
                break;
            }
            count += add_events;
            cost += add_cost;
            if new_run {
                groups.push(shown.map(String::from));
                // ★원본은 **직전 것 하나**만 든다★ — 이 대입이 앞 값을 버리므로 누적되지 않는다.
                last_raw = Some(entry.turn_id);
            }
            newest_first.push((groups.len() as u32 - 1, translated));
        }
        if hit_ceiling {
            break;
        }
        match next {
            // ★★같은 커서를 다시 주면 멈춘다★★ — 그렇지 않으면 같은 페이지를 페이지 상한까지 다시 받아
            //   **같은 대화가 여러 벌** 실리고, 그 중복이 진짜 이력을 링에서 밀어낸다. 상한은 루프를
            //   묶지만 이 오염은 못 막는다.
            //   ★★막는 것은 **고정점 하나**뿐이다 — 어떤 순환이 새는지 정확히 적는다★★: 길이 1 인
            //   순환(A→A)만 잡히고 **길이 2 이상은 전부 샌다**(A→B→A, A→B→C→A, …). 그 경우 멈추는 것은
            //   페이지 상한·링 천장·예산이고, 그때까지 **같은 대화가 여러 벌** 링에 실린다 — 화면에
            //   중복이 보이고 활성화가 예산만큼 늦어진다. 「상한이 있으니 무해하다」로 읽지 말 것.
            //   ★안 고치는 이유는 값이 아니라 **비용의 축**이다★ — 더 긴 순환을 잡으려면 본 커서를
            //   기억해야 하는데, 커서는 상대가 길이를 정하는 문자열이라 그 기억이 곧 위 `turnId` 와 같은
            //   누적 비용이 된다. 몇 개를 기억할지에 근거가 없으므로 상수를 지어내지 않는다.
            Some(next) if next == cursor => {
                tracing::warn!(
                    page = pages,
                    "codex 이력 복원: 상대가 같은 커서를 되돌려 줬다 — 중복을 싣지 않고 멈춘다"
                );
                break;
            }
            Some(next) => cursor = next,
            None => break,
        }
    }

    // ★여기서 한 번 뒤집어 시간순으로 되돌린다★ — 링은 넣은 순서가 곧 화면 순서다.
    newest_first.reverse();
    let items = newest_first.len();
    let mut events: Vec<OutputEvent> = Vec::with_capacity(count);
    let mut open_group: Option<u32> = None;
    for (group, translated) in newest_first {
        if open_group != Some(group) {
            if let Some(previous) = open_group {
                events.push(history_turn_boundary(groups[previous as usize].as_deref()));
            }
            open_group = Some(group);
        }
        events.extend(translated);
    }
    // ★마지막 턴도 반드시 닫는다★ — 안 닫으면 복원된 슬롯의 대기 표시가 영영 돈다
    //   ([`history_turn_boundary`] 가 그 인과의 정본).
    if let Some(last) = open_group {
        events.push(history_turn_boundary(groups[last as usize].as_deref()));
    }

    debug_assert_eq!(
        events.len(),
        count,
        "천장 회계가 실제 이벤트 수와 갈렸다 — 경계를 안 센 회귀"
    );
    tracing::info!(
        pages,
        items,
        events = events.len(),
        bytes = cost,
        ring_full = hit_ceiling,
        "codex 이력 복원"
    );
    events
}

/// 게이트를 연다 — ★**이 화신의 연결이 아직 우리 것일 때만**★.
///
/// ★★없으면 죽은 통로가 `Ready` 로 선다 — 그것이 이 함수의 존재 이유다★★: 핸드셰이크가 성공한 뒤
///   이력을 받는 동안 상대가 stdout 을 닫으면 [`ReaderExit`] 의 `Drop` 이 `closed`/[`Link::Down`] 을
///   세우고 대기표를 오류로 깨운다. 이력 쪽은 그 오류를 **부분 성공**으로 처리하도록 되어 있어(그것이
///   옳다 — 페이지 하나가 안 온 것과 연결이 끊긴 것은 다른 사건이다), 그대로 두면 그 다음 줄이 방금
///   내려간 상태를 `Ready` 로 덮어쓰고 **이미 죽은 것을 활성화 성공으로 배달한다.**
/// ★그래서 가르는 축은 「무엇이 실패했나」가 아니라 **「지금 연결이 서 있나」**다★ — 상태를 직접 보므로
///   오류 문자열을 해석할 필요가 없다. 페이지가 안 왔을 뿐이면 이 검사는 통과한다.
/// ★`Err` 의 문구는 상태가 들고 있던 사유 그대로다★ — 그 사유는 stdout 의 JSON-RPC 오류나 스트림 종료라
///   콘솔 꼬리에도 stderr 꼬리에도 없다(`LinkResolution::Failed` 의 doc 이 그 인과의 정본).
// ADR-0203
// ADR-0201
fn open_gate(state: &SharedState, thread_id: String) -> Result<(), String> {
    let (lock, cv) = &**state;
    let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
    if let Link::Down(reason) = &s.link {
        return Err(reason.clone());
    }
    if s.closed {
        return Err("연결이 닫혔다".to_string());
    }
    s.thread_id = Some(thread_id);
    s.link = Link::Ready;
    cv.notify_all();
    Ok(())
}

enum Job {
    /// 게이트를 안 타는 제어 줄.
    Line(String),
    /// 큐에서 꺼낸 유저 턴 — 쓰기 전에 대기표를 먼저 건다. `seq` = 이 Job 이 연 턴의 표식.
    Turn { id: i64, seq: u64, line: String },
    /// 이번 깨어남은 시한 훑기뿐이다.
    Sweep,
}

/// ★"보낼 수 있나" 와 "진행 중으로 전이" 가 한 락 아래서 붙는 유일한 자리★(ADR-0193).
fn take_turn_locked(s: &mut State, next_id: &AtomicI64) -> Option<Job> {
    if !matches!(s.link, Link::Ready) || !matches!(s.turn, TurnState::Idle) {
        return None;
    }
    let thread_id = s.thread_id.clone()?;
    let body = s.input.front()?;
    let params = TurnStartParams {
        thread_id,
        input: vec![UserInput::Text {
            text: String::from_utf8_lossy(body).into_owned(),
        }],
    };
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    match protocol::request_line(&RequestId::Num(id), method::TURN_START, &params) {
        Ok(line) => {
            s.input.pop_front();
            let seq = s.next_turn_seq;
            s.next_turn_seq += 1;
            s.turn = TurnState::Active { seq, turn_id: None };
            Some(Job::Turn { id, seq, line })
        }
        Err(e) => {
            // 우리 타입은 직렬화가 실패할 수 없지만 계약상 열려 있다. 여기서 본문을 버리면 조용한
            //   유실이라, 큐에 그대로 두고 다음 깨어남에 다시 시도한다.
            tracing::warn!("turn/start 직렬화 실패 — 이 본문은 큐에 남는다: {e}");
            None
        }
    }
}

fn next_job(state: &SharedState, next_id: &AtomicI64) -> Option<Job> {
    let (lock, cv) = &**state;
    let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if s.closed {
            return None;
        }
        if let Some(line) = s.outbox.pop_front() {
            return Some(Job::Line(line));
        }
        if let Some(job) = take_turn_locked(&mut s, next_id) {
            return Some(job);
        }
        let (guard, timeout) = cv
            .wait_timeout(s, SWEEP_INTERVAL)
            .unwrap_or_else(|p| p.into_inner());
        s = guard;
        if timeout.timed_out() {
            return Some(Job::Sweep);
        }
    }
}

/// `seq` 가 **지금 진행 중인 바로 그 턴**일 때만 끝내고, 큐를 풀고, 그 턴의 **경계를 낸다**.
///
/// ★표식을 안 보고 끝내면 늦게 온 신호가 다음 턴을 닫는다★ — 그러면 턴 둘이 동시에 열려 입력 큐가 막고자
/// 하는 바로 그 상태가 된다. 돌려주는 값 = 실제로 끝냈나.
///
/// ★경계를 내는 것이 선택이 아니라 이 함수의 일부인 것이 요점이다★(ADR-0127): 이 경로로 끝나는 턴에는
///   `turn/completed` 가 **오지 않는다**(쓰기 실패·응답 해독 실패·오류 응답·시한 만료). 그런데 사실
///   계층을 「턴 중 아님」으로 되돌리는 유일한 입력이 턴 경계 신호라, 경계 없이 상태만 되돌리면 그
///   화신은 **한가한데도 턴 중으로 관측된 채** 남아 30 분 fail-open 밸브가 쓸어 갈 때까지 우편이 막힌다.
///   ★그래서 [`OutputEvent::Error`] 한 줄로 대신하지 않는다★ — 그 어휘는 「종료 아님」이라 분류자가
///   턴 신호로 세지 않는다(그것이 바로 위 결함의 기전이었다).
/// ★사유를 경계 **안에** 싣고 따로 오류 줄을 앞세우지 않는다★ — 쪼개면 「턴이 끝났다」와 「어떻게
///   끝났다」가 두 이벤트로 갈려 소비자에게 순서 계약이 하나 더 생긴다(같은 판정을 번역기의
///   `turn/completed` 자리가 이미 했다).
/// ★이 경로의 결말은 언제나 `Failed` 다★ — 네 호출자 전부 「우리가 이 턴을 포기한다」이고, 그중 어느
///   것도 상대가 말해 준 결말이 아니다. 상대가 말해 준 결말은 번역기를 지나 [`Reader::note_turn`] 이
///   귀속한다.
fn end_turn_if(state: &SharedState, core: &OutputCore, seq: u64, detail: String) -> bool {
    let ended = {
        let (lock, cv) = &**state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        match &s.turn {
            TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                let turn_id = turn_id.clone();
                s.turn = TurnState::Idle;
                // ★붙들어 둔 종료는 이 턴과 함께 버린다★: 그 칸은 **열려 있던 그 턴**의 id 를
                //   기다리는 것뿐이라, 턴이 다른 길로 끝나면 영영 대조될 일이 없다. 남겨 두면 다음
                //   턴이 그 id 를 재사용하는 순간 [`Reader::resolve`] 가 갓 열린 턴을 그 자리에서
                //   닫고 낡은 경계를 올린다. 턴은 언제나 `Idle` 에서만 열리므로([`take_turn_locked`])
                //   idle 자리마다 비우면 어느 칸도 다음 턴으로 넘어가지 않는다.
                s.early_completions.clear();
                cv.notify_all();
                Some(turn_id)
            }
            _ => None,
        }
    };
    match ended {
        // ★락을 놓은 뒤에 emit 한다★(ADR-0006) — 구독자는 이 호출 안에서 임의 코드를 돈다.
        Some(turn_id) => {
            core.emit(OutputEvent::TurnEnd {
                turn_id,
                outcome: TurnOutcome::Failed {
                    detail: Some(detail),
                },
            });
            true
        }
        None => false,
    }
}

fn sweep_deadlines(state: &SharedState, pending: &Pending, core: &OutputCore) {
    for entry in pending.expired(Instant::now()) {
        let reason = format!(
            "{}: {}초 안에 답이 없다",
            entry.method,
            REQUEST_DEADLINE.as_secs()
        );
        match entry.waiter {
            // ★이 채널은 이미 `recv_timeout` 으로도 깬다★ — 둘 다 서면 먼저 온 쪽이 이기고, 채널이
            //   닫혀 있으면 send 가 조용히 실패한다.
            Waiter::Handshake(tx) => {
                let _ = tx.send(Err(reason));
            }
            Waiter::TurnStart { seq } => {
                tracing::warn!("{reason}");
                if !end_turn_if(state, core, seq, format!("codex app-server: {reason}")) {
                    tracing::debug!("시한이 지난 turn/start 가 연 턴은 이미 끝났다 — 그대로 둔다");
                }
            }
            Waiter::Fire => tracing::warn!("{reason}"),
        }
    }
}

/// 받은 thread id 를 조립점의 기록 동사에 넘긴다. `Err` = 이 화신의 연결을 세우면 안 된다.
///
/// ★게이트의 정직한 조건은 「디스크에 있다」가 아니라 「기록 호출이 돌아왔다」다★ — 그 포트는 실패를 자기
/// 안에서 로그로 삼키고 호출자를 막지 않으므로, 그 위에 영속성을 주장하면 없는 보장을 인용하게 된다.
/// 포트가 `None` 이면 기록되는 곳도 없다.
/// ★[`Link::Ready`] 보다 먼저 불린다 — 뒤집지 말 것★: `turn/start` 가 허용되는 선이 `Ready` 라, 순서가
/// 뒤집히면 기록되기 전에 그 세션으로 턴이 나가고 포트는 「보내기 전에 불린다」는 계약을 잃는다. 그 계약이
/// 서 있는 덕에 기록됐는지 되묻는 둘째 동사가 없다.
///
/// ★닫힘 울타리는 창을 **좁힐 뿐 닫지 못한다 — 「늦은 쓰기를 막는다」로 읽지 말 것**★. 막는 것은 하나다:
/// **이 스레드가 여기 닿기 전에 이미 관측된 종료**(자식이 답하고 곧장 죽어 리더가 EOF 를 본 경우 ·
/// [`AgentTransport::shutdown`] 이 먼저 돈 경우).
///
/// ★**남는 창을 「몇 개 명령」으로 어림하지 말 것 — 그 크기의 지배항은 락 경합이다**★. 창은 위 상태 락을
/// 놓는 순간부터 **프로필 맵이 실제로 바뀌는 순간**까지이고, 그 사이에 이 스레드는 프로필 레지스트리 락을
/// 기다린다. 그 락은 다른 스레드가 **`agents.json` 전체 재기록이 끝날 때까지** 쥐고 있을 수 있다(ADR-0071
/// 이 저장을 락 안에 둔다). 그래서 이 창 안에 `shutdown()` 도, `join_pump` 의 반환도, reaper 의 수거도
/// 통째로 들어갈 수 있고, 그 뒤에 도착한 쓰기가 **이미 거둬진 세션의 프로필에 sid 를 적고 옛 sid 를
/// 이력으로 민다.** 화신 표식 가드도 이 창을 안 덮는다 — 종료 경로 중 어느 것도 `epoch` 을 건드리지 않아
/// 거둬진 세션과 산 세션의 표식이 **같다**.
///
/// 그 창을 닫으려면 「살아 있나」 판정이 기록과 **한 임계구역**에 들어가야 하는데, 그 임계구역은 프로필
/// 레지스트리 쪽이고 이 통로의 상태 락은 거기까지 들고 갈 수 없다(락 보유 중 디스크 쓰기가 된다 — 모듈
/// 헤더의 락 순서).
///
/// ★`catch_unwind` 가 실제로 사는 값을 부풀리지 말 것 — 「기록이 터져도 안전하다」가 아니다★.
/// 이 포트 아래의 호출 그래프에는 **우리 코드가 만드는 패닉원이 없다**(해독은 `Result` 로 돌아오고,
/// `now_millis` 는 `unwrap_or(0)` 이며, `normalize_hierarchy` 는 인덱싱을 안 하고, 저장소는 IO 오류를
/// 자기 안에서 삼킨다). 닿을 수 있는 것은 **poison 가드**(`expect`)들이고 — 레지스트리 맵 락과, 쓰기가
/// 실제로 일어날 때 그 아래에서 잡히는 저장소 락, 서로 다른 모듈에 하나씩 — 어느 쪽이든 서려면 **먼저
/// 다른 패닉이 그 락을 오염시켜 놓았어야** 한다. 그러니 이 봉쇄가 사는 것은 딱 하나다:
/// ★이미 남의 패닉으로 레지스트리가 망가진 상태에서, codex 라이터가 **그 위에 겹쳐 죽지는 않는다**★.
/// ★**게다가 릴리스에서는 이 갈래가 죽은 코드다**★ — 워크스페이스 루트 `Cargo.toml` 의
/// `[profile.release]` 가 `panic = "abort"` 라 [`std::panic::catch_unwind`] 가 아무것도 잡지 않고
/// 프로세스가 그대로 죽는다. 즉 `Err` 로 도는 결말은 unwind 빌드(개발·테스트)에서만 관측된다.
/// 그래도 두는 이유 둘 — 그 빌드에서는 실제로 서고, 패닉 전략이 바뀌면 방어가 저절로 산다.
///
/// unwind 빌드에서 이것이 막는 것: 여기서 unwind 가 올라가면 [`Link::Ready`] 도 [`Link::Down`] 도 서지
/// 않아 링크가 `Connecting` 에 멈추고, 그 상태에서 [`AgentTransport::send_input`] 은 닫힘 표식도 `Down`
/// 도 못 보고 **`Ok` 를 돌려주며 큐가 상한까지 찬다** — 에이전트는 Running 으로 보이는데 벙어리가 되고
/// kill 말고는 복구가 없다(ADR-0190 의 「조용히 통과시키지 않는다」가 「조용히 멈춘다」로 무너지는 자리).
fn record_session_id(
    state: &SharedState,
    shutdown: &AtomicBool,
    sid_sink: Option<&SessionIdSink>,
    thread_id: &str,
) -> Result<(), String> {
    let Some(sink) = sid_sink else {
        tracing::debug!(
            "codex thread id 를 받았으나 기록할 곳이 없다 — 조립점이 기록 포트를 주지 않았다"
        );
        return Ok(());
    };

    // ★두 표식을 함께 본다★ — [`AgentTransport::shutdown`] 은 이 원자를 **먼저** 세우고 그 다음에 상태
    //   락을 잡아 `closed` 를 세운다. 그 사이는 비어 있지 않다(그 순간 락은 `send_input`·[`next_job`]·
    //   리더의 거절 경로가 쥐고 있을 수 있다). 하나만 보면 그 구간이 통째로 새는데, 둘을 OR 하면 공짜로 닫힌다.
    let already_ended = shutdown.load(Ordering::Acquire) || {
        let (lock, _) = &**state;
        let s = lock.lock().unwrap_or_else(|p| p.into_inner());
        s.closed
    };
    if already_ended {
        // ★정상 종료라 실패 결말을 내지 않는다★ — `Err` 로 돌리면 이미 끝난 세션에 오류 경계가 하나 더 선다.
        tracing::debug!("codex thread id 를 기록하지 않는다 — 이 화신은 이미 끝났다");
        return Ok(());
    }

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(thread_id))).map_err(|_| {
        tracing::error!("codex 세션 id 기록이 패닉했다 — 이 화신의 연결을 내린다");
        "세션 id 기록이 패닉했다".to_string()
    })
}

/// 연결이 서지 못한 사유 한 건. ★두 칸이 **다른 질문**에 답한다★ — `reason` 은 무엇이 틀어졌나,
/// `while_recording` 은 **어느 단계에서 틀어졌나**다. 뒤쪽은 사람에게 뭐라고 말할지(머리말·`detail`)
/// 만 가른다.
///
/// ★**stdin 처분은 더 이상 이 칸으로 갈리지 않는다**★ — 실패했으면 갈래를 가리지 않고 닫는다
/// ([`writer_loop`] 의 그 자리가 이유의 정본). 한때 이 칸이 「상대가 살아 있는가」를 뜻하며 그 판정을
/// 겸했는데, 그 겸직이 **왕복 실패인데 상대는 사는** 조합(codex 의 `thread/resume` 거절)을 통째로
/// 새게 했다. 그 뜻을 여기에 되돌리지 말 것.
struct HandshakeFailure {
    reason: String,
    /// 왕복 둘은 성공했고 기록 단계에서 넘어졌나. 사람에게 내는 문장만 가른다(위 doc).
    while_recording: bool,
}

impl HandshakeFailure {
    /// 화면·로그의 머리말. ★두 갈래를 같은 문장으로 신고하지 않는다★ — 기록 실패를 「핸드셰이크 실패」로
    /// 적으면 프로토콜 왕복을 의심하게 만들어 분류 비용이 엉뚱한 곳으로 간다(왕복은 성공했다).
    fn headline(&self) -> &'static str {
        if self.while_recording {
            "codex app-server 세션 id 기록 실패(프로토콜 왕복은 성공했다)"
        } else {
            "codex app-server 핸드셰이크 실패"
        }
    }
}

/// 연결의 결말을 배달한다 — ★**많아야 한 번**(zero-or-once)★. 사용자가 껐으면 배달하지 않는다.
///
/// ★kill 로 풀린 핸드셰이크는 「상대가 답했다」가 아니다★: [`AgentTransport::shutdown`] 은 대기표를 닫아
///   핸드셰이크를 `Err` 로 깨우는데, 그것을 그대로 배달하면 **사용자의 취소가 이어받기 실패로 기록되고**
///   그 위에 정리까지 한 번 더 돈다(결함 ③). 그 갈래에서 감독자가 볼 사실은 종점 상태 하나뿐이고,
///   거기엔 「사용자가 끈 것은 실패가 아니다」 규율이 이미 있다.
/// ★`shutdown` 원자를 보는 것으로 충분하다★ — 그 표식은 kill 경로에서 **가장 먼저** 서고(그 함수의 첫 줄)
///   되돌아가지 않는다. 즉 여기서 읽는 것은 래치이지 오가는 셀이 아니다.
/// ★★「많아야 한 번」을 지키는 것은 **호출 자리의 모양**이지 이 함수가 아니다★★ — 정확히 적는다:
///   이 함수는 `&Option<LinkSink>` 를 빌려 볼 뿐 소비하지 않고, 불릴 때마다 배달한다. 한 번으로
///   묶이는 근거는 둘이다 — ① 운영 호출 자리가 [`writer_loop`] 의 **한 `match` 의 두 갈래**뿐이라
///   실행 경로상 하나만 돈다 ② 포트를 쥔 것이 라이터 스레드 하나이고 [`AgentTransport::start`] 의
///   `take` 가 그 스레드가 둘 뜨는 것을 막는다.
///   ★한때 이 자리에 「`take()` 로 포트를 소비한다」로 적혀 있었는데 **그런 줄이 없다**★ — 유일한
///   `take` 는 `start` 에 있고 그것이 막는 것은 두 번째 **호출**이 아니라 두 번째 **스레드**다.
///   호출 자리를 늘리면 이 불변식이 그 자리에서 깨지므로, 늘릴 때 여기를 함께 볼 것.
/// ★★억제되는 것은 **메시지뿐이다 — 채널 소멸은 못 막는다**★★: 이 포트는 라이터 스레드가 단독으로
///   들고 있어서, 배달을 건너뛰고 [`writer_loop`] 가 돌아가면 보내는 끝이 떨어지고 받는 쪽은
///   `Disconnected` 로 깨어난다. 그 신호를 「통로가 결말을 못 냈다」로 읽으면 **억제가 무의미해진다**
///   (사용자의 취소가 도로 이어받기 실패가 된다) — 그래서 받는 쪽이 종료 의도 래치를 **먼저** 본다
///   ([`crate::manager::AgentManager::link_activation_verdict`]). 이 억제만으로는 절반이다.
fn deliver_link(shutdown: &AtomicBool, sink: &Option<LinkSink>, resolution: LinkResolution) {
    if shutdown.load(Ordering::Acquire) {
        tracing::debug!("연결 결말을 배달하지 않는다 — 사용자가 이 화신을 껐다");
        return;
    }
    if let Some(sink) = sink {
        sink(resolution);
    }
}

#[allow(clippy::too_many_arguments)]
fn writer_loop(
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    state: SharedState,
    pending: Arc<Pending>,
    next_id: Arc<AtomicI64>,
    shutdown: Arc<AtomicBool>,
    core: Arc<OutputCore>,
    open_params: ThreadOpen,
    sid_sink: Option<SessionIdSink>,
    link_sink: Option<LinkSink>,
) {
    // ★두 실패를 **갈라서** 든다★ — 이 통로가 말하는 「핸드셰이크」는 왕복 둘만이 아니라 **기록 호출이
    //   돌아오는 데까지**이므로(위 [`record_session_id`] 의 게이트 조건) 둘 다 `Ready` 를 막고 아래 수습을
    //   함께 탄다. 그래도 **뭉치면 안 되는 사실이 하나** 있다: 기록에서 넘어진 갈래는 왕복이 이미 성공한
    //   뒤라 **상대가 살아서 우리 stdin 을 읽고 있는 것이 확정**이다. 왕복 자체가 실패한 갈래에는 그
    //   확정이 없다 — ★없다는 것이 「죽었다」가 아니다★: 거절(`thread/resume` 이 오류로 돌아온 경우)은
    //   상대가 멀쩡히 살아서 낸 답이다.
    //   ★그래서 이 차이가 가르는 것은 **사람에게 뭐라고 말할지 하나뿐**이다★ — stdin 처분은 아래에서
    //   갈래와 무관하게 같다. 그 둘을 다시 묶지 말 것(사유 정본 = 아래 닫기 자리).
    // ★★예산 시계는 **여기서 한 번** 잡는다 — 핸드셰이크와 이력 복원이 그것을 나눠 쓴다★★.
    //   따로 잡으면 연결이 서기까지가 최대 두 예산이 되어 매니저의 백스톱을 넘고, 이력을 받느라
    //   성공한 이어받기가 실패로 판정된다([`hydrate_history`] 의 doc 이 그 인과의 정본).
    let link_deadline = Instant::now() + HANDSHAKE_BUDGET;
    let outcome = match handshake(
        &stdin,
        &state,
        &pending,
        &next_id,
        &open_params,
        link_deadline,
    ) {
        Err(reason) => Err(HandshakeFailure {
            reason,
            while_recording: false,
        }),
        Ok(opened) => {
            match record_session_id(&state, &shutdown, sid_sink.as_ref(), &opened.thread_id) {
                Ok(()) => Ok(opened),
                Err(reason) => Err(HandshakeFailure {
                    reason,
                    while_recording: true,
                }),
            }
        }
    };

    match outcome {
        Ok(opened) => {
            // ★★지난 화면을 **게이트를 열기 전에** 올린다 — 순서를 뒤집지 마라★★(ADR-0203):
            //   게이트가 먼저 열리면 그 순간부터 큐에 선 유저 턴이 나갈 수 있고, 그 턴의 응답이 아직
            //   안 실린 이력보다 **먼저** 링에 들어간다. 그러면 복원된 대화가 새 답 뒤에 붙는다.
            //   ★이 순서가 ADR-0079 의 seed-before-publish 가 지키던 것을 대신 진다★ — 그쪽은 이력이
            //   디스크에 있어서 자식을 띄우기 전에 읽을 수 있었지만, 이쪽 이력은 **핸드셰이크가 선 뒤에야
            //   존재하는 상대에게 요청해서** 받으므로 세션이 명부에 오르기 전으로 당길 수가 없다.
            //   그 대신 ⓐ 게이트 앞이라 우리 쪽 턴이 아직 없고 ⓑ 아래 문이 **fanout 까지 한다**.
            // ★`emit_without_turn_observation` 인 것이 그 ⓑ의 실물이다★ — `seed` 는 링에 넣기만 하고
            //   fanout 을 안 해서, 세션이 이미 명부에 오른 이 자리에서 쓰면 그 사이 붙은 구독자가 지난
            //   화면을 **영영 못 본다**(빈 링을 replay 한 뒤라 되받을 길이 없다). 이 문은 링에도 넣고
            //   붙어 있는 구독자에게도 보내므로 두 부류가 같은 것을 본다.
            // ★그러면서 관측·상태·finalize 는 건드리지 않는다★ — `seed` 가 지키던 그 규율 그대로다
            //   (ADR-0005 · ADR-0113 · ADR-0127). 지나간 기록을 진행 신호로 먹이면 그 턴의 종료가
            //   영영 오지 않아 우편이 30 분 fail-open 까지 막힌다.
            // ★**한 덩이로** 올린다★ — 낱개로 부르면 그 틈마다 리더의 라이브 줄이 seq 를 가져가
            //   복원된 대화 한가운데에 새 줄이 박힌다([`OutputCore::emit_batch_without_turn_observation`]).
            core.emit_batch_without_turn_observation(hydrate_history(
                &stdin,
                &state,
                &pending,
                &next_id,
                &opened,
                link_deadline,
            ));
            // ★★게이트는 **연결이 아직 서 있을 때만** 열린다★★ — 이력을 받는 동안 스트림이 끝났으면
            //   여기서 `Ready` 를 세우는 것이 곧 「죽은 것을 활성화 성공으로 배달」이다([`open_gate`]).
            match open_gate(&state, opened.thread_id) {
                // ★게이트를 연 **직후** 배달한다 — 락을 쥔 채로는 부르지 않는다★(ADR-0006: 상태 락 보유
                //   중 외부 호출 금지. 이 포트는 조립점 코드를 부르고 그쪽은 채널을 만진다).
                Ok(()) => deliver_link(&shutdown, &link_sink, LinkResolution::Ready),
                Err(reason) => {
                    // ★여기서 화면에 오류·경계를 더하지 않는다★ — 스트림이 끝났다는 것은 pump 가 곧
                    //   종점 전이를 낸다는 뜻이고(ADR-0005 단독 주체), 그 경로가 이미 화면을 닫는다.
                    //   사용자 kill 이면 이 배달은 `deliver_link` 가 억제한다(그 함수의 래치).
                    tracing::warn!(
                        "codex app-server: 이력을 받는 동안 연결이 끝났다 — 게이트를 열지 않는다: {}",
                        sanitize(&reason, LOG_STRING_LIMIT)
                    );
                    deliver_link(&shutdown, &link_sink, LinkResolution::Failed { reason });
                }
            }
        }
        Err(failure) => {
            let HandshakeFailure {
                reason,
                while_recording,
            } = &failure;
            tracing::warn!("{}: {reason}", failure.headline());
            let dropped = {
                let (lock, _) = &*state;
                let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
                s.link = Link::Down(reason.clone());
                let n = s.input.len();
                s.input.clear();
                n
            };
            if dropped > 0 {
                tracing::warn!("핸드셰이크 실패로 대기 중이던 입력 {dropped}건이 사라졌다");
            }
            // ★사유를 **들고** 배달한다★ — 이 문구는 stdout 의 JSON-RPC 오류라 콘솔 꼬리에도 stderr
            //   진단 꼬리에도 없다. 여기서 안 실으면 분류가 닿을 길이 아예 없다.
            deliver_link(
                &shutdown,
                &link_sink,
                LinkResolution::Failed {
                    reason: reason.clone(),
                },
            );
            // ★이 경계는 통로 쪽 턴에 대응하지 않는다★: 턴을 여는 유일한 자리([`take_turn_locked`])가
            //   [`Link::Ready`] 를 요구하는데 그 상태는 핸드셰이크가 성공해야 선다. 그래서 큐에 선
            //   `dropped` 건도 통로 쪽에서 보면 **아직 턴이 아니라 큐에 선 본문**이고, 사실 계층도 이
            //   화신을 한 번도 「턴 중」으로 관측한 적이 없다(첫 진행 신호는 상대가 되울리는 item 에서
            //   오고, 합성 입력 에코는 선언하지 않는다 — ADR-0193).
            // ★그래도 **버린 입력이 있든 없든** 실패 결말 하나를 낸다★ — 화면을 닫는 것이 이것뿐이기
            //   때문이다. 프론트는 `Error` 를 턴 종료로 읽지 않고(재시도되는 스트림 오류가 그 어휘로 오기
            //   때문 — [`OutputEvent::Error`] 의 doc), 그래서 오류 하나만 든 슬롯은 **대기 표시가 영영
            //   돈다**. 링크는 이미 [`Link::Down`] 이라 그 뒤에 도착할 것도 없다.
            // ★열린 턴이 없어 실을 id 가 없으니 `turn_id` 는 지어내지 않는다★.
            // ★몇 건을 버렸든 경계는 하나다★ — `dropped` 는 버린 본문 수이지 턴 수가 아니다.
            // ★두 갈래의 사유 문장이 갈리는 것은 의도다★ — 버린 쪽은 사람이 보낸 본문이 파괴됐다고
            //   말하고, 안 버린 쪽은 연결이 서지 못했다고만 말한다. 뒤쪽에 입력 손실을 적으면 일어나지
            //   않은 일을 보고하게 된다.
            // ★사유가 오류 줄과 경계 안에 겹쳐 실리는 것은 의도다★ — 경계 하나만 읽는 소비자도 무슨 일이
            //   있었는지 알아야 하기 때문이고([`end_turn_if`] 와 같은 사유), 그래서 둘 사이에 순서 계약이
            //   새로 생기지는 않는다.
            for entry in pending.close() {
                if let Waiter::Handshake(tx) = entry.waiter {
                    let _ = tx.send(Err(reason.clone()));
                }
            }
            core.emit(OutputEvent::Error(format!(
                "{}: {}",
                failure.headline(),
                sanitize(reason, LOG_STRING_LIMIT)
            )));
            let detail = if dropped > 0 {
                format!(
                    "{} — 보낸 입력 {dropped}건이 사라졌다: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            } else if *while_recording {
                // ★여기서는 「연결이 서지 못했다」가 거짓이다★ — 왕복은 섰고, 우리가 안 쓰기로 한 것이다.
                format!(
                    "{} — 이 화신을 쓰지 않는다: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            } else {
                format!(
                    "{} — 연결이 서지 못했다: {}",
                    failure.headline(),
                    sanitize(reason, LOG_STRING_LIMIT)
                )
            };
            core.emit(OutputEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some(detail),
                },
            });

            // ★**두 갈래 모두**에서 우리 쪽 stdin 을 닫는다★ — 여기까지 왔다는 것은 「이 화신은 쓰지
            //   않는다」가 확정됐다는 뜻이고, 그 결론은 상대의 생사와 무관하게 같다. 닫지 않으면 아래
            //   루프가 `Job::Sweep` 만 물고 영원히 돌면서 자식·리더·라이터를 붙들고, 리더는 EOF 를 못 봐
            //   [`OutputCore::finish`] 가 영영 안 돌아 **종료 전이도 수거도 일어나지 않는다**
            //   (화면은 Running 인데 입력은 전부 거절 — 사람이 kill 할 때까지).
            // ★한때 여기 「기록에서 넘어진 갈래에서만 닫는다 — 그 갈래에만 상대 생존 확정이 있다」로
            //   적혀 있었다. 전제는 참이었고 **결론이 틀렸다**★: 생존 확정은 닫기의 *전제*가 아니다.
            //   상대가 이미 죽었으면 `take()` 는 핸들 하나를 떨구는 것뿐이라 no-op 이고(그 뒤의 쓰기는
            //   어차피 전부 실패한다), 살아 있으면 EOF 를 받는다 — **양쪽 다 안전하다.** 반대로 닫지 않는
            //   쪽에는 안전한 결말이 없다: 왕복이 실패했는데 상대는 멀쩡히 사는 조합이 실재하고, 그것이
            //   ★codex 가 `thread/resume` 을 JSON-RPC 오류로 **거절**하는 경우★다(거절은 stdout 으로 오고
            //   자식은 죽지 않는다). 그 조합이 위 wedge 그대로이며 — ★기록 실패 갈래와 달리 **릴리스에서
            //   닿는다**★(그쪽은 `catch_unwind` 뒤라 `panic = "abort"` 에서 죽은 코드다) — 부팅 복원이면
            //   이어받을 수 있는 codex 프로필 수만큼 한꺼번에 선다.
            // ★그래도 **사유 문장은 갈린 채로 둔다**★ — 바뀐 것은 stdin 의 처분이지 진단이 아니다
            //   ([`HandshakeFailure::headline`] 과 위 `detail`). 왕복 갈래는 연결이 못 섰고, 기록 갈래는
            //   섰는데 우리가 안 쓰기로 한 것이다.
            // ★이 통로가 자기 수명을 스스로 끝내는 것이 아니다★ — kill 은 여전히 안 부른다(ADR-0001 의
            //   2 동사는 kill 핸들러의 것). 여기서 하는 것은 **우리 쪽 쓰기 끝을 놓는 것**뿐이고, 그 다음은
            //   이미 있는 인과가 굴린다: 상대 exit → 리더 EOF → [`ReaderExit`] 의 `Drop` 이
            //   `closed`+`Link::Down` → pump 가 종료 전이(ADR-0005 단독) → reaper 수거.
            //   ★실측 41–51ms 는 **기록 실패 갈래**(턴이 없는 상태에서 닫았을 때)의 수치다 — 거절 갈래에서
            //   상대가 얼마나 빨리 끝나는지는 재 본 적이 없다★. 인과가 같을 뿐 수치를 옮겨 적지 말 것.
            // ★여기서 stdin 락을 잡는 것이 안전한 이유(`shutdown` 의 순서 위험과 다르다)★: 그 위험은
            //   **남의 스레드**가 `write_all` 에 매달린 라이터의 락을 기다리는 모양이다. 여기서 잡는 것은
            //   라이터 자신이고, 이 지점은 루프에 들어가기 전이라 `write_all` 안이 아니다.
            //   ★그 판정은 갈래를 가리지 않는다 — 이번에 `if` 를 걷은 것이 이 근거를 건드리지 않는다★:
            //   근거의 축은 「어느 스레드가 잡나」이지 「어느 실패였나」가 아니고, 취득 **자리**도 하나
            //   그대로다(아래 한 줄). 그래서 개수도 그대로 둘이다.
            // ★**그 안전의 범위를 정확히 적는다 — 「파일 전체」가 아니라 「운영 호출 그래프」다**★:
            //   블로킹으로 이 락을 잡는 자리는 운영 구획에 **둘**뿐이고([`write_line`] 과 바로 아래 이 줄)
            //   [`write_line`] 을 부르는 것은 라이터뿐이다(`shutdown` 은 `try_lock` 이라 세지 않는다).
            //   ★단 **시험 구획은 그 그래프 밖에서 같은 락을 블로킹으로 잡는다**★ —
            //   [`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`] 가 별도 스레드에서
            //   [`write_line`] 으로 8MiB 를 밀어 넣고 kill 이 올 때까지 락을 쥔다. 그런 채움 스레드와 이
            //   갈래를 **같은 시험대에서** 돌리면 여기서 영원히 막히고, 그러면 stdin 이 안 닫혀 이 라운드가
            //   세운 인과가 통째로 사라진다. 오늘 그렇게 조합하는 항목은 없다 — 그 항목은 [`AgentTransport::start`]
            //   를 부르지 않아 라이터 스레드 자체가 안 뜬다(그 사실이 이 예외를 무해하게 만드는 것이고,
            //   `start` 를 더하는 순간 무해가 깨진다).
            //   운영 구획의 그 개수는 [`tests::the_production_blocking_stdin_locks_are_counted`] 가 지킨다 —
            //   셋째가 생기면 위 판정을 다시 해야 하므로 조용히 늘지 않게 막는다.
            let had_stdin = stdin
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .is_some();
            tracing::warn!(
                had_stdin,
                while_recording = *while_recording,
                "연결이 서지 못해 codex 쪽 stdin 을 닫는다 — 상대가 EOF 를 보고 끝나면 세션이 수거된다"
            );

            // ★여기서 끝난다 — 이 통로는 **아무것도 죽이지 않는다**★.
            //   stdin 을 놓으면 상대가 EOF 를 보고 스스로 끝나고(실측 41–51ms, 턴이 없는 상태) 그 EOF 가
            //   리더를 끝내 [`OutputCore::finish`] → reaper 로 이어진다. ★그 첫 고리가 **우리 것이 아니고**,
            //   상대가 stdout 을 열어 둔 채 남으면 그 사슬이 통째로 안 돈다★ — 거절 뒤에도 상대가 EOF 로
            //   끝나는지는 **미검**이다(재 본 것은 기록 실패 갈래뿐).
            // ★그 처분은 **매니저 몫**이다 — 여기서 kill 하지 말 것★: 위 [`Link::Down`] 은
            //   [`AgentTransport::link_state`] 로 즉시 밖에 보이고, 활성화 판정이 그것을 실패로 확정한
            //   뒤 이미 있는 teardown(ADR-0001 의 2 동사)을 돌린다 — 그 kill 이 매달린 리더까지 함께 푼다.
            //   ★한때 이 자리에서 라이터가 직접 `child.kill()`·`TerminateJobObject` 를 불렀는데, 그것이
            //   ADR-0199 가 그은 선(라이터가 그 동사들을 부르는 순간 위반)을 넘는 것이라 걷어냈다.
            //   되살리지 말 것 — 되살리려면 그 ADR 을 먼저 고쳐야 한다.★
            // ★**한때 남아 있던 구멍 — 지금은 닫혔다**★: 예전에는 Fresh 로 띄운 codex 에 판정이 아예
            //   안 걸려서(감독자가 배달함을 버렸다) `thread/start` 거절의 `Down` 을 **아무도 보지
            //   않았다.** 지금은 활성화 입구 둘(`AgentManager::activate_profile` 의 Fresh 갈래 ·
            //   `AgentManager::restore_one` 의 비-이어받기 갈래)이 같은 판정을 돌므로 두 모드 모두
            //   주인이 있다(`AgentManager::spawn_fresh_settled`).
            // ★주인이 없는 갈래가 하나 남지만 **오늘은 여기 닿지 않는다**★: `AgentManager::spawn_agent`
            //   을 직접 부르는 즉석 생성(데몬의 by-cwd 갈래)은 배달함을 그대로 버린다. 그 입구가 codex 에
            //   주는 출력 형식이 **터미널**이라 이 통로 자체가 안 뜨는 것이 그 갈래를 무해하게 만드는
            //   전부다 — 그 기본값이 app-server 로 바뀌는 날 이 구멍이 그대로 열린다.
        }
    }

    // ★핸드셰이크가 실패해도 루프는 돈다★ — 나가는 길이 하나여야 하기 때문이다: 리더가 EOF 를 보아
    //   `closed` 를 세우면 [`next_job`] 이 `None` 을 돌려 스스로 빠져나온다. 여기서 `return` 해 버리면
    //   그 정리 지점이 둘이 된다.
    // ★단 **「상대의 요청에 답하려고 돈다」는 이제 사실이 아니다**★ — 바로 위에서 stdin 을 이미 놓았으므로
    //   그 뒤의 [`write_line`] 은 전부 실패한다. 상대는 답을 기다리는 대신 EOF 를 본다. 옛 문장
    //   (「상대는 살아 있을 수 있고, 그러면 서버 요청에 답할 자리가 필요하다」)은 닫기가 한 갈래에만
    //   있던 시절의 것이다.
    // ★**「상대가 정지하는 일이 없다」를 무조건으로 적지 말 것 — 그만큼은 재 본 적이 없다**★: 재 본 것은
    //   「턴이 없는 상태에서 stdin 을 닫으면 41–51ms 안에 exit 한다」 하나다. 이 헤더가 따로 적는 조건
    //   ——핸드셰이크 동안 상대의 요청이 쌓이고 거절이 [`OUTBOX_LIMIT`] 에서 떨어진다——아래에서는, 닫은 뒤
    //   모든 [`write_line`] 이 실패하므로 **미답 요청이 남은 채로** 상대가 EOF 를 만난다. 그 상태에서도
    //   상대가 그냥 끝나는지는 **미검**이다. 끝나지 않으면 위 「첫 고리가 상대의 행동」 항목 그대로다.
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        sweep_deadlines(&state, &pending, &core);
        match next_job(&state, &next_id) {
            None => break,
            Some(Job::Sweep) => {}
            Some(Job::Line(line)) => {
                if let Err(e) = write_line(&stdin, &line) {
                    tracing::debug!("codex app-server 제어 줄 쓰기 실패: {e}");
                }
            }
            Some(Job::Turn { id, seq, line }) => {
                // ★대기표를 쓰기 **전에** 건다★ — 뒤에 걸면 빠른 응답이 먼저 도착해 "모르는 id" 로 버려진다.
                // ★알려진 잔여★: 여기서 나가면 방금 연 턴이 Active 로 남는다. 대기표가 닫혔다는 것은
                //   통로가 이미 닫혔다는 뜻이라 그 상태를 읽을 소비자가 없지만, 「나가는 길마다 턴을
                //   정리한다」가 성립하지는 않는다.
                if !pending.register(id, Waiter::TurnStart { seq }, method::TURN_START) {
                    break;
                }
                if let Err(e) = write_line(&stdin, &line) {
                    pending.forget(id);
                    // ★이 실패는 호출자에게 돌아갈 길이 없다★ — 그 호출은 이미 `Ok` 를 받고 떠났다.
                    //   남는 것은 이 턴을 실패로 닫는 것뿐이다(정정 채널을 새로 만들지 않는다).
                    end_turn_if(
                        &state,
                        &core,
                        seq,
                        format!("codex app-server 입력 전송 실패: {e}"),
                    );
                }
            }
        }
    }
}

// ── 리더 쪽 ───────────────────────────────────────────────────────────────────

/// 리더가 EOF 를 본 뒤 **종료 코드가 잡힐 때까지** 다시 보는 상한.
///
/// ★EOF 직후의 `try_wait` 한 번으로는 코드를 못 받는다★ — stdout 이 닫히는 것과 프로세스가 수거되는 것은
/// 같은 순간이 아니라, 그 한 번은 대개 `Ok(None)` 으로 떨어지고 코드가 조용히 **유실**된다. 그 유실은 두 번
/// 아프다: 화면에 종료 코드가 안 뜨고, `Exited { code: None }` 이 「자식이 아직 살아 있다」와 구별되지 않아
/// 시험대가 그 둘을 가를 수단을 잃는다.
/// ★값의 근거★: stdout close 와 프로세스 수거 사이는 밀리초 단위다. 2 초는 그 여유를 크게 잡은 것이고,
/// 넘기면 그건 「끝나지 않은 상대」다 — 그 처분은 이 통로가 아니라 매니저의 teardown 이 진다.
const CHILD_REAP_GRACE: Duration = Duration::from_secs(2);

/// 임의 청크를 줄로 자른다. ★완성 줄이 확정되기 전에는 UTF-8 로 읽지 않는다★ — pump 는 문자 경계를
/// 모르는 청크로 던지므로 멀티바이트 문자가 경계에서 잘릴 수 있고, 개행(0x0A)은 UTF-8 연속 바이트로
/// 등장할 수 없어 바이트 레벨 탐색이 안전하다.
struct LineSplitter {
    buf: Vec<u8>,
    /// 상한을 넘긴 줄의 꼬리를 다음 개행까지 통째 버리는 중인가. ★버퍼만 비우면 그 줄의 남은 바이트가
    /// 다음 개행까지 "새 줄" 로 파싱돼 가짜 봉투를 만든다★.
    discarding: bool,
}

impl LineSplitter {
    fn new() -> Self {
        LineSplitter {
            buf: Vec::new(),
            discarding: false,
        }
    }

    fn feed(&mut self, chunk: &[u8], mut on_line: impl FnMut(&[u8])) {
        let mut rest = chunk;
        if self.discarding {
            match rest.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    rest = &rest[nl + 1..];
                }
                None => return,
            }
        }
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let head = &rest[..nl];
            rest = &rest[nl + 1..];
            if self.buf.is_empty() {
                if head.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        bytes = head.len(),
                        "codex app-server: 줄 상한 초과 — 버린다"
                    );
                    continue;
                }
                on_line(head);
            } else {
                let mut line = std::mem::take(&mut self.buf);
                if line.len() + head.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        bytes = line.len() + head.len(),
                        "codex app-server: 줄 상한 초과 — 버린다"
                    );
                    continue;
                }
                line.extend_from_slice(head);
                on_line(&line);
            }
        }
        if !rest.is_empty() {
            // ★붙이기 전에 잰다★ — 붙인 뒤 재면 그 순간 이미 넘긴 뒤다.
            if self.buf.len() + rest.len() > MAX_LINE_BYTES {
                tracing::warn!(
                    bytes = self.buf.len() + rest.len(),
                    "codex app-server: 줄 상한 초과 — 다음 개행까지 버린다"
                );
                self.buf = Vec::new();
                self.discarding = true;
            } else {
                self.buf.extend_from_slice(rest);
            }
        }
    }
}

/// 리더가 **어떻게 끝나든** 통로를 닫는다 — 정상 EOF 든, `handle_line` 안에서 올라온 panic 이든.
///
/// ★`Drop` 이어야 하는 이유★: `catch_unwind` 는 리더 루프를 **밖에서** 감싸므로, 루프 꼬리에 적어 둔
///   정리는 unwind 가 그냥 지나간다. 그 경로는 가정이 아니다 — `core.emit` 안에서 구독자가 panic 하거나
///   `OutputCore` 의 뮤텍스가 poison 되면(그쪽은 `.expect` 다) 바로 거기로 간다. 그러면 닫힘 표식이 안
///   서고 라이터 스레드가 core·stdin·대기표를 든 채 영원히 돈다.
/// ★여기서 하는 일은 「닫는다」뿐이다★ — 디코더 flush 와 종료 사유 산출은 정상 경로의 일이라 루프 안에
///   남는다(unwind 로 건너뛰어도 잃을 것이 없다).
struct ReaderExit {
    state: SharedState,
    pending: Arc<Pending>,
}

impl Drop for ReaderExit {
    /// ★★순서가 계약이다 — **상태를 먼저 쓰고, 그 다음에 깨운다**★★.
    ///
    /// ★한때 반대였고 그것이 결함이었다★: 대기표를 먼저 깨우면 **죽음이 스스로 연 창**으로 라이터가
    ///   돌아온다 — 깨어난 라이터는 아직 `Connecting` 인 상태를 읽고 게이트를 통과해([`open_gate`])
    ///   **이미 끝난 통로에 `Ready` 를 배달한다.** 그 창은 우연이 아니라 스트림 종료마다 열렸다.
    /// ★지금 순서가 그것을 **구조적으로** 닫는다★: 상태 쓰기(뮤텍스 release) → `send`(채널) →
    ///   라이터의 `recv` → 라이터의 상태 읽기(뮤텍스 acquire) 가 happens-before 사슬로 이어져, 이 종료로
    ///   깨어난 라이터는 [`Link::Down`] 을 **반드시** 본다. 시간 여유가 아니라 메모리 모델이 근거다.
    /// ★그래도 깨우기를 빼지 말 것★ — 남기면 영구 hang 이고 그 hang 에는 신호가 없다. 바뀐 것은
    ///   **순서뿐**이다.
    /// ★락을 겹쳐 쥐지 않는다★(ADR-0006 상태→대기표): 상태 락을 놓은 **뒤에** 대기표를 닫는다.
    // ADR-0203
    // ADR-0201
    fn drop(&mut self) {
        {
            let (lock, cv) = &*self.state;
            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
            // 진행 중이던 턴은 결말을 모른다 — ★로그 한 줄이 전부다★. 이 사실을 상태로 남기면 그것을 지울
            //   세 번째 호출자가 필요해지고(ADR-0127 결정 5), 다음 화신은 이 화신의 통로를 이어받지 않는다
            //   (ADR-0192)라 읽을 소비자도 없다.
            if matches!(s.turn, TurnState::Active { .. }) {
                tracing::warn!("codex app-server: 턴 결말을 모른 채 스트림이 끝났다");
            }
            s.link = Link::Down("스트림이 끝났다".to_string());
            s.turn = TurnState::Idle;
            // 붙들어 둔 종료도 함께 버린다(사유 정본 = [`end_turn_if`]).
            s.early_completions.clear();
            s.closed = true;
            cv.notify_all();
        }
        // ★대기 중 RPC 를 전부 오류로 깨운다★ — 남기면 영구 hang 이고, 그 hang 에는 신호가 없다.
        for entry in self.pending.close() {
            match entry.waiter {
                Waiter::Handshake(tx) => {
                    let _ = tx.send(Err(format!("{}: 스트림이 끝났다", entry.method)));
                }
                Waiter::TurnStart { .. } | Waiter::Fire => {
                    tracing::debug!("{}: 답을 받기 전에 스트림이 끝났다", entry.method)
                }
            }
        }
    }
}

/// [`Reader::note_turn`] 이 **락을 놓은 뒤에** 남기는 한 줄.
///
/// ★상태 락을 쥔 채 찍지 않는다★: tracing 의 파일 sink 는 이벤트마다 동기 기록이라, 그 락 안에서
///   찍으면 [`AgentTransport::send_input`] 과 라이터가 그 디스크 쓰기 뒤에 줄을 선다. 그리고
///   [`Self::Unattributable`] 은 **일상** 경로다 — 포기한 턴의 진짜 `turn/completed` 가 뒤늦게 오면
///   여기로 온다. 수다스러운 상대 하나가 락 보유 디스크 쓰기를 그만큼 만든다.
// ADR-0006
enum TurnNoteLog<'a> {
    ForeignThread(&'a str),
    OversizedTurnId(usize),
    Held(&'a str),
    Unattributable(Option<&'a str>),
}

impl TurnNoteLog<'_> {
    fn write(self) {
        match self {
            TurnNoteLog::ForeignThread(thread) => tracing::warn!(
                thread = %sanitize(thread, LOG_STRING_LIMIT),
                "codex app-server: 남의 thread 의 turn/completed — 경계를 막는다"
            ),
            TurnNoteLog::OversizedTurnId(bytes) => tracing::warn!(
                bytes,
                "codex app-server: turn id 가 상한을 넘어 붙들지 않는다 — 경계를 막는다"
            ),
            TurnNoteLog::Held(turn) => tracing::debug!(
                turn = %sanitize(turn, LOG_STRING_LIMIT),
                "codex app-server: 응답보다 먼저 온 종료 — 대조를 기다린다"
            ),
            TurnNoteLog::Unattributable(turn) => tracing::warn!(
                // 빈 문자열로 두면 「id 가 없었다」와 「id 가 빈 문자열이었다」가 로그에서 같아진다.
                turn = %turn
                    .map(|t| sanitize(t, LOG_STRING_LIMIT))
                    .unwrap_or_else(|| "(칸 없음)".to_string()),
                "codex app-server: 귀속할 수 없는 turn/completed — 경계를 막는다"
            ),
        }
    }
}

struct Reader {
    core: Arc<OutputCore>,
    decoder: Option<Box<dyn OutputDecoder>>,
    state: SharedState,
    pending: Arc<Pending>,
}

impl Reader {
    /// 서버 요청을 거절한다. ★답하지 않으면 그 에이전트는 영구 정지한다★ — 그래서 모르는 요청에도
    /// 반드시 답한다. ★성공을 위장하지 않는다★: 승인 요청에 성공 응답을 돌려주면 그것이 자동 승인이다.
    fn refuse(&self, id: &RequestId, method_name: &str) {
        let shown = sanitize(method_name, LOG_STRING_LIMIT);
        let line = protocol::error_response_line(
            id,
            METHOD_NOT_FOUND,
            &format!("engram-dashboard 는 `{shown}` 를 처리하지 않는다"),
        );
        let dropped = {
            let (lock, cv) = &*self.state;
            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
            if s.closed {
                return;
            }
            if s.outbox.len() >= OUTBOX_LIMIT {
                true
            } else {
                s.outbox.push_back(line);
                cv.notify_all();
                false
            }
        };
        if dropped {
            // ★여기서 오류를 돌려줄 호출자가 없다 — 리더 자신이 만든 줄이다★. 그래서 로그로만 두면 이
            //   사건은 **아무 데도 안 보인다**: 상대는 답을 영영 기다리고 화면에는 신호가 없다. 형제
            //   사건(turn/start 시한)이 화면에 오르므로 이 자리도 같은 등급으로 올린다.
            tracing::warn!(
                cap = OUTBOX_LIMIT,
                "codex app-server: 제어 큐가 가득 차 요청 거절을 못 보낸다"
            );
            self.core.emit(OutputEvent::Error(format!(
                "codex app-server: 제어 큐가 가득 차 `{shown}` 요청에 답하지 못했다 — 그쪽은 그 답을 계속 기다린다"
            )));
        }
    }

    /// 턴이 끝났다는 알림 하나를 이 상태 기계에 먹이고, ★그 줄에서 번역기가 낸 **턴 경계를 화면으로
    /// 올릴지**를 정한다★. 돌려주는 값 = 지금 올릴 경계(없으면 `None`).
    ///
    /// ★귀속을 아는 것은 이 층뿐이다★ — 번역기는 같은 줄을 보지만 우리 thread·turn id 를 모른다. 그래서
    ///   번역은 무조건 하되, 그 산출이 **우리 턴의 경계인가**는 여기서 판정한다. 이 게이트가 없으면 남의
    ///   턴 종료 한 줄이 살아 있는 우리 턴을 화면에서 닫고(이후 출력이 가짜 경계로 쪼개진다) 사실 계층도
    ///   한가함으로 뒤집혀 턴 도중에 우편이 꽂힌다.
    /// ★막은 줄은 조용히 사라지지 않는다★ — warn 으로 남는다. 「경계가 안 떴다」의 원인을 로그 없이는
    ///   이 층과 번역기 중 어디서 찾아야 할지 알 수 없다.
    ///
    /// ★끝내는 조건은 둘 다 맞을 때뿐이다★: 우리 thread id 와 같고(알면), **우리가 아는 turn id 와 같다.**
    ///   thread 만 보고 끝내면 같은 스레드의 다른 턴(이미 끝난 앞 턴의 늦은 신호)이 지금 도는 턴을 닫고,
    ///   그 자리에서 큐가 풀려 턴이 겹친다.
    /// ★turn id 를 아직 모르면 아무 것도 안 끝낸다★. 그 무시가 **그 시점에** 정지를 만들지는 않는다:
    ///   id 가 비어 있다는 것은 그 턴의 `turn/start` 가 아직 답을 못 받았다는 뜻이고(답이 성공이든 오류든
    ///   해독 실패든 그 세 갈래가 전부 턴을 끝내거나 id 를 채운다), 그 요청에는 [`REQUEST_DEADLINE`] 이
    ///   걸려 있어 만료가 [`end_turn_if`] 로 그 턴을 끝낸다.
    /// ★종료가 응답보다 **먼저** 오는 경우는 그 논증이 안 덮는다 — 그래서 따로 닫는다★: 그 알림을 여기서
    ///   버리기만 하면, 뒤이어 온 응답이 id 를 채우면서 대기표까지 걷어 가 그 턴은 끝낼 것이 아무 것도
    ///   없이 남는다. 그래서 그 알림의 id **와 경계**를 [`State::early_completions`] 에 붙들어 두고,
    ///   응답이 우리 턴의 id 를 정하는 자리에서 대조해 그때 올린다([`Reader::resolve`]).
    /// ★붙드는 것이 「종료가 있었다」가 아니라 **그 id** 인 것이 핵심이다★ — 전자는 무엇이든 끝낼 수 있는
    ///   근거 없는 기억이라 오귀속 그 자체지만, 후자는 이 함수가 이미 하는 id 대조를 **미루는** 것뿐이다.
    ///   끝내는 판정은 어느 경로에서도 id 일치 하나뿐이다. ★그 성질을 넓히지 말 것★ — id 대조 없이 턴을
    ///   끝내는 갈래를 더하는 순간 이 파일이 걷어낸 오귀속이 그대로 돌아온다.
    // ADR-0127
    fn note_turn(
        &self,
        method_name: &str,
        params: Option<&Value>,
        boundary: Option<OutputEvent>,
    ) -> Option<OutputEvent> {
        if method_name != TURN_COMPLETED {
            // ★번역기는 이 이름 밖에서 턴 경계를 내지 않는다★ — 그래도 받은 것을 그대로 돌려준다:
            //   그 전제가 깨지는 날 경계가 조용히 사라지는 것보다 화면에 뜨는 편이 낫다.
            return boundary;
        }
        let incoming_thread = params
            .and_then(|p| p.get("threadId"))
            .and_then(|v| v.as_str());
        let incoming_turn = params
            .and_then(|p| p.get("turn"))
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str());

        let (result, log) = {
            let (lock, cv) = &*self.state;
            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
            // ★모르는 threadId 는 오류가 아니다★ — 알림이 그 스레드의 id 를 알려 줄 응답보다 먼저
            //   도착하는 경우가 실측됐다. 우리 id 를 아직 모르면 그 축으로는 거르지 않는다.
            let foreign_thread = match (s.thread_id.as_deref(), incoming_thread) {
                (Some(mine), Some(theirs)) if mine != theirs => Some(theirs),
                _ => None,
            };
            if let Some(theirs) = foreign_thread {
                (None, Some(TurnNoteLog::ForeignThread(theirs)))
            } else {
                match (&s.turn, incoming_turn) {
                    (
                        TurnState::Active {
                            turn_id: Some(mine),
                            ..
                        },
                        Some(theirs),
                    ) if mine == theirs => {
                        s.turn = TurnState::Idle;
                        s.early_completions.clear();
                        cv.notify_all();
                        (boundary, None)
                    }
                    // ★지금 턴의 id 를 아직 모르는 동안 온 종료★: 이 시점에는 귀속할 수 없으므로 **끝내지
                    //   않고 경계도 안 올린다**. 대신 그 id 와 경계를 붙들어 두고, 뒤이어 올 `turn/start` 응답이
                    //   우리 턴의 id 를 권위 있게 알려 줄 때 대조한다([`Reader::resolve`]). 그 대조가 없으면 이
                    //   턴은 끝낼 것이 아무 것도 없이 남는다 — 응답이 대기표를 걷어 가 시한 backstop 도 사라진다.
                    (TurnState::Active { turn_id: None, .. }, Some(theirs)) => {
                        if theirs.len() > MAX_TURN_ID_BYTES {
                            (None, Some(TurnNoteLog::OversizedTurnId(theirs.len())))
                        } else if s.early_completions.iter().any(|k| k.turn_id == theirs) {
                            (None, None)
                        } else {
                            if s.early_completions.len() >= EARLY_COMPLETION_SLOTS {
                                s.early_completions.pop_front();
                            }
                            s.early_completions.push_back(EarlyCompletion {
                                turn_id: theirs.to_string(),
                                boundary,
                            });
                            (None, Some(TurnNoteLog::Held(theirs)))
                        }
                    }
                    // 열린 턴이 없거나, 우리가 아는 id 와 다르거나, 알림에 turn id 가 아예 없다.
                    _ => (None, Some(TurnNoteLog::Unattributable(incoming_turn))),
                }
            }
        };
        if let Some(entry) = log {
            entry.write();
        }
        result
    }

    /// 이 알림이 **우리가 지금 열어 둔 턴의 것으로 셀 수 있나** — 화면에 올릴지가 아니라 ★사실 계층에
    /// 진행 신호를 적을지★의 판정이다(ADR-0113).
    ///
    /// ★두 판정을 가른 이유★: 경계를 막으면 잃는 것이 「경계 하나」지만 본문을 막으면 **에이전트가 낸
    ///   말이 화면에서 사라진다**. 그래서 본문은 언제나 올리고, 그 줄이 「이 화신은 턴 중」의 근거가
    ///   되는지만 여기서 가른다. 이 가름이 없으면 우리가 연 적 없는 턴의 `item/*` 한 줄이 사실 계층을
    ///   켜고, 그 턴의 종료는 [`Reader::note_turn`] 에 막혀 **아무도 그것을 못 끈다** — 30 분 fail-open
    ///   밸브가 쓸어 갈 때까지 그 에이전트에는 우편이 안 간다(ADR-0127 이 닫은 그 기전과 같은 모양).
    /// ★판정은 「모순되지 않는가」다 — 「일치하는가」가 아니다★: `item/*` 는 `turn/start` **응답보다
    ///   먼저** 오는 것이 정상이라(그 창이 [`State::early_completions`] 가 존재하는 이유다) 그 시점에
    ///   id 일치를 요구하면 우리 턴의 스트리밍 출력이 통째로 사실 계층 밖으로 나간다. 그래서 열린 턴이
    ///   있고 그 턴의 id 와 **어긋나지만 않으면** 우리 것으로 센다.
    /// ★열린 턴이 없으면 무조건 아니다★ — 그 순간 이 통로는 「턴 중 아님」이고, 두 축이 같은 판정을
    ///   받는 것이 이 모양의 요점이다(모듈 헤더).
    /// ★이 판정과 emit 사이에는 락이 없다(알려진 잔여)★ — 라이터가 그 틈에 턴을 열거나 닫으면 줄
    ///   하나가 반대쪽으로 센다. 락을 emit 까지 끌면 ADR-0006 을 어기고, 그 한 줄의 오차는 다음 줄과
    ///   턴 경계가 덮는다.
    // ADR-0113
    fn claims_our_turn(&self, params: Option<&Value>) -> bool {
        let (lock, _) = &*self.state;
        let s = lock.lock().unwrap_or_else(|p| p.into_inner());
        if let (Some(mine), Some(theirs)) = (
            s.thread_id.as_deref(),
            params
                .and_then(|p| p.get("threadId"))
                .and_then(|v| v.as_str()),
        ) {
            if mine != theirs {
                return false;
            }
        }
        match &s.turn {
            TurnState::Idle => false,
            TurnState::Active { turn_id: None, .. } => true,
            TurnState::Active {
                turn_id: Some(mine),
                ..
            } => match params
                .and_then(|p| p.get("turnId"))
                .and_then(|v| v.as_str())
            {
                Some(theirs) => mine == theirs,
                None => true,
            },
        }
    }

    fn resolve(&self, id: &RequestId, outcome: Result<Value, String>) {
        let entry = match self.pending.take(id) {
            Some(e) => e,
            None => {
                // 우리가 낸 적 없는 id — 서버 id 공간의 값이거나 이미 시한으로 거둔 자리다.
                tracing::debug!(?id, "codex app-server: 모르는 id 의 응답 — 버린다");
                return;
            }
        };
        match entry.waiter {
            Waiter::Handshake(tx) => {
                let _ = tx.send(outcome);
            }
            Waiter::TurnStart { seq } => match outcome {
                Ok(v) => match serde_json::from_value::<TurnStartResponse>(v) {
                    // ★응답이 정하는 id 에도 길이를 건다★ — 이 값은 그대로 `turn/interrupt` 의 params 와
                    //   [`OutputEvent::TurnEnd`] 의 칸으로 나가므로, 상한이 없으면 상대가 응답 하나로 replay
                    //   링의 단일 이벤트 상한을 넘는 경계를 만들고 그 링은 그 한 건으로 나머지를 전부 비운다.
                    //   ★대조 토큰이라 자르지 않고 거른다★(같은 판단 = [`MAX_TURN_ID_BYTES`]) — 그러면 이
                    //   턴은 귀속할 재료가 없으므로 아래 오류 갈래와 **같은 등급**으로 여기서 닫는다.
                    Ok(r) if r.turn.id.len() > MAX_TURN_ID_BYTES => {
                        tracing::warn!(
                            bytes = r.turn.id.len(),
                            "turn/start 응답의 turn id 가 상한을 넘었다"
                        );
                        end_turn_if(
                            &self.state,
                            &self.core,
                            seq,
                            format!(
                                "codex app-server: turn/start 응답의 turn id 가 상한({MAX_TURN_ID_BYTES}B)을 넘었다"
                            ),
                        );
                    }
                    Ok(r) => {
                        let released = {
                            let (lock, _) = &*self.state;
                            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
                            // ★표식이 안 맞으면 이 답이 연 턴은 이미 끝났다★ — 그 id 를 지금 턴의 칸에
                            //   적으면 그 뒤의 interrupt 가 엉뚱한 턴을 겨눈다.
                            match &mut s.turn {
                                TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                                    if turn_id.is_none() {
                                        *turn_id = Some(r.turn.id);
                                    }
                                    // ★이 응답이 우리 턴의 id 를 권위 있게 정한 자리다★ — 그 id 로 온
                                    //   종료가 이미 지나갔다면 여기서 끝내고, **그때 붙들어 둔 경계를
                                    //   지금 올린다**. 이 대조가 없으면 그 턴은 영영 열린 채 남고
                                    //   (대기표는 방금 걷혔다), 그 뒤의 `send_input` 이 그 정지를 "큐가
                                    //   찼다" 로 신고한다 — 사유가 어긋난 신고다.
                                    // ★끝내는 판정은 여전히 id 일치 하나뿐이다★ — 붙들어 둔 목록에
                                    //   있다는 사실만으로 끝나는 턴은 없다.
                                    let ours = turn_id.clone();
                                    let hit = match ours.as_deref() {
                                        Some(ours) => s
                                            .early_completions
                                            .iter()
                                            .position(|k| k.turn_id == ours),
                                        None => None,
                                    };
                                    match hit {
                                        Some(pos) => {
                                            let held = s.early_completions.remove(pos);
                                            s.turn = TurnState::Idle;
                                            // 같은 창에서 붙든 나머지도 이 턴과 함께 버린다
                                            //   (사유 정본 = [`end_turn_if`]).
                                            s.early_completions.clear();
                                            let (_, cv) = &*self.state;
                                            cv.notify_all();
                                            held.and_then(|h| h.boundary)
                                        }
                                        None => None,
                                    }
                                }
                                _ => {
                                    tracing::debug!(
                                        "codex app-server: 이미 끝난 턴의 turn/start 응답 — 버린다"
                                    );
                                    None
                                }
                            }
                        };
                        // ★락을 놓은 뒤에 올린다★(ADR-0006) — 구독자는 emit 안에서 임의 코드를 돈다.
                        if let Some(ev) = released {
                            self.core.emit(ev);
                        }
                    }
                    // ★오류 응답과 **같은 등급**이어야 한다★: 대기표는 위에서 이미 걷혔으므로, 여기서
                    //   턴을 안 끝내면 그 턴은 `turn_id` 없이 남고 만료시킬 대기표도 없다 — `note_turn`
                    //   은 영영 귀속을 못 하고 게이트가 풀리지 않는다(그 뒤 `send_input` 은 그 정지를
                    //   "큐가 찼다" 로 신고한다). 해독 못 한 성공은 명시적 오류보다 덜 치명이 아니다.
                    Err(e) => {
                        let masked = sanitize(&e.to_string(), LOG_STRING_LIMIT);
                        tracing::warn!("turn/start 응답 해독 실패: {masked}");
                        end_turn_if(
                            &self.state,
                            &self.core,
                            seq,
                            format!("codex app-server: turn/start 응답을 읽지 못했다: {masked}"),
                        );
                    }
                },
                Err(msg) => {
                    let masked = sanitize(&msg, LOG_STRING_LIMIT);
                    tracing::warn!("turn/start 실패: {masked}");
                    end_turn_if(
                        &self.state,
                        &self.core,
                        seq,
                        format!("codex app-server: {masked}"),
                    );
                }
            },
            Waiter::Fire => {
                if let Err(msg) = outcome {
                    tracing::warn!(
                        "{} 실패: {}",
                        entry.method,
                        sanitize(&msg, LOG_STRING_LIMIT)
                    );
                }
            }
        }
    }

    fn handle_line(&mut self, line: &[u8]) {
        let text = match std::str::from_utf8(line) {
            Ok(t) => t,
            Err(_) => {
                tracing::debug!(
                    bytes = line.len(),
                    "codex app-server: UTF-8 이 아닌 줄 — 버린다"
                );
                return;
            }
        };
        if text.trim().is_empty() {
            return;
        }
        match protocol::classify(text) {
            Ok(Inbound::Request { id, method, .. }) => self.refuse(&id, &method),
            Ok(Inbound::Notification { method, params }) => {
                // ★번역기에는 **원본 줄 바이트**를 그대로 넣는다★ — 두 번째 입구를 만들면 그쪽의 라인
                //   재조립·상한·마스킹 규율이 배송 경로 밖으로 나간다(사유 정본 = `decoder.rs` 헤더).
                //   번역기는 개행으로 줄을 가르므로 종단을 함께 준다.
                let mut events = match self.decoder.as_mut() {
                    Some(dec) => {
                        let mut events = dec.decode(line);
                        events.extend(dec.decode(b"\n"));
                        events
                    }
                    None => Vec::new(),
                };
                // ★턴 경계만 뽑아 **귀속 게이트**를 지난다 — 나머지는 그대로 흐른다★: 번역기는 우리
                //   thread·turn id 를 몰라 「이 결말이 우리 턴의 것인가」를 답할 수 없고, 이 층은 그것만
                //   안다(사유 정본 = [`TURN_COMPLETED`] doc).
                let boundary = events
                    .iter()
                    .position(|e| matches!(e, OutputEvent::TurnEnd { .. }))
                    .map(|i| events.remove(i));
                // ★본문은 언제나 화면으로 올리되, 그 줄을 「이 화신은 턴 중」의 근거로 셀지는 가른다★
                //   (사유 정본 = [`Reader::claims_our_turn`]). 이 두 문이 갈리지 않으면 미귀속 줄이
                //   사실 계층만 켜 놓고 그 종료는 아래 게이트에 막혀 아무도 그것을 못 끈다.
                // 경계 하나만 나오는 줄(대부분의 `turn/completed`)에서는 이 판정이 필요 없다 — 락을 아낀다.
                let ours = !events.is_empty() && self.claims_our_turn(params.as_ref());
                for ev in events {
                    // ★종료 어휘는 그 문으로 보내지 않는다★ — 그 문은 도어벨(`StatusSink::turn_ended`)도
                    //   건너뛰므로 종료를 기다리는 소비자가 안 깨어난다. 번역기 계약상 한 줄이 내는
                    //   경계는 하나뿐이라 그것은 위에서 이미 뽑혔지만, 그 계약이 깨지는 날 조용히
                    //   잃는 것보다 여기서 지키는 편이 싸다.
                    if ours || matches!(ev, OutputEvent::TurnEnd { .. }) {
                        self.core.emit(ev);
                    } else {
                        self.core.emit_without_turn_observation(ev);
                    }
                }
                if let Some(ev) = self.note_turn(&method, params.as_ref(), boundary) {
                    self.core.emit(ev);
                }
            }
            Ok(Inbound::Response { id, result }) => self.resolve(&id, Ok(result)),
            Ok(Inbound::Error { id, error }) => {
                // ★마스킹은 경계가 아니라 **여기**에서 한다★ — 이 문자열은 로그로도 화면으로도 가고,
                //   호출자에게도 돌아간다. 나가는 문마다 다시 거르면 그중 하나는 반드시 잊힌다.
                let msg = format!(
                    "[{}] {}",
                    error.code,
                    sanitize(&error.message, LOG_STRING_LIMIT)
                );
                self.resolve(&id, Err(msg))
            }
            Err(e) => {
                // ★해독 실패를 치명으로 두지 않는다★ — 한 줄로 스트림을 끊으면 에이전트가 죽는다.
                tracing::debug!(
                    "codex app-server: 봉투를 못 읽었다({e}) — {}",
                    sanitize(text, LOG_STRING_LIMIT)
                );
            }
        }
    }
}

fn reader_loop(
    stdout: ChildStdout,
    shutdown: Arc<AtomicBool>,
    mut reader: Reader,
    child: Arc<Mutex<Child>>,
) -> TerminalReason {
    // ★이 가드가 서는 자리가 함수 맨 앞인 것이 요점이다★ — 아래 어디서 unwind 가 올라와도 `Drop` 은 돈다.
    let _exit = ReaderExit {
        state: reader.state.clone(),
        pending: reader.pending.clone(),
    };

    let mut source = stdout;
    let mut buf = [0u8; READ_BUF_BYTES];
    let mut splitter = LineSplitter::new();

    loop {
        let n = match source.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let mut lines: Vec<Vec<u8>> = Vec::new();
        splitter.feed(&buf[..n], |line| lines.push(line.to_vec()));
        for line in lines {
            reader.handle_line(&line);
        }
    }

    // ★닫는 일은 위 `ReaderExit` 이 진다 — 여기 다시 적지 않는다★(unwind 도 같은 자리를 지나야 한다).
    // ★kill 경로에서는 flush 하지 않는다★ — 그 꼬리는 우리가 프로세스를 죽여 잘린 조각이다.
    if !shutdown.load(Ordering::Acquire) {
        if let Some(dec) = reader.decoder.as_mut() {
            for ev in dec.flush() {
                reader.core.emit(ev);
            }
        }
    }

    // ★EOF 직후의 `try_wait` 한 번으로는 종료 코드를 못 받는다★ — stdout 이 닫히는 것과 프로세스가
    //   수거되는 것은 같은 순간이 아니라, 그 한 번은 대개 `Ok(None)` 으로 떨어지고 코드가 조용히
    //   **유실**된다. 그 유실은 두 번 아프다: 화면에 종료 코드가 안 뜨고, `Exited { code: None }` 이
    //   「자식이 아직 살아 있다」와 구별되지 않아 **시험대가 그 둘을 가를 수단을 잃는다**.
    // ★그래서 유계로 다시 본다★ — 락은 시도마다 놓는다(쥔 채 자면 `shutdown` 의 kill 이 그만큼 막힌다).
    // ★kill 경로는 이 루프를 한 바퀴도 안 돈다★ — `shutdown()` 이 이미 `wait()` 했으므로 첫 시도가
    //   바로 값을 준다.
    // ★그래도 `None` 이 남을 수 있고 그것이 정직한 결말이다★ — 상대가 stdout 만 닫고 안 끝난 경우가
    //   그것이고, 그 처분은 [`wait_for_close_or_tear_down`] 이 따로 진다.
    let code = {
        let deadline = Instant::now() + CHILD_REAP_GRACE;
        loop {
            let reaped = {
                let mut c = child.lock().unwrap_or_else(|p| p.into_inner());
                match c.try_wait() {
                    Ok(Some(status)) => Some(status.code()),
                    _ => None,
                }
            };
            if let Some(code) = reaped {
                break code;
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    if shutdown.load(Ordering::Acquire) {
        TerminalReason::Killed
    } else {
        TerminalReason::Exited { code }
    }
}

// ── AgentTransport ────────────────────────────────────────────────────────────

impl AgentTransport for CodexAppServerTransport {
    /// stdout 이 이미 take 됐으면(재호출) 아무것도 안 한다(멱등 방어).
    fn start(&self, core: Arc<OutputCore>) {
        let agent_id = core.id();

        let stdout = match self.stdout.lock().unwrap_or_else(|p| p.into_inner()).take() {
            Some(s) => s,
            None => return,
        };

        // ── stderr drain ──
        // 비우지 않으면 자식이 stderr 버퍼 full 로 블록한다. 이 스트림에는 상대의 진단 텍스트가
        //   오므로(실측 0.154.0 — 오류는 stdout 이 아니라 이쪽으로 갔다) 활성화 실패의 유일한 증거다.
        if let Some(stderr) = self.stderr.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let diag_core = core.clone();
            let spawn_result = std::thread::Builder::new()
                .name("engram-codex-stderr".into())
                .spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            Ok(l) if !l.is_empty() => {
                                let masked = mask_secrets(&l);
                                diag_core.push_diagnostic(&masked);
                                tracing::debug!(target: "agent_stderr", agent = %agent_id, "{}", masked)
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                });
            if let Err(e) = spawn_result {
                tracing::warn!(agent = %agent_id, "codex stderr drain 스레드 기동 실패: {e}");
            }
        }

        // ── 라이터 ──
        // ★아무도 join 하지 않는다★ — `core` 는 pump 핸들을 하나만 들고(그 자리를 넓히는 것은 코어
        //   변경이다), `shutdown()` 안에서 기다리는 것은 계약 위반이다. 이 스레드는 kill 이 파이프를
        //   깨면 블록된 write 가 풀리고 닫힘 표식을 보아 스스로 끝난다.
        if let Some(params) = self
            .open_params
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let stdin = self.stdin.clone();
            let state = self.state.clone();
            let pending = self.pending.clone();
            let next_id = self.next_id.clone();
            let shutdown = self.shutdown.clone();
            let writer_core = core.clone();
            let sink = self.sid_sink.clone();
            let link = self
                .link_sink
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            let spawn_result = std::thread::Builder::new()
                .name("engram-codex-writer".into())
                .spawn(move || {
                    writer_loop(
                        stdin,
                        state,
                        pending,
                        next_id,
                        shutdown,
                        writer_core,
                        params,
                        sink,
                        link,
                    )
                });
            match spawn_result {
                Ok(handle) => {
                    *self.writer_handle.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
                }
                Err(e) => {
                    // 라이터가 없으면 핸드셰이크도 입력도 영영 안 나간다 — 조용히 두지 않는다.
                    tracing::warn!(agent = %agent_id, "codex writer 스레드 기동 실패: {e}");
                    let (lock, cv) = &*self.state;
                    let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
                    s.link = Link::Down(format!("writer 스레드 기동 실패: {e}"));
                    cv.notify_all();
                }
            }
        }

        // ── 리더(pump) ──
        let (done_tx, done_rx) = mpsc::channel();
        let reader = Reader {
            core: core.clone(),
            decoder: self
                .decoder
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take(),
            state: self.state.clone(),
            pending: self.pending.clone(),
        };
        let pump_core = core.clone();
        let child = self.child.clone();
        let shutdown = self.shutdown.clone();
        let handle = std::thread::spawn(move || {
            let normal_reason = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reader_loop(stdout, shutdown, reader, child)
            }));
            pump_core.finish(resolve_pump_reason(normal_reason));
            let _ = done_tx.send(());
        });

        core.attach_pump(handle, done_rx);
    }

    /// 바이트는 **완결된 유저 턴 본문**이다 — 봉투는 이 통로가 만든다.
    ///
    /// ★수령 의미는 하나다★: `Ok` = 순서까지 확정해 전량을 받았다, `Err` = 받지 않았다. ★짧은 `Ok` 로
    ///   축소 보고하지 않는다★.
    /// ★`Err` 는 "우리가 받겠다" 고 말하기 **전에** 결정되는 것뿐이다★ — ① 큐 상한 초과 ② 이 화신의
    ///   연결이 이미 끝났다. 핸드셰이크 창에서 큐에 선 입력은 이미 `Ok` 를 받았으므로 ②를 그 지점 뒤로
    ///   넓히지 않는다.
    /// ★알려진 한계★: 받아 둔 뒤에 실패한 쓰기는 이 호출자에게 돌아갈 길이 없다. 그때 남는 것은 출력
    ///   스트림에 오르는 **실패 결말의 턴 경계** 하나이고([`end_turn_if`], 그리고 핸드셰이크가 실패해
    ///   큐가 통째로 버려지는 갈래는 [`writer_loop`] 가 직접 낸다), 이미 준 `Ok` 는 정정되지 않는다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        let (lock, cv) = &*self.state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        if s.closed {
            return Err(PtyError::WriteFailed(
                "codex app-server: 통로가 닫혔다".into(),
            ));
        }
        if let Link::Down(reason) = &s.link {
            return Err(PtyError::WriteFailed(format!(
                "codex app-server 연결이 끝났다: {}",
                sanitize(reason, LOG_STRING_LIMIT)
            )));
        }
        if s.input.len() >= INPUT_QUEUE_LIMIT {
            return Err(PtyError::WriteFailed(format!(
                "codex app-server: 대기 중인 입력이 상한({INPUT_QUEUE_LIMIT})에 찼다"
            )));
        }
        s.input.push_back(bytes);
        cv.notify_all();
        Ok(())
    }

    fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "CodexAppServerTransport::resize (파이프는 터미널 크기 없음)".into(),
        ))
    }

    /// 진행 중인 턴 하나를 `turn/interrupt` 로 끊는다(≠kill — 프로세스는 살아 있다).
    ///
    /// ★턴 id 를 아직 모르면 `Err` 다★ — 상대는 그 칸을 필수로 요구하고, 모르는 채 보낸 봉투는 답조차
    ///   오지 않는다(실측 0.154.0: 해독 못 한 봉투에는 답이 없다).
    fn interrupt(&self) -> Result<(), PtyError> {
        let (lock, cv) = &*self.state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        let (thread_id, turn_id) = match (&s.thread_id, &s.turn) {
            (
                Some(t),
                TurnState::Active {
                    turn_id: Some(turn),
                    ..
                },
            ) => (t.clone(), turn.clone()),
            _ => {
                return Err(PtyError::Unsupported(
                    "CodexAppServerTransport::interrupt (중단할 턴이 없다)".into(),
                ))
            }
        };
        if s.outbox.len() >= OUTBOX_LIMIT {
            return Err(PtyError::WriteFailed(format!(
                "codex app-server: 제어 큐가 상한({OUTBOX_LIMIT})에 찼다"
            )));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = protocol::request_line(
            &RequestId::Num(id),
            method::TURN_INTERRUPT,
            &TurnInterruptParams { thread_id, turn_id },
        )
        .map_err(|e| PtyError::WriteFailed(format!("turn/interrupt 직렬화 실패: {e}")))?;
        if !self
            .pending
            .register(id, Waiter::Fire, method::TURN_INTERRUPT)
        {
            return Err(PtyError::WriteFailed(
                "codex app-server: 통로가 닫혔다".into(),
            ));
        }
        s.outbox.push_back(line);
        cv.notify_all();
        Ok(())
    }

    /// ADR-0001 2 동사의 app-server 판. ★이 안에서 아무것도 기다리지 않는다★(대기는 `core.join_pump` 몫).
    ///
    /// ★순서 불변 — stdin close 는 kill 보다 절대 먼저 오면 안 된다★: 라이터는 stdin 락을 블로킹
    ///   `write_all` 내내 쥔다. 상대가 stdin 을 안 읽으면(파이프 backpressure) 그 write 가 영원히 블록해
    ///   락을 놓지 않으므로, kill 전에 `stdin.lock()` 을 잡으려 하면 kill 에 도달조차 못 하고 pump 가
    ///   못 깨어나 `join_pump` 가 영구 hang 한다. 자식을 먼저 죽이면 파이프가 깨져 그 write 가 에러로
    ///   풀리고 락이 해제된다.
    /// ※stdin 을 닫는 것만으로도 상대가 스스로 exit 하는 것은 실측됐지만(턴이 없는 상태에서 41–51ms),
    ///   그 관측은 위 순서 불변을 바꾸지 않는다 — 락을 못 잡으면 닫는 자리까지 가지도 못한다.
    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);

        // ★여기서 잡는 것은 상태 락뿐이다★ — 블로킹 write 를 쥔 스레드는 이 락을 갖고 있지 않으므로
        //   (쓰기 전에 놓는다) 이 단계는 매달릴 수 없다. 아직 못 나간 큐는 여기서 사라진다.
        {
            let (lock, cv) = &*self.state;
            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
            s.closed = true;
            cv.notify_all();
        }

        {
            let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }

        // 손자(cmd shim 아래 codex, 그리고 codex 가 thread/start 에서 띄운 MCP 자식들)까지 함께 끝난다.
        #[cfg(windows)]
        {
            let _ = self.job_handle.terminate(1);
        }

        // try_lock 을 못 얻으면(아직 write_all 이 안 풀린 찰나) skip — 미정리 ChildStdin 은 drop 시 OS 가
        //   회수한다. ★블로킹 lock 금지★.
        if let Ok(mut guard) = self.stdin.try_lock() {
            let _ = guard.take();
        }

        // 대기 중이던 요청은 여기서도 깨운다 — 리더가 EOF 를 못 보고 끝나는 경우에도 남지 않게.
        for entry in self.pending.close() {
            if let Waiter::Handshake(tx) = entry.waiter {
                let _ = tx.send(Err(format!("{}: 통로가 닫혔다", entry.method)));
            }
        }
    }

    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                // ★`raw` 가 false 인 것은 이 채널이 키 입력을 나르지 않기 때문이다★ — 받는 바이트는
                //   완결된 메시지 본문이고, 한 글자씩 흘려 넣으면 글자마다 턴이 하나씩 열린다.
                raw: false,
                message: true,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                structured: self.structured,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: true,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{ThreadItemEntry, ThreadStartParams};
    use super::*;
    use crate::output_core::TurnWiring;
    use crate::turn::TurnObservations;
    use crate::types::{
        AgentId, AgentStatus, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId, StatusSink,
    };

    // ── 하네스 ──────────────────────────────────────────────────────────────

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    /// emit 된 이벤트를 그대로 모은다 — `snapshot()` 은 `TerminalBytes` 만 돌려주므로 구조화 이벤트는
    /// 구독으로만 볼 수 있다.
    struct EventSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }
    impl OutputSink for EventSink {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
            if let OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push(e.clone());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// `core.emit` 안에서 panic 하는 구독자 — 리뷰가 이름한 두 unwind 경로 중 하나다(다른 하나는
    /// `OutputCore` 뮤텍스 poison). `emit` 은 sink 의 panic 을 잡지 않고 그대로 올려 보낸다.
    struct PanickingSink(SinkId);
    impl OutputSink for PanickingSink {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            panic!("subscriber blew up inside emit");
        }
        fn sink_id(&self) -> SinkId {
            self.0
        }
    }

    /// 알림 줄을 하나 받으면 이벤트를 낸다 — 그 emit 이 위 구독자를 지나며 **읽기 루프 안에서** 리더를
    /// unwind 시킨다.
    ///
    /// ★flush 가 아니라 `decode` 여야 한다★: 닫기를 루프 꼬리에 적어 두던 옛 모양은 flush **보다 먼저**
    ///   닫았으므로, flush 에서 터지는 panic 은 그 모양에서도 새지 않았다. 새는 자리는 읽기 루프다.
    struct EmittingDecoder;
    impl OutputDecoder for EmittingDecoder {
        fn decode(&mut self, _chunk: &[u8]) -> Vec<OutputEvent> {
            vec![OutputEvent::Error("mid-stream".into())]
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            Vec::new()
        }
    }

    /// 알림 한 줄을 stdout 으로 흘리는 프로브. ★`echo` 로 만들지 않는다★ — 그 경로는 따옴표가 셸을
    /// 지나며 바뀌어 봉투가 알림으로 분류되지 않는다.
    #[cfg(windows)]
    struct NotificationFile(std::path::PathBuf);

    #[cfg(windows)]
    impl NotificationFile {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("engram-codex-{tag}-{}.ndjson", std::process::id()));
            std::fs::write(&path, "{\"method\":\"turn/completed\",\"params\":{}}\n")
                .expect("temp write");
            NotificationFile(path)
        }
        fn args(&self) -> Vec<String> {
            vec![
                "/c".to_string(),
                "type".to_string(),
                self.0.to_string_lossy().into_owned(),
            ]
        }
    }

    #[cfg(windows)]
    impl Drop for NotificationFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[cfg(windows)]
    fn await_writer_end(t: &CodexAppServerTransport, why: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let finished = t
                .writer_handle
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .map(|h| h.is_finished())
                .unwrap_or(false);
            if finished {
                return;
            }
            assert!(Instant::now() < deadline, "{why}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn core_with_sink() -> (Arc<OutputCore>, Arc<Mutex<Vec<OutputEvent>>>) {
        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        (core, seen)
    }

    fn shared() -> SharedState {
        Arc::new((Mutex::new(State::new()), Condvar::new()))
    }

    fn with_state<R>(state: &SharedState, f: impl FnOnce(&mut State) -> R) -> R {
        let mut g = state.0.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g)
    }

    /// 표식 `seq` 로 턴 하나를 연다. ★표식을 시험대가 직접 고르는 것이 요점이다★ — 늦게 온 신호가 어느
    /// 턴의 것인지를 그 표식으로만 가르기 때문이다.
    fn activate(state: &SharedState, seq: u64, turn_id: Option<&str>) {
        with_state(state, |s| {
            s.turn = TurnState::Active {
                seq,
                turn_id: turn_id.map(|t| t.to_string()),
            };
            s.next_turn_seq = seq + 1;
        });
    }

    fn turn_seq(state: &SharedState) -> Option<u64> {
        with_state(state, |s| match s.turn {
            TurnState::Active { seq, .. } => Some(seq),
            TurnState::Idle => None,
        })
    }

    fn turn_id_of(state: &SharedState) -> Option<String> {
        with_state(state, |s| match &s.turn {
            TurnState::Active { turn_id, .. } => turn_id.clone(),
            TurnState::Idle => None,
        })
    }

    fn make_ready(state: &SharedState, thread_id: &str) {
        with_state(state, |s| {
            s.link = Link::Ready;
            s.thread_id = Some(thread_id.to_string());
        });
    }

    /// 통로가 번역기에 정확히 무엇을 넣었는지 보는 가짜 번역기.
    struct RecordingDecoder {
        chunks: Arc<Mutex<Vec<u8>>>,
    }
    impl OutputDecoder for RecordingDecoder {
        fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
            self.chunks.lock().unwrap().extend_from_slice(chunk);
            Vec::new()
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            Vec::new()
        }
    }

    struct Harness {
        reader: Reader,
        state: SharedState,
        pending: Arc<Pending>,
        next_id: Arc<AtomicI64>,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
        decoded: Arc<Mutex<Vec<u8>>>,
    }

    fn harness() -> Harness {
        let (core, seen) = core_with_sink();
        let state = shared();
        let pending = Arc::new(Pending::default());
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let reader = Reader {
            core,
            decoder: Some(Box::new(RecordingDecoder {
                chunks: decoded.clone(),
            })),
            state: state.clone(),
            pending: pending.clone(),
        };
        Harness {
            reader,
            state,
            pending,
            next_id: Arc::new(AtomicI64::new(0)),
            seen,
            decoded,
        }
    }

    /// 실 번역기를 꽂은 시험대. ★귀속 게이트를 재려면 이쪽이어야 한다★ — [`RecordingDecoder`] 는
    /// 이벤트를 하나도 내지 않아, 그것으로 재면 「경계를 막았다」가 **막아서가 아니라 애초에 없어서**
    /// 통과한다.
    fn harness_with_real_decoder() -> Harness {
        let mut h = harness();
        h.reader.decoder = Some(Box::new(super::super::decoder::CodexAppServerDecoder::new()));
        h
    }

    /// 실 번역기 **와** 실 사실 계층을 함께 꽂은 시험대 — 「화면 경계」와 「이 화신이 턴 중인가」가 같은
    /// 판정을 받는지를 한 자리에서 잰다. 화신 표식은 1 로 고정한다(이 시험대에 화신은 하나뿐이다).
    fn harness_with_facts() -> (Harness, Arc<TurnObservations>, AgentId) {
        let id = AgentId::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = Arc::new(OutputCore::new(
            id,
            1,
            Arc::new(NoopStatus),
            TurnWiring::new(turns.clone(), super::super::classify_turn),
        ));
        turns.register(id, 1);
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        let state = shared();
        let pending = Arc::new(Pending::default());
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let reader = Reader {
            core,
            decoder: Some(Box::new(super::super::decoder::CodexAppServerDecoder::new())),
            state: state.clone(),
            pending: pending.clone(),
        };
        (
            Harness {
                reader,
                state,
                pending,
                next_id: Arc::new(AtomicI64::new(0)),
                seen,
                decoded,
            },
            turns,
            id,
        )
    }

    fn boundaries(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<(Option<String>, TurnOutcome)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TurnEnd { turn_id, outcome } => {
                    Some((turn_id.clone(), outcome.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn failure_boundaries(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<String> {
        boundaries(seen)
            .into_iter()
            .filter_map(|(_, outcome)| match outcome {
                TurnOutcome::Failed { detail } => Some(detail.unwrap_or_default()),
                _ => None,
            })
            .collect()
    }

    /// 한 줄짜리 `turn/completed` — 실 번역기가 읽을 수 있는 모양이다.
    fn completed_line(thread_id: &str, turn_id: &str, status: &str) -> String {
        serde_json::json!({
            "method": "turn/completed",
            "params": {"threadId": thread_id,
                       "turn": {"id": turn_id, "items": [], "status": status}},
        })
        .to_string()
    }

    /// 턴 **중간**의 알림 한 줄 — 경계가 아니라 진행 신호가 되는 부류.
    fn progress_line(thread_id: &str, turn_id: &str) -> String {
        serde_json::json!({
            "method": "item/agentMessage/delta",
            "params": {"threadId": thread_id, "turnId": turn_id, "itemId": "i-1", "delta": "hi"},
        })
        .to_string()
    }

    /// 표 + codex 분류자 + 실 번역기를 꽂은 시험대 — ★사실 계층을 실제로 재는 항목 전용★.
    ///
    /// 기본 [`harness`] 는 `TurnWiring::detached()` 라 표가 침묵한다 — 그것으로 재면 「턴 중을 안
    /// 켰다」가 **안 켜서가 아니라 표가 꺼져 있어서** 통과한다.
    struct FactHarness {
        reader: Reader,
        state: SharedState,
        table: Arc<TurnObservations>,
        id: AgentId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }

    fn fact_harness() -> FactHarness {
        let id = AgentId::new_v4();
        let table = Arc::new(TurnObservations::new());
        table.register(id, 1);
        let core = Arc::new(OutputCore::new(
            id,
            1,
            Arc::new(NoopStatus),
            TurnWiring::new(table.clone(), super::super::classify_turn),
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: seen.clone(),
        }));
        let state = shared();
        let reader = Reader {
            core,
            decoder: Some(Box::new(super::super::decoder::CodexAppServerDecoder::new())),
            state: state.clone(),
            pending: Arc::new(Pending::default()),
        };
        FactHarness {
            reader,
            state,
            table,
            id,
            seen,
        }
    }

    /// ★열린 턴이 없는 동안 온 줄은 화면에는 오르되 「턴 중」을 켜지 않는다★ — 켜면 그 턴의 종료는
    /// 귀속 게이트에 막혀 아무도 끄지 못하고, 30 분 fail-open 밸브가 쓸어 갈 때까지 그 에이전트에는
    /// 우편이 안 간다(통로는 Idle = 큐 열림인데 사실 계층만 바쁨 — 두 축이 갈린다).
    #[test]
    fn a_line_that_arrives_with_no_open_turn_is_shown_but_never_marks_the_agent_busy() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");

        h.reader
            .handle_line(progress_line("T", "U-OTHER").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1), "미귀속 줄이 사실 계층을 켰다");
        // ★버려서 통과한 것이 아니다★ — 그 줄은 화면에 그대로 올라간다. 가른 것은 사실 계층뿐이다.
        assert!(
            h.seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, OutputEvent::TextDelta { .. })),
            "본문이 화면에서 사라졌다"
        );

        // 그 종료는 귀속 못 해 막힌다 — 그래도 표에는 끌 것이 남아 있지 않다.
        h.reader
            .handle_line(completed_line("T", "U-OTHER", "completed").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1));
        assert!(
            boundaries(&h.seen).is_empty(),
            "미귀속 경계가 화면에 올랐다"
        );
    }

    /// 그 짝 — 우리 턴이 열려 있는 동안 온 같은 줄은 켜고, 그 턴의 종료가 끈다.
    #[test]
    fn a_line_inside_our_open_turn_marks_the_agent_busy_and_its_completion_clears_it() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));

        h.reader
            .handle_line(progress_line("T", "U-MINE").as_bytes());
        assert!(h.table.is_in_turn(h.id, 1));
        h.reader
            .handle_line(completed_line("T", "U-MINE", "completed").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1));
    }

    /// ★응답보다 먼저 오는 item 알림은 정상이다★ — 그 창을 id 일치로 막으면 우리 턴의 스트리밍이
    /// 통째로 사실 계층 밖으로 나간다. 그래서 판정은 「일치하나」가 아니라 「어긋나지 않나」다.
    #[test]
    fn a_line_that_arrives_before_the_turn_id_is_known_still_counts_as_ours() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);

        h.reader
            .handle_line(progress_line("T", "U-MINE").as_bytes());
        assert!(h.table.is_in_turn(h.id, 1));
    }

    /// ★붙들어 둔 종료는 그 턴과 함께 버려진다★ — 남기면 상대가 id 를 재사용하는 순간
    /// [`Reader::resolve`] 가 **갓 열린 턴**을 그 자리에서 닫고 낡은 경계를 화면에 올린다.
    #[test]
    fn a_held_completion_does_not_survive_the_turn_that_was_open_when_it_arrived() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);

        h.reader
            .handle_line(completed_line("T", "U-1", "completed").as_bytes());
        assert_eq!(
            with_state(&h.state, |s| s.early_completions.len()),
            1,
            "응답보다 먼저 온 종료를 붙들지 않았다"
        );

        // 그 턴이 다른 길로 끝난다(시한 만료·쓰기 실패·오류 응답 — 전부 end_turn_if 로 온다).
        let core = h.reader.core.clone();
        assert!(end_turn_if(&h.state, &core, 0, "포기한다".into()));
        assert_eq!(
            with_state(&h.state, |s| s.early_completions.len()),
            0,
            "붙들어 둔 종료가 턴보다 오래 살았다"
        );

        // 다음 턴이 같은 id 를 받아도 열린 채로 산다.
        activate(&h.state, 1, None);
        assert!(h
            .pending
            .register(5, Waiter::TurnStart { seq: 1 }, method::TURN_START));
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-1"}}}"#);
        assert_eq!(
            turn_seq(&h.state),
            Some(1),
            "갓 열린 턴이 낡은 칸으로 닫혔다"
        );
        assert_eq!(
            boundaries(&h.seen).len(),
            1,
            "낡은 경계가 화면에 한 번 더 올랐다: {:?}",
            boundaries(&h.seen)
        );
    }

    /// ★응답이 정하는 turn id 에도 상한이 걸린다★ — 없으면 그 값이 그대로 경계의 칸으로 나가 replay
    /// 링의 단일 이벤트 상한을 홀로 넘고, 그 링이 나머지를 전부 쫓아낸다. 대조 토큰이라 자르지 않고
    /// 거르므로 그 턴은 귀속할 재료가 없다 — 그래서 오류 응답과 같은 등급으로 닫는다.
    #[test]
    fn an_oversized_turn_id_in_the_response_closes_the_turn_instead_of_becoming_a_boundary_field() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        assert!(h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START));

        let huge = "u".repeat(MAX_TURN_ID_BYTES + 1);
        h.reader.resolve(
            &RequestId::Num(5),
            Ok(serde_json::json!({"turn": {"id": huge}})),
        );

        assert_eq!(turn_seq(&h.state), None, "턴이 열린 채 남았다");
        let seen = boundaries(&h.seen);
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(matches!(seen[0].1, TurnOutcome::Failed { .. }), "{seen:?}");
        assert!(seen[0].0.is_none(), "상한을 넘은 id 가 경계에 실렸다");
    }

    /// 남의 턴·남의 thread 는 우리 턴이 열려 있어도 세지 않는다 — 경계 게이트와 같은 축이다.
    #[test]
    fn a_line_from_another_turn_or_thread_does_not_count_even_while_ours_is_open() {
        let mut h = fact_harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));

        h.reader
            .handle_line(progress_line("T", "U-OTHER").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1), "남의 턴이 켰다");
        h.reader
            .handle_line(progress_line("OTHER", "U-MINE").as_bytes());
        assert!(!h.table.is_in_turn(h.id, 1), "남의 thread 가 켰다");
    }

    fn outbox_lines(state: &SharedState) -> Vec<Value> {
        with_state(state, |s| {
            s.outbox
                .iter()
                .map(|l| serde_json::from_str(l).expect("나가는 줄은 JSON 이다"))
                .collect()
        })
    }

    /// 짧은 대기 — "깨지 **않았다**" 를 재는 자리에만 쓴다.
    const NOT_WOKEN: Duration = Duration::from_millis(50);

    // ── 봉투 라우팅: 서버 요청 ────────────────────────────────────────────

    #[test]
    fn a_server_request_is_refused_with_its_own_id_and_never_a_success() {
        let mut h = harness();
        h.reader
            .handle_line(br#"{"id":7,"method":"item/approval/request","params":{}}"#);

        let lines = outbox_lines(&h.state);
        assert_eq!(lines.len(), 1, "요청 하나에 답 하나: {lines:?}");
        assert_eq!(lines[0]["id"], 7, "받은 id 를 그대로 되돌려야 한다");
        assert_eq!(lines[0]["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            lines[0].get("result").is_none(),
            "★성공을 위장하면 그것이 자동 승인이다★: {:?}",
            lines[0]
        );
    }

    #[test]
    fn a_server_request_id_is_echoed_verbatim_when_it_is_a_string() {
        let mut h = harness();
        h.reader.handle_line(br#"{"id":"srv-1","method":"x"}"#);
        let lines = outbox_lines(&h.state);
        assert_eq!(
            lines[0]["id"], "srv-1",
            "문자열 id 를 숫자로 정규화하면 그 요청이 영영 안 풀린다"
        );
    }

    /// ★TRD §4-4 의 본체★ — 서버 id 와 우리 id 는 겹칠 수밖에 없다(서버 id 를 그대로 되돌려 주므로).
    /// 가르는 것은 값이 아니라 **봉투 모양**이다.
    #[test]
    fn a_server_request_whose_id_collides_with_ours_never_touches_the_pending_map() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::THREAD_START);

        h.reader
            .handle_line(br#"{"id":0,"method":"item/approval/request"}"#);

        assert_eq!(h.pending.len(), 1, "요청 봉투가 대기표를 건드렸다");
        assert!(
            rx.recv_timeout(NOT_WOKEN).is_err(),
            "승인 요청이 우리 thread/start 대기자에게 배달됐다"
        );
        assert_eq!(outbox_lines(&h.state).len(), 1, "그래도 답은 나가야 한다");
    }

    #[test]
    fn a_notification_is_never_looked_up_in_the_pending_map() {
        let mut h = harness();
        let (tx, _rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);
        h.reader
            .handle_line(br#"{"method":"turn/completed","params":{"threadId":"t"}}"#);
        assert_eq!(h.pending.len(), 1);
    }

    // ── 봉투 라우팅: 응답 ─────────────────────────────────────────────────

    #[test]
    fn two_waiters_do_not_cross() {
        let mut h = harness();
        let (tx0, rx0) = mpsc::channel();
        let (tx1, rx1) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx0), method::INITIALIZE);
        let _ = h
            .pending
            .register(1, Waiter::Handshake(tx1), method::THREAD_START);

        h.reader
            .handle_line(br#"{"id":1,"result":{"thread":{"id":"T-1"}}}"#);

        let got = rx1.recv_timeout(NOT_WOKEN).expect("1번 대기자가 깨야 한다");
        let parsed: ThreadStartResponse =
            serde_json::from_value(got.expect("성공 응답")).expect("해독");
        assert_eq!(parsed.thread.id, "T-1");
        assert!(
            rx0.recv_timeout(NOT_WOKEN).is_err(),
            "0번 대기자가 남의 답을 받았다"
        );
        assert_eq!(h.pending.len(), 1, "깬 대기표만 걷힌다");
    }

    #[test]
    fn an_error_envelope_wakes_the_waiter_as_a_failure() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(3, Waiter::Handshake(tx), method::INITIALIZE);
        h.reader
            .handle_line(br#"{"id":3,"error":{"code":-32600,"message":"Already initialized"}}"#);
        let got = rx.recv_timeout(NOT_WOKEN).expect("깨야 한다");
        let msg = got.expect_err("오류 봉투는 실패다");
        assert!(msg.contains("Already initialized"), "{msg}");
    }

    /// ★상대가 준 오류 본문은 로그·화면·호출자 셋으로 동시에 간다 — 마스킹은 그 셋 앞이 아니라 **만드는
    /// 자리**에서 한다★. 문이 셋이면 그중 하나는 반드시 잊힌다.
    #[test]
    fn a_peer_error_message_is_masked_before_it_leaves_this_module() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(4, Waiter::Handshake(tx), method::INITIALIZE);
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        h.reader.handle_line(
            format!(
                r#"{{"id":4,"error":{{"code":-32600,"message":"bad config token={secret}"}}}}"#
            )
            .as_bytes(),
        );
        let msg = rx
            .recv_timeout(NOT_WOKEN)
            .expect("깨야 한다")
            .expect_err("오류다");
        assert!(
            !msg.contains(secret),
            "자격증명이 그대로 새어 나갔다: {msg}"
        );
        assert!(msg.contains("***"), "{msg}");
    }

    /// ★[`clip`] 은 [`sanitize`] 안에서만 불려야 한다★ — 다른 자리에서 부르면 그 문자열이 마스킹을
    /// 건너뛰고 로그·화면으로 나간다. 문이 둘이면 그중 하나는 잊히고, 잊힌 쪽은 **조용히** 샌다.
    ///
    /// 이 항목은 그 성질을 **소스에서** 잰다 — 새어 나가는 경로마다 자격증명 항목을 하나씩 두는 것은
    /// 유지되지 않고, 안 둔 경로가 곧 새는 경로가 되기 때문이다.
    #[test]
    fn clip_is_only_reachable_through_the_masking_door() {
        let src = include_str!("transport.rs");
        // ★`#[cfg(test)]` 로 가르지 않는다★ — 그 속성은 운영 구획 안에도 있어(시험대 전용 접근자)
        //   거기서 잘리면 이 항목이 앞쪽만 훑고 **뒤쪽을 안 본다**.
        // ★줄바꿈이 든 표식도 쓰지 않는다★ — 이 저장소는 CRLF 로 체크아웃되므로 `\n` 이 안 맞는다.
        let production = src.split("mod tests {").next().expect("운영 구획");
        let offenders: Vec<&str> = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.contains("clip("))
            .filter(|l| !l.starts_with("//"))
            .filter(|l| !l.starts_with("fn clip(") && !l.starts_with("clip(&mask_secrets("))
            .collect();
        assert!(
            offenders.is_empty(),
            "마스킹을 건너뛰는 절단 호출이 있다: {offenders:?}"
        );
    }

    #[test]
    fn a_response_for_an_unknown_id_is_discarded_without_disturbing_anything() {
        let mut h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);

        h.reader.handle_line(br#"{"id":9999,"result":{}}"#);
        h.reader.handle_line(br#"{"id":"server-side","result":{}}"#);

        assert_eq!(h.pending.len(), 1, "모르는 id 가 남의 대기표를 걷었다");
        assert!(rx.recv_timeout(NOT_WOKEN).is_err());
        assert!(outbox_lines(&h.state).is_empty(), "응답에는 답하지 않는다");
        assert!(
            h.seen.lock().unwrap().is_empty(),
            "화면에 아무것도 올리지 않는다"
        );
    }

    /// 못 읽는 줄로 스트림이 끊기면 에이전트가 죽는다 — 버리고 계속 간다.
    #[test]
    fn unreadable_lines_are_dropped_and_the_next_line_still_routes() {
        let mut h = harness();
        h.reader.handle_line(b"not json at all");
        h.reader.handle_line(br#"{"no":"envelope"}"#);
        h.reader.handle_line(br#"{"id":1,"method":"x"}"#);
        assert_eq!(outbox_lines(&h.state).len(), 1);
    }

    // ── 알림 → 번역기 ─────────────────────────────────────────────────────

    #[test]
    fn a_notification_reaches_the_decoder_as_the_original_line_with_its_newline() {
        let mut h = harness();
        let line =
            br#"{"method":"item/agentMessage/delta","params":{"turnId":"T","itemId":"I","delta":"hi"}}"#;
        h.reader.handle_line(line);
        let got = h.decoded.lock().unwrap().clone();
        let mut expected = line.to_vec();
        expected.push(b'\n');
        assert_eq!(got, expected, "번역기에는 원본 줄이 그대로 가야 한다");
    }

    #[test]
    fn responses_and_requests_never_reach_the_decoder() {
        let mut h = harness();
        h.reader.handle_line(br#"{"id":1,"result":{}}"#);
        h.reader.handle_line(br#"{"id":2,"method":"x"}"#);
        assert!(
            h.decoded.lock().unwrap().is_empty(),
            "번역기가 받는 것은 알림뿐이다"
        );
    }

    // ── 턴 상태 기계 ──────────────────────────────────────────────────────

    /// ★`turn/started` 는 신원에 관여하지 않는다 — 되살리지 마라★. 그 알림에는 우리가 발급한 식별자가
    /// 없어 어느 턴의 것인지 원리상 못 가린다. 「지금 턴이 답을 기다리는 중인가」로 대신 가르면, 앞 턴의
    /// 늦은 알림이 **새 턴이 자기 요청을 기다리는 동안** 그 조건을 통과해 새 턴의 칸에 자기 id 를 적는다.
    #[test]
    fn a_turn_started_notification_never_sets_the_turn_id() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        // 지금 턴의 요청은 답을 기다리는 중이다 — 옛 정황 게이트가 통과시키던 바로 그 상태.
        let _ = h
            .pending
            .register(9, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        h.reader.handle_line(
            br#"{"method":"turn/started","params":{"threadId":"T","turn":{"id":"U-FROM-NOTIFY"}}}"#,
        );
        assert_eq!(
            turn_id_of(&h.state),
            None,
            "알림이 턴 id 를 정했다 — 그 자리에서 앞 턴의 늦은 알림도 같은 길로 들어온다"
        );
    }

    #[test]
    fn the_turn_id_can_come_from_the_response() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-2"}}}"#);
        assert_eq!(turn_id_of(&h.state).as_deref(), Some("U-2"));
    }

    /// ★앞 턴의 늦은 응답이 지금 턴의 칸에 자기 id 를 적으면 안 된다★ — 적히면 그 뒤의 `interrupt` 가
    /// 엉뚱한 턴을 겨눈다. 가르는 것은 표식이고, 표식이 없으면 둘 다 "턴 id 가 비어 있다" 로 보인다.
    #[test]
    fn a_late_response_from_a_finished_turn_never_lands_on_the_next_turn() {
        let mut h = harness();
        make_ready(&h.state, "T");
        // 0번 턴이 열렸고 그 응답이 아직 안 왔다.
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);
        // 그 턴이 끝나고 1번 턴이 열렸다.
        activate(&h.state, 1, None);
        // 이제 0번 턴의 답이 도착한다.
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-OLD"}}}"#);
        assert_eq!(
            turn_id_of(&h.state),
            None,
            "끝난 턴의 답이 지금 턴의 칸을 채웠다"
        );
        assert_eq!(turn_seq(&h.state), Some(1), "턴 자체는 그대로여야 한다");
    }

    /// ★모르는 threadId 는 오류가 아니다★ — 하지만 **아는데 다른** threadId 는 우리 턴이 아니다.
    #[test]
    fn a_turn_notification_for_another_thread_does_not_end_our_turn() {
        let mut h = harness();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U"));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"OTHER","turn":{"id":"U"}}}"#,
        );
        assert!(
            turn_seq(&h.state).is_some(),
            "남의 스레드가 우리 턴을 닫았다"
        );
    }

    /// ★같은 스레드의 **다른 턴**이 끝났다는 신호도 우리 턴을 닫으면 안 된다★ — 닫으면 다음 턴이 열려
    /// 한 스레드 위에서 턴 둘이 동시에 돈다. 입력 큐가 존재하는 이유가 정확히 그것을 막는 것이다.
    #[test]
    fn a_turn_completed_for_a_different_turn_on_our_thread_does_not_end_ours() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-MINE"));
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-OTHER"}}}"#,
        );
        assert!(
            turn_seq(&h.state).is_some(),
            "남의 턴 종료가 우리 턴을 닫았다"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "그 바람에 큐가 풀려 턴이 겹쳤다"
        );
    }

    /// ★turn id 를 모르는 동안 온 종료는 아무 것도 안 끝낸다★ — 귀속할 수 없는 신호로 살아 있는 턴을
    /// 닫으면 큐가 풀려 턴이 겹친다.
    #[test]
    fn a_turn_completed_we_cannot_attribute_ends_nothing() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-?"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "귀속 못 하는 종료가 턴을 닫았다"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "그 바람에 큐가 풀려 턴이 겹쳤다"
        );
    }

    /// ★실제로 막히던 순서를 그대로 재는 항목★: 종료가 먼저 오고(귀속 불가 → 무시), **사이에 sweep 없이**
    /// 응답이 와서 우리 턴의 id 를 정한다. 그 응답이 대기표를 걷어 가므로 여기서 안 끝내면 그 턴은 끝낼
    /// 것이 영영 없다 — 시한도 없고 두 번째 종료도 오지 않는다.
    ///
    /// ★sweep 을 끼우면 이 순서를 안 재게 된다★ — 시한이 대신 끝내 버려 응답 쪽 대조가 한 번도 안 불린다.
    #[test]
    fn an_early_completion_is_reconciled_when_the_response_names_it() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        // ① 종료가 먼저 온다 — 이 시점에는 우리 턴의 id 를 모르므로 귀속할 수 없다.
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-EARLY"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "귀속 못 하는 종료가 그 자리에서 턴을 닫았다"
        );

        // ② 그 다음 응답이 온다 — 여기서 우리 턴의 id 가 `U-EARLY` 로 정해진다. sweep 은 끼우지 않는다.
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-EARLY"}}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "응답이 id 를 정했는데 먼저 온 종료와 대조하지 않았다 — 이 턴은 끝낼 것이 영영 없다"
        );
        assert_eq!(
            h.pending.len(),
            0,
            "응답이 대기표를 걷어 갔다(시한 backstop 없음)"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "큐가 안 풀렸다"
        );
    }

    /// ★대조는 여전히 **id 일치** 하나뿐이다★ — 붙들어 둔 종료가 있다는 사실만으로 끝나는 턴은 없다.
    #[test]
    fn an_early_completion_for_another_turn_does_not_end_ours() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-SOMEONE-ELSE"}}}"#,
        );
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-MINE"}}}"#);

        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "남의 종료가 붙들려 있다는 이유로 우리 턴이 닫혔다"
        );
        assert_eq!(turn_id_of(&h.state).as_deref(), Some("U-MINE"));
    }

    // ── 턴 경계 귀속 게이트 ─────────────────────────────────────────────────
    //
    // 위 항목들이 재는 것은 **이 통로의 상태 기계**가 남의 종료에 안 속는다는 것이고, 아래 항목들이
    // 재는 것은 **화면과 사실 계층**이 같은 판정을 받는다는 것이다. 둘이 갈리면 통로는 큐를 닫은 채인데
    // 화면은 턴이 끝났다고 그리고, 사실 계층은 한가함을 관측해 턴 도중에 우편을 꽂는다.

    /// ★리뷰가 이름한 그 입력 그대로다★: 우리 턴이 `U-MINE` 인데 같은 thread 의 `U-OTHER` 종료가 온다.
    /// 통로는 그것을 무시하지만, 번역기는 귀속을 몰라 경계를 낸다 — 그 경계가 그대로 흐르면 분류자도
    /// 프론트도 id 를 안 보므로 **살아 있는 `U-MINE` 이 끝난 것으로 찍힌다.**
    #[test]
    fn a_foreign_turn_completed_never_ends_the_live_turn() {
        let (mut h, turns, id) = harness_with_facts();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        // 되울린 유저 메시지 = 이 턴이 도는 중이라는 첫 관측.
        h.reader.core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(turns.is_in_turn(id, 1), "전제: 턴 중으로 관측된다");

        h.reader
            .handle_line(completed_line("MINE", "U-OTHER", "completed").as_bytes());

        assert!(
            boundaries(&h.seen).is_empty(),
            "귀속 안 되는 종료가 화면 경계를 냈다"
        );
        assert!(
            turns.is_in_turn(id, 1),
            "남의 턴 종료가 우리 턴을 한가함으로 뒤집었다 — 턴 도중에 우편이 꽂힌다"
        );
        assert_eq!(turn_seq(&h.state), Some(0), "상태 기계까지 흔들렸다");
    }

    /// 다른 thread 의 종료도 같다 — 막되, 로그로는 남는다(사유 = [`Reader::note_turn`] doc).
    #[test]
    fn a_turn_completed_from_another_thread_makes_no_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "MINE");
        activate(&h.state, 0, Some("U-MINE"));
        h.reader
            .handle_line(completed_line("OTHER", "U-MINE", "completed").as_bytes());
        assert!(boundaries(&h.seen).is_empty(), "{:?}", boundaries(&h.seen));
    }

    /// ★귀속되면 경계는 **그대로** 흐른다 — 결말까지★. 게이트가 「막는 것」만 하고 통과를 못 시키면
    /// 대기 인디케이터가 영영 돈다.
    #[test]
    fn an_attributed_turn_completed_makes_exactly_one_boundary_with_its_outcome() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        h.reader
            .handle_line(completed_line("T", "U-1", "interrupted").as_bytes());
        assert_eq!(
            boundaries(&h.seen),
            vec![(Some("U-1".to_string()), TurnOutcome::Interrupted)]
        );
        assert_eq!(turn_seq(&h.state), None, "상태 기계도 함께 닫혀야 한다");
    }

    /// ★응답보다 먼저 온 종료의 경계는 **버리지도 흘리지도 않고 미룬다**★ — 그 자리에서 흘리면 아직
    /// 귀속되지 않은 종료가 살아 있는 턴을 닫고, 버리면 대조가 성립한 뒤에 올릴 것이 없어 대기 표시가
    /// 영영 돈다. 결말을 알고 있었는데도 그렇다.
    #[test]
    fn an_early_completion_boundary_is_held_until_the_response_names_it() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(completed_line("T", "U-EARLY", "interrupted").as_bytes());
        assert!(
            boundaries(&h.seen).is_empty(),
            "귀속 전에 경계가 흘렀다 — 살아 있는 턴이 화면에서 닫힌다"
        );

        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-EARLY"}}}"#);
        assert_eq!(
            boundaries(&h.seen),
            vec![(Some("U-EARLY".to_string()), TurnOutcome::Interrupted)],
            "붙들어 둔 결말이 대조 뒤에도 안 올라왔다"
        );
    }

    /// 대조는 여전히 **id 일치** 하나뿐이다 — 붙들려 있다는 사실만으로 올라오는 경계는 없다.
    #[test]
    fn an_early_completion_for_another_turn_never_releases_its_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(completed_line("T", "U-SOMEONE-ELSE", "completed").as_bytes());
        h.reader
            .handle_line(br#"{"id":5,"result":{"turn":{"id":"U-MINE"}}}"#);
        assert!(boundaries(&h.seen).is_empty(), "{:?}", boundaries(&h.seen));
    }

    // ── 결말 없이 죽는 턴(ADR-0127) ─────────────────────────────────────────

    /// ★턴을 접는 유일한 함수는 **언제나** 경계를 낸다★ — 그것이 이 통로의 포기 경로 넷(쓰기 실패·응답
    /// 해독 실패·오류 응답·시한 만료)이 사실 계층을 되돌리는 유일한 수단이다. 그리고 표식이 안 맞으면
    /// 아무 것도 안 끝내고 **경계도 안 낸다** — 늦게 온 신호가 다음 턴의 경계를 지어내면 안 된다.
    #[test]
    fn ending_a_turn_always_makes_a_failure_boundary_and_a_stale_marker_makes_none() {
        let (core, seen) = core_with_sink();
        let state = shared();
        activate(&state, 3, Some("U-3"));

        assert!(
            !end_turn_if(&state, &core, 2, "늦게 온 신호".into()),
            "표식이 다른데 끝냈다"
        );
        assert!(boundaries(&seen).is_empty(), "안 끝냈는데 경계를 냈다");

        assert!(end_turn_if(&state, &core, 3, "포기한다".into()));
        assert_eq!(
            boundaries(&seen),
            vec![(
                Some("U-3".to_string()),
                TurnOutcome::Failed {
                    detail: Some("포기한다".to_string())
                }
            )],
            "사유는 경계 **안**에 실린다 — 따로 오류 줄을 앞세우지 않는다"
        );
    }

    /// ★이것이 ADR-0127 이 막으려는 결말 그 자체다★: 턴 도중에 통로가 포기했는데 사실 계층이 되돌아가지
    /// 않으면, 그 화신은 한가한데도 「턴 중」으로 관측된 채 남아 30 분 fail-open 밸브가 쓸어 갈 때까지
    /// 우편이 막힌다. ★[`OutputEvent::Error`] 한 줄로는 이것이 안 된다★ — 분류자가 그 어휘를 턴 신호로
    /// 세지 않는다(그것이 이 결함의 기전이었다).
    #[test]
    fn a_turn_abandoned_without_a_completion_clears_the_fact_layer() {
        let (h, turns, id) = harness_with_facts();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        h.reader.core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(turns.is_in_turn(id, 1), "전제: 턴 중으로 관측된다");

        assert!(end_turn_if(
            &h.state,
            &h.reader.core,
            0,
            "codex app-server 입력 전송 실패".into()
        ));
        assert!(
            !turns.is_in_turn(id, 1),
            "턴을 접었는데 사실 계층은 여전히 턴 중이다"
        );
    }

    /// `turn/start` 에 오류 응답이 오는 경로 — 포기 넷 중 하나. 사유가 경계 안에 실려 나간다.
    #[test]
    fn an_error_response_to_turn_start_ends_the_turn_with_its_reason_inside_the_boundary() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        h.reader
            .handle_line(br#"{"id":5,"error":{"code":-32000,"message":"turn refused"}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "오류 응답이 턴을 열어 둔 채 남겼다"
        );
        let failures = failure_boundaries(&h.seen);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("turn refused"), "{failures:?}");
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "큐가 안 풀렸다"
        );
    }

    /// ★포기로 닫은 턴에 뒤늦게 진짜 종료가 와도 경계는 **하나**다★ — 둘이 나가면 화면이 같은 턴을 두 번
    /// 닫고, 사실 계층에는 짝 없는 종료가 한 번 더 쌓인다.
    #[test]
    fn an_abandoned_turn_and_its_late_completion_make_exactly_one_boundary() {
        let mut h = harness_with_real_decoder();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U-1"));
        let _ = h.pending.register_at(
            9,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );

        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        h.reader
            .handle_line(completed_line("T", "U-1", "completed").as_bytes());

        let seen = boundaries(&h.seen);
        assert_eq!(seen.len(), 1, "같은 턴이 두 번 닫혔다: {seen:?}");
        assert!(
            matches!(seen[0].1, TurnOutcome::Failed { .. }),
            "먼저 선 결말이 뒤집혔다: {seen:?}"
        );
    }

    /// stdin 없이 라이터를 한 번 돌려 핸드셰이크를 실패시킨다. `queued` = 그 창에서 이미 `Ok` 를 받고
    /// 큐에 서 있던 본문.
    fn run_failed_handshake(queued: &[&str]) -> (SharedState, Arc<Mutex<Vec<OutputEvent>>>) {
        let (core, seen) = core_with_sink();
        let state = shared();
        with_state(&state, |s| {
            for body in queued {
                s.input.push_back(body.as_bytes().to_vec());
            }
        });

        writer_loop(
            // stdin 이 없어 첫 쓰기에서 핸드셰이크가 실패한다.
            Arc::new(Mutex::new(None)),
            state.clone(),
            Arc::new(Pending::default()),
            Arc::new(AtomicI64::new(0)),
            // 핸드셰이크 뒤 루프는 첫 검사에서 끝난다 — 이 항목들이 재는 것은 그 앞 구획이다.
            Arc::new(AtomicBool::new(true)),
            core,
            ThreadOpen::Start(ThreadStartParams {
                cwd: None,
                approval_policy: None,
                sandbox: None,
            }),
            None,
            None,
        );

        (state, seen)
    }

    /// 화면에 오른 오류·경계를 **온 순서대로** 뽑는다 — 이 갈래에서 재야 하는 것이 개수만이 아니라
    /// 「사유가 경계보다 먼저 오르나」이기 때문이다.
    fn error_and_boundary_order(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<&'static str> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::Error(_) => Some("error"),
                OutputEvent::TurnEnd { .. } => Some("boundary"),
                _ => None,
            })
            .collect()
    }

    /// ★버린 입력이 하나도 없어도 실패 결말 **하나**가 나간다★ — 턴을 여는 자리가 [`Link::Ready`] 를
    /// 요구해 이 화신에 턴이 선 적은 없지만, 프론트는 `Error` 를 턴 종료로 읽지 않으므로 오류 하나만
    /// 든 슬롯의 **대기 표시가 영영 돈다**(링크는 `Down` 이라 뒤에 도착할 것도 없다).
    /// ★그 사유는 입력 손실을 주장하지 않는다★ — 버린 것이 없기 때문이다.
    #[test]
    fn a_failed_handshake_with_nothing_queued_still_ends_the_turn_exactly_once() {
        let (state, seen) = run_failed_handshake(&[]);

        assert_eq!(
            error_and_boundary_order(&seen),
            vec!["error", "boundary"],
            "사유 한 줄 뒤 경계 하나가 아니다: {:?}",
            error_and_boundary_order(&seen)
        );
        assert_eq!(
            boundaries(&seen)[0].0,
            None,
            "열린 적 없는 턴의 id 를 지어냈다: {:?}",
            boundaries(&seen)
        );
        let failures = failure_boundaries(&seen);
        assert_eq!(failures.len(), 1, "실패 결말이 아니다: {failures:?}");
        assert!(
            !failures[0].contains("사라졌다"),
            "버린 입력이 없는데 손실을 보고한다: {failures:?}"
        );
        assert!(with_state(&state, |s| matches!(s.link, Link::Down(_))));
    }

    /// ★큐에 선 본문을 파괴했으면 실패 결말 **하나**가 나간다★ — 보낸 쪽에는 턴이 있었기 때문이고,
    /// `dropped` 는 버린 본문 수이지 턴 수가 아니다. ★이 경계가 없으면★ 프론트는 `Error` 를 턴 종료로
    /// 읽지 않으므로(재시도되는 스트림 오류가 그 어휘로 온다) 오류 하나만 받은 슬롯의 **대기 표시가
    /// 영영 돈다**.
    #[test]
    fn a_failed_handshake_that_destroyed_queued_input_ends_the_turn_exactly_once() {
        let (state, seen) = run_failed_handshake(&["queued one", "queued two"]);

        assert_eq!(
            error_and_boundary_order(&seen),
            vec!["error", "boundary"],
            "사유 한 줄 뒤 경계 하나가 아니다: {:?}",
            error_and_boundary_order(&seen)
        );
        assert_eq!(
            boundaries(&seen)[0].0,
            None,
            "열린 적 없는 턴의 id 를 지어냈다: {:?}",
            boundaries(&seen)
        );
        let failures = failure_boundaries(&seen);
        assert_eq!(failures.len(), 1, "실패 결말이 아니다: {failures:?}");
        assert!(failures[0].contains("2건"), "{failures:?}");
        assert!(with_state(&state, |s| s.input.is_empty()));
        assert!(with_state(&state, |s| matches!(s.link, Link::Down(_))));
    }

    /// ★턴을 idle 로 되돌리는 자리를 늘리면 여기가 빨개진다★ — 그 자리마다 「경계는 누가 내나」를 답해야
    /// 하고, 안 답한 자리가 곧 사실 계층이 굳는 자리다(ADR-0127). 오늘 넷 = [`end_turn_if`](경계를
    /// 스스로 낸다) · [`Reader::note_turn`](번역기 경계를 통과시킨다) · [`Reader::resolve`](붙들어 둔
    /// 경계를 올린다) · [`ReaderExit::drop`](스트림이 끝났다 — 곧 `OutputCore::finish` 가 표를 거둔다).
    #[test]
    fn every_place_that_idles_a_turn_is_accounted_for() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let sites = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("s.turn = TurnState::Idle"))
            .count();
        assert_eq!(
            sites, 4,
            "턴을 idle 로 되돌리는 자리가 넷이 아니다 — 새 자리는 자기 턴 경계를 함께 내야 한다"
        );
        // ★그 자리마다 붙들어 둔 종료도 버려야 한다★ — 안 버리면 그 칸이 다음 턴까지 살아남아,
        //   상대가 id 를 재사용하는 순간 갓 열린 턴이 낡은 경계로 닫힌다([`end_turn_if`] 의 그 줄).
        let drops = production
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("s.early_completions.clear()"))
            .count();
        assert_eq!(
            drops, sites,
            "idle 자리 수와 붙들어 둔 종료를 버리는 자리 수가 다르다"
        );
    }

    /// ★귀속 못 한 종료를 버려도 그 턴의 **시한은 그대로 남아야 한다**★ — 무시하는 김에 대기표까지
    /// 걷으면 그 턴은 끝낼 것이 아무 것도 없어진다. 이 항목이 재는 것은 그 하나이고, 「귀속 못 한 턴에는
    /// 언제나 시한이 있다」는 **아니다**(그 일반화는 거짓이다 — 종료가 응답보다 먼저 오는 경우가 있고,
    /// 그 칸은 모듈 헤더 「알려진 한계」가 진다).
    #[test]
    fn an_ignored_completion_leaves_the_turn_s_deadline_intact() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        let _ = h.pending.register_at(
            7,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );

        // 귀속 못 하는 종료가 먼저 지나간다 — 아무 것도 끝내지 않고, 아무 것도 걷어 가지 않아야 한다.
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U-?"}}}"#,
        );
        assert_eq!(
            turn_seq(&h.state),
            Some(0),
            "귀속 못 하는 종료가 턴을 닫았다"
        );
        assert_eq!(
            h.pending.len(),
            1,
            "무시하면서 대기표까지 걷었다 — 그러면 끝낼 것이 없어진다"
        );

        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(
            turn_seq(&h.state),
            None,
            "남아 있던 시한이 그 턴을 안 끝냈다"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "큐가 안 풀렸다"
        );
    }

    /// ★해독 못 한 성공 응답은 명시적 오류와 **같은 등급**이다★ — 대기표는 이미 걷힌 뒤라, 여기서 턴을
    /// 안 끝내면 그 턴은 `turn_id` 도 시한도 없이 영영 열린 채 남는다(그 뒤 `send_input` 은 그 정지를
    /// "큐가 찼다" 로 신고한다).
    #[test]
    fn a_success_response_we_cannot_read_ends_the_turn_like_an_error_would() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        let _ = h
            .pending
            .register(5, Waiter::TurnStart { seq: 0 }, method::TURN_START);

        // `turn` 칸이 없는 성공 응답 — 봉투는 멀쩡하고 payload 만 우리 타입으로 안 읽힌다.
        h.reader
            .handle_line(br#"{"id":5,"result":{"not-a-turn":true}}"#);

        assert_eq!(
            turn_seq(&h.state),
            None,
            "해독 못 한 성공이 턴을 열어 둔 채 남겼다"
        );
        assert_eq!(
            h.pending.len(),
            0,
            "대기표는 이미 걷혔다 — 시한 backstop 이 없다"
        );
        assert_eq!(
            failure_boundaries(&h.seen).len(),
            1,
            "조용히 접으면 화면에도, 사실 계층에도 신호가 하나도 없다"
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "큐가 안 풀렸다"
        );
    }

    /// ★이미 끝난 턴의 시한이 지금 도는 턴을 닫으면 안 된다★ — 같은 겹침이 시한 쪽으로도 열린다.
    #[test]
    fn an_expired_deadline_from_a_finished_turn_does_not_end_the_current_turn() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 1, None);
        let _ = h.pending.register_at(
            5,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(
            turn_seq(&h.state),
            Some(1),
            "끝난 턴의 시한이 지금 턴을 닫았다"
        );
    }

    // ── 입력 큐 ───────────────────────────────────────────────────────────

    #[test]
    fn queued_input_flushes_in_order_one_turn_at_a_time() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.input.push_back(b"first".to_vec());
            s.input.push_back(b"second".to_vec());
            s.input.push_back(b"third".to_vec());
        });

        let mut bodies = Vec::new();
        for _ in 0..3 {
            let job = with_state(&h.state, |s| take_turn_locked(s, &h.next_id))
                .expect("idle 이면 한 건이 나간다");
            let Job::Turn { line, .. } = job else {
                panic!("turn job 이어야 한다")
            };
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["params"]["threadId"], "T");
            bodies.push(
                v["params"]["input"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );

            assert!(
                with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
                "턴이 진행 중인데 두 번째 turn/start 가 나갔다"
            );
            with_state(&h.state, |s| s.turn = TurnState::Idle);
        }
        assert_eq!(bodies, vec!["first", "second", "third"]);
    }

    #[test]
    fn turn_completed_releases_the_queue() {
        let mut h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, Some("U"));
        with_state(&h.state, |s| s.input.push_back(b"queued".to_vec()));
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "턴 중에는 안 나간다"
        );
        h.reader.handle_line(
            br#"{"method":"turn/completed","params":{"threadId":"T","turn":{"id":"U"}}}"#,
        );
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "턴이 끝났는데 큐가 안 풀렸다"
        );
    }

    /// ★이 항목은 원자성을 **재지 못한다** — 그 성질은 시그니처가 이미 지고 있다★: [`take_turn_locked`]
    /// 는 `&mut State` 를 받으므로 배타 접근이 컴파일러 보장이고, 락은 이 파일의 하네스(`with_state`)에
    /// 있다. 그래서 이 항목을 빨갛게 만들 수 있는 변이는 사실상 없다.
    /// 그래도 남겨 두는 것은 **회계가 맞는지**(여덟이 달려들어도 큐에서 한 건만 소비된다)를 보기 때문이고,
    /// 그것 하나가 이 항목이 재는 전부다.
    #[test]
    fn two_concurrent_takers_open_exactly_one_turn() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.input.push_back(b"a".to_vec());
            s.input.push_back(b"b".to_vec());
        });
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let opened = Arc::new(AtomicI64::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let state = h.state.clone();
            let next_id = h.next_id.clone();
            let barrier = barrier.clone();
            let opened = opened.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                if with_state(&state, |s| take_turn_locked(s, &next_id)).is_some() {
                    opened.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(opened.load(Ordering::Relaxed), 1, "턴이 둘 이상 열렸다");
        assert_eq!(
            with_state(&h.state, |s| s.input.len()),
            1,
            "한 건만 소비돼야 한다"
        );
    }

    /// 이 항목이 재는 것은 **`Ready` 전이 하나**다 — 「그 값이 디스크에 있다」가 아니다(그 보장은 이
    /// 통로가 지지 않는다: 기록 포트는 실패를 자기 안에서 삼킨다).
    /// ★`Ready` 를 세우지 않고 thread id 만 채워 재는 것이 요점이다★ — 둘을 함께 채우면 id 부재가 대신
    /// 막아 주어 **게이트를 지워도 초록이 된다**(변이로 실측).
    #[test]
    fn nothing_is_sent_before_the_link_is_ready_even_when_the_thread_id_is_known() {
        let h = harness();
        with_state(&h.state, |s| {
            s.thread_id = Some("T".into());
            s.input.push_back(b"early".to_vec());
        });
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "기록 호출이 돌아오기 전에 turn/start 가 나갔다"
        );
        with_state(&h.state, |s| s.link = Link::Ready);
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some(),
            "준비됐는데 큐가 안 풀렸다"
        );
    }

    // ── 기록 호출의 두 예외 갈래 ──────────────────────────────────────────────

    /// ★이미 끝난 화신에는 기록하지 않는다★ — 적히면 죽은 세션의 프로필이 남의 대화를 자기 것으로 들고,
    /// 옛 sid 가 이력으로 밀린다. ★단 이것이 재는 것은 **이미 관측된** 종료뿐이다★ — 그 판정과 실제
    /// 기록 사이에 끝나는 세션은 이 항목이 재는 범위 밖이고 오늘 통과한다(`record_session_id` doc).
    /// ★표식 **둘 다** 재는 것이 요점이다★ — `shutdown()` 은 원자를 먼저 세우고 그 다음에 상태 락을 잡아
    /// `closed` 를 세운다. 그 사이 구간은 비어 있지 않으므로 한쪽만 보는 울타리는 그만큼 샌다.
    #[test]
    fn a_session_that_already_ended_is_not_recorded() {
        for (closed, shutting_down) in [(true, false), (false, true), (true, true)] {
            let state = shared();
            with_state(&state, |s| s.closed = closed);
            let calls = Arc::new(AtomicI64::new(0));
            let sink: SessionIdSink = {
                let calls = calls.clone();
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                })
            };

            assert!(
                record_session_id(&state, &AtomicBool::new(shutting_down), Some(&sink), "T")
                    .is_ok(),
                "정상 종료를 실패 결말로 올렸다 (closed={closed}, shutdown={shutting_down})"
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                0,
                "끝난 세션에 기록했다 (closed={closed}, shutdown={shutting_down})"
            );
        }
    }

    /// 짝 방향 — 둘 다 안 섰으면 기록한다. 없으면 위 항목이 「언제나 건너뜀」으로도 초록이다.
    #[test]
    fn a_live_session_is_recorded() {
        let state = shared();
        let calls = Arc::new(AtomicI64::new(0));
        let sink: SessionIdSink = {
            let calls = calls.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
            })
        };

        assert!(record_session_id(&state, &AtomicBool::new(false), Some(&sink), "T").is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    /// ★기록이 패닉해도 라이터를 데려가지 않는다★ — unwind 가 올라가면 `Ready` 도 `Down` 도 서지 않아
    /// 링크가 `Connecting` 에 멈추고, 그 상태에서 [`AgentTransport::send_input`] 은 닫힘도 `Down` 도 못
    /// 보고 `Ok` 를 돌려주며 큐만 찬다(ADR-0190 이 금한 「조용히 멈춤」). `Err` 로 바뀌어야 라이터가
    /// 실패 갈래로 가 링크를 내린다.
    #[test]
    fn a_panicking_sink_becomes_a_failure_instead_of_unwinding() {
        let state = shared();
        let sink: SessionIdSink = Arc::new(|_| panic!("기록 포트가 터졌다"));

        // ★훅 교체를 맨손으로 하지 않는다★ — 훅은 프로세스 전역이라, 같은 바이너리에서 동시에 도는 다른
        //   항목이 자기 패닉 출력을 잃거나(진단 불가) 중첩 구간이 조용한 훅을 영구히 남긴다. 그 두 사고를
        //   막는 헬퍼가 이미 있다.
        let out = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            record_session_id(&state, &AtomicBool::new(false), Some(&sink), "T")
        });

        assert!(out.is_err(), "패닉이 실패로 바뀌지 않았다: {out:?}");
        assert!(
            with_state(&state, |s| matches!(s.link, Link::Connecting)),
            "이 함수가 링크를 직접 옮겼다 — 옮기는 것은 호출자(라이터의 실패 갈래)다"
        );
    }

    /// 반대 방향의 짝 — `Ready` 인데 thread id 가 없으면 봉투를 만들 수 없다.
    #[test]
    fn nothing_is_sent_without_a_thread_id() {
        let h = harness();
        with_state(&h.state, |s| {
            s.link = Link::Ready;
            s.input.push_back(b"early".to_vec());
        });
        assert!(
            with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_none(),
            "thread id 없이 turn/start 가 나갔다"
        );
    }

    /// 라이터가 제어 줄을 유저 턴보다 **먼저** 집는다 — 거절이 큐 뒤에서 기다리면 상대가 영구 정지한다.
    #[test]
    fn the_writer_takes_control_lines_before_queued_input() {
        let h = harness();
        make_ready(&h.state, "T");
        with_state(&h.state, |s| {
            s.input.push_back(b"user turn".to_vec());
            s.outbox.push_back("{\"id\":1,\"error\":{}}\n".to_string());
        });
        match next_job(&h.state, &h.next_id).expect("할 일이 있다") {
            Job::Line(l) => assert!(l.contains("error")),
            other => panic!("제어 줄이 먼저여야 한다: {}", matches!(other, Job::Sweep)),
        }
    }

    // ── 시한 ──────────────────────────────────────────────────────────────

    #[test]
    fn a_request_with_no_answer_wakes_its_waiter_at_the_deadline() {
        let h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register_at(0, Waiter::Handshake(tx), method::INITIALIZE, Instant::now());
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        let msg = rx
            .recv_timeout(NOT_WOKEN)
            .expect("시한이 대기자를 깨워야 한다")
            .expect_err("시한은 실패다");
        assert!(msg.contains(method::INITIALIZE), "{msg}");
        assert_eq!(h.pending.len(), 0, "만료된 대기표는 걷힌다");
    }

    #[test]
    fn a_turn_start_deadline_ends_the_turn_and_releases_the_queue() {
        let h = harness();
        make_ready(&h.state, "T");
        activate(&h.state, 0, None);
        with_state(&h.state, |s| s.input.push_back(b"next".to_vec()));
        let _ = h.pending.register_at(
            0,
            Waiter::TurnStart { seq: 0 },
            method::TURN_START,
            Instant::now(),
        );
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);

        assert!(
            with_state(&h.state, |s| matches!(s.turn, TurnState::Idle)),
            "시한이 지났는데 턴이 영원히 진행 중이다"
        );
        assert_eq!(
            failure_boundaries(&h.seen).len(),
            1,
            "조용히 접으면 화면에도, 사실 계층에도 신호가 하나도 없다"
        );
        assert!(with_state(&h.state, |s| take_turn_locked(s, &h.next_id)).is_some());
    }

    #[test]
    fn a_deadline_that_has_not_passed_does_not_fire() {
        let h = harness();
        let (tx, rx) = mpsc::channel();
        let _ = h
            .pending
            .register(0, Waiter::Handshake(tx), method::INITIALIZE);
        sweep_deadlines(&h.state, &h.pending, &h.reader.core);
        assert_eq!(h.pending.len(), 1);
        assert!(rx.recv_timeout(NOT_WOKEN).is_err());
    }

    // ── 라인 재조립 ───────────────────────────────────────────────────────

    #[test]
    fn a_line_split_across_chunks_is_reassembled() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        sp.feed(br#"{"met"#, |l| lines.push(l.to_vec()));
        assert!(lines.is_empty(), "개행 전에는 줄이 완성되지 않는다");
        sp.feed("hod\":\"한글\"}\n{\"a\":1}\n".as_bytes(), |l| {
            lines.push(l.to_vec())
        });
        assert_eq!(lines.len(), 2);
        assert_eq!(
            String::from_utf8(lines[0].clone()).unwrap(),
            "{\"method\":\"한글\"}"
        );
    }

    #[test]
    fn a_multibyte_char_split_at_a_chunk_boundary_survives() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let payload = "가".as_bytes();
        sp.feed(&payload[..1], |l| lines.push(l.to_vec()));
        sp.feed(&payload[1..], |l| lines.push(l.to_vec()));
        sp.feed(b"\n", |l| lines.push(l.to_vec()));
        assert_eq!(lines.len(), 1);
        assert_eq!(String::from_utf8(lines[0].clone()).unwrap(), "가");
    }

    #[test]
    fn an_oversize_line_is_dropped_and_the_next_line_survives() {
        let mut sp = LineSplitter::new();
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let huge = vec![b'x'; MAX_LINE_BYTES + 1];
        sp.feed(&huge, |l| lines.push(l.to_vec()));
        assert!(lines.is_empty());
        // ★그 줄의 꼬리가 새 줄로 파싱되면 안 된다★ — 다음 개행까지 통째로 버린다.
        sp.feed(b"tail-of-the-huge-line\n{\"a\":1}\n", |l| {
            lines.push(l.to_vec())
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(String::from_utf8(lines[0].clone()).unwrap(), "{\"a\":1}");
    }

    // ── 실 프로세스: 수령 의미 · teardown · 자식 누수 ─────────────────────

    #[cfg(windows)]
    fn probe_spec(args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    #[cfg(windows)]
    fn open_probe_owned(args: Vec<String>) -> (CodexAppServerTransport, Option<u32>) {
        let spec = CommandSpec {
            program: "cmd.exe".into(),
            args,
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        CodexAppServerTransport::open(
            &spec,
            true,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open")
    }

    #[cfg(windows)]
    fn open_probe(args: &[&str]) -> (CodexAppServerTransport, Option<u32>) {
        CodexAppServerTransport::open(
            &probe_spec(args),
            true,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open")
    }

    #[cfg(windows)]
    #[test]
    fn capabilities_reflect_the_injected_structured_flag_and_the_real_control_axis() {
        let (json, _) = open_probe(&["/c", "echo caps-probe"]);
        let caps = json.capabilities();
        assert!(caps.output.structured, "주입값 그대로 신고");
        assert!(!caps.output.terminal_bytes);
        assert!(!caps.control.resize);
        assert!(
            caps.control.interrupt,
            "이 통로는 turn/interrupt 를 실제로 낸다"
        );
        assert!(!caps.input.raw, "키 입력 채널이 아니다");
        assert!(caps.input.message);
        json.shutdown();

        let (plain, _) = CodexAppServerTransport::open(
            &probe_spec(&["/c", "echo caps-probe"]),
            false,
            None,
            ThreadOpen::Start(ThreadStartParams::default()),
            None,
            None,
        )
        .expect("open");
        assert!(
            !plain.capabilities().output.structured,
            "★하드코딩 금지★ — 통로는 자기가 무엇을 나르는지 모른다"
        );
        plain.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn input_is_queued_before_ready_and_refused_over_the_bound() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        for i in 0..INPUT_QUEUE_LIMIT {
            assert!(
                t.send_input(InputEvent::Raw(format!("turn {i}").into_bytes()))
                    .is_ok(),
                "핸드셰이크 창의 입력은 큐가 받는다(항목 {i})"
            );
        }
        let over = t.send_input(InputEvent::Raw(b"one too many".to_vec()));
        assert!(
            matches!(over, Err(PtyError::WriteFailed(_))),
            "★상한 초과는 Err 다 — 짧은 Ok 로 축소 보고하지 않는다★: {over:?}"
        );
        assert_eq!(
            with_state(&t.state, |s| s.input.len()),
            INPUT_QUEUE_LIMIT,
            "거절한 항목이 큐에 들어갔다"
        );
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn input_after_the_link_is_down_is_an_error() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        with_state(&t.state, |s| s.link = Link::Down("핸드셰이크 실패".into()));
        let out = t.send_input(InputEvent::Raw(b"hi".to_vec()));
        assert!(matches!(out, Err(PtyError::WriteFailed(_))), "{out:?}");
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn interrupt_needs_a_turn_and_carries_both_ids() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        assert!(
            matches!(t.interrupt(), Err(PtyError::Unsupported(_))),
            "중단할 턴이 없으면 봉투를 만들지 않는다"
        );
        make_ready(&t.state, "T-9");
        activate(&t.state, 0, Some("U-9"));
        t.interrupt().expect("턴이 있으면 나간다");
        let lines = outbox_lines(&t.state);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["method"], method::TURN_INTERRUPT);
        assert_eq!(lines[0]["params"]["threadId"], "T-9");
        assert_eq!(lines[0]["params"]["turnId"], "U-9");
        t.shutdown();
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_is_idempotent_and_leaves_no_child_behind() {
        let (t, pid) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let pid = pid.expect("pid");
        assert!(engram_dashboard_base::platform::pid_alive(pid));
        t.shutdown();
        t.shutdown();
        t.shutdown();
        assert!(
            !engram_dashboard_base::platform::pid_alive(pid),
            "shutdown 뒤에도 자식이 살아 있다"
        );
        assert!(
            t.send_input(InputEvent::Raw(b"x".to_vec())).is_err(),
            "닫힌 통로가 입력을 받았다"
        );
    }

    /// ★통로가 닫힌 뒤에 걸린 대기표는 시한이 다 찰 때까지 자기 스레드를 붙든다★ — 핸드셰이크가 두
    /// 요청을 잇달아 내므로 그 사이에 닫히는 창이 실재한다. 대기표 자체가 닫히면 그 대기가 즉시 끝난다.
    ///
    /// ★아래 **시간** 단언은 대기표 닫힘을 재지 못한다★ — `shutdown()` 이 stdin 을 이미 가져가 버려
    /// 쓰기가 바로 실패하므로, 닫힘 표식을 지워도 이 함수는 빨리 돌아온다. 그 표식을 실제로 재는 것은 위
    /// 두 단언(새 대기표가 거절되나 · `interrupt` 가 거절되나)뿐이다.
    #[cfg(windows)]
    #[test]
    fn after_shutdown_a_request_fails_at_once_instead_of_waiting_out_the_deadline() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        make_ready(&t.state, "T-1");
        activate(&t.state, 0, Some("U-1"));
        t.shutdown();

        assert!(
            !t.pending.register(77, Waiter::Fire, method::TURN_INTERRUPT),
            "닫힌 대기표가 새 항목을 받았다"
        );
        assert!(t.interrupt().is_err(), "닫힌 통로가 봉투를 만들었다");

        let start = Instant::now();
        let out: Result<Value, String> = request_blocking(
            &t.stdin,
            &t.state,
            &t.pending,
            &t.next_id,
            method::TURN_INTERRUPT,
            &TurnInterruptParams {
                thread_id: "T-1".into(),
                turn_id: "U-1".into(),
            },
            REQUEST_DEADLINE,
        );
        assert!(out.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "★{}초짜리 시한을 다 기다렸다★ — 아무도 join 하지 않는 스레드가 그만큼 매달린다",
            REQUEST_DEADLINE.as_secs()
        );
    }

    /// ★spawn 뒤 실패 경로에서 자식이 남으면 아무도 닿을 수 없다★ — `Child` 는 drop 으로 죽이지 않는다.
    ///
    /// ★이 항목이 덮는 것과 안 덮는 것★: 재는 것은 **가드 자체의 `Drop` 이 자식을 거둔다**는 것뿐이고,
    /// **가드가 [`CodexAppServerTransport::open`] 안에서 충분히 이른 자리에 서 있는가**는 아니다. 그
    /// 배치는 아래 [`the_child_guard_is_armed_before_the_first_fallible_step_after_spawn`] 이 소스에서
    /// 잰다 — 가드를 Job 생성·편입 `?` 아래로 내리면 그쪽이 빨개진다.
    #[cfg(windows)]
    #[test]
    fn the_child_guard_reaps_the_child_on_an_early_return() {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/c", "ping", "-n", "30", "127.0.0.1"]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("spawn");
        let pid = child.id();
        assert!(engram_dashboard_base::platform::pid_alive(pid));
        drop(ChildGuard(Some(child)));
        assert!(
            !engram_dashboard_base::platform::pid_alive(pid),
            "조기 반환 경로에서 자식이 샜다"
        );
    }

    /// ★상대가 스스로 끝나도 라이터 스레드가 끝나야 한다★ — 안 끝나면 그 스레드가 core·stdin·대기표의
    /// `Arc` 를 든 채 남아 세션 하나치 메모리가 함께 남는다. `shutdown()` 은 이 경로에서 불리지 않는다
    /// (reaper 는 세션을 명부에서 뺄 뿐이다).
    ///
    /// ★이 항목이 두 루프를 **실제로 돌리는 유일한 자리다**★ — 나머지 통로 항목은 `handle_line` ·
    /// `next_job` · `sweep_deadlines` 를 직접 부른다.
    #[cfg(windows)]
    #[test]
    fn the_writer_thread_ends_when_the_peer_exits_on_its_own() {
        let (t, _) = open_probe(&["/c", "echo", "{}"]);
        let (core, _seen) = core_with_sink();
        t.start(core.clone());

        // 리더(pump)는 EOF 로 끝난다.
        core.join_pump(Duration::from_secs(10));

        await_writer_end(
            &t,
            "상대가 스스로 끝났는데 라이터 스레드가 안 끝났다 — 에이전트마다 스레드 하나가 영구히 남는다",
        );
        assert!(
            with_state(&t.state, |s| s.closed),
            "EOF 처리가 닫힘 표식을 안 세웠다"
        );
    }

    /// ★같은 표식이 리더 **panic** 도 살아남아야 한다★ — `catch_unwind` 는 리더 루프를 밖에서 감싸므로,
    /// 루프 꼬리에 적어 둔 정리는 unwind 가 그냥 지나간다. 그 경로에서 표식이 안 서면 라이터가 core·
    /// stdin·대기표를 든 채 영원히 돌고, 그것이 정상 EOF 항목이 못 보는 나머지 절반이다.
    ///
    /// unwind 는 **구독자가 `core.emit` 안에서 panic** 하게 만들어 낸다 — 리뷰가 이름한 경로 그대로이고,
    /// 이 파일이 실 codex 없이 그 지점에 닿는 유일한 길이다.
    #[cfg(windows)]
    #[test]
    fn the_closing_mark_survives_a_reader_panic() {
        let probe = NotificationFile::new("panic");
        let (t, _) = open_probe_owned(probe.args());
        *t.decoder.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(EmittingDecoder));

        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        core.subscribe(Arc::new(PanickingSink(SinkId::new_v4())));
        t.start(core.clone());
        core.join_pump(Duration::from_secs(10));

        assert!(
            with_state(&t.state, |s| s.closed),
            "리더가 panic 으로 끝나자 닫힘 표식이 안 섰다"
        );
        await_writer_end(&t, "리더 panic 뒤 라이터 스레드가 안 끝났다");
        drop(probe);
    }

    /// ★핸드셰이크가 답을 기다리는 동안에도 제어 큐는 나가야 한다★ — 라이터가 그 큐를 비우는 유일한
    /// 스레드라, 통째로 park 하면 리더가 넣은 서버 요청 거절이 시한이 다 찰 때까지 못 나가고 상대가 그
    /// 답을 기다리는 중이었다면 양쪽이 서로를 기다린다.
    #[cfg(windows)]
    #[test]
    fn the_handshake_keeps_draining_the_control_queue_while_it_waits() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let stdin = t.stdin.clone();
        let state = t.state.clone();
        let pending = t.pending.clone();
        let next_id = t.next_id.clone();

        let waiter = std::thread::spawn(move || -> Result<Value, String> {
            request_blocking(
                &stdin,
                &state,
                &pending,
                &next_id,
                method::INITIALIZE,
                &serde_json::json!({}),
                REQUEST_DEADLINE,
            )
        });

        // 답을 기다리는 중에 리더가 거절 하나를 넣는다.
        with_state(&t.state, |s| {
            s.outbox
                .push_back("{\"id\":1,\"error\":{\"code\":-32601}}\n".to_string())
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while with_state(&t.state, |s| !s.outbox.is_empty()) {
            assert!(
                Instant::now() < deadline,
                "★핸드셰이크가 제어 큐를 막았다★ — 상대는 그 답을 시한까지 못 받는다"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // 답을 줘서 그 스레드를 끝낸다(시한을 다 기다리지 않게).
        let entry = t
            .pending
            .take(&RequestId::Num(0))
            .expect("우리가 낸 요청의 대기표");
        match entry.waiter {
            Waiter::Handshake(tx) => tx.send(Ok(serde_json::json!({}))).expect("send"),
            _ => panic!("핸드셰이크 대기표여야 한다"),
        }
        waiter.join().expect("waiter thread").expect("응답");
        t.shutdown();
    }

    /// ★자른 대기의 순서가 계약이다 — 시한을 **먼저** 재고 그 다음에 한 줄을 쓴다★.
    ///
    /// 재는 것은 정확히 그 순서 하나다: 시한이 이미 지난 슬라이스에서 **쓰기를 시도하지 않고 돌아온다.**
    /// 순서를 뒤집으면 그 한 번의 쓰기가 stdin 락을 못 얻어 매달리고, 시한은 영영 평가되지 않는다.
    ///
    /// ★이 항목이 **재지 못하는 것**을 분명히 해 둔다★: 시한이 아직 남은 슬라이스에서 쓰기가 막히면 이
    /// 대기는 여전히 무한이다. 그것은 이 순서로 못 고치고, 빠져나오는 길은 `shutdown()` 의 kill 뿐이다
    /// (모듈 헤더 「알려진 한계」). 이 항목을 그 보장으로 인용하지 말 것.
    #[cfg(windows)]
    #[test]
    fn an_expired_slice_returns_without_attempting_another_write() {
        // 첫 요청 쓰기는 성공해야 하므로(작은 줄) 시한만 짧게 잡는다. 슬라이스 하나(500ms)가 도는 사이에
        //   이미 지나 있다.
        const BUDGET: Duration = Duration::from_millis(50);
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);

        // 뒤집힌 순서가 실제로 쓰기를 시도하도록 제어 줄을 하나 세워 둔다.
        with_state(&t.state, |s| {
            s.outbox
                .push_back("{\"id\":1,\"error\":{\"code\":-32601}}\n".to_string())
        });

        let stdin = t.stdin.clone();
        let state = t.state.clone();
        let pending = t.pending.clone();
        let next_id = t.next_id.clone();
        let waiter = std::thread::spawn(move || -> Result<Value, String> {
            request_blocking(
                &stdin,
                &state,
                &pending,
                &next_id,
                method::INITIALIZE,
                &serde_json::json!({}),
                BUDGET,
            )
        });

        // 첫 요청 쓰기가 끝난 **뒤에** stdin 을 붙든다 — 상대는 stdin 을 안 읽으므로 이 쓰기는 락을 쥔 채
        //   영원히 매달린다.
        std::thread::sleep(Duration::from_millis(100));
        let filler = t.stdin.clone();
        let filling = std::thread::spawn(move || {
            let _ = write_line(&filler, &"x".repeat(8 * 1024 * 1024));
        });

        let hard_stop = Instant::now() + Duration::from_secs(15);
        while !waiter.is_finished() {
            assert!(
                Instant::now() < hard_stop,
                "★시한이 지난 슬라이스가 쓰기를 먼저 시도했다★ — 그 락을 못 얻어 시한이 평가되지 않는다"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = waiter.join().expect("waiter thread");
        assert!(out.is_err(), "답이 온 적 없는데 성공으로 끝났다: {out:?}");

        t.shutdown();
        let _ = filling.join();
    }

    /// ★제어 큐가 가득 차 거절을 못 보내는 것은 화면에 올라야 한다★ — 로그로만 두면 상대는 답을 영영
    /// 기다리고 이쪽에는 아무 신호도 없다. 형제 사건(`turn/start` 시한)이 이미 화면에 오른다.
    #[test]
    fn a_full_control_queue_drops_a_protocol_obligation_visibly() {
        let mut h = harness();
        with_state(&h.state, |s| {
            for i in 0..OUTBOX_LIMIT {
                s.outbox.push_back(format!("{{\"filler\":{i}}}\n"));
            }
        });
        h.reader
            .handle_line(br#"{"id":42,"method":"item/approval/request"}"#);

        assert_eq!(
            with_state(&h.state, |s| s.outbox.len()),
            OUTBOX_LIMIT,
            "상한을 넘겨 밀어 넣었다"
        );
        let seen = h.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|e| matches!(e, OutputEvent::Error(m) if m.contains("제어 큐"))),
            "거절을 못 보냈다는 사실이 화면에 안 올랐다: {seen:?}"
        );
    }

    /// ★가드는 spawn 뒤 **첫 실패 가능 단계보다 먼저** 서 있어야 한다★ — 그 아래로 내려가면 그 사이의
    /// `?` 가 이미 도는 자식을 남긴 채 돌아가고, 그 자식은 아직 어느 Job 에도 안 들어가 아무도 닿을 수 없다.
    ///
    /// 소스에서 재는 이유 = 그 배치는 **실패를 주입할 수 없는 자리**다(`JobObjectHandle::new` 를 실패시키는
    /// seam 이 없다). 위 `Drop` 항목은 가드가 도는 것만 재고 어디에 서 있는지는 못 본다.
    #[test]
    fn the_child_guard_is_armed_before_the_first_fallible_step_after_spawn() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let open_body = production
            .split("pub(crate) fn open(")
            .nth(1)
            .expect("open 본문");
        let armed = open_body
            .find("ChildGuard(Some(child))")
            .expect("가드 무장 지점");
        for step in ["JobObjectHandle::new()?", "job.assign(pid)?"] {
            let at = open_body
                .find(step)
                .unwrap_or_else(|| panic!("`{step}` 가 open 안에 없다 — 이 항목의 전제가 낡았다"));
            assert!(
                armed < at,
                "가드가 `{step}` 보다 뒤에 선다 — 그 사이의 실패가 자식을 남긴다"
            );
        }
    }

    /// ★기록 호출은 게이트가 열리기 **전에** 나와야 한다★ — 이것이 한 동사 포트의 존재 근거다
    /// ([`crate::backend::SessionIdSink`]: 「적혔나」를 되묻는 둘째 동사가 없는 이유가 이 순서다).
    /// 뒤집히면 기록되기 전에 그 세션으로 턴이 나가고, 포트는 **아무것도 보장하지 않는 통보**가 된다.
    ///
    /// ★왜 소스에서 재나★ — 포트는 `Fn(&str)` 이라 불린 시점의 링크 상태를 볼 수 없고, 보게 만들려면
    /// 이 항목이 지키려는 바로 그 한 동사 계약을 깨야 한다. 게이트의 반대쪽 절반(「`Ready` 여야 턴이
    /// 나간다」)은 [`tests::nothing_is_sent_before_the_link_is_ready_even_when_the_thread_id_is_known`]
    /// 이 실제로 돌려서 잰다 — 둘이 합쳐 「기록 → 게이트 → 전송」 순서를 덮는다.
    /// 선례·같은 사유 = [`tests::the_child_guard_is_armed_before_the_first_fallible_step_after_spawn`].
    #[test]
    fn the_session_id_is_recorded_before_the_gate_opens() {
        let body = writer_loop_code();

        let recorded = body
            .find("record_session_id(")
            .expect("`record_session_id(` 호출이 writer_loop 에 없다 — 이 항목의 전제가 낡았다");
        // ★앵커가 `Link::Ready` 에서 `open_gate(` 로 옮겼다★ — 게이트를 세우는 줄이 그 함수 안으로
        //   들어가면서, 이 본문에 남은 `Link::Ready` 는 **주석뿐**이 됐다. 그대로 두면 이 항목이 주석
        //   위치를 재며 조용히 통과한다(실제로 그렇게 통과했다).
        let gate_open = body
            .find("open_gate(")
            .expect("`open_gate(` 호출이 writer_loop 에 없다 — 이 항목의 전제가 낡았다");

        assert!(
            recorded < gate_open,
            "기록 호출이 게이트(`Link::Ready`)보다 뒤에 선다 — 기록되기 전에 턴이 나갈 수 있고, 그러면 한 \
             동사 포트가 보장하는 것이 없어진다(둘째 동사가 필요해진다)"
        );
    }

    /// `writer_loop` 본문에서 **주석을 걷어낸** 코드만 — 순서·문 선택을 소스에서 재는 항목들의 공용 렌즈.
    ///
    /// ★★주석을 걷는 것이 이 헬퍼의 존재 이유다★★: 이 파일의 주석은 자기가 지키는 이름을 그대로
    ///   인용하므로(그것이 이 저장소의 주석 규약이다), 날것으로 훑으면 **주석 한 줄이 실물 호출 행세를
    ///   한다.** 실제로 그렇게 통과한 적이 있다 — 게이트를 세우는 줄이 `open_gate` 안으로 들어간 뒤에도
    ///   `Link::Ready` 를 찾던 두 항목이 본문에 남은 **주석**을 재며 초록이었다.
    fn writer_loop_code() -> String {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        production
            .split("fn writer_loop(")
            .nth(1)
            .expect("writer_loop 본문")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    }

    // ── 이력 복원 페이징(ADR-0203) ─────────────────────────────────────────────

    fn entry(turn: &str, item: Value) -> ThreadItemEntry {
        ThreadItemEntry {
            turn_id: turn.to_string(),
            item,
        }
    }

    fn agent_message(id: &str, text: &str) -> Value {
        serde_json::json!({"type": "agentMessage", "id": id, "text": text})
    }

    fn page(data: Vec<ThreadItemEntry>, next: Option<&str>) -> ThreadItemsListResponse {
        ThreadItemsListResponse {
            data,
            next_cursor: next.map(String::from),
        }
    }

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    /// 텍스트만 뽑는다 — 순서를 재는 항목들이 쓰는 렌즈.
    fn texts(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// ★페이지는 최신부터 오고, 화면에 실리는 것은 **시간순**이어야 한다★ — 그 뒤집기가 이 경로의
    /// 유일한 순서 계약이다(실측 2026-09-16, 실 0.154.0: `sortDirection: desc` 의 첫 페이지 `data[0]`
    /// 이 마지막 어시스턴트 응답이고 마지막 페이지 끝이 최초 `userMessage` 다).
    /// ★같은 항목이 요청 모양도 함께 잰다★ — 방향·커서 이어받기가 틀리면 화면이 거꾸로 서거나 같은
    /// 페이지를 영원히 다시 받는다.
    #[test]
    fn history_pages_backwards_and_lands_in_time_order() {
        let mut asked: Vec<(String, Option<String>)> = Vec::new();
        let events = collect_history("th-1", "c0".to_string(), far_deadline(), |params, _| {
            asked.push((params.thread_id.clone(), params.cursor.clone()));
            assert!(
                matches!(params.sort_direction, Some(SortDirection::Desc)),
                "역방향이 아니면 최신부터 걷지 못한다"
            );
            assert_eq!(params.limit, Some(HISTORY_PAGE_LIMIT));
            Ok(match params.cursor.as_deref() {
                Some("c0") => page(
                    vec![
                        entry("t1", agent_message("m3", "third")),
                        entry("t1", agent_message("m2", "second")),
                    ],
                    Some("c1"),
                ),
                _ => page(vec![entry("t1", agent_message("m1", "first"))], None),
            })
        });

        assert_eq!(
            asked,
            vec![
                ("th-1".to_string(), Some("c0".to_string())),
                ("th-1".to_string(), Some("c1".to_string())),
            ],
            "핸드셰이크가 연 스레드로, 응답이 준 커서를 이어 물어야 한다"
        );
        assert_eq!(texts(&events), vec!["first", "second", "third"]);
    }

    /// ★턴마다 정확히 하나씩 닫는다★ — 안 닫으면 복원된 슬롯의 대기 표시가 영영 돌고, 겹쳐 닫으면
    /// 빈 구분선이 쌓인다. 마지막 턴도 닫는 것이 이 항목의 절반이다.
    #[test]
    fn every_history_turn_is_closed_exactly_once() {
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry("t2", agent_message("m2", "later")),
                    entry("t1", agent_message("m1", "earlier")),
                ],
                None,
            ))
        });

        let shape: Vec<&str> = events
            .iter()
            .map(|e| match e {
                OutputEvent::TextDelta { .. } => "text",
                OutputEvent::MessageDone { .. } => "close",
                _ => "other",
            })
            .collect();
        assert_eq!(shape, vec!["text", "close", "text", "close"]);
        let closed: Vec<Option<String>> = events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::MessageDone { turn_id, .. } => Some(turn_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closed,
            vec![Some("t1".to_string()), Some("t2".to_string())],
            "경계는 그 턴의 id 를 달고, 시간순으로 선다"
        );
    }

    /// ★옮길 것이 하나도 없으면 **아무것도 내지 않는다**★ — 경계만 남은 목록을 만들면 빈 슬롯에
    /// 구분선이 생긴다. 라이브가 버리는 변형(추론 등)은 이력에서도 같은 판정을 받는다.
    #[test]
    fn a_history_of_nothing_translatable_produces_no_events() {
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry("t1", serde_json::json!({"type": "reasoning", "id": "r1"})),
                    entry(
                        "t1",
                        serde_json::json!({"type": "subAgentActivity", "id": "s1"}),
                    ),
                ],
                None,
            ))
        });
        assert!(events.is_empty(), "경계만 남은 목록이 나왔다: {events:?}");
    }

    /// ★★페이지 하나가 실패해도 그것은 **활성화 실패가 아니다**★★ — 그 앞까지 받은 화면을 그대로
    /// 싣고 정상으로 닫는다(ADR-0203: 이력 실패는 조용하고 무해하다).
    #[test]
    fn a_failed_page_keeps_everything_that_came_before_it() {
        let mut calls = 0;
        let events = collect_history("th", "c0".to_string(), far_deadline(), |_, _| {
            calls += 1;
            if calls == 1 {
                Ok(page(
                    vec![entry("t1", agent_message("m1", "kept"))],
                    Some("c1"),
                ))
            } else {
                Err("상대가 거절했다".to_string())
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(texts(&events), vec!["kept"]);
        assert!(
            matches!(events.last(), Some(OutputEvent::MessageDone { .. })),
            "중간에 끊겨도 마지막 턴은 닫아야 한다: {events:?}"
        );
    }

    /// ★★천장은 링이다★★(ADR-0203) — 2MiB 를 넘겨 받아 봐야 링이 앞에서부터 버리므로 순 비용이다.
    /// 그 판정은 링이 실제로 세는 축([`estimate_cost_bytes`])으로 해야 한다.
    #[test]
    fn history_stops_once_the_replay_ring_is_full() {
        // 한 건이 상한(64Ki 문자)까지 찬 어시스턴트 본문 — 32 건이면 2MiB 다.
        let big = "x".repeat(64 * 1024);
        let mut calls = 0;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                (0..HISTORY_PAGE_LIMIT)
                    .map(|i| entry("t1", agent_message(&format!("m{calls}-{i}"), &big)))
                    .collect(),
                // 커서가 끝나지 않는다 — 멈추는 것은 오직 천장이어야 한다.
                Some("more"),
            ))
        });

        let weight: usize = events.iter().map(estimate_cost_bytes).sum();
        // ★★넘기 **전에** 멈춘다 — 「넘고 나서 멈춘다」로 되돌리지 말 것★★: 넣고 나서 재면 언제나 한
        //   건만큼 넘기고, 그 한 건은 링이 착지하자마자 버린다. 천장을 링 상수에 묶어 둔 이유가 그
        //   어긋남을 없애려던 것이다.
        assert!(
            weight <= REPLAY_MAX_BYTES,
            "천장을 넘겨 받았다: {weight}B > {REPLAY_MAX_BYTES}B"
        );
        // 그러면서 **거의** 채운다 — 한 건(64KiB)만큼의 여유 안에 들어야 한다.
        assert!(
            weight + 64 * 1024 >= REPLAY_MAX_BYTES,
            "링을 한참 못 채우고 멈췄다: {weight}B"
        );
        assert_eq!(
            calls, 2,
            "링이 찬 뒤로도 페이지를 더 물었다(왕복 낭비) — {calls}회"
        );
    }

    /// ★★천장 회계가 **합성한 경계까지** 세야 한다★★ — 안 세면 항목마다 턴이 갈리는 이력에서 실제
    /// 이벤트 수가 센 것의 두 배가 되고, 링(4096 건)이 착지하자마자 절반을 버린다. 그 절반은 **가장
    /// 오래된 쪽**이라, 복원된 대화의 머리가 통째로 잘린다.
    #[test]
    fn the_ceiling_counts_the_boundaries_it_synthesizes() {
        let mut n = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            let data = (0..HISTORY_PAGE_LIMIT)
                .map(|_| {
                    n += 1;
                    // 항목마다 턴이 다르다 = 항목마다 경계가 하나씩 더 붙는 최악.
                    entry(&format!("t{n}"), agent_message(&format!("m{n}"), "x"))
                })
                .collect();
            Ok(page(data, Some(&format!("c{n}"))))
        });

        assert!(
            events.len() <= REPLAY_MAX_EVENTS,
            "링 건수 천장을 넘겨 받았다: {} > {REPLAY_MAX_EVENTS}",
            events.len()
        );
        // 경계를 안 세던 시절이면 여기서 항목 수가 천장까지 갔을 것이다 — 절반 언저리여야 맞다.
        let texts = events
            .iter()
            .filter(|e| matches!(e, OutputEvent::TextDelta { .. }))
            .count();
        let closes = events
            .iter()
            .filter(|e| matches!(e, OutputEvent::MessageDone { .. }))
            .count();
        assert_eq!(texts, closes, "턴마다 경계 하나 — 회계의 전제가 깨졌다");
        assert!(
            events.len() > REPLAY_MAX_EVENTS - 2 * HISTORY_PAGE_LIMIT as usize,
            "천장을 한참 못 채우고 멈췄다: {}",
            events.len()
        );
    }

    /// ★★`turnId` 에는 와이어 상한이 없다 — 담기 전에 거른다★★: 줄 상한은 페이지 **한 장**에만 걸리고
    /// 여기 쌓이는 누적에는 안 걸려서, 긴 id 를 단 item 이 페이지마다 오면 원본 문자열만으로 수백 MiB 가
    /// 된다(25 건 × 160KiB 는 한 장에서 4MiB 를 안 넘는데, 건수 천장까지 모으면 ~650MiB 다).
    /// ★★그러면서 **서로 다른 턴은 그대로 둘로 남아야 한다**★★ — 상한을 넘긴 id 를 전부 한 칸(`None`)
    /// 으로 접으면 메모리는 잡히지만 **서로 다른 대화 둘이 한 턴으로 렌더된다.** 그래서 구별은 직전
    /// 항목과의 원본 비교(O(1))가 지고, 거른 id 는 경계에 **표시**로만 실린다. 이 항목이 재는 것이 그
    /// 두 축이다 — 경계는 둘인데 실린 id 는 없다.
    #[test]
    fn an_oversized_turn_id_is_filtered_before_it_is_retained() {
        let huge_a = "a".repeat(MAX_TURN_ID_BYTES + 1);
        let huge_b = "b".repeat(MAX_TURN_ID_BYTES + 1);
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry(&huge_b, agent_message("m2", "later")),
                    entry(&huge_a, agent_message("m1", "earlier")),
                ],
                None,
            ))
        });

        let closes: Vec<Option<String>> = events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::MessageDone { turn_id, .. } => Some(turn_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            closes,
            vec![None, None],
            "상한 넘긴 turn 둘이 한 턴으로 접혔다 — 서로 다른 대화가 한 덩이로 렌더된다: {events:?}"
        );
        assert_eq!(
            texts(&events),
            vec!["earlier", "later"],
            "내용은 전부 남는다"
        );
        // 같은 id 가 이어지면 그때는 한 묶음이다 — 위 분리가 「언제나 쪼갠다」가 아님을 못 박는다.
        let same = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            Ok(page(
                vec![
                    entry(&huge_a, agent_message("m2", "later")),
                    entry(&huge_a, agent_message("m1", "earlier")),
                ],
                None,
            ))
        });
        assert_eq!(
            same.iter()
                .filter(|e| matches!(e, OutputEvent::MessageDone { .. }))
                .count(),
            1,
            "같은 turn 인데 둘로 쪼갰다: {same:?}"
        );
    }

    /// ★★같은 커서를 되돌려 주는 상대에게 같은 페이지를 다시 받지 않는다★★ — 받으면 같은 대화가 여러
    /// 벌 실리고 그 중복이 진짜 이력을 링에서 밀어낸다. 페이지 상한은 루프를 묶지만 이 오염은 못 막는다.
    #[test]
    fn a_repeating_cursor_stops_instead_of_duplicating_history() {
        let mut calls = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                vec![entry("t1", agent_message("m1", "once"))],
                // 첫 답이 준 커서를 그 뒤로 계속 되돌려 준다.
                Some("same"),
            ))
        });
        assert_eq!(calls, 2, "되풀이되는 커서를 알아보지 못했다 — {calls}회");
        assert_eq!(
            texts(&events),
            vec!["once", "once"],
            "첫 커서와 둘째 커서는 서로 다르므로 두 장까지는 정상이다: {events:?}"
        );
    }

    /// ★끝나지 않는 커서에 영원히 매달리지 않는다★ — 예산이 먼저 끊는 것이 보통이지만, 답이 즉시
    /// 오는 상대에게는 이 backstop 만이 남는다.
    #[test]
    fn an_unending_cursor_stops_at_the_page_cap() {
        let mut calls = 0usize;
        let events = collect_history("th", "c".to_string(), far_deadline(), |_, _| {
            calls += 1;
            Ok(page(
                vec![entry(
                    "t1",
                    serde_json::json!({"type": "reasoning", "id": "r"}),
                )],
                // ★매번 **다른** 커서다★ — 같은 값을 주면 되풀이 가드가 먼저 끊어, 이 항목이 재려는
                //   상한이 아니라 그 가드를 재게 된다.
                Some(&format!("c{calls}")),
            ))
        });
        assert_eq!(calls, MAX_HISTORY_PAGES);
        assert!(events.is_empty());
    }

    /// ★★연결이 이력을 받는 도중에 끝났으면 게이트를 열지 않는다★★ — 이력 쪽은 그 오류를 「페이지가
    /// 안 왔다」로 처리하므로(그것이 옳다), 게이트가 상태를 안 보면 **이미 죽은 통로가 `Ready` 로 서서
    /// 활성화 성공으로 배달된다.** 그 뒤엔 매니저가 시체를 산 에이전트로 들고 간다.
    #[test]
    fn a_gate_does_not_open_over_a_link_that_died_while_hydrating() {
        let state = shared();
        with_state(&state, |s| {
            s.link = Link::Down("스트림이 끝났다".to_string());
            s.closed = true;
        });
        let verdict = open_gate(&state, "th".to_string());
        assert_eq!(verdict, Err("스트림이 끝났다".to_string()));
        with_state(&state, |s| {
            assert!(matches!(s.link, Link::Down(_)), "내려간 연결을 덮어썼다");
            assert!(s.thread_id.is_none(), "죽은 통로에 thread id 를 심었다");
        });
    }

    /// ★★스트림 종료는 **상태를 먼저 쓰고 그 다음에 깨운다**★★ — 반대면 죽음이 스스로 연 창으로
    /// 라이터가 돌아와, 아직 `Connecting` 인 상태를 읽고 게이트를 통과해 **이미 끝난 통로에 `Ready` 를
    /// 배달한다.** 그 창은 우연이 아니라 스트림 종료마다 열렸다.
    ///
    /// ★★왜 **소스 순서**로 재나 — 돌려서 재 봤고 못 잰다★★: 깨우기를 먼저 하는 판을 만들어 관측
    /// 스레드를 붙여 200 회 돌렸더니 **한 번도 안 걸렸다.** 깨어난 쪽은 스케줄러를 기다려야 하는데
    /// 깨운 쪽은 곧바로 다음 몇 줄에서 락을 잡으므로, 경합이되 사실상 언제나 깨운 쪽이 이긴다. 즉 그
    /// 하네스는 **순서가 틀려도 초록**이라 회귀망이 아니었다. 재는 대상이 「두 문장의 순서」인 이상
    /// 그것을 직접 재는 것이 정직하다(형제 = [`tests::the_history_is_emitted_before_the_gate_opens`]).
    /// ★그래서 실행으로 재는 몫은 **아래 형제 항목**이 진다★ — 순서가 아니라 「둘 다 일어났나」다.
    // ADR-0203
    #[test]
    fn a_stream_end_writes_the_state_before_it_wakes_the_waiters() {
        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split("impl Drop for ReaderExit {")
            .nth(1)
            .expect("ReaderExit::drop 본문")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        let wrote_down = body
            .find("s.closed = true;")
            .expect("`s.closed = true;` 가 ReaderExit::drop 에 없다 — 이 항목의 전제가 낡았다");
        let woke = body
            .find("self.pending.close()")
            .expect("`self.pending.close()` 가 ReaderExit::drop 에 없다 — 이 항목의 전제가 낡았다");
        assert!(
            wrote_down < woke,
            "깨우기가 상태 쓰기보다 먼저 선다 — 깨어난 라이터가 살아 있는 상태를 읽고 죽은 통로에 \
             Ready 를 배달한다"
        );
    }

    /// 순서가 아니라 **둘 다 일어났나**를 실행으로 잰다 — 어느 한쪽을 지우면 여기서 걸린다
    /// (깨우기를 지우면 영구 hang, 상태 쓰기를 지우면 죽은 통로가 살아 있는 것으로 남는다).
    #[test]
    fn a_stream_end_both_wakes_the_waiters_and_marks_the_link_down() {
        let state = shared();
        let pending = Arc::new(Pending::default());
        let (tx, rx) = mpsc::channel();
        assert!(pending.register(1, Waiter::Handshake(tx), method::THREAD_ITEMS_LIST));

        drop(ReaderExit {
            state: state.clone(),
            pending: pending.clone(),
        });

        assert!(
            matches!(rx.try_recv(), Ok(Err(_))),
            "대기표를 오류로 깨우지 않았다 — 영구 hang 이다"
        );
        with_state(&state, |s| {
            assert!(matches!(s.link, Link::Down(_)), "연결을 내리지 않았다");
            assert!(s.closed, "닫힘 표식을 세우지 않았다");
        });
    }

    /// 배달 채널이 아니라 **닫힘 표식만** 선 경우도 같다 — 사유 문구가 없을 뿐 결론은 같다.
    #[test]
    fn a_gate_does_not_open_over_a_closed_link() {
        let state = shared();
        with_state(&state, |s| s.closed = true);
        assert!(open_gate(&state, "th".to_string()).is_err());
        with_state(&state, |s| {
            assert!(!matches!(s.link, Link::Ready));
        });
    }

    /// 멀쩡한 연결에서는 그대로 열리고 손잡이가 실린다 — 위 둘의 반대편.
    #[test]
    fn a_healthy_link_opens_the_gate_and_carries_the_thread_id() {
        let state = shared();
        assert_eq!(open_gate(&state, "th-9".to_string()), Ok(()));
        with_state(&state, |s| {
            assert!(matches!(s.link, Link::Ready));
            assert_eq!(s.thread_id.as_deref(), Some("th-9"));
        });
    }

    /// ★한 왕복이 예산을 다 써 버리면 **그 다음은 묻지 않는다**★ — 이 루프가 공유 예산을 실제로 강제하는
    /// 자리가 여기다. 한 요청 **안**의 초과분(응답 대기 슬라이스 하나, 500ms)은 이 루프가 못 막는다.
    ///
    /// ★★한때 여기 「10s + 0.5s」로 적혀 있었고 그 수치는 **틀렸다**★★ — 그 계산은 초과가 한 번뿐이라고
    /// 보았는데, 초과를 세는 단위는 루프가 아니라 [`request_blocking`] **호출**이다: 성공 갈래는 시한을
    /// 다시 보지 않고, 예산이 0 이어도 다음 슬라이스를 통째로 기다린다. 그래서 [`handshake`] 의 두
    /// 왕복이 각각 한 슬라이스씩 넘길 수 있고(첫 왕복이 자기 초과 중에 성공하면 둘째는 잔여 0 으로
    /// 시작해 또 한 슬라이스를 쓴다), **`Ready` 까지의 상한은 10s + 2×0.5s = 11s** 다.
    /// ★그 뒤로 더 늘지 않는 이유가 이 항목이 재는 것이다★ — 페이징의 첫 검사가 잔여 0 을 보고 끊으므로
    /// 세 번째 초과가 없다. ★핸드셰이크에 왕복을 하나 더 들이면 그만큼 는다★(11s → 11.5s).
    /// 백스톱 15s 는 그대로 위에 있다.
    #[test]
    fn a_page_that_burns_the_budget_is_the_last_one() {
        let mut calls = 0usize;
        let deadline = Instant::now() + Duration::from_millis(80);
        let events = collect_history("th", "c0".to_string(), deadline, |_, _| {
            calls += 1;
            std::thread::sleep(Duration::from_millis(160));
            Ok(page(
                vec![entry("t1", agent_message("m1", "only"))],
                Some("c1"),
            ))
        });
        assert_eq!(calls, 1, "예산이 끝났는데 또 물었다 — {calls}회");
        assert_eq!(texts(&events), vec!["only"], "받아 둔 것은 그대로 싣는다");
    }

    /// ★예산이 이미 없으면 **한 번도 묻지 않는다**★ — 핸드셰이크가 예산을 다 쓰고 겨우 성공한 경우가
    /// 그것이고, 거기서 한 왕복이라도 더 나가면 `Ready` 가 매니저의 백스톱 뒤로 밀린다.
    #[test]
    fn an_exhausted_budget_asks_for_nothing() {
        let mut calls = 0;
        let events = collect_history(
            "th",
            "c".to_string(),
            Instant::now() - Duration::from_secs(1),
            |_, _| {
                calls += 1;
                Ok(page(vec![entry("t1", agent_message("m", "never"))], None))
            },
        );
        assert_eq!(calls, 0);
        assert!(events.is_empty());
    }

    /// ★이어받을 것이 없으면 왕복 자체가 없다★ — `thread/start`(새 대화)로 뜬 화신이 여기 든다.
    #[test]
    fn a_thread_without_a_history_cursor_never_pages() {
        let state = shared();
        let events = hydrate_history(
            &Mutex::new(None),
            &state,
            &Pending::default(),
            &AtomicI64::new(0),
            &Opened {
                thread_id: "th".to_string(),
                items_backwards_cursor: None,
            },
            far_deadline(),
        );
        assert!(events.is_empty());
    }

    /// ★★이력은 게이트가 열리기 **전에** 링에 올라야 한다 — 이것이 이 경로의 순서 계약이다★★:
    /// 게이트가 먼저 열리면 큐에 선 유저 턴이 나가고, 그 응답이 아직 안 실린 이력보다 먼저 링에
    /// 들어가 복원된 대화가 새 답 **뒤에** 붙는다. ADR-0079 의 seed-before-publish 가 지키던 것을
    /// 이 순서가 대신 진다(그쪽처럼 명부 등록 전으로 당길 수가 없다 — 이력이 핸드셰이크 뒤에야
    /// 존재하기 때문).
    ///
    /// ★소스에서 재는 사유는 형제 항목과 같다★ — 실제로 돌려서 재려면 자식 프로세스와 왕복하는
    /// 상대가 필요하고, 그 하네스가 재는 것은 순서가 아니라 하네스 자신이 된다.
    /// 형제 = [`tests::the_session_id_is_recorded_before_the_gate_opens`].
    #[test]
    fn the_history_is_emitted_before_the_gate_opens() {
        let body = writer_loop_code();

        let hydrated = body
            .find("hydrate_history(")
            .expect("`hydrate_history(` 호출이 writer_loop 에 없다 — 이 항목의 전제가 낡았다");
        // 앵커를 `open_gate(` 로 두는 사유 = 형제 항목의 같은 자리.
        let gate_open = body
            .find("open_gate(")
            .expect("`open_gate(` 호출이 writer_loop 에 없다 — 이 항목의 전제가 낡았다");
        assert!(
            hydrated < gate_open,
            "이력 복원이 게이트보다 뒤에 선다 — 복원된 대화가 새 답 뒤에 붙는다"
        );

        // ★★들어가는 문이 **fanout 까지 하는 쪽**이어야 한다★★: 이 자리는 세션이 이미 명부에 오른
        //   뒤라, `OutputCore::seed`(링에 넣기만 하고 fanout 없음)를 쓰면 그 사이 붙은 구독자가 빈 링을
        //   replay 한 뒤라서 지난 화면을 **영영 못 본다**. ADR-0079 가 `seed` 로 충분했던 것은 그쪽이
        //   명부 등록 **전**이었기 때문이고, 이 경로는 그 자리로 당길 수가 없다.
        // ★요구하는 것은 **덩이 문**이다★ — 낱개 문으로 바꾸면 fanout 은 유지되지만 호출 사이마다
        //   리더의 라이브 줄이 seq 를 가져가 복원된 대화 한가운데가 쪼개진다
        //   ([`crate::output_core::OutputCore::emit_batch_without_turn_observation`]).
        assert!(
            body.contains("emit_batch_without_turn_observation"),
            "이력이 낱개 문으로 들어간다 — fanout 은 되지만 덩이 한가운데가 쪼개진다"
        );
        assert!(
            !body.contains("core.seed("),
            "`seed` 는 이 자리에서 fanout 을 잃는다(위 doc)"
        );
    }

    /// ★★핸드셰이크와 이력이 **한 예산을 나눠 쓴다**★★ — 각자 잡으면 연결이 서기까지가 최대 두
    /// 예산이 되어 매니저의 백스톱을 넘고, 이력을 받느라 **성공한 이어받기가 실패로 판정된다.**
    /// 그 계약의 실물은 「`writer_loop` 이 시계를 한 번만 잡아 둘에게 같은 값을 넘긴다」이다.
    #[test]
    fn the_handshake_and_the_history_share_one_budget() {
        let code = writer_loop_code();

        // ★호출 모양이 아니라 **이름의 등장 횟수**로 잰다★ — 인자 목록의 줄바꿈은 `cargo fmt` 이
        //   정하므로, 호출 문자열을 통째로 대조하면 서식만 바뀌어도 이 항목이 깨진다(실제로 깨졌다).
        assert_eq!(
            code.matches("HANDSHAKE_BUDGET").count(),
            1,
            "writer_loop 이 예산 시계를 한 번만 잡아야 한다 — 둘째 시계는 곧 둘째 예산이다"
        );
        assert_eq!(
            code.matches("link_deadline").count(),
            3,
            "그 시한이 한 번 묶이고 **두 소비자**(핸드셰이크·이력)에게 각각 넘어가야 한다"
        );
    }

    /// ★사용자가 껐으면 연결 결말을 **배달하지 않는다**★ — 결함 ③ 의 뿌리.
    ///
    /// kill 은 대기표를 닫아 핸드셰이크를 `Err` 로 깨운다. 그것을 그대로 배달하면 **사용자의 취소가
    /// 이어받기 실패로 기록되고** 그 위에 정리가 한 번 더 돈다. 그 갈래에서 감독자가 볼 사실은 종점 상태
    /// 하나여야 하고, 거기엔 「사용자가 끈 것은 실패가 아니다」 규율이 이미 있다.
    #[test]
    fn a_shutdown_suppresses_the_link_delivery() {
        let seen: Arc<Mutex<Vec<LinkResolution>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: Option<LinkSink> = {
            let seen = seen.clone();
            Some(Arc::new(move |r| {
                seen.lock().expect("seen poisoned").push(r)
            }))
        };

        let quiet = AtomicBool::new(false);
        deliver_link(&quiet, &sink, LinkResolution::Ready);
        assert_eq!(
            seen.lock().expect("seen poisoned").len(),
            1,
            "평소에는 배달돼야 한다 — 이 항목의 대조군이 죽으면 아래 단언이 공허해진다"
        );

        let killed = AtomicBool::new(true);
        deliver_link(
            &killed,
            &sink,
            LinkResolution::Failed {
                reason: "대기표가 닫혔다".into(),
            },
        );
        assert_eq!(
            seen.lock().expect("seen poisoned").len(),
            1,
            "사용자 kill 이 만든 실패가 배달됐다 — 그러면 사용자의 취소가 이어받기 실패로 기록되고 그              위에 정리까지 한 번 더 돈다"
        );
    }

    /// ★통로의 핸드셰이크 상한이 매니저의 liveness 백스톱보다 **작아야** 한다★.
    ///
    /// 둘은 다른 일을 한다: 상한은 **사유를 만들고**(상대가 답을 안 하면 `Down` 에 그 사실을 적는다),
    /// 백스톱은 **사유 없이 포기한다**(통로가 결말 자체를 못 내는 경우 — 매달린 쓰기). 백스톱이 더 작으면
    /// 사유 있는 결말이 사유 없는 포기에 덮여, 화면과 「마지막 실패」에 원인이 안 남는다.
    /// ★이 관계는 두 파일에 흩어진 두 상수 사이에만 있어 컴파일러가 못 본다★ — 한쪽만 조정하면 아무 것도
    /// 안 깨지고 결함만 되살아나므로 여기서 잰다.
    #[test]
    fn the_handshake_budget_expires_before_the_managers_backstop() {
        assert!(
            HANDSHAKE_BUDGET < crate::manager::LINK_RESOLUTION_BACKSTOP,
            "핸드셰이크 상한({HANDSHAKE_BUDGET:?})이 매니저 백스톱({:?}) 이상이다 — 그러면 사유 없는              포기가 사유 있는 거절을 덮어, 원인이 어디에도 안 남는다",
            crate::manager::LINK_RESOLUTION_BACKSTOP
        );
    }

    /// ★운영 구획에서 stdin 락을 **블로킹으로** 잡는 자리의 개수를 못 박는다★.
    ///
    /// 왜 개수인가 = [`writer_loop`] 의 핸드셰이크 실패 갈래가 그 락을 잡는 것이 안전한 근거가 「운영
    /// 그래프에서 이 락을 블로킹으로 잡는 자리가 저 둘뿐이고, 그중 [`write_line`] 은 라이터만 부른다」이기
    /// 때문이다. 셋째가 조용히 생기면 그 근거가 말없이 낡는다 — 그때 나는 것은 컴파일 에러가 아니라
    /// **데드락**이다.
    ///
    /// ★이 항목은 자리를 못 박지 않고 개수만 본다★ — 위치를 박으면 줄이 밀릴 때마다 낡는다. 늘었으면
    /// 새 자리가 어느 스레드에서 불리는지 **직접 판정한 뒤** 이 숫자를 고친다(숫자만 올리지 말 것).
    /// ★`try_lock` 은 안 센다★ — [`AgentTransport::shutdown`] 의 그 자리는 기다리지 않으므로 이 위험에
    /// 애초에 안 든다. ★주석 줄도 안 센다★ — 이 파일은 본문에서 `stdin.lock()` 을 인용한다.
    /// ★시험 구획은 범위 밖이다★ — 그쪽은 일부러 락을 붙드는 항목을 갖는다
    /// ([`tests::shutdown_completes_even_if_a_write_blocks_on_a_full_pipe`]). 그 항목이 무해한 이유는
    /// [`AgentTransport::start`] 를 부르지 않아 라이터 스레드 자체가 안 뜬다는 것이고, `start` 를 더하는
    /// 순간 무해가 깨진다.
    #[test]
    fn the_production_blocking_stdin_locks_are_counted() {
        /// `stdin` 뒤에 공백을 건너뛰고 `.lock()` 이 오는 자리 — 여러 줄로 쪼개 쓴 형태도 같이 잡는다.
        fn blocking_acquisitions(src: &str) -> usize {
            let mut found = 0;
            let mut from = 0;
            while let Some(rel) = src[from..].find("stdin") {
                let after = from + rel + "stdin".len();
                let rest = src[after..].trim_start();
                if rest.starts_with(".lock()") {
                    found += 1;
                }
                from = after;
            }
            found
        }

        let src = include_str!("transport.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join(
                "
",
            );

        assert_eq!(
            blocking_acquisitions(&code),
            2,
            "운영 구획의 블로킹 stdin 락 취득 수가 달라졌다 — `write_line` 과 `writer_loop` 의              **핸드셰이크 실패 갈래**(기록 실패만이 아니라 왕복 실패·거절도 같은 자리를 지난다) 둘이              전부여야 한다. 늘었다면 그 새 자리가 어느 스레드에서 불리는지 먼저 판정할 것: 라이터가              `write_all` 에 매달린 동안 그 락을 블로킹으로 기다리는 자리가 생기면 데드락이다"
        );
    }

    /// stdin 락을 블로킹 write 가 쥐고 있어도 `shutdown` 이 완료된다 — 순서를 뒤집으면(stdin 을 kill
    /// 보다 먼저 닫으려 하면) 이 항목이 타임아웃으로 잡는다.
    #[cfg(windows)]
    #[test]
    fn shutdown_completes_even_if_a_write_blocks_on_a_full_pipe() {
        let (t, _) = open_probe(&["/c", "ping", "-n", "30", "127.0.0.1"]);
        let t = Arc::new(t);

        let stdin = t.stdin.clone();
        let writer = std::thread::spawn(move || {
            let big = "x".repeat(8 * 1024 * 1024);
            let _ = write_line(&stdin, &big);
        });
        std::thread::sleep(Duration::from_millis(500));

        let killer = t.clone();
        let start = Instant::now();
        let shutdown_thread = std::thread::spawn(move || killer.shutdown());
        let deadline = start + Duration::from_secs(10);
        while !shutdown_thread.is_finished() {
            assert!(
                Instant::now() < deadline,
                "shutdown 이 10s 안에 안 끝났다 — stdin 락 데드락 회귀"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown_thread.join().expect("shutdown thread");
        let _ = writer.join();
    }
}
