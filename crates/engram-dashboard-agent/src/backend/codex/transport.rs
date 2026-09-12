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
//!   더하지 않고, ★stdin 은 kill 보다 먼저 닫지 않는다★(사유 정본 = `transport/stdio.rs` 의 같은 자리).
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
//!   - 핸드셰이크가 실패하면 큐에 선 입력이 사라지는데, **몇 건이 사라졌는지는 로그에만** 남는다
//!     (화면에 오르는 것은 "핸드셰이크 실패" 뿐이다).
//!   - pump 는 panic 하는 `thread::spawn` 으로 띄운다(나머지 둘은 실패를 로그로 흡수한다).
//!   - **한 번의 블로킹 쓰기는 무한히 매달릴 수 있다** — 상대가 우리 stdin 을 안 읽으면 `write_all` 이
//!     파이프 backpressure 로 멈추고, 거기서 빠져나오는 길은 `shutdown()` 의 kill 뿐이다. 시한도
//!     조건변수도 그 한 번의 쓰기 안쪽에는 닿지 않는다. ★이것이 이 파일 전체에 걸리는 한계이지
//!     핸드셰이크만의 것이 아니다★.
//!   - **라이터에는 panic 봉쇄가 없다**(pump 에는 `catch_unwind` 가 있다). 라이터가 panic 하면 link 는
//!     `Ready` 이고 닫힘 표식도 안 서므로, [`AgentTransport::send_input`] 이 아무도 보내지 않을 턴을
//!     상한까지 받아들이다가 그 정지를 "큐가 찼다" 로 신고한다 — 위 첫 항목과 같은 사유 어긋남이다.
//!   - **종료 알림이 `turn/start` 응답보다 먼저 오면 그 턴은 끝나지 않는다.** 그 알림은 귀속할 수 없어
//!     버려지고, 뒤이어 온 응답이 id 를 채우면 대기표는 이미 걷힌 뒤라 시한 backstop 도 없다. 그 순서가
//!     이 짝에서 관측된 적은 없지만, 같은 기제(알림이 자기 id 를 알려 줄 응답을 앞지른다)가 `thread/start`
//!     짝에서 실측됐다. ★고치겠다고 「귀속 안 된 종료를 기억해 둔다」를 들이지 말 것★ — 그 기억이 곧
//!     이 파일이 걷어낸 오귀속이다.
//!   - **나간 요청의 실제 상한은 `budget + SWEEP_INTERVAL` 이고, 그 시계는 첫 쓰기 *뒤에* 시작한다.**
//!     첫 `recv_timeout` 한 슬라이스가 지나야 시한을 처음 읽고, 그 앞의 요청 쓰기 자체는 유계가 아니다.
//!   - **핸드셰이크 중에는 제어 줄이 슬라이스당 하나씩만 나간다.** 서버 요청이 그보다 빨리 쌓이면
//!     [`OUTBOX_LIMIT`] 에 닿아 거절을 떨구기 시작한다 — 화면에는 보이고 핸드셰이크 시한으로 끝나지만,
//!     그 동안의 의무 누락은 실재한다.
//!   - `turn/completed` 의 귀속은 **상대가 준 turn id 문자열**에 기댄다. 상대가 지금 쓰는 id 를 그대로
//!     되보내면 엉뚱한 턴이 닫힌다 — 우리가 발급한 값이 아니므로 이 층에서 더 셀 수 있는 것이 없다.
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

