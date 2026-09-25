//! OutputCore — 에이전트 1개의 출력 측 핵심 상태(seq/replay/subscribers/status)와
//! 그 위의 동작(emit/finish/subscribe/...)을 transport·session에서 분리한 공용 struct.
//!
//! 왜 분리하는가: 출력 fanout·종료 전이·구독은 PTY든 API든 transport 종류와 무관하게
//! 동일하다. transport는 바이트·이벤트를 만들어 `emit`/`finish`로 넘기기만 하면 된다.
//!
//! transport(pump)가 emit/finish로 출력·종료를 넘기고, manager/AgentSession이 subscribe/
//! status/snapshot으로 조회한다.
//!
//! tauri import 0. S9 drain/session의 락 규율·불변식을 글자 그대로 보존한다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::backend::TurnClassifier;
use crate::inputs_pending::InputsPendingTable;
use crate::queued_input::QueuedInputs;
use crate::turn::{TurnObservations, TurnSignal};
use crate::types::{
    AgentId, AgentStatus, DropCause, OutputChunk, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, QueuedInputEvent, ReplayKind, SinkId, StatusSink, SubscribeOutcome, TerminalReason,
    TurnOutcome,
};

type OnTerminalHook = Box<dyn Fn(TerminalReason) + Send + Sync>;

/// 진단 스트림 버퍼의 상한 — 이 화신의 stderr 를 **뒤에서부터** 이만큼만 붙든다.
///
/// ★출력 링과 다른 물건이다 — 합치지 말 것★: 링은 터미널 화면과 stream-json 파서가 **함께** 먹는
///   스트림이라, 거기 stderr 를 섞으면 NDJSON 중간에 비-JSON 라인이 껴 파서가 깨지고 화면에도 진단이
///   새 나온다(근거 정본 = `transport::stdio` 모듈의 ADR-0044 주석). 그래서 별도의 작은 버퍼다.
/// ★크기 근거★: 이 버퍼를 읽는 소비자는 활성화 실패 분류 하나뿐이고(`AgentManager` 의 조기 판정),
///   그가 찾는 것은 claude 가 내는 한 줄짜리 진단 문구다. 몇 KB 면 그 줄이 뒤 출력에 밀려나지 않는다.
///   ★상한을 지우면 안 된다★ — 이 버퍼는 세션이 사는 내내 자라므로 무제한이면 진행 로그를 뱉는
///   에이전트에서 그대로 메모리 누수가 된다.
// ADR-0172
const DIAGNOSTIC_CAP_BYTES: usize = 8 * 1024;

/// 에이전트 1개의 출력 측 핵심 상태. 필드별 독립 Mutex(session.rs 모듈 주석의 분리 동기와 동일):
/// emit이 replay/subscribers lock만 짧게 잡는 동안 다른 경로(status 등)와 교착 없이 병행 가능.
pub struct OutputCore {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    id: AgentId,
    epoch: u32,

    // ── 출력 시퀀스 / 상태 ────────────────────────────────────
    seq: AtomicU64,
    status: Mutex<AgentStatus>,
    finalized: AtomicBool,

    // ── 출력 구독 ─────────────────────────────────────────────
    subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,

    // ── Replay buffer ─────────────────────────────────────────
    replay: Mutex<Ring>,

    // ── 진단 스트림 (ADR-0172 분류 입구) ──────────────────────
    /// 이 화신의 **stderr 텍스트** 꼬리(bounded). 출력 링과 **분리된** 별도 버퍼이고, 그 이유와
    /// 크기 근거는 `DIAGNOSTIC_CAP_BYTES`. PTY 세션은 stderr 가 콘솔 스트림에 병합돼 오므로 여기는
    /// 항상 비어 있다(정상 — 그쪽 증거는 `terminal_tail` 이 진다).
    diagnostics: Mutex<String>,

    // ── 상태 알림 ─────────────────────────────────────────────
    status_sink: Arc<dyn StatusSink>,

    // ── pump thread 제어 ──────────────────────────────────────
    drain_handle: Mutex<Option<JoinHandle<()>>>,
    drain_done_rx: Mutex<Option<Receiver<()>>>,

    // ── finalize 1회 hook (ADR-0019 reaper) ───────────────────
    /// 단위테스트는 OutputCore::new 만 쓰고 hook 을 주입하지 않으므로 Option(None=no-op).
    on_terminal: Mutex<Option<OnTerminalHook>>,

    // ── ADR-0113 턴 관측 ──────────────────────────────────────
    /// 생성 후 불변이라 hot path 에 원자 load 도 락도 없다.
    turn: TurnWiring,

    // ── ADR-0231 대기 입력 명부 ───────────────────────────────
    /// 이 화신의 명부 — 세션이 이 Arc 로 읽는다. 명부를 바꾸는 길은 이 코어가 replay 락 안에서 먹이는
    /// 환원 하나뿐이다(그래서 명부 = 링 접두의 환원값).
    queued_inputs: Arc<QueuedInputs>,
    /// 우편 바쁨이 읽는 대기 목록 표. `None` = 명부가 우편에 안 보이는 조립(하네스) — 표도 초인종도 없다.
    inputs_pending: Option<Arc<InputsPendingTable>>,
    /// 「봉인됨」 — `finish` 의 종료 합성이 세운다. 그 뒤의 `Queued` 는 링에 `Dropped{AgentEnded}` 로 선다.
    /// ★replay 락 안에서만 읽고 쓴다★ — 순서(합성이 훑은 뒤인가)는 그 락이 나르고 원자값은 내부 가변성일 뿐이라
    /// `Relaxed` 다. 락 밖에서 읽는 자리를 만들면 합성과 봉인 사이로 `Queued` 가 빠져나간다.
    sealed: AtomicBool,
}

/// 대기 입력 명부 배선(ADR-0231) — 명부와 대기 목록 표는 항상 같이 꽂힌다. 명부만 있고 표가 없으면 사용자
/// 목록이 찬 동안에도 우편이 한가로 읽혀 사용자 글보다 먼저 stdin 에 닿는다.
///
/// ★`registry` 는 그 화신 전용 새 명부다★ — 화신마다 새로 만든다(묘비·「받음 불가 판명」이 화신 사실이다).
/// ★`pending` 은 그 화신을 `register` 한 표여야 한다★ — 표는 등록 없는 쓰기를 버리므로, 빠뜨리면 목록이
/// 우편에 영영 안 보인다(오류는 없다).
// ADR-0231
pub struct QueuedWiring {
    pub registry: Arc<QueuedInputs>,
    pub pending: Arc<InputsPendingTable>,
}

/// 목록 사건 하나가 replay 락 안에서 남긴 것 — 락을 놓은 뒤의 단계(턴 관측 · 「비었다」 · fanout)가 쓴다.
struct ListStep {
    /// 링에 선 줄(발급 순) — 바꿔 적기로 한 사건이 두 줄이 될 수 있다.
    numbered: Vec<(u64, OutputEvent)>,
    /// 분류기에 넘길 원 사건의 자리. `None` = 원 사건이 봉인으로 바꿔 적혔다.
    classify_at: Option<usize>,
    /// 이 사건이 명부를 비웠다 — 그 seq 로 「비었다」를 적고 초인종을 울린다(락 밖, 턴 관측 뒤).
    drained_at: Option<u64>,
}

/// 턴 관측 배선 한 벌(ADR-0113) — 표(어디에 쌓나)와 분류자(무엇이 신호인가)는 항상 같이 꽂힌다.
/// 둘을 따로 두면 "표는 있는데 분류자가 없는" 반쪽 상태가 생긴다.
///
/// ★`OutputCore::new` 의 **필수 인자**인 게 이 타입의 요점이다(ADR-0002 §0 — 싸고 오래 가는 경계는
///   지금 깐다)★: 사후 주입(setter)이면 새 spawn 경로가 그걸 빠뜨려도 컴파일이 통과하고, 그 에이전트는
///   **영구히 idle 로 보고돼** 턴 중에 메일이 꽂힌다 — 에러도 로그도 없이. 인자로 만들면 그 실수가
///   컴파일 에러가 된다. 관측이 필요 없는 조립은 `detached()` 로 **명시**한다(빠뜨림과 구분된다).
pub struct TurnWiring {
    table: Arc<TurnObservations>,
    classify: TurnClassifier,
}

impl TurnWiring {
    /// 운영 배선 — 매니저의 공용 표 + 그 세션 백엔드의 분류자(`backend::turn_classifier`).
    pub fn new(table: Arc<TurnObservations>, classify: TurnClassifier) -> Self {
        Self { table, classify }
    }

    /// ★관측을 쓰지 않는 조립 전용(하네스·독립 smoke)★ — 아무도 읽지 않는 자기 표 + 침묵 분류자.
    ///   신호를 하나도 만들지 않으므로 관측 소비자 관점에서 "그 에이전트는 관측된 적 없음" 과 같다.
    ///
    /// ★운영 spawn 경로는 절대 쓰지 않는다★: 이걸 쓰면 그 에이전트의 턴 게이트가 조용히 꺼진다
    ///   (= 턴 중 주입). 운영은 반드시 `new(manager 표, backend 분류자)` 를 쓴다 — 이름이 다른 이유가
    ///   그것이다. ★그 구분을 강제하는 장치는 **없다**★: `use tauri` 격리나 메시징 격리와 달리 이 호출을
    ///   막는 grep 게이트가 없으므로, 이건 컴파일러가 아니라 규약이 지키는 경계다(운영 호출이 0 인지는
    ///   리뷰가 본다).
    ///
    /// ★왜 `test-harness` feature 로 잠그지 않았나(기록된 트레이드오프)★: 그렇게 하면 core 자체 통합
    ///   테스트(`tests/*.rs`)가 lib 을 **외부 crate 로** 링크하는데 그 기능이 꺼져 있어, core 에도
    ///   self-dev-dependency 를 얹어야 한다. 그 패턴은 이 워크스페이스에 이미 있다(daemon Cargo.toml 의
    ///   ADR-0088 트릭) — 그러니 불가능해서가 아니다. 걸리는 건 그 파일이 함께 문서화한 비용이다:
    ///   cargo 는 한 호출 안에서 같은 crate 의 feature 를 **합집합**하므로, `--all-targets` 로 lib 과
    ///   테스트 타깃을 같이 빌드하면 그 기능이 공유 lib 빌드로 유니피케이션돼 **배포 산출물에 하네스
    ///   표면이 박힌다**. 그 위험을 지금 core 까지 넓히는 값이, "이름이 다른 생성자" 가 이미 주는 방어보다
    ///   크다고 봤다. 잠그기로 결정이 바뀌면 그 릴리스 빌드 규칙(`--all-targets` 금지)이 core 에도 걸린다.
    #[doc(hidden)]
    pub fn detached() -> Self {
        Self {
            table: Arc::new(TurnObservations::new()),
            classify: crate::backend::no_turn_signals,
        }
    }
}

impl OutputCore {
    pub fn new(
        id: AgentId,
        epoch: u32,
        status_sink: Arc<dyn StatusSink>,
        turn: TurnWiring,
    ) -> Self {
        Self {
            id,
            epoch,
            seq: AtomicU64::new(0),
            status: Mutex::new(AgentStatus::Running),
            finalized: AtomicBool::new(false),
            subscribers: Mutex::new(Vec::new()),
            replay: Mutex::new(Ring::new()),
            diagnostics: Mutex::new(String::new()),
            status_sink,
            drain_handle: Mutex::new(None),
            drain_done_rx: Mutex::new(None),
            on_terminal: Mutex::new(None),
            turn,
            queued_inputs: Arc::new(QueuedInputs::new()),
            inputs_pending: None,
            sealed: AtomicBool::new(false),
        }
    }

    /// 대기 입력 명부를 꽂는다 — 부르지 않으면(기본) 아무도 안 읽는 자기 명부에 표가 없다. 봉인·바꿔 적기·사본
    /// 채우기는 어느 쪽이든 같게 돈다(링의 모양은 배선과 무관하다).
    /// ★생성 직후, 첫 사건 전에 부른다★ — 뒤에 갈아 끼우면 명부가 링 접두의 환원값이 아니게 된다.
    /// ★운영 spawn 은 반드시 부른다★ — 빠뜨려도 컴파일되고, 그 화신의 사용자 목록은 우편에 안 보인다.
    // ADR-0231
    pub fn with_queued(mut self, wiring: QueuedWiring) -> Self {
        self.queued_inputs = wiring.registry;
        self.inputs_pending = Some(wiring.pending);
        self
    }

    /// 이 화신의 명부. ★가드를 쥔 채 이 코어에 emit 하지 말 것★ — 락 순서가 replay → 명부라 거꾸로 잡는다.
    // ADR-0231
    pub fn queued_inputs(&self) -> &Arc<QueuedInputs> {
        &self.queued_inputs
    }

    pub fn id(&self) -> AgentId {
        self.id
    }

    /// finalize-시점 hook 주입 — spawn_session 이 sessions 맵 등록 전에 1회 호출한다.
    /// 클로저는 finalized.swap 승자 경로에서만(=정확히 1회) 불린다. ★race 방지★:
    /// 클로저 내부에서 intent·shutting_down 을 그 순간 snapshot 해 ReapMsg 를 빌드·송신해야 한다.
    pub fn set_on_terminal(&self, hook: OnTerminalHook) {
        *self.on_terminal.lock().expect("on_terminal poisoned") = Some(hook);
    }

    /// ADR-0079: resume 스폰 시 `.jsonl` transcript 에서 복원한 과거 이벤트를 replay 버퍼에 **seed**한다.
    ///
    /// ★seed-before-publish 불변식(ADR-0079, load-bearing — seed before publish: closes
    ///   empty-ring-replay + seq-interleave window, cross-family review 2026-07-13)★: manager.
    ///   spawn_session 이 이 core 를 **sessions 맵에 insert 하기 전에**(= 세션이 관측 가능해지기 전에)
    ///   호출한다. 구독·emit 경로 모두 sessions 맵 조회(get_session)를 거치므로, insert 전이면 다른
    ///   스레드가 이 core 에 닿을 수 없다 → seed 도중 재접속 구독이 빈 Ring 을 replay 하거나(seed 는
    ///   fanout 안 함 → 과거 영구 유실), 동시 emit 이 seq 를 뒤섞는(Ring 순서 [0,2,1]) 윈도가 원천 차단된다.
    ///   insert 전이니 당연히 pump(라이브 emit) 시작 전이기도 하다 — 그래서 아직 구독자·라이브 이벤트가 없다.
    ///
    /// ★fanout 없음(seed 시점 특성)★: emit 과 달리 subscribers 로 send 하지 않는다 — 구독자가 아직
    ///   없으므로 무의미하고, 있더라도 seed 는 "버퍼 사전 적재"라 replay 경로(subscribe→replay)로만
    ///   전달돼야 한다(라이브 fanout 이 아니다). 그래서 replay lock 만 짧게 잡아 Ring 에 push 하고 seq 만
    ///   전진시킨다(ADR-0006 락 규율 유지 — subscribers lock 미취득).
    ///
    /// ★seq 연속성★: seed 한 이벤트마다 emit 과 동일하게 `seq.fetch_add(1)` 로 seq 를 발급한다. 그래서
    ///   seed 뒤 첫 라이브 emit 의 seq 가 seed 마지막 seq+1 로 자연히 이어진다(gap·중복 없음).
    ///   finalize·status 는 건드리지 않는다(ADR-0005 — seed 는 종료 전이가 아니다).
    ///
    /// ★턴 관측도 건드리지 않는다 — 되살리지 말 것(ADR-0113)★: transcript 는 **지나간 기록**이라
    ///   턴 중간에 끊긴 것일 수 있고(killed incarnation), 그러면 진행 신호로 끝나고 종료 신호가 없다. 그걸
    ///   관측으로 먹이면 새 화신이 "턴 중" 으로 찍히는데 그 턴의 종료는 **영원히 오지 않는다** — 그 상태를
    ///   기다리는 소비자(우편 파킹 등)가 깨울 수 없이 멈춘다. 관측은 라이브 emit 에서만 시작한다.
    pub fn seed(&self, events: Vec<OutputEvent>) {
        let mut replay = self.replay.lock().expect("replay poisoned");
        for event in events {
            debug_assert_not_list_event(&event);
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event,
                cost_bytes,
            });
        }
    }

    /// **payload-generic (S15 B4)** — 모든 OutputEvent variant(콘솔 바이트 + 구조화)를 받아
    /// Ring 에 저장하고 payload-generic 으로 fanout 한다(ADR-0002 출력 종류 비가정).
    ///
    /// ★핵심 불변식 (ADR-0006 §10 규칙3 — payload 만 바뀌지 락 구조는 불변)★
    /// - `sink.send()` 호출 시 어떤 lock도 보유하지 않는다. subscribers를 clone으로
    ///   스냅샷 뜨고 lock을 즉시 해제한 뒤, 복사본을 돌며 send.
    /// - replay lock과 subscribers lock을 동시에 보유하지 않는다(각각 짧게).
    ///   두 lock 동시 취득은 subscribe 함수 단독 예외이며 emit은 절대 금지.
    pub fn emit(&self, event: OutputEvent) {
        self.emit_inner(event, true);
    }

    /// 같은 이벤트를 **똑같이** 내보내되, ★이 줄을 「이 화신이 턴 중이라는 증거」로 세지 않는다★
    /// (ADR-0113 의 사실 계층에 아무것도 적지 않는다).
    ///
    /// ★존재 이유★: 「화면에 보여야 한다」와 「이 화신이 턴 중이다」는 같은 사실이 아니다. 상대가
    ///   **우리가 연 적 없는 턴**의 줄을 흘리면 그 내용은 화면에 보여야 하지만(버리면 조용한 유실이다),
    ///   그것을 진행 신호로 적으면 그 턴의 종료는 우리 것이 아니라 귀속 게이트에 막히고 — 아무도 그
    ///   사실을 되돌리지 못한 채 30 분 fail-open 밸브까지 우편이 막힌다. 그 비대칭을 닫는 문이다.
    /// ★그래서 종료 신호를 낼 이벤트는 이 문으로 보내지 않는다★ — 이 문은 도어벨
    ///   (`StatusSink::turn_ended`)도 함께 건너뛰므로, 종료를 여기로 보내면 그것을 기다리는 소비자가
    ///   깨어나지 않는다. 판정 한 줄: **이 문은 진행 신호를 적지 않는 자리이지 종료 신호를 지우는 자리가
    ///   아니다.**
    /// ★정리 호출자를 늘리지 않는다(ADR-0127)★ — 관측을 아예 안 적으므로 지울 것도 없다. 그래서
    ///   `finish` + `emit` 의 finalize 재확인이라는 두 자리는 그대로다.
    /// ★호출자는 오늘 둘이고 **둘 다 같은 파일**이다★(`backend/codex/transport.rs`):
    ///   ① 상대가 흘린 미귀속 줄 ② 이어받기 직후 페이지로 받아 온 지난 화면(ADR-0203).
    ///   ★②가 `seed` 가 아니라 이 문으로 오는 것은 **시점이 다르기 때문**이다★ — `seed` 는 세션이
    ///   명부에 오르기 전에만 옳고(fanout 이 없어도 구독자가 없다), ②는 핸드셰이크 뒤라 이미 붙은
    ///   구독자가 있을 수 있다. 거기서 fanout 없는 문을 쓰면 그 구독자는 빈 링을 replay 한 뒤라 지난
    ///   화면을 **영영 못 본다**.
    // ADR-0113
    // ADR-0127
    pub(crate) fn emit_without_turn_observation(&self, event: OutputEvent) {
        self.emit_inner(event, false);
    }

    /// 같은 문으로 **여러 건을 한 덩이로** 내보낸다 — ★그 사이에 다른 emit 이 끼어들 수 없다★.
    ///
    /// ★존재 이유 = 「끼어들 수 없다」 하나다★: 낱개로 부르면 각 호출이 replay 락을 따로 잡으므로, 그
    ///   틈마다 pump 의 라이브 emit 이 seq 를 가져갈 수 있다. 복원된 대화 **한가운데**에 새 줄이 박히는
    ///   것이 그 결과다(ADR-0203 의 이력 복원이 이 문을 쓰는 이유). 한 락 구간 안에서 seq 를 연달아
    ///   발급하면 그 덩이는 링에서 반드시 연속이다.
    /// ★★그 「끼어들 수 없다」는 **링 안에서만** 참이다 — 배달까지로 넓혀 읽지 말 것★★: 아래 구현이
    ///   fanout 은 락을 놓고 하므로(그 규율의 근거 = [`Self::emit`]), **이미 붙어 있는** 구독자에게는
    ///   [덩이 seq N] → [라이브 seq N+k] → [덩이 seq N+1] 순서로 갈 수 있다. 그리고 그 결말은 재정렬이
    ///   아니라 **폐기**다 — 프론트 구독 콜백이 `seq <= lastSeq` 를 버리므로 끼어든 뒤의 덩이 프레임이
    ///   화면에서 사라진다. ★**나중에 붙는** 구독자는 영향이 없다★(링에서 replay 하므로 순서가 온전하다).
    /// ★그리고 **덩이 앞**도 막지 못한다★ — 이 덩이보다 먼저 도착한 라이브 줄은 여전히 앞에 선다.
    /// ★그래서 이 문이 파는 것은 「**링 안에서** 이력이 쪼개지지 않는다」 하나다★ — 배달 순서와 「이력이
    ///   맨 앞이다」는 이 문이 주는 것이 아니고, 그 둘을 함께 닫는 길은 생산자 쪽에서 라이브 emit 을
    ///   게이트까지 붙드는 것뿐이다(그 큐의 상한·넘침 처분이 선결이라 별건이다 — 그 별건은 **안 만들기로
    ///   닫혔다**: ADR-0205. 위 한계는 열린 채로 남는다).
    /// ★락 규율은 [`Self::emit`] 과 같다(ADR-0006)★ — 발급·push 는 replay 락 안에서, fanout 은 락을 놓고
    ///   subscribers 스냅샷으로. 두 락을 동시에 쥐지 않는다.
    /// ★관측·상태·finalize 를 건드리지 않는 것도 낱개 문과 같다★(ADR-0005/0113/0127).
    // ADR-0203
    // ADR-0205
    pub(crate) fn emit_batch_without_turn_observation(&self, events: Vec<OutputEvent>) {
        if events.is_empty() {
            return;
        }
        // 1. 발급 + push 를 **한 번의** replay 락 안에서 — 이 구간이 곧 「끼어들 수 없다」의 실물이다.
        let numbered: Vec<(u64, OutputEvent)> = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            events
                .into_iter()
                .map(|event| {
                    debug_assert_not_list_event(&event);
                    let seq = self.seq.fetch_add(1, Ordering::Relaxed);
                    let cost_bytes = estimate_cost_bytes(&event);
                    replay.push(StoredOutput {
                        seq,
                        event: event.clone(),
                        cost_bytes,
                    });
                    (seq, event)
                })
                .collect()
        };

        // 2. fanout 은 락을 놓고.
        self.fan_out(&numbered);
    }

    /// 여러 줄을 subscribers 스냅샷으로 락 없이 내보낸다 — 죽은 sink 는 낱개 문과 같은 방식으로 한 번에
    /// 걷어낸다. 호출자는 replay 락을 놓은 뒤에 부른다(ADR-0006).
    fn fan_out(&self, numbered: &[(u64, OutputEvent)]) {
        if numbered.is_empty() {
            return;
        }
        let sinks = self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();
        let mut dead = Vec::new();
        for sink in sinks {
            for (seq, event) in numbered {
                let payload = match event {
                    OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                    other => OutputPayload::Event(other),
                };
                if sink
                    .send(OutputFrame {
                        agent_id: self.id,
                        epoch: self.epoch,
                        seq: *seq,
                        payload,
                    })
                    .is_err()
                {
                    dead.push(sink.sink_id());
                    break;
                }
            }
        }
        if !dead.is_empty() {
            self.subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }

    fn emit_inner(&self, event: OutputEvent, observe_turn: bool) {
        let event = match event {
            OutputEvent::QueuedInput(op) => return self.emit_list_event(op, observe_turn),
            other => other,
        };
        let cost_bytes = estimate_cost_bytes(&event);

        // 3~4. ★seq 발급 + replay push 를 replay 락 안에서 원자적으로★ — brief lock(락 순서 1단계,
        //    ADR-0006). **순서 중요(gap 방지)**: replay.push 가 fanout 보다 먼저여야, subscribe 가
        //    이 사이에 끼어들어도 새 sink 는 replay 에서 이 seq 를 받는다(최악 dup, 프론트 seq dedup 이
        //    흡수). 역순이면 gap 발생.
        //
        // ★ADR-0079: seq 발급을 replay 락 안에서 push 직전에 한다(왜 락 밖이면 안 되나)★
        //    pump 스레드(transport stdio/pty)와 write_input 의 synthetic user-echo(session.rs)가
        //    동시에 emit 을 호출한다. seq 를 락 밖에서 fetch_add 하면 두 caller 가 N/N+1 을 발급받고도
        //    락 진입 순서가 뒤집혀 ring 에 N+1 을 N 보다 먼저 push 할 수 있다 → ring 이 seq 로 정렬
        //    깨짐. subscribe_from 은 `partition_point(|c| c.seq <= s)`(seq 오름차순 전제)로 replay
        //    slice 를 자르므로, ring 비단조면 replay 슬라이싱이 무너진다. 발급+push 를 같은 락 구간에
        //    묶어 원자화하면 락 획득 순서가 곧 seq 순서 = ring 항상 단조. (cross-family review 2026-07-13
        //    발견 — seed() 도 동일하게 락 안에서 발급하므로 두 경로가 일치한다.)
        //    Ordering::Relaxed 유지: 락이 순서(happens-before)를 제공하므로 atomic 은 유일성만 담당.
        let seq;
        {
            let mut replay = self.replay.lock().expect("replay poisoned");
            seq = self.seq.fetch_add(1, Ordering::Relaxed);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
        }

        if observe_turn {
            self.record_turn_signal(seq, &event);
        }

        let payload = match &event {
            OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
            other => OutputPayload::Event(other),
        };

        // 5. ★불변식 1(ADR-0006 §10 규칙3)★ send 는 blocking/try_send 가능하므로 lock 을 쥔 채
        //    send 하면 subscribe/다른 send 와 교착·정체.
        let frame = OutputFrame {
            agent_id: self.id,
            epoch: self.epoch,
            seq,
            payload,
        };
        let sinks = self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();

        let mut dead = Vec::new();
        for sink in sinks {
            if sink.send(frame).is_err() {
                dead.push(sink.sink_id());
            }
        }

        if !dead.is_empty() {
            self.subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }

    /// 목록 사건(`QueuedInput`)의 문 — 낱개 문과 같은 단계에 명부 환원과 대기 목록 표 쓰기가 끼어 있다.
    ///
    /// ★replay 락 안(한 구간)★: 봉인 → 판정 뒤 바꿔 적기 → `AckUnavailable` 사본 채우기 → 발급·push·환원 →
    ///   「찼다」. 명부를 이 구간에서 먹이므로 명부 = 링 접두의 환원값이고, 봉인·바꿔 적기의 판정도 링 순서를
    ///   따른다(원자값 `DeliveryAck` 를 읽지 않는다 — 읽으면 링 재생과 명부가 서로 다른 순서를 본다).
    /// ★락 밖★: 턴 관측(바꿔 적지 않은 원 사건만) → 「비었다」 + 초인종 → fanout.
    ///   「비었다」를 턴 관측 **뒤**에 적는 이유: 목록을 비우는 claude `Delivered` 는 그 턴의 진행이라, 락
    ///   안에서 적으면 진행이 표에 오르기 전 두 사실이 함께 「한가」인 순간이 생기고 그때 바쁨을 물은 우편이
    ///   사용자의 턴에 접혀 든다. 「찼다」는 반대로 락 안이다 — 늦추면 `Queued` 가 링에 선 뒤에도 우편이 한가로
    ///   읽혀 사용자 글보다 먼저 stdin 에 닿을 수 있다. 락 밖으로 미룬 「비었다」가 그 사이 다른 스레드가 적은
    ///   더 늦은 「찼다」를 덮지 않는 것은 표의 seq 규칙이 진다.
    // ADR-0231
    fn emit_list_event(&self, op: QueuedInputEvent, observe_turn: bool) {
        let step = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            self.record_list_event(&mut replay, op)
        };
        if observe_turn {
            if let Some(at) = step.classify_at {
                let (seq, event) = &step.numbered[at];
                self.record_turn_signal(*seq, event);
            }
        }
        if let Some(seq) = step.drained_at {
            self.write_drained(seq);
        }
        self.fan_out(&step.numbered);
    }

    /// replay 락 구간의 목록 단계. 호출자가 replay 락을 쥐고 부른다.
    // ADR-0231: 락 순서 = (세션 `input_order` →) replay → 명부 → 대기 목록 표. 명부 가드는 이 replay 구간
    //   안에서만 잡고, 표는 잎이며, `StatusSink` 는 어느 락도 쥐지 않은 채 부른다(`write_drained`). 명부를 먼저
    //   쥐고 replay 를 기다리는 자리를 만들면 이 둘이 서로를 기다린다 — 명부만 읽는 쪽(목록 조회·취소 검증)은
    //   명부 락 하나만 잡고 emit 하지 않는다.
    fn record_list_event(&self, replay: &mut Ring, op: QueuedInputEvent) -> ListStep {
        let mut registry = self.queued_inputs.lock();
        let was_empty = registry.is_empty();
        // 봉인이 바꿔 적기보다 먼저다 — 종료 뒤의 `Queued` 는 받음이 아니라 버림이다.
        let (events, classify_at) = match op {
            QueuedInputEvent::Queued { id, .. } if self.sealed.load(Ordering::Relaxed) => {
                // 사용자 글이 전달 없이 버려지는 자리라 흔적을 남긴다 — 본문은 싣지 않는다(id 만).
                tracing::debug!(
                    agent = %self.id,
                    epoch = self.epoch,
                    id = %id,
                    "봉인 뒤의 Queued 를 Dropped(AgentEnded) 로 바꿔 적음"
                );
                (
                    vec![QueuedInputEvent::Dropped {
                        id,
                        cause: DropCause::AgentEnded,
                    }],
                    None,
                )
            }
            // 판명 뒤의 `Queued` 는 그 자리에서 받음으로 닫는다 — `Delivered` 가 아니라 사본을 실은
            //   `AckUnavailable` 을 잇는 이유: 링 상한이 두 줄 사이를 잘라도 닫는 줄이 본문을 쥔다.
            queued @ QueuedInputEvent::Queued { .. } if registry.ack_unavailable_seen() => (
                vec![
                    queued,
                    QueuedInputEvent::AckUnavailable {
                        delivered: Vec::new(),
                    },
                ],
                Some(0),
            ),
            other => (vec![other], Some(0)),
        };
        let mut numbered = Vec::with_capacity(events.len());
        for mut op in events {
            // ★사본은 늘 코어가 채운다★(디코더는 빈 채로 낸다) — 환원 **전**의 열린 항목이 그 환원이 받음으로
            //   닫는 항목이다.
            if let QueuedInputEvent::AckUnavailable { delivered } = &mut op {
                *delivered = registry.open_copies();
            }
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            registry.reduce(seq, &op);
            let event = OutputEvent::QueuedInput(op);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
            numbered.push((seq, event));
        }
        let now_empty = registry.is_empty();
        drop(registry);

        let last_seq = numbered.last().map(|(seq, _)| *seq).expect("한 줄 이상");
        let mut drained_at = None;
        match (was_empty, now_empty) {
            (true, false) => {
                if let Some(pending) = &self.inputs_pending {
                    pending.set(self.id, self.epoch, last_seq, true);
                }
            }
            (false, true) => drained_at = Some(last_seq),
            // 판정 뒤 바꿔 적기는 빔 → 빔이다 — 목록이 찬 순간을 아무도 못 보므로 적을 것도 울릴 것도 없다.
            _ => {}
        }
        ListStep {
            numbered,
            classify_at,
            drained_at,
        }
    }

    /// 「비었다」를 적고 초인종을 울린다 — 어느 락도 쥐지 않은 채. 표가 없는 조립은 둘 다 하지 않는다.
    /// 표가 먼저다: 초인종을 받은 쪽이 곧바로 바쁨을 다시 묻는다.
    // ADR-0231
    fn write_drained(&self, seq: u64) {
        if let Some(pending) = &self.inputs_pending {
            pending.set(self.id, self.epoch, seq, false);
            self.status_sink.inputs_drained(self.id, self.epoch);
        }
    }

    /// 분류자가 낸 턴 신호를 표에 적고, 종료면 도어벨을 울린다. 호출자는 replay 락을 놓은 뒤에 부른다.
    fn record_turn_signal(&self, seq: u64, event: &OutputEvent) {
        // ★ADR-0113 턴 관측★: 표 갱신을 **fanout·통지보다 먼저** 한다. 통지를 받은 소비자가 곧바로
        //   표를 조회하므로(도어벨→flush 등) 순서가 뒤집히면 그 조회가 갱신 전 값을 본다.
        //   락 규율: 표 갱신은 자기 락 하나만 짧게 잡고(core 락 미보유), 통지는 그 락을 놓은 뒤 한다.
        if let Some(signal) = (self.turn.classify)(event) {
            // ★신호에 **출력 순서(seq)** 를 실어 보낸다★: emit 호출자는 둘이라(pump · 입력 에코를 낸
            //   주입 스레드) 두 emit 이 병행하면 표 적용 순서가 발행 순서와 뒤집힐 수 있다. seq 는 replay
            //   락 안에서 발급돼 **출력의 정본 순서**이므로, 표가 그걸로 늦은 신호를 걸러낸다(turn.rs).
            self.turn.table.observe(self.id, self.epoch, seq, signal);
            // ★종료 후 지각 emit 이 유령 항목을 되살리지 못하게(load-bearing)★: 주입 스레드가 transport
            //   write 에 막혀 있는 동안 pump 가 EOF→`finish` 를 지나 표를 비울 수 있고, 그 뒤 깨어난 에코가
            //   **같은 epoch** 으로 항목을 다시 만든다(더 작은 epoch 만 버리는 표 쪽 규칙으론 못 막는다).
            //   그 항목은 종료 신호가 영영 오지 않아 아무도 못 지운다.
            //   ★무엇이 순서를 만드나 = **표의 뮤텍스**(이 인자를 지우지 말 것)★: 그 락이 우리 insert 와
            //   `finish` 의 forget 을 **전순서**로 놓는다. forget 이 먼저인 순서에서는 forget 의 unlock
            //   (release)이 우리 lock(acquire)과 synchronizes-with 하므로, forget 앞의 `finalized` swap 이
            //   우리 load 보다 happens-before → load 는 false 를 읽을 수 없다. 반대 순서면 우리 insert 가
            //   먼저이므로 뒤따르는 forget 이 그걸 지운다. 어느 쪽이든 유령이 남지 않는다.
            //   ★그래서 이 load 의 `Acquire` 가 근거가 아니다★ — `Relaxed` 여도 결론은 같고, 순서를 나르는
            //   것은 뮤텍스다. 표 갱신을 락 밖(lock-free)으로 "최적화" 하면 이 증명이 조용히 무너진다.
            if self.finalized.load(Ordering::Acquire) {
                self.turn.table.forget(self.id, self.epoch);
            }
            // ★통지는 epoch·finalize·seq 게이트를 걸지 않는다(위 표 갱신과 의도적으로 비대칭)★: 표는
            //   **상태**라 죽은/옛 화신이 쓰면 산 화신의 사실이 오염되지만, 도어벨은 **일회성 자극**이라
            //   잉여는 빈 큐 no-op 으로 흡수되고(소비자가 실행 시점에 재검증) 누락은 대기를 만든다
            //   ("누락 < 잉여" — StatusSink::turn_ended 계약).
            if matches!(signal, TurnSignal::Ended(_)) {
                self.status_sink.turn_ended(self.id, self.epoch);
            }
        }
    }

    /// 종료 전이 — pump가 루프 탈출 후 1회 호출. finalize 정확히 1회 게이트로 중복 호출을 흡수한다.
    ///
    /// terminal 알림 주체는 pump(=여기) 단독. reason→AgentStatus 매핑은 impl-spec 표 그대로.
    /// ★남은 목록 항목의 `Dropped{AgentEnded}` 가 종점 전이보다 먼저 나간다(ADR-0231)★ — 구독자는 목록이
    ///   닫힌 뒤에 종점을 본다.
    pub fn finish(&self, reason: TerminalReason) {
        if self.finalized.swap(true, Ordering::AcqRel) {
            return;
        }

        // ★ADR-0231 종료 합성 + 봉인 = 한 replay 락 구간★: 낱개 emit 은 호출마다 락을 따로 잡으므로, 훑기와
        //   봉인 사이에 선 `Queued` 가 둘 다를 빠져나가 영구 항목(= 상한 없는 우편 막힘)이 된다. 이 덩이는 턴
        //   관측을 지나지 않는다(관측 정리 지점을 늘리지 않는다 — ADR-0127).
        let synthesized = {
            let mut replay = self.replay.lock().expect("replay poisoned");
            self.synthesize_ended_and_seal(&mut replay)
        };
        if !synthesized.is_empty() {
            tracing::info!(
                agent = %self.id,
                epoch = self.epoch,
                count = synthesized.len(),
                "종료 합성: 남은 대기 입력을 Dropped(AgentEnded) 로 닫음"
            );
        }
        // ★종료 합성은 목록을 비워도 「비었다」를 적지도 초인종을 울리지도 않는다★: 대기 목록 표의 이 화신
        //   항목은 아래에서 턴 항목과 함께 거두므로 적어 봐야 곧 지워지고, 초인종은 종점 전이를 앞둔 화신에게
        //   우편을 흘려보내라는 자극이 된다. 파킹된 우편은 다음 화신의 등장 flush 가 나른다.
        // ADR-0231
        self.fan_out(&synthesized);

        // AgentStatus 변형 추가 금지(impl-spec 표).
        // ★reason 은 reaper hook 에도 넘겨야 하므로 매핑 전에 clone 해 둔다(소비 전 보존).
        let new_status = match reason.clone() {
            TerminalReason::Exited { code } => AgentStatus::Exited { code },
            TerminalReason::Killed => AgentStatus::Killed,
            TerminalReason::Interrupted => AgentStatus::Killed,
            TerminalReason::StreamClosed => AgentStatus::Exited { code: None },
            TerminalReason::Cancelled => AgentStatus::Killed,
            TerminalReason::Error(s) => AgentStatus::Failed { message: s },
        };

        {
            let mut status = self.status.lock().expect("status poisoned");
            *status = new_status.clone();
        }

        // ★ADR-0113: 이 화신의 턴 관측을 여기서 거둔다★ — finalize 승자 경로라 **정확히 1회**고(ADR-0005),
        //   terminal 로 넘어간 화신은 더 이상 사실을 만들지 않는다. 안 지우면 턴 중에 죽은 에이전트가
        //   "턴 중" 으로 남아 그 앞에서 기다리던 소비자(우편 파킹 등)가 영영 풀리지 않는다.
        //   ★왜 여기고 reaper 가 아닌가★: reaper 는 이 finish 가 보낸 ReapMsg 로만 깨어나므로 **늘 뒤**고,
        //   무엇보다 `finalized` 플래그와 같은 지점이어야 emit 쪽 지각 삽입과의 경쟁이 순서로 닫힌다
        //   (emit 의 finalize 재확인 주석이 그 인과의 다른 반쪽). epoch 일치 검사는 forget 이 한다.
        // ADR-0113
        self.turn.table.forget(self.id, self.epoch);
        // 대기 목록 표도 같은 자리에서 거둔다. ★emit 쪽 finalize 재확인은 이 표에 필요 없다★ — 표의 `set` 은
        //   항목을 만들지 않으므로(`register` 만 만든다) 지각한 쓰기가 거둔 항목을 되살릴 수 없다.
        // ADR-0231
        if let Some(pending) = &self.inputs_pending {
            pending.forget(self.id, self.epoch);
        }

        // status lock 해제 후 외부 호출(§10: status lock 보유 중 외부호출 금지).
        self.status_sink
            .status_changed(self.id, new_status, self.epoch);

        // ★ADR-0019 reaper hook★: on_terminal lock 은 짧게 잡고 즉시 clone 없이 호출 — 다른 core
        //   lock 미보유 구간이라 안전.
        if let Some(hook) = self
            .on_terminal
            .lock()
            .expect("on_terminal poisoned")
            .as_ref()
        {
            hook(reason);
        }
    }

    /// 열린 항목마다 `Dropped{AgentEnded}` 를 발급·push·환원하고 봉인을 세운다. 호출자가 replay 락을 쥐고
    /// 부른다(`finish` 한 곳). 반환 = 링에 선 줄.
    // ADR-0231
    fn synthesize_ended_and_seal(&self, replay: &mut Ring) -> Vec<(u64, OutputEvent)> {
        let mut registry = self.queued_inputs.lock();
        let open: Vec<String> = registry.rows().iter().map(|row| row.id.clone()).collect();
        let mut numbered = Vec::with_capacity(open.len());
        for id in open {
            let op = QueuedInputEvent::Dropped {
                id,
                cause: DropCause::AgentEnded,
            };
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            registry.reduce(seq, &op);
            let event = OutputEvent::QueuedInput(op);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
            numbered.push((seq, event));
        }
        debug_assert!(registry.is_empty(), "종료 합성 뒤 열린 항목이 남았다");
        self.sealed.store(true, Ordering::Relaxed);
        numbered
    }

    /// 과도기 Exiting 전이 — manager kill 0.5단계용. Exiting 알림 주체가 이 경로.
    /// 이미 finalize됐거나 terminal이면 false(덮어쓰지 않음). Running 등 비-terminal일 때만
    /// Exiting 기록 + status_changed(Exiting) 발행 후 true.
    ///
    /// **보호 범위(정확히):** 아래 finalized/terminal 검사가 보호하는 것은 **status 필드 값**이다
    /// (terminal이 Exiting으로 덮여 고착되는 것 방지). status_sink로 나가는 **알림 순서**는
    /// 보호하지 않는다 — finish의 status_changed와 lock 밖에서 경합할 수 있다.
    /// 단, 정상 kill 경로는 manager가 enter_exiting() 완주 후에야 transport.shutdown()을 호출하므로
    /// (그제서야 pump가 깨어 finish), Exiting 알림이 terminal 알림보다 먼저 완주해 순서가 보장된다.
    /// 알림 역전 창은 "프로세스 자연 종료가 kill과 동시에 겹치는 순간"뿐이며, 이는 S9 원본
    /// (manager.kill_agent의 Exiting 발행 vs drain.transition의 terminal 발행)에도 동일하게 존재하는
    /// 기존 동작이다. 프론트는 terminal 판정을 status_changed가 아니라 agent-list-updated(목록)로
    /// 하므로(CLAUDE.md 핵심 불변식) 늦은 Exiting을 받아도 고착되지 않는다 — 설계상 완화됨.
    pub fn enter_exiting(&self) -> bool {
        // 이미 종료 처리됐으면 Exiting을 쓰지 않는다(빠른 경로 — 실제 status 필드 보호는 아래 lock 구간).
        if self.finalized.load(Ordering::Acquire) {
            return false;
        }

        {
            let mut status = self.status.lock().expect("status poisoned");
            if matches!(
                *status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return false;
            }
            *status = AgentStatus::Exiting;
        }

        // status lock 해제 후 외부 호출.
        self.status_sink
            .status_changed(self.id, AgentStatus::Exiting, self.epoch);
        true
    }

    /// pump 종료 대기 — kill 6단계. 수신측이 이미 사라졌어도(타임아웃 후 detach) 무시 가능하도록
    /// 결과를 버린다.
    pub fn join_pump(&self, timeout: Duration) {
        let rx = self
            .drain_done_rx
            .lock()
            .expect("drain_done_rx poisoned")
            .take();
        if let Some(rx) = rx {
            let _ = rx.recv_timeout(timeout);
        }
    }

    /// pump 핸들/done_rx 적재 — transport.start(stage 3)가 pump 스레드를 띄운 뒤 호출.
    pub fn attach_pump(&self, handle: JoinHandle<()>, done_rx: Receiver<()>) {
        *self.drain_handle.lock().expect("drain_handle poisoned") = Some(handle);
        *self.drain_done_rx.lock().expect("drain_done_rx poisoned") = Some(done_rx);
    }

    /// 구독자 등록 + replay 전송. SinkId 반환(unsubscribe용).
    ///
    /// **C4 (LLD §7, 절대 준수):** subscribers lock을 보유한 채로 replay를 전송한다.
    /// 이렇게 하면 emit의 live send와 이 replay send가 같은 subscribers lock으로
    /// 직렬화되어 replay→live 순서 역전이 원천 차단된다. emit은 step 5에서 subscribers
    /// lock을 잡으려다 잠깐 대기하지만, replay 전송은 일회성이라 허용된다.
    ///
    /// **락 순서 규칙 3 예외 (LLD §10):** subscribe 함수만 subscribers→replay 두 lock을
    /// 동시에 취득한다(항상 이 순서). emit은 두 lock 동시 보유 절대 금지.
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        let sink_id = sink.sink_id();

        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());

        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        // snapshot의 seq와 이후 live chunk의 seq가 끊기지 않아 프론트가 seq로 dedup/정렬 가능.
        // 막 등록된 sink라 send 실패는 unlikely → 무시(§7).
        for stored in &snapshot {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }

        drop(subscribers_guard);

        sink_id
    }

    /// after_seq/epoch 기반 선택적 replay 구독. `subscribe`의 C4 패턴(subscribers lock 보유 중
    /// replay 전송)을 그대로 따르되, 보낼 범위를 분기한다.
    ///
    /// 분기:
    /// - epoch 불일치(epoch_matches=false) 또는 after_seq=None → FromOldest(전체).
    /// - epoch 일치 & after_seq=Some(s):
    ///     - 버퍼 비었으면 → Resumed(전송 0).
    ///     - s < oldest → Truncated(oldest 부터 전체).
    ///     - s >= oldest → Resumed(seq>s 인 tail 만).
    ///
    /// `on_ready`: 분기·메타(oldest/latest/kind/replay_from)가 확정된 뒤 **replay 를 sink 로
    ///   전송하기 직전**에 1회 호출된다. 데몬이 이 안에서 SubscribeAck 를 먼저 큐잉해
    ///   "Ack→replay binary" FIFO 순서(불변식 2)를 보장하면서도, Ack 필드를 이 단일 스냅샷
    ///   기준 outcome 으로 채워 TOCTOU(스냅샷 A/B 불일치)를 제거한다. **on_ready 안에서 블로킹 금지**
    ///   (subscribers lock 보유 중 호출 — non-blocking try_send 만).
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> SubscribeOutcome {
        let sink_id = sink.sink_id();
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());
        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        let oldest = snapshot.first().map(|c| c.seq).unwrap_or(0);
        let latest = snapshot.last().map(|c| c.seq).unwrap_or(0);

        let (kind, start_idx) = match after_seq {
            _ if !epoch_matches => (ReplayKind::FromOldest, 0usize),
            None => (ReplayKind::FromOldest, 0usize),
            Some(s) => {
                if snapshot.is_empty() {
                    (ReplayKind::Resumed, 0usize)
                } else if s < oldest {
                    (ReplayKind::Truncated, 0usize)
                } else {
                    // seq<=s 인 prefix 를 건너뛴다(seq 연속 보장 → partition_point 안전).
                    let idx = snapshot.partition_point(|c| c.seq <= s);
                    (ReplayKind::Resumed, idx)
                }
            }
        };

        let to_send = &snapshot[start_idx..];

        let replay_from = to_send
            .first()
            .map(|c| c.seq)
            .unwrap_or_else(|| match after_seq {
                Some(s) => s.saturating_add(1),
                None => latest.saturating_add(1),
            });

        let outcome = SubscribeOutcome {
            kind,
            sink_id,
            oldest_seq: oldest,
            latest_seq: latest,
            replay_from,
            replayed: to_send.len(),
        };

        on_ready(&outcome);

        for stored in to_send {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }
        drop(subscribers_guard);

        outcome
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.subscribers
            .lock()
            .expect("subscribers poisoned")
            .retain(|s| s.sink_id() != sink_id);
    }

    /// replay 스냅샷 — 늦게 붙는 창의 초기 복원용(get_snapshot → wire SnapshotChunk 경로).
    ///
    /// ★계약 유지(B5)★: 이 게터는 여전히 `Vec<OutputChunk>`(바이트 전용 wire 미러)를 반환한다 —
    /// 호출부(manager.get_snapshot → daemon snapshot_chunk_to_wire)가 이 형태에 의존한다. Ring 은
    /// payload-generic(StoredOutput) 이지만, 이 경로의 wire 타입(OutputChunk{seq,data})은 아직 바이트
    /// 전용이라 **TerminalBytes 만** 변환한다. 구조화 이벤트의 snapshot wire 매핑은 B7(daemon adapter)
    /// 몫이라 여기선 스킵한다 — 현재 구조화 이벤트 생산자가 없어(B3 미배선) 런타임 유실 없음.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.replay
            .lock()
            .expect("replay poisoned")
            .snapshot()
            .into_iter()
            .filter_map(|s| match s.event {
                OutputEvent::TerminalBytes(data) => Some(OutputChunk { seq: s.seq, data }),
                // ★무음 유실 관측 훅★: subscribe_from replay 경로는 payload-generic 이라 정상 전달돼
                // 두 복원 경로가 비대칭 — B3 배선 순간 무음 유실이 되므로 drop 을 warn 으로 관측
                // 가능하게 남긴다.
                ref other => {
                    // variant 태그만 로그(payload 내용은 로그에 싣지 않음 — 민감/대용량 회피).
                    let kind = match other {
                        OutputEvent::TerminalBytes(_) => "TerminalBytes", // 위 arm 이 처리 — 도달 안 함
                        OutputEvent::TextDelta { .. } => "TextDelta",
                        OutputEvent::ToolCall { .. } => "ToolCall",
                        OutputEvent::Usage { .. } => "Usage",
                        OutputEvent::MessageDone { .. } => "MessageDone",
                        OutputEvent::TurnEnd { .. } => "TurnEnd",
                        OutputEvent::Error(_) => "Error",
                        OutputEvent::Structured { .. } => "Structured",
                        OutputEvent::QueuedInput(_) => "QueuedInput",
                    };
                    tracing::warn!(
                        seq = s.seq,
                        kind,
                        "structured event dropped from wire snapshot (B7 미배선 — get_snapshot 경로 무음 유실)"
                    );
                    None
                }
            })
            .collect()
    }

    /// 이 화신이 마지막으로 낸 **콘솔 바이트**를 최대 `max_bytes` 만큼(뒤에서부터) 돌려준다.
    ///
    /// ★`snapshot()` 의 형제가 아니다 — 그걸 대신 쓰지 말 것★: `snapshot()` 은 링 전체를 clone 하고
    ///   (문서상 최대 2MB) 콘솔 바이트가 아닌 항목마다 warn 을 찍는다. 이 게터의 호출자는 죽은 세션의
    ///   종료 문구 몇 줄만 보므로, 그 비용과 로그 폭주를 치를 이유가 없다.
    /// ★상한은 정확히 지켜진다★: 마지막 청크가 상한을 넘으면 그 청크의 **끝쪽만** 잘라 담는다.
    /// ★빈 결과가 정상이다★: 구조화(NDJSON) 세션의 링에는 콘솔 바이트가 하나도 없다.
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        self.replay
            .lock()
            .expect("replay poisoned")
            .terminal_tail(max_bytes)
    }

    /// 이 화신의 진단(stderr) 한 줄을 붙든다 — transport 의 stderr drain 스레드가 부른다.
    ///
    /// ★로그와 **독립**이어야 한다(load-bearing)★: 같은 줄이 `tracing::debug!(target: "agent_stderr")`
    ///   로도 나가지만 기본 로그 필터는 `warn` 이라 그 줄은 평소 **버려진다**. 분류를 그 로그에 매달면
    ///   로그 레벨이 기능을 켜고 끄게 된다(= 개발자 머신에서만 되는 기능). 그래서 여기 쌓는 것은 레벨과
    ///   무관하게 항상 일어나고, 호출자는 로그 매크로보다 **먼저** 이걸 부른다.
    /// ★마스킹된 텍스트를 받는다(호출자 의무)★: 외부 프로세스 출력이라 자격증명이 섞일 수 있고, 이
    ///   버퍼는 분류 근거로 읽히는 것에 더해 실패 사유로 로그·보고에 실릴 수 있는 자리다.
    /// ★상한은 앞에서부터 버려 지킨다★ — 최신 줄이 증거이므로 뒤를 남긴다.
    // ADR-0172
    pub fn push_diagnostic(&self, line: &str) {
        let mut buf = self.diagnostics.lock().expect("diagnostics poisoned");
        buf.push_str(line);
        buf.push('\n');
        if buf.len() > DIAGNOSTIC_CAP_BYTES {
            // ★자르는 위치를 **문자 경계**로 밀어 올린다★: 바이트로 그냥 자르면 멀티바이트 문자를
            //   쪼개 `String` 불변식이 깨지고 `drain` 이 panic 한다 — claude 진단에 한글·이모지가
            //   섞이면 실제로 걸린다.
            let mut cut = buf.len() - DIAGNOSTIC_CAP_BYTES;
            while cut < buf.len() && !buf.is_char_boundary(cut) {
                cut += 1;
            }
            buf.drain(..cut);
        }
    }

    /// 지금까지 붙든 진단 텍스트 전부(최대 `DIAGNOSTIC_CAP_BYTES`).
    ///
    /// ★빈 결과가 정상이다★: PTY 세션은 stderr 를 콘솔 스트림에 병합해 내보내므로 이 버퍼를 쓰지
    ///   않는다 — 그쪽 증거는 `terminal_tail` 이 진다. 호출자는 둘을 **합쳐** 분류에 넘긴다.
    // ADR-0172
    pub fn diagnostic_tail(&self) -> String {
        self.diagnostics
            .lock()
            .expect("diagnostics poisoned")
            .clone()
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().expect("status poisoned").clone()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::types::AgentId;

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    fn core() -> OutputCore {
        OutputCore::new(
            AgentId::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )
    }

    /// 진단 버퍼는 **분리돼 있고 비어 있는 것이 기본**이다 — 링에 아무것도 흘리지 않는다.
    ///
    /// ★이 단언이 지키는 것★: 이 버퍼를 출력 링으로 합치는 변경이 오면 여기가 붉어진다. 합치면 NDJSON
    ///   파서 중간에 비-JSON 라인이 껴 구조화 세션이 깨지고, 터미널 화면에도 진단이 새 나온다.
    #[test]
    fn diagnostics_never_leak_into_the_output_ring() {
        let core = core();
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        assert!(
            core.diagnostic_tail().contains("No conversation found"),
            "진단은 진단 버퍼에 남는다"
        );
        assert!(
            core.terminal_tail(4096).is_empty(),
            "진단이 출력 링에 들어가면 안 된다 — 링은 화면과 stream-json 파서가 함께 먹는다"
        );
        assert!(core.snapshot().is_empty(), "링에 이벤트가 생겨서도 안 된다");
    }

    /// 상한을 넘겨도 **최신 줄이 남고**, 멀티바이트 문자를 쪼개지 않는다.
    ///
    /// ★후자가 panic 회귀다★: 바이트 위치로 그냥 자르면 `String` 불변식이 깨져 `drain` 이 panic 한다 —
    ///   claude 진단에 한글·이모지가 섞이면 실제로 걸린다. 그 패닉은 stderr drain 스레드에서 나므로
    ///   조용히 진단 캡처만 죽는다(런타임에 거의 무신호).
    #[test]
    fn the_diagnostic_buffer_is_bounded_and_utf8_safe() {
        let core = core();
        // 한 줄이 순수 멀티바이트라 어느 지점에서 잘라도 경계에 걸린다.
        let noisy = "가".repeat(512); // 3 bytes each = 1536B/line
        for _ in 0..32 {
            core.push_diagnostic(&noisy);
        }
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        let tail = core.diagnostic_tail();
        assert!(
            tail.len() <= DIAGNOSTIC_CAP_BYTES,
            "상한이 안 지켜지면 세션이 사는 내내 자라는 누수가 된다 — got {}B",
            tail.len()
        );
        assert!(
            tail.contains("No conversation found"),
            "앞에서부터 버리므로 **최신** 줄이 증거로 남아야 한다"
        );
    }
}