use super::protocol::{
    self, method, ClientInfo, Inbound, InitializeParams, InitializeResponse, RequestId,
    ThreadStartParams, ThreadStartResponse, TurnInterruptParams, TurnStartParams,
    TurnStartResponse, UserInput, METHOD_NOT_FOUND,
};
use crate::output_core::OutputCore;
use crate::transport::{AgentTransport, OutputDecoder};
use crate::types::{
    CommandSpec, ControlCaps, InputCaps, InputEvent, OutputCaps, OutputEvent, PtyError,
    TerminalReason, TransportCaps,
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

// ★오류 코드로 분기하는 자리가 없는 것은 의도다 — 재시도 백오프 상수도 그래서 없다★.
//   오류 응답은 코드가 무엇이든 그 요청의 대기자를 깨우므로 조용히 멎지 않는다. 그 위에 「특정 코드면
//   같은 요청을 다시 보낸다」를 얹지 않은 이유는 셋이다: ① 과부하 코드가 실제로 이 봉투로 오는지
//   미검증이다(스키마에 `-32xxx` 대역이 0 회이고, 과부하·레이트리밋은 `error` **알림**의
//   `codexErrorInfo` 로 온다) ② 우리가 내는 세 요청은 재시도가 위험하다 — `initialize` 는 한 번만
//   보낼 수 있고(실측: 두 번째는 오류), `thread/start` 재시도는 스레드를 둘 만들며, `turn/start`
//   재시도는 상대가 이미 받은 턴을 한 번 더 연다 ③ 그래서 값을 고르려면 실 서버에서 그 코드를 보는 것이
//   먼저다. ★다시 열 때 필요한 것★ = 어느 요청에 어떤 코드가 언제 오는지의 관측.

/// 턴이 끝났다는 알림 — ★이 통로의 상태 기계 입력이지 번역 대상이 아니다★. 큐 해제는 턴이 끝났다는
/// 사실을 알아야 하므로 봉투를 분류하는 이 층이 자기 몫으로 읽는다.
///
/// ★`turn/started` 는 **일부러** 읽지 않는다 — 되살리지 말 것★: 그 알림에는 **우리가 발급한 식별자가
/// 하나도 없어서** 어느 턴의 것인지 원리상 귀속시킬 수 없다. 「지금 턴이 답을 기다리는 중인가」 같은
/// 정황으로 대신 가르려 하면, 앞 턴의 늦은 알림이 새 턴의 칸에 자기 id 를 적고 그 뒤로는 회복되지
/// 않는다(그 id 로 온 종료 알림이 살아 있는 턴을 닫고, 큐가 풀려 턴이 겹치고, `interrupt` 가 죽은 턴을
/// 겨눈다). 턴 id 를 정하는 것은 **우리 요청 id 로 짝지어지는 `turn/start` 응답 하나뿐**이다.
///
/// ★[`protocol::method`] 에 없는 이유★: 그 모듈이 모은 것은 **번역기가 이름으로 아는** 알림이고, 이것은
/// 번역되지 않는다(턴 경계를 화면 어휘로 내는 것은 이 단계 밖이다).
const TURN_COMPLETED: &str = "turn/completed";

// ── 세션 id 기록 포트 ─────────────────────────────────────────────────────────

/// codex 가 발급한 thread id 를 기록하는 **중립 한 동사** 포트.
///
/// ★`ProfileRegistry` 를 여기로 들이지 않는다(ADR-0004)★ — 그 타입은 `backend/` 에서 보이지 않고, 보이게
/// 만들면 통로가 프로필 스키마를 알게 된다. 조립점이 자기 기록 수단을 이 한 동사로 감싸 넘긴다.
/// ★이름에 백엔드가 들어가지 않는 것도 계약이다★ — "codex 용" 이라 부르는 순간 같은 격리가 샌다.
pub(crate) type SessionIdSink = Arc<dyn Fn(&str) + Send + Sync>;

// ── 상태 기계 ─────────────────────────────────────────────────────────────────

/// 연결 축. ★끊기면 이 화신에서 되살아나지 않는다(ADR-0192)★ — 재연결도 재`initialize` 도 없다.
#[derive(Debug)]
enum Link {
    /// 핸드셰이크 전 또는 진행 중. 입력은 거절이 아니라 **큐에 선다**.
    Connecting,
    /// `thread/start` 응답을 받고 기록 호출이 돌아왔다 — `turn/start` 가 여기서부터 허용된다.
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
    /// 이 통로는 더 보낼 것이 없다 — 라이터가 이것을 보고 루프를 끝낸다.
    ///
    /// ★세우는 자리가 둘이다★: [`AgentTransport::shutdown`](우리가 죽였다)과 [`ReaderExit`] 의 `Drop`
    /// (상대가 스스로 끝났거나 **리더가 panic 했다** — `Drop` 이라 unwind 도 반드시 지난다). ★둘째를
    /// 빠뜨리면 자연 종료한 에이전트마다 라이터 스레드가 영영 남는다★ — 그 스레드가 core·stdin·대기표의
    /// `Arc` 를 들고 있어 세션 하나치 메모리가 함께 남고, `shutdown()` 은 reaper 경로에서 불리지 않아
    /// 아무도 그것을 깨우지 않는다.
    closed: bool,
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
    /// `start()` 에서 take 해 라이터로 move — 핸드셰이크가 그것으로 `thread/start` 를 낸다.
    start_params: Mutex<Option<ThreadStartParams>>,
    shutdown: Arc<AtomicBool>,
    state: SharedState,
    pending: Arc<Pending>,
    /// 우리 요청 id 계수기. ★서버 id 는 여기 안 들어온다★ — 두 공간은 따로다.
    next_id: Arc<AtomicI64>,
    sid_sink: Option<SessionIdSink>,
    /// 라이터 스레드 핸들. ★아무도 join 하지 않는다 — `shutdown()` 안에서 기다리는 것은 계약 위반이다★.
    ///
    /// 이 핸들을 드는 이유는 하나다: **그 스레드가 실제로 끝나는지를 밖에서 볼 수 있어야 한다.** 안 끝나면
    /// 세션 하나치 메모리가 함께 남는데, 그 사실은 관측할 수단이 없으면 어디에도 안 나타난다.
    writer_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 이 통로가 나르는 출력이 구조화 스트림인가. ★주입값이다(ADR-0044/0030/0191)★ — 통로는 자기가
    /// 무엇을 나르는지 모르고, 아는 쪽은 이 모드를 고른 backend 다.
    structured: bool,
    #[cfg(windows)]
    job_handle: JobObjectHandle,
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
    /// `start_params` = `thread/start` 에 실을 값. ★정책을 통로가 하드코딩하지 않는다★ — 어느 폴더를
    ///   워크스페이스로 믿고 어떤 샌드박스·승인 정책으로 돌 것인가는 backend 지식이라 주입받는다.
    /// `sid_sink` = codex 가 발급한 thread id 를 기록할 곳. `None` = 기록할 곳이 없다.
    pub(crate) fn open(
        spec: &CommandSpec,
        structured: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
        start_params: ThreadStartParams,
        sid_sink: Option<SessionIdSink>,
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
            job
        };

        let transport = CodexAppServerTransport {
            child: Arc::new(Mutex::new(guard.into_inner())),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            decoder: Mutex::new(decoder),
            start_params: Mutex::new(Some(start_params)),
            shutdown: Arc::new(AtomicBool::new(false)),
            state: Arc::new((Mutex::new(State::new()), Condvar::new())),
            pending: Arc::new(Pending::default()),
            next_id: Arc::new(AtomicI64::new(0)),
            sid_sink,
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

/// 핸드셰이크. 순서가 계약이다 — `initialize` → (`initialized`) → `thread/start`.
///
/// ★`initialize` 는 한 번만 보낼 수 있다(실측 0.154.0)★ — 두 번째는 오류로 돌아온다. 그래서 이 함수는
/// 화신마다 정확히 한 번 돌고, 실패해도 다시 부르지 않는다(ADR-0192).
fn handshake(
    stdin: &Mutex<Option<ChildStdin>>,
    state: &SharedState,
    pending: &Pending,
    next_id: &AtomicI64,
    start_params: &ThreadStartParams,
) -> Result<String, String> {
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
        REQUEST_DEADLINE,
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

    let started: ThreadStartResponse = request_blocking(
        stdin,
        state,
        pending,
        next_id,
        method::THREAD_START,
        start_params,
        REQUEST_DEADLINE,
    )?;
    tracing::info!(
        cli_version = ?started.thread.cli_version.as_deref().map(|v| sanitize(v, LOG_STRING_LIMIT)),
        "codex thread 개시"
    );
    Ok(started.thread.id)
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

/// `seq` 가 **지금 진행 중인 바로 그 턴**일 때만 끝내고 큐를 푼다. `message` 가 있으면 화면에도 올린다.
///
/// ★표식을 안 보고 끝내면 늦게 온 신호가 다음 턴을 닫는다★ — 그러면 턴 둘이 동시에 열려 입력 큐가 막고자
/// 하는 바로 그 상태가 된다. 돌려주는 값 = 실제로 끝냈나.
fn end_turn_if(state: &SharedState, core: &OutputCore, seq: u64, message: Option<String>) -> bool {
    let ended = {
        let (lock, cv) = &**state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        match s.turn {
            TurnState::Active { seq: cur, .. } if cur == seq => {
                s.turn = TurnState::Idle;
                cv.notify_all();
                true
            }
            _ => false,
        }
    };
    if ended {
        if let Some(m) = message {
            core.emit(OutputEvent::Error(m));
        }
    }
    ended
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
                if !end_turn_if(
                    state,
                    core,
                    seq,
                    Some(format!("codex app-server: {reason}")),
                ) {
                    tracing::debug!("시한이 지난 turn/start 가 연 턴은 이미 끝났다 — 그대로 둔다");
                }
            }
            Waiter::Fire => tracing::warn!("{reason}"),
        }
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
    start_params: ThreadStartParams,
    sid_sink: Option<SessionIdSink>,
) {
    match handshake(&stdin, &state, &pending, &next_id, &start_params) {
        Ok(thread_id) => {
            // ★게이트의 정직한 조건은 「디스크에 있다」가 아니라 「기록 호출이 돌아왔다」다★ — 그 포트는
            //   실패를 자기 안에서 로그로 삼키고 호출자를 막지 않으므로, 그 위에 영속성을 주장하면 없는
            //   보장을 인용하게 된다. 포트가 없으면(오늘 조립점이 그렇다) 기록되는 곳도 없다.
            if let Some(sink) = &sid_sink {
                sink(&thread_id);
            }
            let (lock, cv) = &*state;
            let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
            s.thread_id = Some(thread_id);
            s.link = Link::Ready;
            cv.notify_all();
        }
        Err(reason) => {
            tracing::warn!("codex app-server 핸드셰이크 실패: {reason}");
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
            for entry in pending.close() {
                if let Waiter::Handshake(tx) = entry.waiter {
                    let _ = tx.send(Err(reason.clone()));
                }
            }
            core.emit(OutputEvent::Error(format!(
                "codex app-server 핸드셰이크 실패: {}",
                sanitize(&reason, LOG_STRING_LIMIT)
            )));
        }
    }

    // ★핸드셰이크가 실패해도 루프는 돈다★ — 상대는 살아 있을 수 있고, 그러면 서버 요청에 답할 자리가
    //   여전히 필요하다(답하지 않으면 그쪽이 영구 정지한다).
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
                    //   남는 것은 화면에 사실을 올리는 것뿐이다(정정 채널을 새로 만들지 않는다).
                    end_turn_if(
                        &state,
                        &core,
                        seq,
                        Some(format!("codex app-server 입력 전송 실패: {e}")),
                    );
                }
            }
        }
    }
}