// ── S15 B4: payload-generic replay 버퍼 ───────────────────────────────────────────
//
// ADR-0002: 출력 종류 비가정 — 버퍼가 바이트/이벤트를 차별하지 않는다.

#[derive(Debug, Clone)]
pub struct StoredOutput {
    pub seq: u64,
    pub event: OutputEvent,
    /// eviction 바이트 예산 산정용 근사 크기(정확한 wire 크기 아님 — 아래 estimate_cost_bytes 참조).
    pub cost_bytes: usize,
}

/// 목록 사건은 명부를 먹이는 낱개 문(`emit` 계열)으로만 링에 든다 — `seed`·덩이 문으로 들면 명부가 링 접두의
/// 환원값이 아니게 된다(그 줄은 명부를 모른 채 링에만 선다).
// ADR-0231
fn debug_assert_not_list_event(event: &OutputEvent) {
    debug_assert!(
        !matches!(event, OutputEvent::QueuedInput(_)),
        "목록 사건이 명부를 거치지 않는 문으로 들었다: {event:?}"
    );
}

/// OutputEvent 의 **eviction 예산용** 크기 근사.
///
/// ★왜 근사인가(TRD 핵심)★: core 는 직렬화를 못 한다(ADR-0003 — wire 변환은 daemon adapter 몫).
/// 그래서 정확한 wire 바이트 수를 계산할 수 없다. 하지만 큰 `args_json`(도구 인자)·`json`(Structured)
/// 이벤트가 "건수 1" 로만 세지면 max_bytes(2MB) 상한을 우회해 버퍼가 무한정 커진다. 이를 막으려
/// **payload 문자열 필드들의 바이트 길이 합**을 구조적으로 근사해 예산에 반영한다. 이 값은 eviction
/// 판단 전용이며 정확한 직렬화 크기가 아니다(태그·구분자·escape 오버헤드 무시).
/// ★`pub(crate)` 인 것은 **링 바깥에서 「이만큼이면 링을 채운다」를 셀 수 있어야 하기 때문이다**★:
/// 복원 이력을 상대에게 **요청해서** 받는 backend(codex app-server)는 몇 건을 받아 올지 스스로 정해야
/// 하는데, 그 천장은 [`REPLAY_MAX_BYTES`]·[`REPLAY_MAX_EVENTS`] 이고 무게를 세는 축은 이 함수다. 다른
/// 축으로 어림잡으면 그 backend 가 링이 버릴 것을 더 받아 오거나(순 비용) 실을 수 있는 것을 덜 받아
/// 온다(화면 손실). (ADR-0203)
pub(crate) fn estimate_cost_bytes(event: &OutputEvent) -> usize {
    match event {
        OutputEvent::TerminalBytes(v) => v.len(),
        OutputEvent::TextDelta {
            text,
            turn_id,
            message_id,
        } => text.len() + opt_len(turn_id) + opt_len(message_id),
        OutputEvent::ToolCall {
            name,
            args_json,
            id,
            turn_id,
            message_id,
        } => name.len() + args_json.len() + opt_len(id) + opt_len(turn_id) + opt_len(message_id),
        // Usage 는 고정 크기 수치 필드 — turn_id 문자열만 반영(u64 두 개는 무시).
        OutputEvent::Usage { turn_id, .. } => opt_len(turn_id),
        OutputEvent::MessageDone {
            turn_id,
            message_id,
        } => opt_len(turn_id) + opt_len(message_id),
        // 결말의 실패 사유만 길이를 상대가 정한다 — 나머지 세 갈래는 무게가 없다.
        OutputEvent::TurnEnd { turn_id, outcome } => {
            opt_len(turn_id)
                + match outcome {
                    TurnOutcome::Failed { detail } => opt_len(detail),
                    TurnOutcome::Completed | TurnOutcome::Interrupted | TurnOutcome::Unknown => 0,
                }
        }
        OutputEvent::Error(s) => s.len(),
        OutputEvent::Structured { kind, json } => kind.len() + json.len(),
        // 본문을 싣는 둘 — `Queued` 의 글과 받음 불가 판명의 말풍선 사본.
        // ADR-0231
        OutputEvent::QueuedInput(ev) => match ev {
            QueuedInputEvent::Queued { id, text } => id.len() + text.len(),
            QueuedInputEvent::AckUnavailable { delivered } => delivered
                .iter()
                .map(|copy| copy.id.len() + copy.text.len())
                .sum(),
            QueuedInputEvent::CancelRequested { id }
            | QueuedInputEvent::CancelAnswered { id, .. }
            | QueuedInputEvent::CancelFailed { id }
            | QueuedInputEvent::Delivered { id }
            | QueuedInputEvent::Dropped { id, .. } => id.len(),
        },
    }
}

fn opt_len(s: &Option<String>) -> usize {
    s.as_deref().map(str::len).unwrap_or(0)
}

/// 늦게 붙는 창을 위한 출력 replay ring buffer — 상한 2MB **그리고** event 수 상한.
/// StoredOutput 전용 구체 타입으로 둔다(제네릭 `Ring<T>` 는 이 프로젝트에 다른 저장 대상이 없어
/// 과함 — 단순한 쪽).
///
/// ★event 수 상한 이유(S12 consult, GPT 단독 catch): byte 상한만 있으면 1바이트 청크가
/// 폭주할 때 event 수가 수백만으로 불어, 신규 구독자가 replay를 받을 때 bounded mpsc를
/// 즉시 가득 채워 매 재연결이 slow-consumer로 끊기는 영구 루프가 생긴다. 둘 중 하나라도
/// 초과하면 앞부터 evict.
pub struct Ring {
    items: VecDeque<StoredOutput>,
    total_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

/// 링의 바이트 천장. ★상수로 꺼내 둔 것은 [`estimate_cost_bytes`] 와 같은 사유다★ — 이력을 요청해서
/// 받는 backend 가 「여기까지만 받으면 된다」를 이 값으로 판정한다(ADR-0203). 링 자신이 쓰는 자리는
/// 아래 [`Ring::new`] 하나다.
pub(crate) const REPLAY_MAX_BYTES: usize = 2 * 1024 * 1024;

/// 링의 건수 천장 — 사유는 [`REPLAY_MAX_BYTES`] 와 같다.
pub(crate) const REPLAY_MAX_EVENTS: usize = 4096;

impl Ring {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            total_bytes: 0,
            max_bytes: REPLAY_MAX_BYTES,
            // 4096: 데몬 WS 송신 큐 cap(예 4608) − control_slack(512) 이하로 잡아
            // replay만으로 신규 구독자 큐가 넘치지 않게 한다.
            max_events: REPLAY_MAX_EVENTS,
        }
    }

    /// StoredOutput 을 뒤에 추가하고, 이중 상한(cost_bytes 합 OR 건수) 초과 시 앞부터 evict.
    /// total_bytes 는 push 마다 cost_bytes 로 누적, evict 마다 차감해 항상 items 합과 일치한다.
    pub fn push(&mut self, item: StoredOutput) {
        self.total_bytes += item.cost_bytes;
        self.items.push_back(item);
        // ★최신 1건 보존 불변식(len() > 1 가드)★: 방금 push 한 단일 이벤트의 cost_bytes 가
        // max_bytes(2MB)를 홀로 초과하면(예: 큰 args_json/Structured.json), len() > 1 가드가 없을 때
        // eviction 루프가 그 최신 이벤트까지 pop_front 로 빼내 버퍼가 비어 버린다 → 늦게 붙는 구독자가
        // **최신 seq 를 통째로 놓친다**. 그래서 오래된 것만 evict 하고 마지막 1건은 항상 남긴다.
        // 트레이드오프: 단일 이벤트가 예산을 넘으면 메모리는 "가장 큰 단일 이벤트 크기"까지 초과할 수
        // 있으나(byte 상한 일시 위반), replay 정합(늦은 구독자가 늘 최신을 본다) > 엄격 byte 상한.
        while self.items.len() > 1
            && (self.total_bytes > self.max_bytes || self.items.len() > self.max_events)
        {
            if let Some(oldest) = self.items.pop_front() {
                self.total_bytes -= oldest.cost_bytes;
            } else {
                break;
            }
        }
    }

    /// 뒤에서부터 `TerminalBytes` 만 최대 `max_bytes` 모아 **시간순**으로 돌려준다.
    ///
    /// ★상한은 하드다★: 경계에 걸친 청크는 그 **끝쪽**만 잘라 담으므로 반환 길이가 `max_bytes` 를 넘지
    ///   않는다(청크 하나가 통째로 넘어오면 상한이 상한이 아니게 된다).
    /// ★clone 하지 않는다★: 슬라이스만 모아 마지막에 한 번 이어 붙인다 — 링 전체 복사(`snapshot`)와의
    ///   차이가 이 메서드의 존재 이유다.
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        let mut parts: Vec<&[u8]> = Vec::new();
        let mut total = 0usize;
        for item in self.items.iter().rev() {
            if total >= max_bytes {
                break;
            }
            if let OutputEvent::TerminalBytes(data) = &item.event {
                let take = data.len().min(max_bytes - total);
                parts.push(&data[data.len() - take..]);
                total += take;
            }
        }
        parts.reverse();
        parts.concat()
    }

    /// 현재 버퍼 전체를 clone 해 반환(seq 오름차순). 호출부가 after_seq 필터는 partition_point 로.
    /// 호출부(subscribe)가 lock 밖에서 borrow 하려면 소유 스냅샷이 필요하다(락 보유 시간 최소화).
    pub fn snapshot(&self) -> Vec<StoredOutput> {
        self.items.iter().cloned().collect()
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 받은 출력을 (seq, bytes, is_event)로 순서대로 수집하는 mock OutputSink.
    struct MockSink {
        id: SinkId,
        events: Mutex<Vec<(u64, Vec<u8>, bool)>>,
    }

    impl MockSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                events: Mutex::new(Vec::new()),
            })
        }

        fn seqs(&self) -> Vec<u64> {
            self.events.lock().unwrap().iter().map(|e| e.0).collect()
        }

        fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    impl OutputSink for MockSink {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), crate::types::SinkError> {
            let (bytes, is_event) = match frame.payload {
                OutputPayload::Bytes(b) => (b.to_vec(), false),
                OutputPayload::Event(e) => (format!("{e:?}").into_bytes(), true),
            };
            self.events
                .lock()
                .unwrap()
                .push((frame.seq, bytes, is_event));
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    struct MockStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
        turn_ends: Mutex<Vec<(AgentId, u32)>>,
    }

    impl MockStatusSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
                turn_ends: Mutex::new(Vec::new()),
            })
        }

        fn statuses(&self) -> Vec<AgentStatus> {
            self.statuses.lock().unwrap().clone()
        }

        fn turn_ends(&self) -> Vec<(AgentId, u32)> {
            self.turn_ends.lock().unwrap().clone()
        }
    }

    impl StatusSink for MockStatusSink {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push(status);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            self.turn_ends.lock().unwrap().push((id, epoch));
        }
    }

    use crate::types::AgentInfo;

    fn new_core(status_sink: Arc<dyn StatusSink>) -> OutputCore {
        OutputCore::new(uuid::Uuid::new_v4(), 0, status_sink, TurnWiring::detached())
    }

    // ── ADR-0113: emit → 턴 관측 표 + 턴 종료 push ───────────────────────────────────

    /// 표 + claude 분류자를 꽂은 core(운영 spawn_session 배선과 동형 — 같은 dispatch 로 뽑는다).
    fn core_with_turns(
        status_sink: Arc<dyn StatusSink>,
        epoch: u32,
    ) -> (OutputCore, Arc<TurnObservations>, AgentId) {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            epoch,
            status_sink,
            TurnWiring::new(
                turns.clone(),
                crate::backend::turn_classifier(&AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: AgentOutputFormat::StreamJson,
                }),
            ),
        );
        (core, turns, id)
    }

    fn delta() -> OutputEvent {
        OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        }
    }

    fn message_done() -> OutputEvent {
        OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        }
    }

    #[test]
    fn emit_records_turn_signals_into_the_shared_table() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 7);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 7), "진행 신호 → 턴 중");
        core.emit(message_done());
        assert!(!turns.is_in_turn(id, 7), "종료 신호 → 턴 아님");
    }

    /// 초인종 순간의 표를 읽는 sink — 「표 갱신 → 통지」 순서의 관찰창.
    struct HaltAtDoorbell {
        turns: Arc<TurnObservations>,
        seen: Mutex<Vec<bool>>,
    }
    impl StatusSink for HaltAtDoorbell {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            let halted = self.turns.get(id, epoch).expect("관측됨").last_end_failed;
            self.seen.lock().unwrap().push(halted);
        }
    }

    /// 오류 끝을 싣는 분류자 — 운영 분류기가 아직 오류 끝을 안 내므로 시험이 직접 짓는다.
    fn classify_outcome(event: &OutputEvent) -> Option<TurnSignal> {
        use crate::turn::TurnEndKind;
        use crate::types::TurnOutcome;
        match event {
            OutputEvent::TurnEnd { outcome, .. } => Some(TurnSignal::Ended(match outcome {
                TurnOutcome::Completed => TurnEndKind::Clean,
                TurnOutcome::Failed { .. } => TurnEndKind::Failed,
                _ => TurnEndKind::Other,
            })),
            _ => None,
        }
    }

    // ADR-0231
    #[test]
    fn the_turn_end_doorbell_rings_after_the_halt_is_written() {
        use crate::types::TurnOutcome;
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        turns.register(id, 2);
        let sink = Arc::new(HaltAtDoorbell {
            turns: turns.clone(),
            seen: Mutex::new(Vec::new()),
        });
        let core = OutputCore::new(
            id,
            2,
            sink.clone(),
            TurnWiring::new(turns.clone(), classify_outcome),
        );
        core.emit(OutputEvent::TurnEnd {
            turn_id: None,
            outcome: TurnOutcome::Failed { detail: None },
        });
        core.emit(OutputEvent::TurnEnd {
            turn_id: None,
            outcome: TurnOutcome::Completed,
        });
        assert_eq!(
            *sink.seen.lock().unwrap(),
            vec![true, false],
            "초인종을 받은 쪽이 곧바로 바쁨을 묻는다 — 그때 표가 이미 멈춤·풀림을 보여야 한다"
        );
    }

    type Park = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
    /// 오류 줄을 분류하는 순간 그 스레드를 세워 두는 자리 — seq 는 발급됐고 표에는 아직 안 적힌 틈이다(시험 전용).
    static PARK_ERROR: Mutex<Option<Park>> = Mutex::new(None);

    /// 오류 줄 = `Failed`(붙잡힘) · 입력 에코 = 진행 · 턴 끝 = 깨끗한 끝.
    fn classify_parking_error(event: &OutputEvent) -> Option<TurnSignal> {
        use crate::turn::TurnEndKind;
        match event {
            OutputEvent::Error(_) => {
                let parked = PARK_ERROR.lock().unwrap().take();
                if let Some((entered, release)) = parked {
                    entered.send(()).expect("entered");
                    release
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release");
                }
                Some(TurnSignal::Failed)
            }
            OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
            OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended(TurnEndKind::Clean)),
            _ => None,
        }
    }

    /// ★멈춤 칸의 커서 회귀(실 emit 경로)★: pump 의 오류 줄(seq N)이 표에 적히기 전에 주입 스레드의 진행
    /// (N+1)이 먼저 적힌다 — 진행 커서로 거르면 그 오류가 버려져 턴 끝(N+2)이 깨끗하게 접힌다.
    // ADR-0231
    #[test]
    fn an_error_written_behind_a_newer_progress_still_halts_at_the_turn_end() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *PARK_ERROR.lock().unwrap() = Some((entered_tx, release_rx));
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        turns.register(id, 4);
        let core = Arc::new(OutputCore::new(
            id,
            4,
            MockStatusSink::new(),
            TurnWiring::new(turns.clone(), classify_parking_error),
        ));

        // 출력 pump — 오류 줄(분류에서 붙잡힘) 뒤에 턴 끝 줄.
        let pump = {
            let core = core.clone();
            std::thread::spawn(move || {
                core.emit(OutputEvent::Error("boom".into()));
                core.emit(message_done());
            })
        };
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("오류 줄의 seq 가 발급된 채 붙잡혔다");
        // 주입 스레드의 입력 에코.
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        release_tx.send(()).expect("release");
        pump.join().expect("pump");

        let order: Vec<&str> = core
            .replay
            .lock()
            .unwrap()
            .snapshot()
            .iter()
            .map(|s| match s.event {
                OutputEvent::Error(_) => "Error",
                OutputEvent::Structured { .. } => "Structured",
                OutputEvent::MessageDone { .. } => "MessageDone",
                _ => "other",
            })
            .collect();
        assert_eq!(
            order,
            ["Error", "Structured", "MessageDone"],
            "링 순서 전제"
        );
        let o = turns.get(id, 4).expect("관측됨");
        assert!(
            o.last_end_failed,
            "링 순서상 오류로 끝난 턴이 깨끗하게 접혔다 — 뒤 seq 의 진행에 밀려 오류가 버려졌다"
        );
        assert!(!o.in_turn, "턴 끝이 가장 새 신호다");
    }

    /// ★진행 신호를 적지 않는 문★ — 같은 이벤트가 화면(fanout)·replay 로는 그대로 가되 사실
    /// 계층에는 아무 것도 안 남는다. 이 성질이 깨지면 미귀속 줄이 턴 중 표시를 켜 놓고 그 종료는 귀속
    /// 게이트에 막혀, 30 분 fail-open 밸브까지 우편이 막힌다(ADR-0127 이 닫은 그 기전).
    #[test]
    fn the_unobserved_door_fans_out_and_replays_but_writes_no_turn_fact() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 3);
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit_without_turn_observation(delta());
        assert_eq!(turns.get(id, 3), None, "미귀속 줄이 사실 계층을 켰다");
        assert_eq!(sink.len(), 1, "화면으로는 나가야 한다");
        assert_eq!(
            core.replay
                .lock()
                .expect("replay poisoned")
                .snapshot()
                .len(),
            1,
            "replay 에도 남아야 한다"
        );

        // 같은 core 의 보통 문은 그대로 적는다 — 문이 갈린 것이지 표가 꺼진 것이 아니다.
        core.emit(delta());
        assert!(turns.is_in_turn(id, 3));
    }

    #[test]
    fn emit_of_non_turn_events_leaves_the_table_untouched() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));
        core.emit(OutputEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            turn_id: None,
        });
        core.emit(OutputEvent::Error("stream hiccup".into()));
        assert_eq!(
            turns.get(id, 0),
            None,
            "턴 신호가 아니면 항목을 만들지 않는다"
        );
    }

    #[test]
    fn every_message_done_pushes_a_turn_end_notification() {
        let sink = MockStatusSink::new();
        let (core, _turns, id) = core_with_turns(sink.clone(), 2);
        core.emit(delta());
        core.emit(message_done());
        core.emit(message_done());
        assert_eq!(sink.turn_ends(), vec![(id, 2), (id, 2)]);
    }

    #[test]
    fn progress_events_do_not_push_turn_end() {
        let sink = MockStatusSink::new();
        let (core, _turns, _id) = core_with_turns(sink.clone(), 0);
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(sink.turn_ends().is_empty());
    }

    #[test]
    fn a_late_echo_after_finish_cannot_resurrect_the_entry() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0), "전제: 턴 중으로 관측됨");

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(turns.get(id, 0), None, "finish 가 자기 항목을 거둔다");

        // 막혀 있던 주입 스레드가 이제야 에코를 낸다(같은 epoch).
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert_eq!(
            turns.get(id, 0),
            None,
            "종료된 화신은 항목을 되살리지 못한다"
        );
        assert!(turns.in_turn_snapshot().is_empty());
    }

    #[test]
    fn a_dead_incarnations_late_echo_cannot_delete_the_live_ones_observation() {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let classify = crate::backend::turn_classifier(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        let wiring = |t: &Arc<TurnObservations>| TurnWiring::new(t.clone(), classify);

        // 앞선 화신(epoch 0) — 한동안 돌다 종료한다.
        let dead = OutputCore::new(id, 0, MockStatusSink::new(), wiring(&turns));
        for _ in 0..5 {
            dead.emit(delta());
        }
        dead.finish(TerminalReason::Killed);
        assert_eq!(turns.get(id, 0), None, "종료가 자기 항목을 거둔다(전제)");

        // 새 화신(epoch 1) — 같은 AgentId, 새 core 라 seq 는 0 부터 다시 센다.
        let live = OutputCore::new(id, 1, MockStatusSink::new(), wiring(&turns));
        live.emit(delta());
        assert!(turns.is_in_turn(id, 1), "산 화신이 턴 중으로 관측됨(전제)");

        // 죽은 화신의 주입 스레드가 이제야 깨어 에코를 낸다 — seq 는 산 화신의 것보다 **크다**.
        dead.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(
            turns.is_in_turn(id, 1),
            "죽은 화신의 지각 emit 이 산 화신의 관측을 덮거나 지우면 안 된다"
        );
        assert_eq!(
            turns.get(id, 0),
            None,
            "죽은 화신의 항목이 되살아나지도 않는다"
        );
    }

    #[test]
    fn a_backend_without_a_turn_mapping_never_records_a_signal() {
        use crate::profile::AgentCommand;
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            0,
            MockStatusSink::new(),
            TurnWiring::new(
                turns.clone(),
                crate::backend::turn_classifier(&AgentCommand::Shell {
                    program: "cmd.exe".into(),
                    args: vec![],
                }),
            ),
        );
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        });
        core.emit(message_done());
        assert_eq!(turns.get(id, 0), None, "분류자가 침묵하면 관측도 없다");
    }

    #[test]
    fn an_unwired_core_records_nothing_at_all() {
        let sink = MockStatusSink::new();
        let core = new_core(sink.clone());
        core.emit(delta());
        core.emit(message_done());
        assert!(sink.turn_ends().is_empty(), "배선 없으면 통지도 없다");
    }

    #[test]
    fn seed_never_bootstraps_turn_observation_from_a_transcript() {
        let sink = MockStatusSink::new();
        let (core, turns, id) = core_with_turns(sink.clone(), 0);
        core.seed(vec![delta(), message_done(), delta()]);
        assert_eq!(turns.get(id, 0), None, "seed 는 관측이 아니다");
        assert!(sink.turn_ends().is_empty(), "seed 는 통지도 내지 않는다");
        // 라이브 emit 부터 관측이 시작된다(seed 가 관측을 막아 버리는 것도 아니다).
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0));
    }

    /// 덩이 문의 기본 계약 — 링에도 들어가고, 붙어 있는 구독자에게도 **순서대로** 나가며, seq 가 이어진다.
    #[test]
    fn a_batch_lands_in_the_ring_and_fans_out_in_order() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit_batch_without_turn_observation(vec![
            OutputEvent::TerminalBytes(b"one".to_vec()),
            OutputEvent::TerminalBytes(b"two".to_vec()),
            OutputEvent::TerminalBytes(b"three".to_vec()),
        ]);

        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        // 늦게 붙는 구독자도 링에서 같은 셋을 받는다.
        let late = MockSink::new();
        core.subscribe(late.clone());
        assert_eq!(late.seqs(), vec![0, 1, 2]);
    }

    /// 빈 덩이는 seq 를 쓰지 않는다 — 안 그러면 복원할 것이 없는 세션마다 번호가 하나씩 밀린다.
    #[test]
    fn an_empty_batch_consumes_no_seq() {
        let core = new_core(MockStatusSink::new());
        core.emit_batch_without_turn_observation(Vec::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());
        core.emit(OutputEvent::TerminalBytes(b"first".to_vec()));
        assert_eq!(sink.seqs(), vec![0]);
    }

    /// 덩이 문도 낱개 문과 같이 **관측을 적지 않는다** — 지나간 기록이 새 화신을 「턴 중」으로 만들면
    /// 그 턴의 종료가 영영 오지 않는다(ADR-0113/0127 · `seed` 가 지키던 그 규율).
    #[test]
    fn a_batch_never_bootstraps_turn_observation() {
        let sink = MockStatusSink::new();
        let (core, turns, id) = core_with_turns(sink.clone(), 0);
        core.emit_batch_without_turn_observation(vec![delta(), message_done(), delta()]);
        assert_eq!(turns.get(id, 0), None, "덩이 문이 관측을 적었다");
        assert!(sink.turn_ends().is_empty(), "덩이 문이 통지를 냈다");
    }

    /// ★★덩이 **안으로** 다른 emit 이 끼어들 수 없다★★ — 이것이 이 문의 존재 이유다(ADR-0203).
    /// 낱개로 부르는 구현이면 경쟁 스레드가 중간 seq 를 가져가 복원된 대화가 쪼개진다.
    #[test]
    fn nothing_can_interleave_inside_a_batch() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let core = Arc::new(new_core(MockStatusSink::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let racer = {
            let core = core.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    core.emit(OutputEvent::TerminalBytes(b"live".to_vec()));
                }
            })
        };

        let batch: Vec<OutputEvent> = (0..200)
            .map(|i| OutputEvent::Structured {
                kind: "history".to_string(),
                json: format!("{i}"),
            })
            .collect();
        core.emit_batch_without_turn_observation(batch);
        stop.store(true, Ordering::Relaxed);
        racer.join().unwrap();

        // 링에서 덩이 항목들의 seq 를 뽑아 **연속**인지 본다.
        let stored = core.replay.lock().unwrap().snapshot();
        let seqs: Vec<u64> = stored
            .iter()
            .filter(
                |c| matches!(&c.event, OutputEvent::Structured { kind, .. } if kind == "history"),
            )
            .map(|c| c.seq)
            .collect();
        assert_eq!(seqs.len(), 200, "덩이가 링에서 잘렸다");
        for pair in seqs.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "덩이 한가운데에 다른 줄이 끼어들었다: {seqs:?}"
            );
        }
    }

    #[test]
    fn emit_increments_seq_and_fans_out() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit(OutputEvent::TerminalBytes(b"hello".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"world".to_vec()));

        assert_eq!(sink.seqs(), vec![0, 1]);
        assert_eq!(sink.len(), 2);
        {
            let ev = sink.events.lock().unwrap();
            assert_eq!(ev[0].1, b"hello");
            assert_eq!(ev[1].1, b"world");
            assert!(!ev[0].2, "TerminalBytes 는 OutputPayload::Bytes 로 와야 함");
            assert!(!ev[1].2);
        }
        let snap = core.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].seq, 0);
        assert_eq!(snap[1].seq, 1);
        assert_eq!(snap[0].data, b"hello");
        assert_eq!(snap[1].data, b"world");
    }

    #[test]
    fn emit_structured_event_fans_out_as_event_and_replays() {
        let core = new_core(MockStatusSink::new());

        let live = MockSink::new();
        core.subscribe(live.clone());
        core.emit(OutputEvent::TextDelta {
            text: "hi".into(),
            turn_id: None,
            message_id: None,
        });
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));

        {
            let ev = live.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert!(
                ev[0].2,
                "구조화 이벤트는 OutputPayload::Event 로 fanout 돼야 함"
            );
            assert!(
                String::from_utf8_lossy(&ev[0].1).contains("TextDelta"),
                "Event payload 가 해당 이벤트를 담아야 함"
            );
            assert!(!ev[1].2, "TerminalBytes 는 Bytes 로 fanout");
            assert_eq!(ev[1].1, b"raw");
        }

        let late = MockSink::new();
        core.subscribe(late.clone());
        {
            let ev = late.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert_eq!(ev[0].0, 0);
            assert!(ev[0].2, "replay 된 구조화 이벤트도 Event payload");
            assert_eq!(ev[1].0, 1);
            assert!(!ev[1].2);
            assert_eq!(ev[1].1, b"raw");
        }
    }

    #[test]
    fn seed_pushes_to_ring_in_order_without_fanout_then_live_seq_continues() {
        let core = new_core(MockStatusSink::new());

        // (1) 구독자 없는 상태에서 과거 이벤트 3건 seed(=resume 시 .jsonl 복원분 흉내).
        core.seed(vec![
            OutputEvent::Structured {
                kind: "user".into(),
                json: r#"{"type":"text","text":"과거 질문"}"#.into(),
            },
            OutputEvent::TextDelta {
                text: "과거 답변".into(),
                turn_id: None,
                message_id: None,
            },
            OutputEvent::MessageDone {
                turn_id: None,
                message_id: None,
            },
        ]);

        // Ring 에 seq 0,1,2 로 순서대로 적재됐는지(snapshot 은 TerminalBytes 만 변환하므로 seq 검증은
        // 아래 늦은 구독자 replay 로 한다 — snapshot() 은 구조화 이벤트를 스킵).
        let late = MockSink::new();
        core.subscribe(late.clone());
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2],
            "seed 한 과거 3건이 Ring 에 seq 0,1,2 로 순서대로 있어야 하고 replay 로 전달됨"
        );

        core.emit(OutputEvent::TextDelta {
            text: "새 라이브 토큰".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2, 3],
            "seed 뒤 라이브 emit 의 seq 가 seed 마지막+1 로 이어져야 함(gap·중복 없음)"
        );
    }

    #[test]
    fn seed_does_not_fanout_to_existing_subscriber() {
        let core = new_core(MockStatusSink::new());
        // (비정상 순서지만 방어 검증용) 구독자를 먼저 붙인 뒤 seed 한다.
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.seed(vec![OutputEvent::TextDelta {
            text: "seed".into(),
            turn_id: None,
            message_id: None,
        }]);

        assert_eq!(sink.len(), 0, "seed 는 기존 구독자로 fanout 하지 않아야 함");

        core.emit(OutputEvent::TextDelta {
            text: "live".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            sink.seqs(),
            vec![1],
            "라이브 emit 만 fanout, seq 는 seed 다음"
        );
    }

    #[test]
    fn subscribe_replays_then_lives_without_seq_gap() {
        let core = new_core(MockStatusSink::new());

        // 구독 전에 2건 emit → replay에만 쌓임.
        core.emit(OutputEvent::TerminalBytes(b"a".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"b".to_vec()));

        let sink = MockSink::new();
        core.subscribe(sink.clone());
        assert_eq!(sink.seqs(), vec![0, 1]);

        core.emit(OutputEvent::TerminalBytes(b"c".to_vec()));
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
    }

    // ── S15 B4 Ring 단위테스트(격리) ──────────────────────────────────────────────

    fn stored(seq: u64, event: OutputEvent) -> StoredOutput {
        let cost_bytes = estimate_cost_bytes(&event);
        StoredOutput {
            seq,
            event,
            cost_bytes,
        }
    }

    // ── ADR-0172: 실패 분류가 읽는 꼬리(상한이 하드인가 · 콘솔 바이트만인가) ────────────────

    #[test]
    fn terminal_tail_returns_the_last_console_bytes_in_order() {
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"first ".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"second".to_vec())));
        assert_eq!(ring.terminal_tail(64), b"first second".to_vec());
    }

    #[test]
    fn terminal_tail_is_a_hard_bound_even_when_one_chunk_exceeds_it() {
        // ★청크를 통째로 담으면 상한이 상한이 아니다★ — 경계에 걸친 청크는 끝쪽만 잘라야 한다.
        let mut ring = Ring::new();
        ring.push(stored(
            0,
            OutputEvent::TerminalBytes(b"xxxxxxxxxx".to_vec()),
        ));
        let tail = ring.terminal_tail(4);
        assert_eq!(tail.len(), 4);
        assert_eq!(tail, b"xxxx".to_vec());

        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"OLD".to_vec())));
        ring.push(stored(
            1,
            OutputEvent::TerminalBytes(b"0123456789".to_vec()),
        ));
        assert_eq!(
            ring.terminal_tail(4),
            b"6789".to_vec(),
            "뒤에서부터 채우므로 남는 건 가장 최근 바이트다"
        );
    }

    #[test]
    fn terminal_tail_skips_structured_events_and_can_be_empty() {
        // 구조화(NDJSON) 세션의 링에는 콘솔 바이트가 하나도 없다 — 빈 값이 정상 결과다.
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::Error("boom".into())));
        ring.push(stored(
            1,
            OutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: None,
                message_id: None,
            },
        ));
        assert!(ring.terminal_tail(64).is_empty());

        ring.push(stored(2, OutputEvent::TerminalBytes(b"raw".to_vec())));
        assert_eq!(
            ring.terminal_tail(64),
            b"raw".to_vec(),
            "사이에 낀 구조화 이벤트는 건너뛰고 콘솔 바이트만 잇는다"
        );
    }

    #[test]
    fn ring_push_snapshot_preserves_order_and_seq() {
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"a".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"bb".to_vec())));
        ring.push(stored(2, OutputEvent::Error("boom".into())));

        let snap = ring.snapshot();
        assert_eq!(
            snap.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(&snap[0].event, OutputEvent::TerminalBytes(v) if v == b"a"));
        assert!(matches!(&snap[2].event, OutputEvent::Error(s) if s == "boom"));
    }

    #[test]
    fn ring_evicts_on_byte_budget_independent_of_count() {
        let mut ring = Ring::new();
        // 각 ~1MB args_json 이벤트 3건 → cost 합 ~3MB > 2MB → 가장 오래된 것 evict.
        let big = "x".repeat(1024 * 1024);
        ring.push(stored(
            0,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        ring.push(stored(
            1,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        ring.push(stored(
            2,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        let snap = ring.snapshot();
        assert!(
            snap.len() < 3,
            "cost_bytes 합이 2MB 초과 시 건수 상한과 무관하게 evict 돼야 함(len={})",
            snap.len()
        );
        assert_eq!(snap.last().unwrap().seq, 2);
    }

    #[test]
    fn ring_evicts_on_event_count_cap() {
        // 작은 이벤트 5000건 → byte 예산엔 한참 못 미치지만 건수 상한(4096)에 걸려 cap.
        let mut ring = Ring::new();
        for seq in 0..5000u64 {
            ring.push(stored(seq, OutputEvent::TerminalBytes(vec![b'x'])));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 4096);
        // 가장 오래된 것부터 evict → 남은 첫 seq = 5000-4096 = 904.
        assert_eq!(snap.first().unwrap().seq, 904);
        assert_eq!(snap.last().unwrap().seq, 4999);
    }

    #[test]
    fn ring_preserves_latest_when_single_event_exceeds_byte_budget() {
        // FIX-A: 방금 push 한 단일 이벤트의 cost_bytes 가 max_bytes(2MB)를 홀로 초과해도,
        // 최신 1건 보존 불변식(len() > 1 가드)에 의해 그 이벤트는 replay 버퍼에 남아야 한다.
        let mut ring = Ring::new();
        let huge = "y".repeat(3 * 1024 * 1024); // > 2MB (max_bytes)
        ring.push(stored(
            42,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "단일 초과 이벤트라도 최신 1건은 보존돼야 함");
        assert_eq!(snap[0].seq, 42, "보존된 이벤트는 방금 push 한 최신 seq");
    }

    #[test]
    fn ring_evicts_only_old_when_latest_exceeds_budget() {
        // FIX-A: 오래된 작은 이벤트들 + 예산을 홀로 초과하는 큰 최신 이벤트.
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"old0".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"old1".to_vec())));
        let huge = "z".repeat(3 * 1024 * 1024); // > 2MB → 이것만으로 예산 초과
        ring.push(stored(
            2,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "오래된 것만 빠지고 최신 1건 남아야 함");
        assert_eq!(snap[0].seq, 2, "남은 것은 최신 이벤트");
    }

    #[test]
    fn ring_cost_bytes_reflects_terminal_and_structured() {
        // TerminalBytes → v.len().
        assert_eq!(
            estimate_cost_bytes(&OutputEvent::TerminalBytes(vec![0u8; 100])),
            100
        );
        // 구조화(ToolCall) → name + args_json + optional 문자열 합.
        let cost = estimate_cost_bytes(&OutputEvent::ToolCall {
            name: "read".into(),           // 4
            args_json: "{\"p\":1}".into(), // 7
            id: Some("abc".into()),        // 3
            turn_id: None,
            message_id: None,
        });
        assert_eq!(cost, 4 + 7 + 3);
        // TextDelta → text + optional.
        let cost2 = estimate_cost_bytes(&OutputEvent::TextDelta {
            text: "hello".into(),       // 5
            turn_id: Some("t1".into()), // 2
            message_id: None,
        });
        assert_eq!(cost2, 5 + 2);
    }

    /// 명부 사건은 본문(글 · 말풍선 사본)을 링 무게로 센다 — 긴 글이 「건수 1」로 링 상한을 우회하지 않게.
    #[test]
    fn ring_cost_bytes_counts_queued_text_and_ack_copies() {
        use crate::types::{DeliveredCopy, DropCause};
        let queued = |ev| estimate_cost_bytes(&OutputEvent::QueuedInput(ev));
        assert_eq!(
            queued(QueuedInputEvent::Queued {
                id: "u1".into(),      // 2
                text: "hello".into(), // 5
            }),
            2 + 5
        );
        assert_eq!(
            queued(QueuedInputEvent::AckUnavailable {
                delivered: vec![
                    DeliveredCopy {
                        id: "a".into(),     // 1
                        text: "xyz".into(), // 3
                    },
                    DeliveredCopy {
                        id: "bb".into(),     // 2
                        text: "wxyz".into(), // 4
                    },
                ],
            }),
            1 + 3 + 2 + 4
        );
        assert_eq!(
            queued(QueuedInputEvent::Dropped {
                id: "u12".into(),
                cause: DropCause::Rejected,
            }),
            3
        );
    }

    #[test]
    fn finish_finalizes_exactly_once() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        core.finish(TerminalReason::Killed);
        core.finish(TerminalReason::Killed);

        let statuses = status_sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], AgentStatus::Killed));
        assert!(matches!(core.status(), AgentStatus::Killed));
    }

    /// ADR-0019 reaper hook(on_terminal) 1회 보장: finalize 승자 경로(finalized.swap 통과)에서
    /// hook 이 정확히 1회 호출되고, 중복 finish(swap 패자)에서는 0회임을 단언한다.
    #[test]
    fn on_terminal_hook_fires_exactly_once() {
        let core = new_core(MockStatusSink::new());
        let calls = Arc::new(AtomicU64::new(0));

        let c = calls.clone();
        core.set_on_terminal(Box::new(move |_reason: TerminalReason| {
            c.fetch_add(1, Ordering::SeqCst);
        }));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "finalize 승자 경로에서 on_terminal hook 이 정확히 1회 호출돼야 함"
        );

        core.finish(TerminalReason::Killed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "중복 finish(finalize 패자)에서 on_terminal hook 이 다시 호출됨(1회 위반)"
        );
    }

    #[test]
    fn subscribe_from_resume_sends_only_tail() {
        let core = new_core(MockStatusSink::new());
        for i in 0..5u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});

        assert_eq!(sink.seqs(), vec![3, 4]);
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 2);
        assert_eq!(out.replay_from, 3);
    }

    #[test]
    fn subscribe_from_truncated_when_after_below_oldest() {
        let core = new_core(MockStatusSink::new());
        // 1바이트 청크 5000개 emit → event 상한(4096) 초과로 oldest=904 까지 evict.
        for _ in 0..5000u64 {
            core.emit(OutputEvent::TerminalBytes(vec![b'x']));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(10), true, |_| {});

        assert_eq!(out.kind, ReplayKind::Truncated);
        assert_eq!(out.oldest_seq, 904);
        assert_eq!(sink.seqs().first().copied(), Some(904));
        assert_eq!(out.replay_from, 904);
    }

    #[test]
    fn subscribe_from_epoch_mismatch_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(1), false, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    #[test]
    fn subscribe_from_caught_up_sends_nothing() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        // after_seq=2(=latest) → 보낼 tail 없음.
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 0);
        assert_eq!(out.replay_from, 3);
        assert_eq!(sink.len(), 0);

        core.emit(OutputEvent::TerminalBytes(b"d".to_vec()));
        assert_eq!(sink.seqs(), vec![3]);
    }

    #[test]
    fn subscribe_from_none_after_seq_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), None, true, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    /// M-A fix 검증: on_ready 콜백이 (1) replay 전송 **전**에, (2) 정확히 1회 호출되고,
    /// (3) 콜백이 받는 outcome 이 반환 outcome 과 동일(단일 스냅샷 기준)임을 확인.
    #[test]
    fn subscribe_from_calls_on_ready_before_replay() {
        struct OrderSink {
            id: SinkId,
            replay_started: Arc<AtomicBool>,
        }
        impl OutputSink for OrderSink {
            fn send(&self, _frame: OutputFrame<'_>) -> Result<(), crate::types::SinkError> {
                self.replay_started.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }

        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }

        let replay_started = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(OrderSink {
            id: uuid::Uuid::new_v4(),
            replay_started: replay_started.clone(),
        });

        let call_count = Arc::new(AtomicU64::new(0));
        // 콜백이 본 outcome 을 캡처해 반환 outcome 과 비교(SubscribeOutcome 는 Copy).
        let seen: Arc<Mutex<Option<SubscribeOutcome>>> = Arc::new(Mutex::new(None));

        let cc = call_count.clone();
        let started = replay_started.clone();
        let seen_cb = seen.clone();
        let out = core.subscribe_from(sink, Some(1), true, move |outcome| {
            assert!(
                !started.load(Ordering::SeqCst),
                "on_ready 는 replay 전송 전에 호출돼야 함"
            );
            cc.fetch_add(1, Ordering::SeqCst);
            *seen_cb.lock().unwrap() = Some(*outcome);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        // replay 가 실제로 전송됐는지(after_seq=1 → seq 2 전송) → started true.
        assert!(replay_started.load(Ordering::SeqCst));
        let seen = seen.lock().unwrap().expect("콜백이 호출됨");
        assert_eq!(seen.kind, out.kind);
        assert_eq!(seen.oldest_seq, out.oldest_seq);
        assert_eq!(seen.latest_seq, out.latest_seq);
        assert_eq!(seen.replay_from, out.replay_from);
        assert_eq!(out.kind, ReplayKind::Resumed);
    }

    #[test]
    fn enter_exiting_true_when_running_false_after_terminal() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        assert!(core.enter_exiting());
        assert!(matches!(core.status(), AgentStatus::Exiting));
        assert!(matches!(
            status_sink.statuses().last().unwrap(),
            AgentStatus::Exiting
        ));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert!(!core.enter_exiting());
    }

    /// ADR-0079 회귀 방지: 동시 emit 하에서도 replay ring 이 seq 오름차순(단조)을 유지하는지.
    ///
    /// ★왜 이 테스트가 성립하나(hermetic)★: pump 스레드와 write_input synthetic echo 가 동시에 emit 을
    ///   부르는 실제 상황을 여러 스레드의 emit 루프로 재현한다. 확률적이지만 스레드×반복이 크면
    ///   발급/push 역전 창을 거의 확실히 밟아 회귀를 잡는다(플래키하지 않게 반복 수를 넉넉히 잡음).
    #[test]
    fn concurrent_emit_keeps_replay_ring_monotonic() {
        let core = Arc::new(new_core(MockStatusSink::new()));
        const THREADS: u64 = 4;
        const PER_THREAD: u64 = 500; // 4*500 = 2000 < max_events(4096) → eviction 없음.

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let core = core.clone();
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        // 작은 payload — 2MB max_bytes 도 안 건드림. emit 이 유일한 seq 소비자.
                        core.emit(OutputEvent::TerminalBytes(vec![b'x']));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("emit thread panicked");
        }

        let stored = core.replay.lock().expect("replay poisoned").snapshot();
        let seqs: Vec<u64> = stored.iter().map(|s| s.seq).collect();
        assert_eq!(
            seqs.len() as u64,
            THREADS * PER_THREAD,
            "eviction 없이 전량 저장돼야(검사 완전성)"
        );
        assert!(
            seqs.windows(2).all(|w| w[0] < w[1]),
            "replay ring 은 seq 로 엄격 오름차순이어야 한다(동시 emit 원자성): {seqs:?}"
        );
    }
}