// ── 리더 쪽 ───────────────────────────────────────────────────────────────────

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
    fn drop(&mut self) {
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
        s.closed = true;
        cv.notify_all();
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

    /// 턴이 끝났다는 알림 하나만 본다 — ★턴 상태는 **분류한 봉투**에서 오지 번역기 산출이 아니다★.
    ///
    /// ★끝내는 조건은 둘 다 맞을 때뿐이다★: 우리 thread id 와 같고(알면), **우리가 아는 turn id 와 같다.**
    ///   thread 만 보고 끝내면 같은 스레드의 다른 턴(이미 끝난 앞 턴의 늦은 신호)이 지금 도는 턴을 닫고,
    ///   그 자리에서 큐가 풀려 턴이 겹친다.
    /// ★turn id 를 아직 모르면 아무 것도 안 끝낸다★. 그 무시가 **그 시점에** 정지를 만들지는 않는다:
    ///   id 가 비어 있다는 것은 그 턴의 `turn/start` 가 아직 답을 못 받았다는 뜻이고(답이 성공이든 오류든
    ///   해독 실패든 그 세 갈래가 전부 턴을 끝내거나 id 를 채운다), 그 요청에는 [`REQUEST_DEADLINE`] 이
    ///   걸려 있어 만료가 [`end_turn_if`] 로 그 턴을 끝낸다.
    /// ★그 논증이 덮지 못하는 경우가 하나 있다 — 「유계다」로 일반화하지 말 것★: 종료 알림이 응답보다
    ///   **먼저** 오면 그 알림은 여기서 버려지고, 뒤이어 온 응답이 id 를 채운 뒤로는 그 턴을 끝낼 것이
    ///   아무 것도 남지 않는다(대기표는 그 응답이 이미 걷어 갔다). 모듈 헤더 「알려진 한계」가 그 칸을 진다.
    ///   ★그것을 고치겠다고 「귀속 안 된 종료를 기억해 둔다」를 들이지 말 것★ — 그 기억이 곧 이 라운드가
    ///   걷어낸 오귀속의 다른 이름이다.
    fn note_turn(&self, method_name: &str, params: Option<&Value>) {
        if method_name != TURN_COMPLETED {
            return;
        }
        let incoming_thread = params
            .and_then(|p| p.get("threadId"))
            .and_then(|v| v.as_str());
        let incoming_turn = params
            .and_then(|p| p.get("turn"))
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str());

        let (lock, cv) = &*self.state;
        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
        // ★모르는 threadId 는 오류가 아니다★ — 알림이 그 스레드의 id 를 알려 줄 응답보다 먼저 도착하는
        //   경우가 실측됐다. 우리 id 를 아직 모르면 그 축으로는 거르지 않는다.
        if let (Some(mine), Some(theirs)) = (s.thread_id.as_deref(), incoming_thread) {
            if mine != theirs {
                return;
            }
        }
        match (&s.turn, incoming_turn) {
            (
                TurnState::Active {
                    turn_id: Some(mine),
                    ..
                },
                Some(theirs),
            ) if mine == theirs => {
                s.turn = TurnState::Idle;
                cv.notify_all();
            }
            _ => {
                tracing::debug!("codex app-server: 귀속할 수 없는 turn/completed — 무시한다")
            }
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
                    Ok(r) => {
                        let (lock, _) = &*self.state;
                        let mut s = lock.lock().unwrap_or_else(|p| p.into_inner());
                        // ★표식이 안 맞으면 이 답이 연 턴은 이미 끝났다★ — 그 id 를 지금 턴의 칸에 적으면
                        //   그 뒤의 interrupt 가 엉뚱한 턴을 겨눈다.
                        match &mut s.turn {
                            TurnState::Active { seq: cur, turn_id } if *cur == seq => {
                                if turn_id.is_none() {
                                    *turn_id = Some(r.turn.id);
                                }
                            }
                            _ => tracing::debug!(
                                "codex app-server: 이미 끝난 턴의 turn/start 응답 — 버린다"
                            ),
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
                            Some(format!(
                                "codex app-server: turn/start 응답을 읽지 못했다: {masked}"
                            )),
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
                        Some(format!("codex app-server: {masked}")),
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
                self.note_turn(&method, params.as_ref());
                // ★번역기에는 **원본 줄 바이트**를 그대로 넣는다★ — 두 번째 입구를 만들면 그쪽의 라인
                //   재조립·상한·마스킹 규율이 배송 경로 밖으로 나간다(사유 정본 = `decoder.rs` 헤더).
                //   번역기는 개행으로 줄을 가르므로 종단을 함께 준다.
                if let Some(dec) = self.decoder.as_mut() {
                    let mut events = dec.decode(line);
                    events.extend(dec.decode(b"\n"));
                    for ev in events {
                        self.core.emit(ev);
                    }
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

    let code = {
        let mut c = child.lock().unwrap_or_else(|p| p.into_inner());
        match c.try_wait() {
            Ok(Some(status)) => status.code(),
            _ => None,
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
            .start_params
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
    ///   스트림에 오르는 오류 한 줄이고, 이미 준 `Ok` 는 정정되지 않는다.
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
    use super::*;
    use crate::output_core::TurnWiring;
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
        assert!(
            h.seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, OutputEvent::Error(_))),
            "조용히 접으면 화면에 신호가 하나도 없다"
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
        assert!(
            h.seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, OutputEvent::Error(_))),
            "조용히 접으면 화면에 신호가 하나도 없다"
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
        CodexAppServerTransport::open(&spec, true, None, ThreadStartParams::default(), None)
            .expect("open")
    }

    #[cfg(windows)]
    fn open_probe(args: &[&str]) -> (CodexAppServerTransport, Option<u32>) {
        CodexAppServerTransport::open(
            &probe_spec(args),
            true,
            None,
            ThreadStartParams::default(),
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
            ThreadStartParams::default(),
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