// ── ADR-0231 대기 입력 명부 — 코어 관찰(명부 환원 · 종료 합성·봉인 · 바꿔 적기 · 대기 목록 표 순서) ──────────
#[cfg(test)]
mod queued_tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::mpsc;

    use crate::queued_input::{CancelAnswer, QueuedRow, Registry, RowPhase};
    use crate::turn::TurnEndKind;
    use crate::types::{AgentInfo, DeliveredCopy, SinkError};

    const EPOCH: u32 = 5;
    /// 시험 스레드가 서로를 기다리는 상한 — 넘으면 매달리지 않고 실패한다.
    const WAIT: Duration = Duration::from_secs(5);

    fn queued(id: &str, text: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        })
    }
    fn delivered(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id: id.into() })
    }
    fn dropped(id: &str, cause: DropCause) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Dropped {
            id: id.into(),
            cause,
        })
    }
    fn cancel_requested(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::CancelRequested { id: id.into() })
    }
    fn cancel_answered(id: &str, removed: bool) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::CancelAnswered {
            id: id.into(),
            removed,
        })
    }
    fn ack_unavailable() -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
            delivered: Vec::new(),
        })
    }
    fn copy(id: &str, text: &str) -> DeliveredCopy {
        DeliveredCopy {
            id: id.into(),
            text: text.into(),
        }
    }
    fn ended(id: &str) -> QueuedInputEvent {
        QueuedInputEvent::Dropped {
            id: id.into(),
            cause: DropCause::AgentEnded,
        }
    }
    fn delta() -> OutputEvent {
        OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        }
    }

    struct Quiet;
    impl StatusSink for Quiet {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    /// 초인종 두 개가 울리는 순간의 두 표를 적는다 — 「표 갱신 → 통지」 · 「턴 관측 → 비었다」 순서의 관찰창.
    struct DoorbellProbe {
        id: AgentId,
        pending: Arc<InputsPendingTable>,
        turns: Arc<TurnObservations>,
        /// 턴 끝 초인종 순간의 대기 목록 표.
        at_turn_end: Mutex<Vec<Option<bool>>>,
        /// 비었다 초인종 순간의 (대기 목록 표, 턴 중).
        at_drain: Mutex<Vec<(Option<bool>, bool)>>,
    }
    impl DoorbellProbe {
        fn drains(&self) -> Vec<(Option<bool>, bool)> {
            self.at_drain.lock().unwrap().clone()
        }
    }
    impl StatusSink for DoorbellProbe {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            assert_eq!((id, epoch), (self.id, EPOCH));
            let pending = self.pending.get(id, epoch);
            self.at_turn_end.lock().unwrap().push(pending);
        }
        fn inputs_drained(&self, id: AgentId, epoch: u32) {
            assert_eq!((id, epoch), (self.id, EPOCH));
            let pending = self.pending.get(id, epoch);
            let in_turn = self.turns.is_in_turn(id, epoch);
            self.at_drain.lock().unwrap().push((pending, in_turn));
        }
    }

    /// 받은 프레임의 seq 와 목록 사건을 적는 sink.
    struct Recorder {
        id: SinkId,
        got: Mutex<Vec<(u64, Option<QueuedInputEvent>)>>,
    }
    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                got: Mutex::new(Vec::new()),
            })
        }
        fn seqs(&self) -> Vec<u64> {
            self.got.lock().unwrap().iter().map(|(s, _)| *s).collect()
        }
    }
    impl OutputSink for Recorder {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
            let op = match frame.payload {
                OutputPayload::Event(OutputEvent::QueuedInput(op)) => Some(op.clone()),
                _ => None,
            };
            self.got.lock().unwrap().push((frame.seq, op));
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn claude_classifier() -> TurnClassifier {
        use crate::profile::{AgentCommand, AgentOutputFormat};
        crate::backend::turn_classifier(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        })
    }

    /// 목록을 비우는 받음이 턴 끝이기도 한 분류자 — 턴 끝 초인종이 「비었다」보다 앞에 오는지 가르려고 짓는다.
    fn delivered_ends_the_turn(event: &OutputEvent) -> Option<TurnSignal> {
        matches!(
            event,
            OutputEvent::QueuedInput(QueuedInputEvent::Delivered { .. })
        )
        .then_some(TurnSignal::Ended(TurnEndKind::Clean))
    }

    /// 한 화신의 두 표 — 코어는 운영 spawn 과 같은 모양으로 짓는다(두 표 등록 + 새 명부).
    struct Fixture {
        id: AgentId,
        turns: Arc<TurnObservations>,
        pending: Arc<InputsPendingTable>,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                id: uuid::Uuid::new_v4(),
                turns: Arc::new(TurnObservations::new()),
                pending: Arc::new(InputsPendingTable::new()),
            }
        }
        fn core(
            &self,
            status_sink: Arc<dyn StatusSink>,
            classify: TurnClassifier,
        ) -> Arc<OutputCore> {
            self.turns.register(self.id, EPOCH);
            self.pending.register(self.id, EPOCH);
            Arc::new(
                OutputCore::new(
                    self.id,
                    EPOCH,
                    status_sink,
                    TurnWiring::new(self.turns.clone(), classify),
                )
                .with_queued(QueuedWiring {
                    registry: Arc::new(QueuedInputs::new()),
                    pending: self.pending.clone(),
                }),
            )
        }
        fn probe(&self) -> Arc<DoorbellProbe> {
            Arc::new(DoorbellProbe {
                id: self.id,
                pending: self.pending.clone(),
                turns: self.turns.clone(),
                at_turn_end: Mutex::new(Vec::new()),
                at_drain: Mutex::new(Vec::new()),
            })
        }
        fn pending(&self) -> Option<bool> {
            self.pending.get(self.id, EPOCH)
        }
    }

    fn ring(core: &OutputCore) -> Vec<StoredOutput> {
        core.replay.lock().unwrap().snapshot()
    }

    /// 링의 목록 사건만(seq 순).
    fn ring_ops(core: &OutputCore) -> Vec<(u64, QueuedInputEvent)> {
        ring(core)
            .into_iter()
            .filter_map(|s| match s.event {
                OutputEvent::QueuedInput(op) => Some((s.seq, op)),
                _ => None,
            })
            .collect()
    }

    fn rows(core: &OutputCore) -> Vec<QueuedRow> {
        core.queued_inputs().snapshot().0
    }

    // ── 명부 = 링 접두의 환원값 ────────────────────────────────────────────────────────────────

    /// 두 스레드가 목록 사건을 섞어 내는 동안 셋째가 명부 스냅숏을 뜬다 — 뜬 (행, S) 는 링 접두 `seq <= S` 를
    /// 새 환원기에 먹인 결과와 같아야 한다. 명부를 replay 락 밖에서 먹이면 두 스레드의 환원 순서가 링 순서와
    /// 갈려 이 대조가 깨진다.
    #[test]
    fn the_registry_snapshot_is_the_reduction_of_the_ring_prefix_it_names() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let stop = Arc::new(AtomicBool::new(false));

        let reader = {
            let (core, stop) = (core.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut samples = Vec::new();
                while !stop.load(Ordering::Acquire) {
                    samples.push(core.queued_inputs().snapshot());
                    std::thread::yield_now();
                }
                samples
            })
        };
        let emitters: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|tag| {
                let core = core.clone();
                std::thread::spawn(move || {
                    for i in 0..150 {
                        let id = format!("{tag}{i}");
                        core.emit(queued(&id, "t"));
                        core.emit(delta());
                        match i % 4 {
                            0 => core.emit(delivered(&id)),
                            1 => core.emit(cancel_requested(&id)),
                            2 => core.emit(dropped(&format!("{tag}{}", i - 1), DropCause::Unknown)),
                            _ => {}
                        }
                    }
                })
            })
            .collect();
        for e in emitters {
            e.join().expect("emitter");
        }
        stop.store(true, Ordering::Release);
        let mut samples = reader.join().expect("reader");
        samples.push(core.queued_inputs().snapshot());

        let ops = ring_ops(&core);
        assert!(
            ring(&core).len() < REPLAY_MAX_EVENTS,
            "축출 없이 전량 남아야 대조가 완전하다"
        );
        samples.sort_by_key(|(_, s)| *s);
        let mut fresh = Registry::new();
        let mut next = ops.iter().peekable();
        for (rows, s) in samples {
            let Some(s) = s else {
                assert!(rows.is_empty(), "목록 사건 없이 행이 있다");
                continue;
            };
            while let Some((seq, op)) = next.next_if(|(seq, _)| *seq <= s) {
                fresh.reduce(*seq, op);
            }
            assert_eq!(fresh.as_of_seq(), Some(s), "S 가 목록 사건의 seq 가 아니다");
            assert_eq!(
                fresh.rows(),
                &rows[..],
                "S={s} 의 행이 링 접두의 환원값과 다르다"
            );
        }
        assert!(
            next.next().is_none(),
            "마지막 스냅숏이 링 끝까지 환원하지 않았다"
        );
    }

    #[test]
    fn without_list_events_the_snapshot_has_no_seq() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        assert_eq!(core.queued_inputs().snapshot(), (vec![], None));
        core.emit(delta());
        assert_eq!(core.queued_inputs().snapshot(), (vec![], None));
        core.emit(queued("a", "A"));
        assert_eq!(core.queued_inputs().snapshot().1, Some(1));
    }

    // ── 종료 합성 + 봉인 ─────────────────────────────────────────────────────────────────────

    /// 합성 `Dropped{AgentEnded}` 가 링·구독자 양쪽에서 종점 전이보다 앞 · 대기 목록 표도 거둔다.
    #[test]
    fn finish_drops_every_open_row_before_the_terminal_transition() {
        struct Log {
            id: SinkId,
            lines: Mutex<Vec<String>>,
        }
        impl OutputSink for Log {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(OutputEvent::QueuedInput(op)) = frame.payload {
                    self.lines.lock().unwrap().push(format!("{op:?}"));
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        impl StatusSink for Log {
            fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
                self.lines
                    .lock()
                    .unwrap()
                    .push(format!("status {status:?}"));
            }
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        }
        let log = Arc::new(Log {
            id: uuid::Uuid::new_v4(),
            lines: Mutex::new(Vec::new()),
        });
        let fx = Fixture::new();
        let core = fx.core(log.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(queued("b", "B"));
        core.emit(cancel_requested("b"));
        core.subscribe(log.clone());
        log.lines.lock().unwrap().clear();
        assert_eq!(fx.pending(), Some(true));

        core.finish(TerminalReason::Exited { code: Some(0) });

        assert_eq!(
            *log.lines.lock().unwrap(),
            vec![
                format!("{:?}", ended("a")),
                format!("{:?}", ended("b")),
                format!("status {:?}", AgentStatus::Exited { code: Some(0) }),
            ],
            "남은 항목이 종점 전이보다 먼저 닫혀야 한다(목록 순)"
        );
        let tail: Vec<_> = ring_ops(&core)
            .into_iter()
            .skip(3)
            .map(|(_, op)| op)
            .collect();
        assert_eq!(tail, vec![ended("a"), ended("b")]);
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(
            registry.tombstone("a"),
            Some(false),
            "에이전트 종료는 되살림 불가"
        );
        drop(registry);
        assert_eq!(
            fx.pending(),
            None,
            "대기 목록 표가 이 화신의 항목을 거두지 않았다"
        );
    }

    /// 봉인: 종료 합성 뒤의 `Queued` 는 링에 `Dropped{AgentEnded}` 로 선다(→ 묘비) — 영구 항목을 못 만든다.
    #[test]
    fn a_queued_after_finish_lands_in_the_ring_as_agent_ended() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.finish(TerminalReason::Killed);
        let sink = Recorder::new();
        core.subscribe(sink.clone());

        core.emit(queued("late", "L"));

        assert_eq!(ring_ops(&core), vec![(0, ended("late"))]);
        assert_eq!(*sink.got.lock().unwrap(), vec![(0, Some(ended("late")))]);
        assert!(rows(&core).is_empty());
        assert_eq!(core.queued_inputs().lock().tombstone("late"), Some(false));
    }

    /// 다른 스레드가 `Queued` 를 쉼 없이 내는 동안 `finish` — 합성 덩이와 봉인 사이에 `Queued` 가 없고, 합성은
    /// 그때 열린 항목 전부를 한 덩이로 닫고, 명부에 비종결 항목이 남지 않는다.
    #[test]
    fn finish_synthesis_and_seal_are_one_lock_section() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let (ready_tx, ready_rx) = mpsc::channel();
        let emitter = {
            let core = core.clone();
            std::thread::spawn(move || {
                // 상한 = 링 축출 없이 전량 남는 크기(`Queued` + 합성 `Dropped` < `REPLAY_MAX_EVENTS`). 스케줄
                //   탓에 상한까지 종료를 못 보면 이 판은 겹침을 못 잰다 — 봉인 단독은 위 시험이 잰다.
                let mut after_final = 0;
                for i in 0..1800 {
                    core.emit(queued(&format!("q{i}"), "t"));
                    if i == 100 {
                        ready_tx.send(()).expect("ready");
                    }
                    if core.finalized.load(Ordering::Acquire) {
                        after_final += 1;
                        if after_final == 100 {
                            break;
                        }
                    }
                }
            })
        };
        ready_rx.recv_timeout(WAIT).expect("emitter ready");
        core.finish(TerminalReason::Exited { code: Some(0) });
        emitter.join().expect("emitter");

        let ops: Vec<QueuedInputEvent> = ring_ops(&core).into_iter().map(|(_, op)| op).collect();
        let first_drop = ops
            .iter()
            .position(|op| matches!(op, QueuedInputEvent::Dropped { .. }))
            .expect("합성 Dropped 가 없다");
        let listed: Vec<&String> = ops[..first_drop]
            .iter()
            .map(|op| match op {
                QueuedInputEvent::Queued { id, .. } => id,
                other => panic!("합성 앞에 Queued 아닌 사건: {other:?}"),
            })
            .collect();
        let dropped_after: Vec<&String> = ops[first_drop..]
            .iter()
            .map(|op| match op {
                QueuedInputEvent::Dropped {
                    id,
                    cause: DropCause::AgentEnded,
                } => id,
                other => panic!("합성 뒤에 AgentEnded 아닌 사건 — 봉인을 빠져나갔다: {other:?}"),
            })
            .collect();
        assert_eq!(
            &dropped_after[..listed.len()],
            &listed[..],
            "합성 덩이가 그때 열린 항목 전부를 목록 순으로 잇달아 닫지 않았다"
        );
        assert!(rows(&core).is_empty(), "명부에 비종결 항목이 남았다");
    }

    // ── 판정 뒤 바꿔 적기 · 사본 ─────────────────────────────────────────────────────────────

    /// 판명 앞에 선 `Queued` 는 그대로 적히고 판명이 옮긴다 · 판명 뒤의 `Queued` 는 `Queued` + 그 항목 하나의
    /// 사본을 실은 `AckUnavailable`(두 seq) — 명부 결과는 받음이고 목록은 찬 적이 없다.
    #[test]
    fn a_queued_after_the_ack_verdict_is_closed_by_its_own_copy_in_the_same_lock() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        assert_eq!(fx.pending(), Some(false));

        core.emit(queued("b", "B"));

        let queued_op = |id: &str, text: &str| QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        };
        let ack_op = |copies| QueuedInputEvent::AckUnavailable { delivered: copies };
        assert_eq!(
            ring_ops(&core),
            vec![
                (0, queued_op("a", "A")),
                (1, ack_op(vec![copy("a", "A")])),
                (2, queued_op("b", "B")),
                (3, ack_op(vec![copy("b", "B")])),
            ]
        );
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(registry.tombstone("b"), Some(false), "받음으로 닫혔다");
        assert_eq!(registry.as_of_seq(), Some(3));
        drop(registry);
        assert_eq!(fx.pending(), Some(false));
        assert_eq!(
            probe.drains().len(),
            1,
            "빔 → 빔인 바꿔 적기는 초인종을 울리지 않는다"
        );
    }

    #[test]
    fn the_seal_comes_before_the_ack_rewrite() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.emit(ack_unavailable());
        core.finish(TerminalReason::Killed);

        core.emit(queued("c", "C"));

        let ops = ring_ops(&core);
        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[1],
            (1, ended("c")),
            "봉인 뒤의 Queued 는 받음이 아니라 버림이다"
        );
    }

    /// 디코더가 빈 채로 낸 판명에 코어가 그 환원이 받음으로 닫는 항목(`Queued` · 취소 대기 못 뺐다·응답
    /// 없음)의 사본을 목록 순으로 채운다 — 취소로 닫힌 것은 없다 · 디코더가 채워 보낸 것은 덮는다 · 링 무게가
    /// 사본 본문을 센다.
    #[test]
    fn the_core_fills_the_ack_copies_from_the_rows_it_closes_as_delivered() {
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.emit(queued("a", "AAAA"));
        core.emit(queued("b", "BBBB"));
        core.emit(cancel_requested("b"));
        core.emit(queued("c", "CCCC"));
        core.emit(cancel_requested("c"));
        core.emit(cancel_answered("c", false));
        core.emit(queued("d", "DDDD"));
        core.emit(cancel_requested("d"));
        core.emit(cancel_answered("d", true));
        assert_eq!(
            rows(&core)
                .iter()
                .map(|r| (r.id.as_str(), r.phase))
                .collect::<Vec<_>>(),
            vec![
                ("a", RowPhase::Queued),
                (
                    "b",
                    RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false
                    }
                ),
                (
                    "c",
                    RowPhase::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: false
                    }
                ),
            ]
        );

        core.emit(OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
            delivered: vec![copy("zz", "디코더가 채운 것")],
        }));

        let stored = ring(&core).pop().expect("판명");
        let filled = QueuedInputEvent::AckUnavailable {
            delivered: vec![copy("a", "AAAA"), copy("b", "BBBB"), copy("c", "CCCC")],
        };
        assert!(
            matches!(&stored.event, OutputEvent::QueuedInput(op) if *op == filled),
            "사본이 닫힌 항목의 목록 순 본문이 아니다: {:?}",
            stored.event
        );
        assert_eq!(
            stored.cost_bytes,
            estimate_cost_bytes(&OutputEvent::QueuedInput(filled))
        );
        assert!(stored.cost_bytes > estimate_cost_bytes(&ack_unavailable()));
        let registry = core.queued_inputs().lock();
        assert!(registry.is_empty());
        assert_eq!(
            registry.tombstone("zz"),
            None,
            "덮인 디코더 사본은 환원되지 않는다"
        );
        assert!(registry.ack_unavailable_seen());
    }

    // ── 턴 관측 가드 ─────────────────────────────────────────────────────────────────────────

    thread_local! {
        static CLASSIFIED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn record_classified(event: &OutputEvent) -> Option<TurnSignal> {
        let label = match event {
            OutputEvent::QueuedInput(QueuedInputEvent::Queued { id, .. }) => format!("Queued {id}"),
            OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable { .. }) => {
                "AckUnavailable".into()
            }
            other => format!("{other:?}"),
        };
        CLASSIFIED.with(|c| c.borrow_mut().push(label));
        None
    }

    /// 코어가 바꿔 적거나 지은 사건(봉인의 `Dropped` · 판정 뒤 잇는 `AckUnavailable` · 종료 합성)은 분류기를
    /// 지나지 않는다 — 벤더 출력의 사실이 아니다. 원 사건(디코더의 판명 포함)은 지난다.
    #[test]
    fn events_the_core_writes_itself_never_reach_the_turn_classifier() {
        let classified = || CLASSIFIED.with(|c| std::mem::take(&mut *c.borrow_mut()));
        classified();

        // 종료 합성(열린 항목 x) · 봉인(y).
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), record_classified);
        core.emit(queued("x", "X"));
        core.finish(TerminalReason::Killed);
        core.emit(queued("y", "Y"));
        assert_eq!(
            ring_ops(&core).len(),
            3,
            "합성 Dropped x · 봉인 Dropped y 가 링에 섰다"
        );
        assert_eq!(
            classified(),
            vec!["Queued x"],
            "종료 합성·봉인이 분류기를 지났다"
        );

        // 판정 뒤 바꿔 적기 — 원 사건(디코더의 판명 포함)만 지난다.
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), record_classified);
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        core.emit(queued("b", "B"));
        assert_eq!(ring_ops(&core).len(), 4);
        assert_eq!(
            classified(),
            vec!["Queued a", "AckUnavailable", "Queued b"],
            "판정 뒤 잇는 AckUnavailable 이 분류기를 지났다"
        );
    }

    /// 운영 claude 분류기로 — 받음(`Delivered`) 말고는 목록 사건이 턴 관측을 켜지 않는다(바꿔 적은 사건 포함).
    #[test]
    fn list_events_other_than_delivered_never_turn_on_the_turn_observation() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(cancel_requested("a"));
        core.emit(cancel_answered("a", false));
        core.emit(queued("b", "B"));
        core.emit(dropped("b", DropCause::Rejected));
        core.emit(ack_unavailable());
        core.emit(queued("c", "C"));
        assert!(!fx.turns.is_in_turn(fx.id, EPOCH), "목록 사건이 턴을 켰다");
        assert!(
            probe.at_turn_end.lock().unwrap().is_empty(),
            "목록 사건이 턴을 끝냈다"
        );
    }

    // ── 대기 목록 표의 적는 순서 ─────────────────────────────────────────────────────────────

    /// 「찼다」는 환원한 replay 락 안 — 그 emit 의 fanout 을 받은 쪽이 이미 참을 읽는다.
    #[test]
    fn filled_is_written_inside_the_lock_that_reduced_it() {
        struct ReadsPendingOnQueued {
            id: SinkId,
            agent: AgentId,
            pending: Arc<InputsPendingTable>,
            seen: Mutex<Vec<Option<bool>>>,
        }
        impl OutputSink for ReadsPendingOnQueued {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(OutputEvent::QueuedInput(QueuedInputEvent::Queued {
                    ..
                })) = frame.payload
                {
                    let v = self.pending.get(self.agent, EPOCH);
                    self.seen.lock().unwrap().push(v);
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        let sink = Arc::new(ReadsPendingOnQueued {
            id: uuid::Uuid::new_v4(),
            agent: fx.id,
            pending: fx.pending.clone(),
            seen: Mutex::new(Vec::new()),
        });
        core.subscribe(sink.clone());
        assert_eq!(fx.pending(), Some(false));
        core.emit(queued("a", "A"));
        assert_eq!(*sink.seen.lock().unwrap(), vec![Some(true)]);
    }

    /// 「비었다」는 그 emit 의 턴 관측 **뒤** — 목록을 비우는 받음이 턴 끝이기도 하면 턴 끝 초인종 순간엔 표가
    /// 아직 참이고, 비었다 초인종 순간엔 거짓이다.
    #[test]
    fn drained_is_written_after_the_turn_observation_of_the_same_emit() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), delivered_ends_the_turn);
        core.emit(queued("a", "A"));
        core.emit(delivered("a"));
        assert_eq!(*probe.at_turn_end.lock().unwrap(), vec![Some(true)]);
        assert_eq!(probe.drains(), vec![(Some(false), false)]);
    }

    /// 목록을 비우는 claude `Delivered`(drain 턴의 진행) — 표가 거짓이 되는 순간 턴 관측은 이미 진행이다.
    /// 어댑터와 같은 순서(표 먼저, 턴 관측 나중)로 쉼 없이 읽는 관찰자가 「둘 다 한가」를 한 번도 못 본다.
    #[test]
    fn a_draining_claude_delivery_is_never_observed_idle_on_both_facts() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        let stop = Arc::new(AtomicBool::new(false));
        let (filled_tx, filled_rx) = mpsc::channel();
        let observer = {
            let (pending, turns, id, stop) =
                (fx.pending.clone(), fx.turns.clone(), fx.id, stop.clone());
            std::thread::spawn(move || {
                let mut filled_seen = false;
                let mut idle_after_filled = 0usize;
                while !stop.load(Ordering::Acquire) {
                    let p = pending.get(id, EPOCH);
                    let in_turn = turns.is_in_turn(id, EPOCH);
                    if p == Some(true) && !filled_seen {
                        filled_seen = true;
                        filled_tx.send(()).expect("filled");
                    }
                    if filled_seen && p == Some(false) && !in_turn {
                        idle_after_filled += 1;
                    }
                }
                idle_after_filled
            })
        };
        core.emit(queued("a", "A"));
        filled_rx.recv_timeout(WAIT).expect("관찰자가 찼다를 봤다");
        core.emit(delivered("a"));
        stop.store(true, Ordering::Release);
        assert_eq!(observer.join().expect("observer"), 0);
        assert_eq!(probe.drains(), vec![(Some(false), true)]);
    }

    /// 턴 관측을 지나지 않는 문도 목록을 비우면 락을 놓은 뒤 곧바로 적고 울린다 · 종료 합성 덩이는 목록을
    /// 비워도 울리지 않고 표에서 거두기만 한다.
    #[test]
    fn unobserved_emits_drain_and_ring_but_the_finish_batch_only_forgets() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit_without_turn_observation(queued("a", "A"));
        assert_eq!(fx.pending(), Some(true));
        core.emit_without_turn_observation(delivered("a"));
        assert_eq!(probe.drains(), vec![(Some(false), false)]);

        core.emit(queued("b", "B"));
        assert_eq!(fx.pending(), Some(true));
        core.finish(TerminalReason::Killed);
        assert!(rows(&core).is_empty(), "종료 합성이 목록을 비웠다(전제)");
        // ADR-0231: 종료 합성은 초인종을 울리지 않는다 — 대기 목록 표 항목은 곧바로 거두고, 초인종은 종점 전이를
        //   앞둔 화신에게 우편을 흘려보내라는 자극이 된다(파킹된 우편은 다음 화신의 등장 flush 가 나른다).
        assert_eq!(
            probe.drains().len(),
            1,
            "종료 합성이 죽어가는 화신 앞으로 초인종을 울렸다"
        );
        assert_eq!(fx.pending(), None, "표에서 거둔다");
    }

    /// 표는 더 작은 seq 의 쓰기를 버린다 — 락 밖으로 미룬 「비었다」(seq N)가 다른 스레드가 그 사이 락 안에서
    /// 적은 「찼다」(seq N+1)를 덮지 않는다.
    #[test]
    fn a_deferred_drain_never_overwrites_a_later_fill() {
        /// 첫 턴 끝 초인종에서 붙잡는다 — 「비었다」를 쓰기 직전의 스레드를 세워 두는 자리다(시험 전용).
        struct HoldFirstTurnEnd {
            hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        }
        impl StatusSink for HoldFirstTurnEnd {
            fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
            fn turn_ended(&self, _id: AgentId, _epoch: u32) {
                let taken = self.hold.lock().unwrap().take();
                if let Some((entered, release)) = taken {
                    entered.send(()).expect("entered");
                    release.recv_timeout(WAIT).expect("release");
                }
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let fx = Fixture::new();
        let core = fx.core(
            Arc::new(HoldFirstTurnEnd {
                hold: Mutex::new(Some((entered_tx, release_rx))),
            }),
            delivered_ends_the_turn,
        );
        core.emit(queued("a", "A"));
        let drainer = {
            let core = core.clone();
            std::thread::spawn(move || core.emit(delivered("a")))
        };
        entered_rx.recv_timeout(WAIT).expect("비우는 스레드가 섰다");
        assert!(rows(&core).is_empty(), "명부는 이미 비었다(락 안)");
        core.emit(queued("b", "B"));
        release_tx.send(()).expect("release");
        drainer.join().expect("drainer");

        assert_eq!(
            fx.pending(),
            Some(true),
            "늦게 쓴 더 작은 seq 의 「비었다」가 「찼다」를 덮었다"
        );
        assert_eq!(rows(&core).len(), 1);
    }

    /// 초인종은 비는 순간 한 번 — 턴 끝 없이 목록만 빈 경우 포함, 찬 채인 동안은 울리지 않는다.
    #[test]
    fn the_drain_doorbell_rings_once_per_drain() {
        let fx = Fixture::new();
        let probe = fx.probe();
        let core = fx.core(probe.clone(), claude_classifier());
        core.emit(queued("a", "A"));
        core.emit(queued("b", "B"));
        core.emit(delivered("a"));
        assert!(probe.drains().is_empty());
        core.emit(delivered("b"));
        assert_eq!(probe.drains().len(), 1);
        core.emit(queued("c", "C"));
        core.emit(dropped("c", DropCause::Withdrawn));
        assert_eq!(probe.drains().len(), 2);
        core.emit(delivered("c"));
        assert_eq!(
            probe.drains().len(),
            2,
            "빈 목록의 묘비 사건은 울리지 않는다"
        );
        assert!(
            probe.at_turn_end.lock().unwrap().is_empty(),
            "턴 끝은 없었다"
        );
    }

    /// 명부를 꽂지 않은 코어도 링의 모양(봉인 · 바꿔 적기 · 사본)은 같다 — 표 쓰기와 초인종만 없다.
    #[test]
    fn a_core_without_queued_wiring_shapes_the_ring_the_same_but_rings_nothing() {
        struct CountsDrains(AtomicU64);
        impl StatusSink for CountsDrains {
            fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
            fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
            fn inputs_drained(&self, _id: AgentId, _epoch: u32) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drains = Arc::new(CountsDrains(AtomicU64::new(0)));
        let core = OutputCore::new(
            uuid::Uuid::new_v4(),
            EPOCH,
            drains.clone(),
            TurnWiring::detached(),
        );
        core.emit(queued("a", "A"));
        core.emit(ack_unavailable());
        core.emit(queued("b", "B"));
        core.emit(queued("open", "O"));
        core.finish(TerminalReason::Killed);
        core.emit(queued("c", "C"));
        let kinds: Vec<String> = ring_ops(&core)
            .into_iter()
            .map(|(_, op)| match op {
                QueuedInputEvent::Queued { id, .. } => format!("Queued {id}"),
                QueuedInputEvent::AckUnavailable { delivered } => format!(
                    "Ack {:?}",
                    delivered.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
                ),
                QueuedInputEvent::Dropped { id, cause } => format!("Dropped {id} {cause:?}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "Queued a",
                "Ack [\"a\"]",
                "Queued b",
                "Ack [\"b\"]",
                "Queued open",
                "Ack [\"open\"]",
                "Dropped c AgentEnded",
            ]
        );
        assert_eq!(drains.0.load(Ordering::SeqCst), 0);
    }

    // ── 라이브 배달(§5-7 전제) ───────────────────────────────────────────────────────────────

    /// 두 스레드가 emit 하고 시험 sink 가 한쪽 send 를 붙잡아 역전을 만든다(시험 전용 — 운영 sink 는 막히지
    /// 않는다) → 그 sink 는 N+1 을 N 보다 먼저 받지만 **둘 다** 받는다 · 그 사이 붙은 새 sink 도 발급된 seq 를
    /// 빠짐없이 받는다(replay 또는 라이브). 근거 = 링 push 가 fanout 보다 앞이고 구독이 subscribers 락을 쥔 채
    /// replay 를 뜬다 — 어느 쪽이 뒤집히면 새 sink 가 N 을 잃는다.
    #[test]
    fn a_held_send_reorders_delivery_but_every_sink_gets_every_seq() {
        struct HoldsSeq {
            id: SinkId,
            hold_seq: u64,
            entered: Mutex<Option<mpsc::Sender<()>>>,
            release: Mutex<Option<mpsc::Receiver<()>>>,
            got: Mutex<Vec<u64>>,
        }
        impl OutputSink for HoldsSeq {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if frame.seq == self.hold_seq {
                    if let Some(tx) = self.entered.lock().unwrap().take() {
                        tx.send(()).expect("entered");
                    }
                    let rx = self.release.lock().unwrap().take();
                    if let Some(rx) = rx {
                        rx.recv_timeout(WAIT).expect("release");
                    }
                }
                self.got.lock().unwrap().push(frame.seq);
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let held = Arc::new(HoldsSeq {
            id: uuid::Uuid::new_v4(),
            hold_seq: 0,
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(Some(release_rx)),
            got: Mutex::new(Vec::new()),
        });
        let fx = Fixture::new();
        let core = fx.core(Arc::new(Quiet), claude_classifier());
        core.subscribe(held.clone());

        let first = {
            let core = core.clone();
            std::thread::spawn(move || core.emit(queued("n", "N")))
        };
        entered_rx
            .recv_timeout(WAIT)
            .expect("seq 0 의 send 가 붙잡혔다");
        core.emit(queued("n1", "N+1"));
        let late = Recorder::new();
        core.subscribe(late.clone());
        core.emit(delivered("n"));
        release_tx.send(()).expect("release");
        first.join().expect("first emitter");

        assert_eq!(
            held.got.lock().unwrap().clone(),
            vec![1, 2, 0],
            "붙잡힌 N 이 N+1 뒤에 도착한다 — 역전은 있다"
        );
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2],
            "새 sink 는 replay + 라이브로 전부 받는다"
        );
    }
}
