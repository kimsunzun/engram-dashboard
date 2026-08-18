//! 명령 배달 — 명부에서 주인을 찾아 **그 연결의 프레임 출구로 봉투를 쓰고**, 돌아온 결말을 **원래 물어본
//! 연결**에 실어 준다(ADR-0148 · TRD §3-5·§3-8).
//!
//! 배달 규칙 자체는 여기 없다 — 도구 crate 의 3단계([`route`])가 전 홉 공통이고(ADR-0149 결정 3) 이
//! 모듈이 더하는 것은 그 3단계가 필요로 하는 **데몬 몫의 두 조각**뿐이다:
//! ① 주인 토큰에서 연결로 가는 링크([`OwnerLink`] — 그 자리가 [`CommandRoster::sink_of`]) ·
//! ② 요청 상관 표([`CommandDeliveries`] — `request_id` → 원 연결 · 마감시각).
//!
//! ## ★한 `request_id` 에 답장은 정확히 하나★(TRD §4-⑤)
//!
//! 그 하나를 지키는 기제가 **상관 표의 자리**이고, 규칙은 두 줄이다:
//!
//! 1. **자리를 먼저 열고**([`CommandDeliveries::open`]) 그 뒤에 무엇이든 한다 — [`deliver`] 가 `route`
//!    보다 **앞서** 연다. 그래서 3단계 중 어느 단계가 답을 만들든 그 답은 자리 하나를 깔고 앉아 있다.
//! 2. **프레임을 내보낸 뒤에야 자리를 놓는다**([`CommandDeliveries::release`]). 결말이 붙는 순간이 아니라
//!    **나간 순간**이 자리의 끝이다 — 그 사이에 놓으면 같은 id 의 재질의가 빈 표를 보고 봉투를 한 번 더
//!    보낸다(같은 조작이 두 번 적용되고 프레임도 둘이 된다).
//!
//! ★자리를 거치지 않고 답장을 만들지 말 것★ — 이 모듈에서 나가는 **모든** 프레임은 표의 단일 판정 하나를
//! 근거로 나간다: 자리를 얻어 낸 답장이거나, 자리를 못 얻었다는 그 판정 자체를 실은 반려다. 예전에는
//! `route` 의 1단계(내 표)와 3단계(주인 부재)가 표를 아예 안 거치는 우회로였고, 그 둘이 진행 중인 왕복의
//! 키로 두 번째 프레임을 냈다.
//! 늦게 온 결말은 [`CommandDeliveries::complete`] 에서 빈손을 받고 버려진다 — 답장 자리를 이미 누가
//! 가져갔기 때문이다.
//!
//! ## ★내준 사본을 왕복 너머로 들지 않는다★(ADR-0148)
//!
//! 명부는 내보낸 프레임 출구 사본을 **회수할 수단이 없다.** 그래서 [`OwnerLink::send`] 는 사본을 받아
//! 그 자리에서 쓰고 버리고, 답장을 낼 때는 **원 연결을 다시 조회한다**. 상관 표가 드는 것은 연결
//! **식별자**뿐이다 — 여기에 출구 사본을 넣으면 그 사본이 `on_disconnect` 를 넘겨 살아 연결이 샌다.
//!
//! ## ★상관 표와 명부는 다른 표다 — 합치지 않는다★(TRD §4-④)
//!
//! 수명 단위가 다르다: 상관 표의 항목은 **왕복 하나**에 매달려 마감시각으로도 사라지고, 명부 등록은
//! **연결 하나**에 매달려 종료로만 사라진다. 합치면 마감시각 하나가 어휘를 지운다. 그래서 이 모듈의
//! 정리는 명부의 단일 제거 지점([`CommandRoster::detach`])을 늘리지 않는다.
//!
//! ## 진입점
//!
//! [`deliver`] 가 왕복 하나를 통째로 돈다(연결 태스크 **밖**에서 돌아야 한다 — 그 사유는 그 함수 doc) ·
//! [`CommandDeliveries::complete`] 가 주인의 결말을 그 왕복에 붙이고 ·
//! [`CommandDeliveries::expire`] 와 [`CommandDeliveries::drop_origin`] 이 나머지 둘을 거둔다.
// ADR-0148

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_command::{
    route, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable, ErrorCode,
    RequestId, RetryMode,
};
use engram_dashboard_net::frame_port::{ConnId, Frame};
use engram_dashboard_protocol::AgentEvent;
use tokio::sync::{oneshot, watch};

use crate::command_roster::CommandRoster;
use crate::connection_core::{event_json, sanitize_for_log};

/// 물어본 연결이 이미 떠났을 때의 문구 — 이 답장도 갈 곳이 없다(그 연결이 사라졌다).
const CALLER_ALREADY_GONE: &str = "the calling connection went away before the envelope was sent";

/// 이 경로가 짓는 **모든** 실패 답 — ★재시도 지시는 예외 없이 `Never` 다★.
///
/// ## 왜 코드에서 파생된 지시를 쓰지 않나
///
/// ADR-0153 의 자는 「안전 × 쓸모」이고, `same-request-id` 의 **안전** 다리는 「같은 번호로 다시 묻는
/// 것이라 같은 조작이 두 번 적용되지 않는다」이다. 그 전제는 **최초 수신 데몬이 완료분을 캐시해 재생해 줄
/// 때만** 성립하는데(TRD §4-⑥ 「완료분 재생」), 그 dedup 저장소는 이 저장소 어디에도 없다(실측). 저장소가
/// 없으면 재질의는 재질의가 아니라 **재실행**이고, 부수효과 있는 명령은 두 번 적용된다. 안전 다리가
/// 거짓이면 쓸모와 무관하게 판정은 「끝」이다 — 그래서 전부 `Never` 다.
///
/// ★코드 어휘는 그대로 둔다★: `OUTCOME_UNKNOWN` 은 **확실성 서술**이라 사실 그대로 남고(적용됐는지 모른다),
/// 바꾸는 것은 **지시**뿐이다. 계약에 두 칸이 따로 있는 이유가 이것이다. 새 오류 코드를 만들지 않는다.
///
/// ★재개 조건★ — dedup 저장소(완료분 재생 + TTL)가 서면 그때 이 함수를 걷고 코드에서 파생된 지시를
/// 되살린다. 그전에 되살리면 「안전하다」는 거짓말이 된다.
///
/// ★이 파일에서 오류를 짓는 길은 여기 하나뿐이다★ — 기계로 지키는 자리는
/// `tests::this_module_can_only_build_do_not_retry_failures`.
// ADR-0153
fn no_retry(code: ErrorCode, detail: impl Into<String>) -> CommandError {
    CommandError::with_retry(code, detail, RetryMode::Never)
}

/// 시계 seam — 마감 판정이 실시간에 묶이지 않게 한다.
///
/// ★주입인 이유★: 마감 초과는 **시각이 지났는가** 하나로 갈리는데, 그것을 `Instant::now()` 로 직접 읽으면
/// 그 갈래를 재는 시험이 실시간 대기를 써야 한다(느리고, 부하가 걸린 러너에서 뒤집힌다). 구현을 밖에서
/// 꽂으면 시험이 시각을 **손으로 밀어** 결정적으로 판정한다(ADR-0012).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// 운영 시계.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// 진행 중인 왕복 하나 — **원 연결**과 **마감시각**, 그리고 답장을 받아 갈 자리.
///
/// ★프레임 출구 사본을 여기 넣지 말 것★ — 이 항목은 연결보다 오래 살 수 있고, 살아남은 사본 하나가
/// 끊긴 연결의 writer 를 자기 종료하지 못하게 만든다(모듈 헤더 · ADR-0148).
struct Pending {
    /// 답장을 되돌릴 연결. **세대 번호가 없다 — 그래서 연결 id 가 재사용되지 않는 데 기대고 있다.**
    ///
    /// ★안전한 이유는 배달 코드가 아니라 **할당기**에 있다(실측 2026-08-18)★: 연결 id 는 네트워크 행
    /// `ws.rs` 의 `ConnRegistry::alloc_id` 가 `AtomicU64`(초기값 1)에서 `fetch_add(1)` 로 뽑고, 해제
    /// (`unregister`)는 맵에서 지울 뿐 그 값을 되돌려 놓지 않는다. 레지스트리는 데몬 조립에서 한 번 나므로
    /// 그 카운터는 프로세스 수명 내내 단조 증가하고, u64 소진은 도달 불가다.
    /// ★그 성질이 바뀌면 여기가 깨진다★ — id 를 재사용(풀 반납·해시·클라이언트 제공 값)하게 되면, 마감
    /// 전에 남은 자리가 **같은 번호를 새로 받은 다른 피어**에게 남의 답장을 건넨다. 그때 필요한 것은 이
    /// 칸에 세대 번호를 붙이는 것이고, 지금 미리 붙이지 않는 이유는 도달 불가한 위험에 대한 값이라서다.
    origin: ConnId,
    deadline: Instant,
    /// ★자리의 신분증 — **키가 아니라 이것이 그 왕복을 가리킨다**★.
    ///
    /// 없으면 뒷정리가 `request_id` 하나로 자리를 집는데, 그 사이 먼저 열린 자리가 사라지고 **같은 키로 새
    /// 자리**가 서 있을 수 있다(마감·종료가 걷어 간 뒤 호출자가 계약대로 같은 id 로 재질의한 경우가
    /// 그것이다 — 계약이 같은 번호의 재질의를 막지 않으므로 이 코드는 그것을 견뎌야 한다). 키로 집으면 늦은 태스크가
    /// **남의 자리**를 거두고 자기 답장을 내보내 프레임이 둘이 된다. 그래서 [`CommandDeliveries::release`]
    /// 는 키와 이 표를 **함께** 대조한다.
    token: SeatToken,
    /// 이 자리를 연 요청의 **정체** — 같은 id 를 다시 본 것이 재질의인지 딴 요청인지 이걸로 가른다.
    ///
    /// ★상주 비용을 지고 산다★: 봉투의 이름·인자를 자리에 한 부 복사해 왕복 동안 들고 있다. 안 들면
    /// 「같은 id + 다른 페이로드」를 구분할 길이 없어(TRD §4-⑥ 셋째 다리) 딴 요청이 남의 답을 받는다.
    /// 표 전체의 상주 상한은 아직 없다 — 그 미결의 기록은 [`CommandDeliveries`] 헤더.
    name: String,
    args: serde_json::Value,
    state: SeatState,
}

/// 자리를 여는 순서에서 나오는 단조 증가 번호 — [`Pending::token`].
type SeatToken = u64;

/// 자리의 처지 — ★답장이 **아직 안 정해졌나 / 결과를 실었나 / 빈손인가**로 갈린다★.
///
/// 이 셋을 구분하는 이유는 「같은 id 로의 재질의」를 어떻게 대접할지가 갈리기 때문이다: 결과를 실은 답장이
/// 나가는 중이면 재질의는 그 답을 같이 쓰면 되지만, 빈손으로 접힌 자리는 **다음 시도를 흡수하면 안 된다**
/// (흡수하면 재시도가 조용히 no-op 이 되고 호출자는 자기가 다시 묻기 전에 정해진 답을 받는다).
enum SeatState {
    /// 답을 기다리는 중 — 이 자리가 그 왕복의 답장 발권이다.
    Awaiting(oneshot::Sender<CommandReply>),
    /// 결말이 붙었다 — 프레임이 나가는 중이고 아직 [`CommandDeliveries::release`] 전이다.
    Fulfilled,
    /// 결말 **없이** 접혔다(마감 초과). 나갈 프레임에 결과가 없으므로 재질의는 새로 시도해야 한다.
    ///
    /// ★그 선택이 남기는 잔여 — 번호를 재사용한 호출자는 남의 결말을 받을 수 있다★
    ///
    /// 이 자리가 갈아엎히고 같은 번호로 **다른 내용**의 요청이 새 자리를 열면, 그 뒤에 도착한 **옛 시도의
    /// 늦은 결말**이 그 새 자리에 들어앉는다 — 결말 프레임이 드는 것은 요청 번호뿐이라 어느 시도의 답인지
    /// 가릴 수단이 이 표에 없다([`CommandDeliveries::complete`] 가 보내는 쪽 연결조차 안 보는 것과 같은
    /// 이유). 가리려면 결말에도 발권이 실려야 하고, 그건 wire 계약을 늘리는 일이다.
    /// ★오늘 이것을 견디는 근거★: 이 경로는 재시도 지시를 전부 `Never` 로 내리므로([`no_retry`]) **번호
    /// 재사용을 권하지 않는다.** 그래도 규약을 어긴 호출자에게는 여전히 일어난다 — 없앤 위험이 아니라
    /// **권고로 좁힌 위험**이다.
    Void,
}

/// 자리를 열어 본 결과 — ★갈래가 **프레임을 몇 장 내는가**와 **무슨 지시를 실어 보내는가**로 갈린다★.
///
/// 표는 `request_id` 하나에 자리 하나다. 그 키가 이미 차 있는데 판정 없이 답장을 내면 같은 키의 프레임이
/// 둘이 되므로(TRD §4-⑤ 위반), 「자리를 못 얻었다」를 한 값으로 뭉치지 않고 타입으로 가른다.
/// ★반려 갈래도 프레임을 낸다 — 그것이 **그 질의에 대한 유일한 답**이다★: 이 판정 자체가 표의 한 잠금
/// 아래서 났으므로 질의 하나에 답 하나라는 계산은 그대로 선다.
enum Seat {
    Opened {
        rx: oneshot::Receiver<CommandReply>,
        token: SeatToken,
    },
    /// ★**같은 연결**이 **같은 요청**을 다시 냈다 — 답장을 하나 더 내지 않는다★.
    ///
    /// 오류가 아니라 **정상 경로**다: 진행 중인 그 왕복의 답장 하나가 두 요청을 다 답한다(TRD §4-⑥
    /// 「in-flight 중복 = 같은 pending 에 coalesce」).
    /// ★여기서 답장을 하나 더 내지 말 것★ — 같은 키의 프레임이 둘이 되고, 호출자의 pending 은 먼저 온
    /// 것으로 풀린 뒤 나머지 하나를 고아로 받는다. 합침은 그 둘째 프레임을 **만들지 않는** 방법이자,
    /// 봉투를 두 번 보내지 않는 방법이다(같은 조작의 이중 적용 방지).
    /// ★합치는 것과 「다시 물어도 된다」는 별개다★ — 이 경로는 재시도를 지시하지 않는다([`no_retry`]).
    /// 합침이 사는 것은 그 지시와 무관하게, **이미 도착한 중복 질의**를 어떻게 대접할지의 문제라서다.
    Coalesced,
    /// **같은 id 인데 딴 요청**이다(이름·인자가 다르다) — TRD §4-⑥ 의 셋째 다리.
    ///
    /// ★합치면 안 되는 이유가 여기서 갈린다★: 합치면 이 요청은 **한 번도 전달되지 않고** 답장도 못 받는
    /// 채, 그 키로 나가는 한 장이 **딴 요청의 결과**를 실어 간다 — 호출자는 엉뚱한 payload 로 자기 promise
    /// 를 푼다. 그래서 `REQUEST_ID_CONFLICT` 로 반려한다 — 코드가 **사실 그대로**다: 아무것도 실행되지
    /// 않았고(확실성 = 확실), 원인은 그 번호가 이미 딴 요청에 물려 있다는 것 하나다. 호출자가 할 일은
    /// **새 id 를 쓰는 것**이다.
    Conflict,
    /// **다른 연결**이 그 id 로 같은 요청을 물고 있다.
    ///
    /// 이쪽엔 답장을 낸다 — 안 내면 이 호출자가 아무 답도 못 받고 매달린다(그 연결엔 자리가 없어 마감
    /// 수거도 못 건드린다). 그 답장은 먼저 열린 왕복의 답장과 **다른 연결**로 가므로 호출자별 「답장
    /// 하나」는 그대로 지켜진다.
    /// ★무슨 코드를 실을지는 **그 자리 임자가 아직 붙어 있나**로 갈린다 — 판정은 [`deliver`] 가 한다★
    /// (여기서 못 하는 이유: 상관 표는 명부를 모른다).
    Taken { holder: ConnId },
    /// 표가 닫혔다 — 데몬이 내려가는 중이라 새 왕복을 받지 않는다([`CommandDeliveries::drain`]).
    Closed,
}

/// 요청 상관 표 — 전 연결이 같은 한 부를 본다(`Arc` clone 이 같은 표를 본다).
///
/// ## ★미결: 이 표엔 상주 상한이 없다★(이번 라운드 범위 밖 — 기록만 한다)
///
/// 형제 격인 명부는 개수·길이 상한을 둘 다 걸어 상주를 ≈16 MiB 로 닫아 두었다
/// (`engram_dashboard_command::Roster::MAX_NAMES` 항목의 셈). 이 표엔 그런 상한이 없다 — 자리 수도, 자리가
/// 드는 이름·인자([`Pending::name`]·[`Pending::args`]) 크기도 안 잰다. 열 수 있는 자리는 왕복 하나당
/// 하나이고 마감(기본 10초)이 걷어 가므로 무한정 자라지는 않지만, **상한이 있는 것과는 다르다.**
/// 지금 안 닫는 이유는 이 입구가 **인증 경계 안**이라서다(ADR-0147) — 아무나 밀어 넣을 수 있는 표면이
/// 아니다. 닫는다면 명부의 그 상한이 선례가 된다.
// ADR-0148
#[derive(Clone)]
pub struct CommandDeliveries {
    inner: Arc<Mutex<Seats>>,
    clock: Arc<dyn Clock>,
    deadline: Duration,
}

/// 표의 속 — 자리들 + 발권 번호 + 닫힘 표식.
#[derive(Default)]
struct Seats {
    open: HashMap<RequestId, Pending>,
    next_token: SeatToken,
    /// ★닫힌 표는 **거절하되 답은 한다**★ — 근거는 [`CommandDeliveries::drain`].
    closed: bool,
}

impl CommandDeliveries {
    /// 마감시각 기본값(TRD §4-⑥).
    ///
    /// ★계약상 마감은 **호출자가 정한다** — 오늘 그 칸이 wire 에 없어 데몬이 기본값을 쓴다★:
    /// `CommandEnvelope` 에 마감 칸이 없으므로 부르는 쪽이 값을 실을 방법이 아직 없다. 칸이 생기면
    /// (additive) 여기가 그 값의 fallback 이 된다.
    pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);

    /// 만료 수거 주기 — 마감 초과가 이 간격만큼 늦게 관측될 수 있다.
    ///
    /// ★수거를 **트래픽에 얹지 않는 것**이 요점이다★: 다음 프레임이 올 때 훑는 형태면 조용한 데몬에서
    /// 마감이 영영 안 지나가고, 그러면 호출자가 답도 오류도 없이 매달린다(ADR-0148 이 지목한 무한 대기).
    const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock), Self::DEFAULT_DEADLINE)
    }

    pub fn with_clock(clock: Arc<dyn Clock>, deadline: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Seats::default())),
            clock,
            deadline,
        }
    }

    /// 만료 수거 태스크를 띄운다 — 데몬 종료 신호가 오면 **남은 자리를 전부 답하고** 멈춘다.
    ///
    /// ★수거가 없으면 「불명」이 「영원한 무응답」이 된다★(TRD §4-④). 이 한 줄이 빠져도 정상 왕복은 전부
    /// 도므로 **증상이 답 없는 요청 하나로만 나타난다** — 조립에서 빠뜨리기 쉬운 자리다.
    ///
    /// ★나가는 길에 [`CommandDeliveries::drain`] 을 부르는 것이 종료를 유계로 만든다★: 배달 태스크는
    /// 자기 답장 자리를 기다리는데 그 자리의 **보내는 쪽이 이 표 안에** 있다. 비우지 않고 끝내면 그
    /// 태스크들은 마감(기본 10초)까지 깨어날 계기가 없고, 그동안 표와 태스크가 「종료했다」고 보고된
    /// 시점보다 오래 산다. 비우면 전부 **즉시** 풀린다.
    /// ★반환한 핸들을 버리지 말 것★ — 조립부가 이것을 기다려야 「수거기가 멈췄다」가 관측된다.
    #[must_use = "조립부가 이 핸들을 기다려야 종료가 유계다"]
    pub fn spawn_sweeper(
        &self,
        mut shutdown: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let deliveries = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Self::SWEEP_INTERVAL);
            // 밀린 틱을 몰아 치지 않는다 — 수거는 「지금 만료된 것」만 보므로 따라잡을 빚이 없다.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let expired = deliveries.expire();
                        if expired > 0 {
                            tracing::debug!(expired, "마감 지난 명령 왕복을 거둬 TIMEOUT 으로 답했다");
                        }
                    }
                    changed = shutdown.changed() => {
                        // 보내는 쪽이 사라진 것도 종료다(Err) — 그 뒤로는 신호가 올 수 없다.
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                }
            }
            let drained = deliveries.drain();
            if drained > 0 {
                tracing::info!(
                    drained,
                    "데몬 종료 — 진행 중이던 명령 왕복을 전부 답하고 거뒀다"
                );
            }
        })
    }

    /// 남은 자리를 **전부** 거둬 답한다(종료 경로). 반환 = 거둔 수.
    ///
    /// ★`OUTCOME_UNKNOWN` 인 이유★: 봉투는 이미 주인에게 갔을 수 있어 적용 여부가 **불명**이다. 「안전 ×
    /// 쓸모」(ADR-0153)의 **확실성** 축이 그것이다. `INTERNAL` 로 접으면 호출자는 적용됐을지도 모르는
    /// 조작을 **확실한 실패**로 읽는다 — 사실이 아니다. (지시는 코드와 무관하게 `Never` 다 — [`no_retry`].)
    /// ★마감과 달리 시계를 보지 않는다★ — 종료는 시각이 아니라 사건이다.
    ///
    /// ★비우는 것으로 끝내지 않고 **표를 닫는다**★: 비우기만 하면 그 뒤에 도착한 `Command` 가 자리를 열고
    /// 답을 기다리는데, 그때 이미 수거기는 멈췄고 마감을 볼 눈이 아무 데도 없다 — 그 왕복은 답장 0장으로
    /// 프로세스가 죽을 때까지 매달린다. 이 창은 이론이 아니다: 수락 루프가 끊긴 뒤에도 연결 태스크들은
    /// 살아 있고(떼어 낸 spawn 이라 종료 수신기가 없다) 데몬은 세션 정리·flush 에 수 초를 더 쓴다.
    /// 닫힌 뒤의 질의는 [`Seat::Closed`] 로 **즉시 반려**되므로 「종료는 유계다」가 그 창에서도 선다.
    /// ★닫은 뒤엔 자리를 통째로 지운다★ — 재사용을 막을 이유(같은 키의 새 왕복)가 닫힘으로 이미 사라졌다.
    pub fn drain(&self) -> usize {
        // 잠금 밖에서 답한다 — 근거는 [`CommandDeliveries::expire`] 와 같다.
        let outstanding: Vec<(RequestId, Pending)> = {
            let mut table = self.lock();
            table.closed = true;
            table.open.drain().collect()
        };
        let mut count = 0;
        for (request_id, pending) in outstanding {
            // 이미 답이 붙은 자리(`Fulfilled`·`Void`)는 셈에서 뺀다 — 그 프레임은 이미 나가는 중이다.
            if let SeatState::Awaiting(waiter) = pending.state {
                count += 1;
                let _ = waiter.send(CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        "the daemon shut down while this command was in flight",
                    ),
                ));
            }
        }
        count
    }

    /// 왕복 하나를 연다 — ★[`deliver`] 가 **제일 먼저** 부른다★. 답장은 돌려준 자리로 온다.
    ///
    /// ★무엇을 하기 전에 연다★ — 봉투를 쓴 뒤에 열면 발 빠른 주인의 결말이 자리보다 먼저 도착해 빈손을
    /// 받고, `route` 의 다른 단계가 답을 만들면 그 답이 표를 아예 안 거치고 나간다(모듈 헤더 규칙 1).
    /// ★이미 찬 키를 덮지 않는다★: 덮으면 먼저 열린 왕복의 자리가 조용히 사라져 그쪽 호출자가 마감까지
    /// 매달린다. 갈래별 뜻은 [`Seat`].
    fn open(
        &self,
        request_id: RequestId,
        origin: ConnId,
        name: &str,
        args: &serde_json::Value,
    ) -> Seat {
        // ★마감 지난 자리를 **여기서 먼저** 정산한다★(잠그기 전에 — `expire` 가 스스로 잠근다).
        //
        // 수거기는 1초마다 도는데 재시도는 마감 직후에 온다. 그 사이에 도착한 재질의가 「아직 안 걷힌
        // 시체」에 합쳐지면 **그 시도는 한 번도 전달되지 않고**, 호출자는 자기가 다시 묻기 전에 정해진
        // 이전 시도의 `TIMEOUT` 을 답으로 받는다 — 재시도가 조용한 no-op 이 된다. 먼저 정산하면 그 자리는
        // 결말 없는 `Void` 가 되고, 아래에서 갈아엎여 이 시도가 제 왕복을 얻는다.
        //
        // ★그 대가로 두 프레임의 **순서가 뒤집힌다**★: 여기서 깨어난 옛 왕복은 제 `TIMEOUT` 을 쓰려고
        // 스케줄만 됐을 뿐이라, 실제 쓰기는 이 재시도가 자리를 얻고 봉투까지 보낸 **뒤에** 일어난다 —
        // 같은 연결에 **옛 답이 새 답보다 늦게** 닿을 수 있다. 두 프레임 다 정상이고(각 시도에 한 장씩)
        // 상관 키도 같지만, 호출자가 「마지막에 온 것이 마지막 답」이라고 가정하면 그 가정이 여기서 깨진다.
        self.expire();

        let (waiter, rx) = oneshot::channel();
        let deadline = self.clock.now() + self.deadline;
        let mut table = self.lock();
        // 닫힌 표는 아무 자리도 열지 않는다 — 열어 봐야 깨워 줄 눈이 없다([`CommandDeliveries::drain`]).
        if table.closed {
            return Seat::Closed;
        }

        // ★판정이 **이 임계 구역 안**이어야 한다★ — 밖에서 「이미 있나」를 먼저 보고 들어오는 형태로 바꾸면
        //   같은 연결이 연달아 낸 두 배달 태스크가 둘 다 빈 표를 보고 각자 자리를 연다.
        enum Occupancy {
            Free,
            /// 결말 없이 접힌 시체 — 다음 시도를 흡수하면 안 되므로 갈아엎는다.
            Stale,
            Held(Seat),
        }
        let occupancy = match table.open.get(&request_id) {
            None => Occupancy::Free,
            Some(held) if matches!(held.state, SeatState::Void) => Occupancy::Stale,
            // ★페이로드를 **먼저** 본다★: 같은 id 라도 딴 요청이면 누가 물었든 합칠 수 없다(TRD §4-⑥
            //   셋째 다리). 이름만 보지 않고 인자까지 보는 이유는 같은 이름의 다른 조작이 흔해서다
            //   (`tab.create` 를 창을 바꿔 두 번 — 합치면 한 번만 열린다).
            Some(held) if held.name != name || &held.args != args => {
                Occupancy::Held(Seat::Conflict)
            }
            Some(held) if held.origin == origin => Occupancy::Held(Seat::Coalesced),
            Some(held) => Occupancy::Held(Seat::Taken {
                holder: held.origin,
            }),
        };
        match occupancy {
            Occupancy::Held(seat) => return seat,
            Occupancy::Stale => {
                // 시체의 발권은 여기서 무효가 된다 — 늦게 오는 `release` 는 표를 대조해 빈손으로 돌아간다.
                //
                // ★형제들과 달리 잠금 **안에서** 떨어뜨린다 — 그래도 되는 전제가 하나 있다★:
                //   `Void` 는 [`CommandDeliveries::expire`] 가 **sender 를 꺼낸 뒤**에만 붙으므로 여기
                //   떨어지는 값에는 깨울 상대가 없다. `expire`·`drop_origin`·`drain` 이 잠금 밖에서
                //   떨어뜨리는 이유(소멸자가 기다리던 태스크를 깨운다)가 이 줄엔 해당되지 않는다.
                //   ★sender 를 남긴 채 `Void` 를 붙이는 코드가 생기면 이 줄이 곧바로 위반이 된다★ —
                //   그때 잡아 줄 시험이 없으니, 그 전제를 바꾸는 편집은 이 줄도 함께 옮겨야 한다.
                table.open.remove(&request_id);
            }
            Occupancy::Free => {}
        }

        let token = table.next_token;
        table.next_token += 1;
        table.open.insert(
            request_id,
            Pending {
                origin,
                deadline,
                token,
                name: name.to_string(),
                args: args.clone(),
                state: SeatState::Awaiting(waiter),
            },
        );
        Seat::Opened { rx, token }
    }

    /// 그 왕복이 끝났다 — ★프레임이 나간 **뒤에** 자리를 놓는다★(모듈 헤더 규칙 2).
    ///
    /// ★키가 아니라 발권으로 집는다★: 이 호출이 늦으면 같은 키에 **다른 왕복**이 앉아 있을 수 있고
    /// (마감·종료가 걷어 간 뒤의 정상 재질의), 키로 지우면 그 왕복이 이유 없이 답을 잃는다.
    fn release(&self, request_id: RequestId, token: SeatToken) {
        let mut table = self.lock();
        if table
            .open
            .get(&request_id)
            .is_some_and(|p| p.token == token)
        {
            table.open.remove(&request_id);
        }
    }

    /// 주인이 돌려준 결말을 그 왕복에 붙인다. **거짓 = 붙일 자리가 없었다**(늦게 왔거나, 아무도 안 물었다).
    ///
    /// ★자리를 **지우지 않는다** — 결말을 붙이고 `Fulfilled` 로 두기만 한다★: 지우면 프레임이 아직 나가기
    /// 전인 그 찰나에 같은 호출자의 재질의가 빈 표를 보고 **봉투를 한 번 더 보낸다**(같은 조작이 두 번
    /// 적용되고 같은 키의 프레임도 둘이 된다). 자리는 [`CommandDeliveries::release`] 가 프레임을 내보낸
    /// 뒤에 놓는다. 그 사이에 온 재질의는 [`Seat::Coalesced`] 로 접혀 **나가는 그 한 장**을 같이 쓴다.
    ///
    /// ★어느 연결이 보냈는지는 보지 않는다★: 상관 키가 추측 불가한 난수(v4 UUID)이고, 이 소켓은 이미 인증
    /// 경계 안이라 문자열 지식만으로 신뢰한다(ADR-0147). 보낸 연결까지 대조하려면 표가 주인 연결도 들어야
    /// 하는데, 그러면 주인이 재접속한 왕복이 자기 답을 못 붙인다.
    pub fn complete(&self, reply: CommandReply) -> bool {
        // 잠금 밖에서 답한다(ADR-0006) — 발권만 안에서 뽑아 온다.
        let waiter = {
            let mut table = self.lock();
            match table.open.get_mut(&reply.request_id) {
                Some(pending) => {
                    match std::mem::replace(&mut pending.state, SeatState::Fulfilled) {
                        SeatState::Awaiting(waiter) => Some(waiter),
                        // 이미 답이 붙은 자리다 — 처지를 되돌려 놓고 빈손으로 나간다.
                        settled => {
                            pending.state = settled;
                            None
                        }
                    }
                }
                None => None,
            }
        };
        let Some(waiter) = waiter else {
            return false;
        };
        // 받는 쪽이 이미 사라졌으면(배달 태스크가 끝났다) 보낼 곳이 없다 — 그것도 답장 하나를 지킨 결과다.
        let _ = waiter.send(reply);
        true
    }

    /// 마감 지난 왕복을 전부 거둬 `TIMEOUT` 으로 답한다. 반환 = 거둔 수.
    ///
    /// ★`TIMEOUT` 은 확실성 「불명」이다★ — 주인이 늦게 답할 수도 있어 적용 여부를 모른다(TRD §4-④·§4-⑥).
    /// 그렇다고 「같은 번호로 다시 물어라」로 이어지지는 않는다 — 그 지시가 안전하려면 완료분을 재생해 줄
    /// dedup 저장소가 있어야 하는데 없다([`no_retry`]).
    /// ★자리를 지우지 않고 `Void` 로 접는다★ — 지우는 것은 [`CommandDeliveries::release`] 뿐이다(근거는
    /// [`CommandDeliveries::complete`] 와 같다: 프레임이 나가기 전에 키가 비면 재질의가 봉투를 한 번 더
    /// 보낸다). 접힌 자리가 다음 시도를 삼키지 않는 근거는 [`SeatState::Void`].
    pub fn expire(&self) -> usize {
        let now = self.clock.now();
        // ★잠금 안에서 답하지 않는다★(ADR-0006): 발권을 뽑고 놓은 뒤에 깨운다 — 깨어난 태스크가 곧바로
        //   이 표를 다시 잠글 수 있고, 답장을 기다리는 배달 하나가 등록·정리·다른 배달을 세우면 안 된다.
        let expired: Vec<(RequestId, oneshot::Sender<CommandReply>)> = {
            let mut table = self.lock();
            table
                .open
                .iter_mut()
                .filter(|(_, pending)| pending.deadline <= now)
                .filter_map(|(id, pending)| {
                    match std::mem::replace(&mut pending.state, SeatState::Void) {
                        SeatState::Awaiting(waiter) => Some((*id, waiter)),
                        // 이미 접혔거나 결말이 붙은 자리는 두 번 세지 않는다.
                        settled => {
                            pending.state = settled;
                            None
                        }
                    }
                })
                .collect()
        };
        let count = expired.len();
        for (request_id, waiter) in expired {
            let _ = waiter.send(CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Timeout,
                    "the owner did not answer before the deadline",
                ),
            ));
        }
        count
    }

    /// 그 연결이 **낸** 왕복을 전부 거둔다 — 답장을 받아 갈 곳이 사라졌다. 반환 = 거둔 수.
    ///
    /// ★답장을 만들지 않고 자리만 놓는다★: 받을 연결이 없으므로 어떤 결말을 지어내도 갈 곳이 없다. 자리를
    /// 놓으면 기다리던 배달이 「기다릴 상대가 사라졌다」로 풀려 태스크가 끝난다.
    /// ★**미구현 — TRD §4-④ 와 어긋나 있다**★(이번 라운드 범위 밖 · 에스컬레이션됨)
    ///
    /// ① **주인 쪽 끊김은 여기서 안 잡힌다.** 이 표는 원 연결만 기억하므로(ADR-0148 이 고정한 항목 모양)
    /// 주인이 왕복 도중 죽으면 그 왕복은 마감까지 기다렸다 `TIMEOUT` 으로 끝난다 — TRD §4-④ 는 그 상황에
    /// `OUTCOME_UNKNOWN` 을 적어 두었다. 코드가 다르다는 사실 자체가 계약 미준수다. 호출자가 **하는 일**은
    /// 갈리지 않는다(둘 다 확실성 「불명」이고 지시도 같다)는 것이 오늘 이것을 견디는 이유지, 맞다는 뜻이
    /// 아니다.
    /// ② **§4-④ 의 「어느 쪽 연결이든 cleanup 시 그 ConnId 키 sweep」도 없다** — 이 함수는 원 연결 쪽
    /// 절반만 한다. 나머지 절반을 하려면 주인 연결 색인이 있어야 하는데 ADR-0148 이 못 박은 항목 모양이
    /// 그 칸을 배제한다. 그래서 이건 코드로 메울 수 있는 구멍이 아니라 **결정으로 풀 건**이다.
    pub fn drop_origin(&self, conn_id: ConnId) -> usize {
        // 거둔 항목은 잠금 **밖에서** 떨어뜨린다 — 소멸자가 기다리던 태스크를 깨운다(위 `expire` 와 같은 이유).
        let dropped: Vec<Pending> = {
            let mut table = self.lock();
            let mine: Vec<RequestId> = table
                .open
                .iter()
                .filter(|(_, pending)| pending.origin == conn_id)
                .map(|(id, _)| *id)
                .collect();
            mine.into_iter()
                .filter_map(|id| table.open.remove(&id))
                .collect()
        };
        dropped.len()
    }

    /// 지금 표에 앉아 있는 자리 수 — 표가 새는지 보는 관측 표면이다.
    ///
    /// 답이 붙었지만 아직 프레임이 안 나간 자리도 센다 — 그 자리는 아직 그 키를 쥐고 있다.
    pub fn in_flight(&self) -> usize {
        self.lock().open.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Seats> {
        self.inner.lock().expect("command deliveries poisoned")
    }
}

impl Default for CommandDeliveries {
    fn default() -> Self {
        Self::new()
    }
}

/// 3단계 배달의 2단계가 쓰는 전송 — 주인 토큰을 그 주인이 붙어 있는 연결의 프레임 출구로 옮긴다.
///
/// ★future 는 이 struct 를 빌리지 않는다★: 명부 조회·봉투 쓰기는 `send` 안에서 **동기로** 끝내고, 밖으로
/// 나가는 것은 답장 자리 하나뿐이다. 그래야 내준 출구 사본이 왕복 너머로 살지 않는다(모듈 헤더).
struct OwnerLink<'a> {
    roster: &'a CommandRoster,
    origin: ConnId,
    /// [`deliver`] 가 **미리 연** 자리의 수신단 — `send` 가 한 번 꺼내 간다.
    ///
    /// ★여기서 자리를 열지 않는다★: 열고 닫는 일이 이 안에 있으면 실패 갈래마다 되돌리기가 필요하고, 그
    /// 되돌리기는 키로 자리를 집어 **남의 왕복**을 지운다(자리는 `.await` 없이도 선점 사이에 바뀔 수 있다).
    /// 자리의 수명을 통째로 [`deliver`] 로 올리면 그 되돌리기 자체가 사라진다.
    waiter: Mutex<Option<oneshot::Receiver<CommandReply>>>,
}

/// 이미 정해진 답을 그대로 내는 자리 — `route` 가 요구하는 반환 모양을 맞춘다.
fn settled(reply: CommandReply) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(std::future::ready(reply))
}

impl CommandLink for OwnerLink<'_> {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        // 오류 갈래의 문구에 실을 이름 — 봉투는 아래에서 통째로 옮겨진다.
        // ★되돌려 보내는 호출자 문자열에 길이 상한을 두지 않는다★(ADR-0146) — 읽는 주체가 LLM 이라 꼬리를
        //   자르면 다음 행동이 사라진다. 자르는 것은 **로그** 쪽뿐이다.
        let name = env.name.clone();

        let taken = self
            .waiter
            .lock()
            .expect("owner link waiter poisoned")
            .take();
        let Some(rx) = taken else {
            // `route` 는 홉당 한 번만 부른다 — 두 번째 호출은 이 모듈의 결함이고 기다린다고 안 바뀐다.
            return settled(CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::Internal,
                    format!("this daemon tried to hand '{name}' over twice for one round trip"),
                ),
            ));
        };

        // ★봉투를 넘기기 **직전** 물어본 연결이 아직 있는지 본다 — 없으면 안 보낸다★
        //
        // 이 태스크는 `dispatch` 가 떼어 낸 것이라 **첫 폴링이 정리보다 늦을 수 있다**: 그 순서에서
        // `drop_origin` 은 빈 표를 보고 지나가고, 뒤늦게 깨어난 우리가 봉투를 보내면 이미 떠난 호출자를
        // 위해 부수효과 있는 명령이 실행된다.
        //
        // ★이 검사가 닫는 것과 못 닫는 것을 갈라 적는다★ — 「완전히 닫는다」가 아니다:
        // ① **닫는다 — 고아 자리.** `on_disconnect` 는 명부에서 먼저 빼고(`detach`) 그다음 자리를
        //    거두므로(`drop_origin`), 정리가 우리보다 앞섰다면 이 조회는 반드시 빈손이고, 우리가 앞섰다면
        //    우리 자리가 표에 있어 정리가 그것을 거둔다. **그 두 줄의 순서를 뒤집으면 이 검사가 무력해진다.**
        // ② **못 닫는다 — 부수효과.** 이 조회는 **그 순간의 표본**이고 봉투는 아래에서 나간다. 그 사이에
        //    끊긴 연결은 여기서 안 보인다(check-then-act 의 본래 잔여라 이 자리에서는 못 없앤다 — 없애려면
        //    명부 잠금을 봉투 쓰기까지 들고 가야 하고, 그건 `route` 가 일부러 놓은 그 잠금이다).
        //    ★그러니 이 검사를 「이제 떠난 호출자의 명령은 절대 실행되지 않는다」로 읽지 말 것★.
        //
        // ★이 갈래의 답장은 **아무 데도 안 나간다**★ — 조회가 방금 빈손이었고 연결 id 는 재사용되지 않으므로
        // ([`Pending::origin`]) 그 연결의 출구가 다시 생길 길이 없다. 그래서 코드 선택은 호출자에게 아무
        // 지시도 못 준다(관측 불가) — `CONFLICT` 인 것은 「나가면 안 될 답장이 샜다」를 한 문구로 잡기 위한
        // 시험용 표식이다([`CALLER_ALREADY_GONE`]).
        if self.roster.sink_for_conn(self.origin).is_none() {
            return settled(CommandReply::err(
                request_id,
                no_retry(ErrorCode::Conflict, CALLER_ALREADY_GONE),
            ));
        }

        let outcome = match self.roster.sink_of(&env.owner) {
            // ★찢어진 창★ — 명부 조회는 `Available` 을 답했는데 그 사이 주인이 끊겼다. `route` 는 조회와
            //   전달 사이에 명부 잠금을 **일부러** 놓으므로 이 경합은 설계된 잔여다.
            //   답은 `OUTCOME_UNKNOWN` 이다: 주인은 다시 붙을 수 있으므로 같은 id 재질의가 **안전하면서
            //   쓸모도 있다**(ADR-0153 의 자). ★`UNKNOWN_COMMAND`·`OWNER_UNAVAILABLE` 로 접지 말 것 —
            //   ADR-0148 이 못 박았고 새 코드도 만들지 않는다★.
            None => Err(no_retry(
                ErrorCode::OutcomeUnknown,
                format!("the owner of '{name}' went away while the envelope was being handed over"),
            )),
            Some(sink) => {
                let written = match event_json(&AgentEvent::CommandRequest { envelope: env }) {
                    // 큐 포화·닫힘 — 봉투는 나가지 못했지만 큐는 비워질 수 있어 같은 id 재질의가 쓸모 있다.
                    Some(text) => sink.try_send(Frame::Text(text)).map_err(|_| {
                        no_retry(
                            ErrorCode::OutcomeUnknown,
                            format!("the owner of '{name}' could not take the envelope right now"),
                        )
                    }),
                    // 직렬화 실패는 우리 쪽 결함이고 기다린다고 달라지지 않는다 — 쓸모 항이 거짓(ADR-0153).
                    None => Err(no_retry(
                        ErrorCode::Internal,
                        format!("this daemon could not encode the envelope for '{name}'"),
                    )),
                };
                // 여기서 사본이 떨어진다 — 받아서 즉시 쓰고 버린다(ADR-0148).
                drop(sink);
                written
            }
        };

        if let Err(error) = outcome {
            // ★자리를 여기서 되돌리지 않는다★ — 자리는 [`deliver`] 것이고, 그쪽이 이 답장을 내보낸 뒤
            //   발권을 대조해 놓는다. 여기서 키로 집으면 그 사이 같은 키에 앉은 **다른 왕복**을 지운다.
            return settled(CommandReply::err(request_id, error));
        }

        settled_later(rx, request_id)
    }
}

/// 답장 자리를 기다린다. 자리가 **답 없이 사라지면**(원 연결 소멸) 그것도 결말 하나로 접는다.
fn settled_later(
    rx: oneshot::Receiver<CommandReply>,
    request_id: RequestId,
) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(async move {
        rx.await.unwrap_or_else(|_| {
            CommandReply::err(
                request_id,
                no_retry(
                    ErrorCode::OutcomeUnknown,
                    "the caller's connection went away before the owner answered",
                ),
            )
        })
    })
}

/// 왕복 하나를 통째로 돈다 — 배달하고, 답장을 원 연결에 실어 준다.
///
/// ★연결 태스크 **밖**에서 돌려야 한다★(TRD §3-6): 한 연결의 프레임은 그 연결의 읽기 루프가 한 장씩
/// 끝까지 기다려 처리하므로, 이 왕복을 그 루프 안에서 기다리면 **주인이 곧 그 연결일 때 자기 답을 자기가
/// 못 꺼낸다**(셸이 자기 이름을 얹고 자기가 부르는 경로가 그것이다 — self-deadlock). 부르는 쪽이
/// `dispatch` 에서 이 future 를 spawn 하는 이유가 그 하나다.
///
/// ★답장은 **원 연결을 다시 조회해** 내보낸다★ — 출구 사본을 왕복 너머로 들지 않기 때문이다(모듈 헤더).
/// 그 사이 원 연결이 끊겼으면 낼 곳이 없고, 그것이 정상 종료다.
///
/// ★내 표는 비어 있다 — 그래서 1단계는 항상 미스다★: 데몬이 스스로 답하는 버스 명령은 아직 없다.
/// 데몬의 `agent.*` 표(`control::commands::make_daemon_table`)를 여기 꽂지 **않는** 이유는 그 표의
/// 핸들러가 프로필 락을 쥔 채 디스크를 쓰는 blocking 계약이라(그 함수 doc) async 태스크에서 그대로 부르면
/// 런타임 워커를 막고, 게다가 그 표는 입구 검문(`CommandTable::check_args`)을 거치는 제어 표면용이라
/// 배선 경로에서 그대로 부르면 검문 없이 도는 두 번째 입구가 생기기 때문이다. 그 둘을 푸는 것은 별건이다.
pub async fn deliver(
    roster: CommandRoster,
    deliveries: CommandDeliveries,
    origin: ConnId,
    envelope: CommandEnvelope,
) {
    let request_id = envelope.request_id;
    let name = sanitize_for_log(&envelope.name);

    // ★무엇을 하기 전에 자리부터 연다★(모듈 헤더 규칙 1) — `route` 의 3단계 중 **어느 단계가 답을 만들든**
    //   그 답이 자리 하나를 깔고 앉게 하려면 여기여야 한다. 예전엔 2단계(전달)만 자리를 열어서, 1단계(내
    //   표)와 3단계(주인 부재)의 답장이 표를 아예 안 거치고 나갔다 — 진행 중인 왕복의 키로 두 번째 프레임을
    //   내는 결정적 경로였다(경합 없이도 난다).
    let (rx, token) = match deliveries.open(request_id, origin, &envelope.name, &envelope.args) {
        Seat::Opened { rx, token } => (rx, token),
        // 답장은 먼저 열린 그 왕복이 낸다 — 여기서 내면 같은 키에 둘이 된다(근거 = [`Seat::Coalesced`]).
        Seat::Coalesced => {
            tracing::debug!(
                conn = origin,
                %request_id,
                %name,
                "같은 연결의 같은 요청 — 진행 중인 왕복에 합쳤다(답장은 그쪽이 낸다)"
            );
            return;
        }
        // 아래 셋은 자리를 못 얻었지만 **답은 낸다** — 그 판정이 이 질의의 유일한 답이다(근거 = [`Seat`]).
        Seat::Conflict => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::RequestIdConflict,
                        format!(
                            "request_id {request_id} is already in flight for a different command"
                        ),
                    ),
                ),
            );
            return;
        }
        Seat::Taken { holder } => {
            // ★임자가 아직 붙어 있나로 갈린다 — 갈리는 것은 **코드가 말하는 사실**뿐이다★
            //
            //   지시는 두 갈래 다 `Never` 로 같다([`no_retry`]) — 그래서 여기서 고르는 것은 「호출자가
            //   무엇을 할까」가 아니라 「무슨 일이 벌어졌다고 말할까」다(ADR-0153 의 확실성 축).
            //   · 붙어 있다 = 살아 있는 남의 왕복과 키가 겹쳤다. 우리는 **아무것도 실행하지 않았고** 그
            //     사실이 확실하다 → `REQUEST_ID_CONFLICT`. 원인도 정확히 그 한 줄이다.
            //   · 없다 = 그 자리 임자는 이미 떠났다. 십중팔구 **같은 클라이언트가 끊겼다 다시 붙어** 그
            //     번호로 다시 묻는 중인데(클라이언트에는 재접속을 건너 사는 신분이 없어 연결 id 로는
            //     남남으로 보인다), 그 앞선 왕복의 봉투는 **이미 주인에게 갔을 수 있다**. 적용 여부가
            //     불명이므로 `OUTCOME_UNKNOWN` 이 사실이다.
            //   ★둘을 `REQUEST_ID_CONFLICT` 로 뭉치지 말 것★ — 그러면 적용됐을지도 모르는 조작을
            //   「아무 일도 없었다」로 적어 보내는 것이고, 그건 진단이 아니라 오보다.
            let reply = if roster.sink_for_conn(holder).is_some() {
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::RequestIdConflict,
                        format!(
                            "request_id {request_id} is already in flight for another connection"
                        ),
                    ),
                )
            } else {
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        format!(
                            "request_id {request_id} is still held by a round trip whose caller is gone"
                        ),
                    ),
                )
            };
            send_reply(&roster, origin, &name, reply);
            return;
        }
        Seat::Closed => {
            send_reply(
                &roster,
                origin,
                &name,
                CommandReply::err(
                    request_id,
                    no_retry(
                        ErrorCode::OutcomeUnknown,
                        "the daemon is shutting down and no longer takes commands",
                    ),
                ),
            );
            return;
        }
    };

    let link = OwnerLink {
        roster: &roster,
        origin,
        waiter: Mutex::new(Some(rx)),
    };
    // 홉마다 같은 3단계를 돈다 — 특별 케이스를 두지 않는다(ADR-0149 결정 3).
    let reply = route(&CommandTable::new(&[]), &roster, &link, envelope).await;

    send_reply(&roster, origin, &name, reply);
    // ★프레임이 나간 **뒤에** 놓는다★(모듈 헤더 규칙 2) — 그 전에 놓으면 재질의가 봉투를 한 번 더 보낸다.
    //   발권으로 집으므로, 이 자리가 이미 다른 왕복으로 갈렸다면 이 호출은 아무것도 안 한다.
    deliveries.release(request_id, token);
}

/// 답장 한 장을 **원 연결을 다시 조회해** 내보낸다 — 출구 사본을 왕복 너머로 들지 않기 때문이다(모듈 헤더).
///
/// ★부르는 자리마다 「이 질의의 유일한 답」이다★ — [`deliver`] 의 갈래 어디서 불려도 그 갈래는 곧바로
/// 끝난다. 두 번 부르는 갈래를 만들지 말 것.
///
/// ★그리고 이 자리가 재시도 지시를 내리는 **유일한 출구**다★ — 근거는 아래 그 줄.
fn send_reply(roster: &CommandRoster, origin: ConnId, name: &str, mut reply: CommandReply) {
    let request_id = reply.request_id;

    // ★출처를 안 가리고 **여기서 한 번** 내린다★: 이 데몬이 호출자에게 내보내는 실패 답은 자기가 지은
    //   것([`no_retry`])이든 `route` 의 패닉 그물이 지은 것이든 **주인이 보내 온 것을 중계하는 것**이든
    //   전부 `Never` 로 나간다.
    //   ★중계 홉이 지시를 내리는 것은 계약이 이미 상정한 동작이다★ — 그래서 코드 칸과 `retry` 칸이 따로
    //   있다: 확실성 서술(무슨 일이 있었나)은 원래 답한 쪽 것이고, 지시(그래서 뭘 해라)는 그 지시를 감당할
    //   장치를 가진 홉의 몫이다. 완료분을 재생해 줄 dedup 이 서기 전까지 그 책임은 이 홉에 있다 —
    //   여기서 안 내리면 「같은 번호로 다시 물어라」가 **재실행**을 부른다(사유 전문·재개 조건 = [`no_retry`]).
    //   ★코드 어휘는 손대지 않는다★ — 내리는 것은 지시뿐이고 확실성 서술은 원문 그대로 간다. 그래서
    //   `set_retry` 가 아니라 `set_retry_for_relay` 다: 앞의 것은 **원문 사본을 통째로 버려** 주인이 실어
    //   보낸 미지 코드와 계약 밖 필드까지 지운다(중계 홉이 남의 어휘를 지우는 것 = `received` 문서가
    //   금하는 바로 그것). 뒤의 것은 사본의 `retry` 칸만 덮어 지시는 실제로 내려가고 어휘는 산다.
    //   ★이 자리를 늘리지 말 것★: 두 군데서 내리면 다음 세션이 한쪽만 고치고 나머지가 조용히 샌다.
    if let Err(error) = &mut reply.outcome {
        error.set_retry_for_relay(RetryMode::Never);
    }
    let Some(sink) = roster.sink_for_conn(origin) else {
        tracing::debug!(
            conn = origin,
            %request_id,
            %name,
            "명령 답장을 낼 곳이 없다 — 물어본 연결이 이미 끊겼다"
        );
        return;
    };
    let Some(text) = event_json(&AgentEvent::CommandReply { reply }) else {
        // `event_json` 이 이미 error! 를 남겼다 — 여기서 한 줄 더 내면 같은 사건이 두 번 적힌다.
        return;
    };
    // ★이 실패를 조용히 삼키지 않는다★: 이것은 그 `request_id` 의 **유일한** 답장이고, 결말은 이미 자리를
    //   거둬 갔으므로 되돌릴 길이 없다 — 삼키면 호출자는 마감도 안 걸린 채 영영 답을 못 받고, 서버 쪽엔
    //   그 사건의 흔적이 아무 데도 안 남는다(로깅 컨벤션 「계측 의무」 · 안티패턴 「무로그 삼킴」).
    //   레벨이 `warn!` 인 이유: 데이터 위험은 없지만(안전) **정상이 아니다** — 큐 포화·닫힘이면 네트워크
    //   행이 그 연결을 곧 닫으므로 호출자는 어차피 끊긴다.
    if sink.try_send(Frame::Text(text)).is_err() {
        tracing::warn!(
            conn = origin,
            %request_id,
            %name,
            "명령 답장을 그 연결에 넣지 못했다 — 이 왕복의 유일한 답장이 사라진다(큐 포화·닫힘)"
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_doubles::FakeFrameSink;
    use engram_dashboard_command::{CommandDecl, OwnerToken};
    use engram_dashboard_net::frame_port::FrameSink;
    use serde_json::json;
    use tokio::sync::mpsc;

    /// 손으로 미는 시계 — 마감 갈래를 실시간 대기 없이 결정적으로 재게 한다.
    pub(crate) struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(Instant::now())))
        }

        pub(crate) fn advance(&self, by: Duration) {
            let mut now = self.0.lock().expect("manual clock poisoned");
            *now += by;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().expect("manual clock poisoned")
        }
    }

    pub(crate) fn deliveries_with(clock: Arc<ManualClock>) -> CommandDeliveries {
        CommandDeliveries::with_clock(clock, Duration::from_secs(10))
    }

    fn sink_with_inbox() -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        sink_with_capacity(16)
    }

    /// 용량을 정해 만드는 출구 — 작게 잡으면 **포화**(넣지 못하는 상태)를 만들 수 있다.
    fn sink_with_capacity(capacity: usize) -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel::<Frame>(capacity);
        (Arc::new(FakeFrameSink::new(tx)), rx)
    }

    /// 본문이 도는 동안 난 WARN 이벤트의 필드를 모은다.
    ///
    /// ★`with_default` 는 **이 스레드에만** 걸리므로 런타임을 이 안에서 만들어 몸통을 통째로 감싼다★ —
    /// 배달은 spawn 된 태스크에서 도는데, 현재 스레드 런타임이면 그 태스크도 같은 스레드에서 돌아 로그가
    /// 이 구독자에게 온다. 그래서 이 함수는 `async` 가 아니다(런타임 안에서 부르면 런타임 중첩이다).
    fn capture_warns<F: std::future::Future<Output = ()>>(body: F) -> Vec<String> {
        use tracing::subscriber;

        struct Collector {
            lines: Arc<Mutex<Vec<String>>>,
        }
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", f.name(), v));
            }
        }
        impl subscriber::Subscriber for Collector {
            fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
                *m.level() == tracing::Level::WARN
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut buf = String::new();
                event.record(&mut Visit(&mut buf));
                self.lines.lock().expect("lines poisoned").push(buf);
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let collector = Collector {
            lines: lines.clone(),
        };
        subscriber::with_default(collector, || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("현재 스레드 런타임")
                .block_on(body);
        });
        let captured = lines.lock().expect("lines poisoned");
        captured.clone()
    }

    fn envelope(name: &str, owner: &OwnerToken) -> CommandEnvelope {
        CommandEnvelope {
            name: name.to_string(),
            request_id: RequestId::new(),
            owner: owner.clone(),
            proto_ver: 7,
            args: json!({ "window": "main" }),
        }
    }

    /// 프레임 하나를 이벤트로 읽는다.
    fn event_from(frame: Frame) -> AgentEvent {
        match frame {
            Frame::Text(text) => serde_json::from_str(&text).expect("이벤트 디코드"),
            other => panic!("Text 여야 함: {other:?}"),
        }
    }

    /// 배달 태스크의 끝을 기다린다 — 상한 근거는 [`next_event`] 와 같다.
    async fn finished(handle: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("배달 태스크가 안 끝난다")
            .expect("배달 태스크");
    }

    /// ★기다림에 상한을 둔다 — 배달이 깨지면 **hang 이 아니라 실패**여야 한다★
    ///
    /// 이 시험들이 재는 것은 「그 프레임이 그 연결로 오는가」라, 답장이 엉뚱한 연결로 새는 구현에서는 여기가
    /// 영영 안 깨어난다. 무기한 대기는 러너에서 **멈춘 것처럼** 보이고 어느 단언이 틀렸는지도 안 알려 준다.
    /// ★상한은 판정 기준이 아니다★ — 정상 경로는 즉시 깨어나므로 이 값은 「깨졌다」의 관측 수단일 뿐이고,
    /// 느린 러너에서 오탐이 나지 않게 넉넉히 잡는다.
    async fn next_event(rx: &mut mpsc::Receiver<Frame>) -> AgentEvent {
        let frame = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("프레임을 기다리다 상한을 넘겼다 — 배달이 이 연결로 오지 않는다")
            .expect("프레임 하나");
        event_from(frame)
    }

    /// 주인 하나가 이름 하나를 얹고 붙어 있는 명부 + 그 주인 앞으로 나가는 프레임을 받는 곳.
    fn roster_with_owner(conn_id: ConnId, name: &str) -> (CommandRoster, mpsc::Receiver<Frame>) {
        let roster = CommandRoster::new();
        let (sink, inbox) = sink_with_inbox();
        roster.attach(conn_id, &sink);
        roster
            .register(
                conn_id,
                vec![CommandDecl {
                    name: name.to_string(),
                    help: "{}".to_string(),
                }],
            )
            .expect("등록");
        (roster, inbox)
    }

    /// 물어보는 쪽 연결을 명부에 세운다 — 답장은 그 연결의 출구로 나간다.
    fn attach_caller(roster: &CommandRoster, conn_id: ConnId) -> mpsc::Receiver<Frame> {
        let (sink, inbox) = sink_with_inbox();
        roster.attach(conn_id, &sink);
        inbox
    }

    // ── 갈래 ① 정상 왕복 ─────────────────────────────────────────────────────────

    /// ★답장이 **원래 물어본 연결**로 돌아온다★ — 다른 연결로 새면 그쪽은 자기가 안 시킨 것의 결과를
    /// 받고, 물어본 쪽은 마감까지 매달린다.
    /// 겉봉의 주인 토큰이 **명부가 답한 값으로 덮여** 나가는 것까지 함께 잰다(TRD §3-8 의 2단 배달에서
    /// 중간 홉이 갈라 주는 근거가 그 칸이다).
    #[tokio::test]
    async fn a_command_reaches_its_owner_and_the_answer_returns_to_the_caller() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let mut bystander_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(ManualClock::new());
        // ★목적지 칸은 부르는 쪽이 아무 값이나 적어 온다★ — 지목은 데몬 몫이다(ADR-0148).
        let env = envelope("tab.create", &OwnerToken::new("whatever-the-caller-thinks"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));

        match next_event(&mut owner_inbox).await {
            AgentEvent::CommandRequest { envelope } => {
                assert_eq!(envelope.request_id, request_id, "id 는 전 구간 동일");
                assert_eq!(
                    envelope.owner,
                    CommandRoster::owner_of(1),
                    "명부가 답한 주인이 겉봉에 실린다"
                );
                assert_eq!(
                    envelope.args,
                    json!({ "window": "main" }),
                    "속은 그대로 통과"
                );
            }
            other => panic!("주인은 CommandRequest 를 받는다: {other:?}"),
        }
        assert_eq!(
            deliveries.in_flight(),
            1,
            "답을 기다리는 동안 표에 앉아 있다"
        );

        assert!(
            deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            "진행 중인 왕복에 붙는다"
        );
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                assert_eq!(reply.outcome, Ok(json!({ "tab": 4 })), "주인의 결말 그대로");
            }
            other => panic!("물어본 연결은 CommandReply 를 받는다: {other:?}"),
        }
        assert!(
            bystander_inbox.try_recv().is_err(),
            "남의 왕복이 다른 연결로 새면 안 된다"
        );
        assert_eq!(deliveries.in_flight(), 0, "끝난 왕복은 표에서 사라진다");
    }

    // ── 갈래 ② 주인 부재 ─────────────────────────────────────────────────────────

    /// 명부에 없는 이름 — 끊긴 주인의 이름과 **같은 답**을 받는다(ADR-0144 가 감수한 구분 손실).
    #[tokio::test]
    async fn a_command_nobody_owns_is_answered_unknown_command() {
        let roster = CommandRoster::new();
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("theme.set", &OwnerToken::new("nobody"));
        let request_id = env.request_id;

        deliver(roster, deliveries.clone(), 2, env).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("주인이 없다");
                assert_eq!(err.code(), ErrorCode::UnknownCommand);
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert_eq!(deliveries.in_flight(), 0, "안 열린 자리가 남으면 안 된다");
    }

    // ── 갈래 ③ 찢어진 창 ─────────────────────────────────────────────────────────

    /// ★조회는 `Available` 인데 닿는 길이 없다★ — `route` 가 조회와 전달 사이에 명부 잠금을 일부러 놓아
    /// 생기는 설계된 잔여다(ADR-0148). 그 상태를 결정적으로 만들려고 표에 든 주인 토큰만 갈아 끼운다:
    /// 명부는 옛 토큰 앞으로 이름을 들고 있고, 그 토큰으로는 이제 아무 출구도 안 나온다.
    ///
    /// ★`UNKNOWN_COMMAND` 가 아닌 것이 요점이다★ — 조회가 방금 주인이 있다고 답했으므로 그 코드는 거짓
    /// 확신이다 — 조회가 방금 주인이 있다고 답했고, 그 주인은 곧 다시 붙을 수 있다.
    #[tokio::test]
    async fn a_command_whose_owner_vanished_mid_handover_is_outcome_unknown() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        roster.overwrite_stored_owner(1, OwnerToken::new("someone-else"));
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        deliver(roster, deliveries.clone(), 2, env).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("닿는 길이 없다");
                assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
                assert_eq!(
                    err.retry(),
                    engram_dashboard_command::RetryMode::Never,
                    "dedup 저장소가 없으므로 이 경로는 재시도를 지시하지 않는다"
                );
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "닿는 길이 없으면 아무 데도 안 나간다"
        );
        assert_eq!(deliveries.in_flight(), 0, "열었던 자리를 도로 거둬야 한다");
    }

    // ── 갈래 ④ 마감 초과 ─────────────────────────────────────────────────────────

    /// ★마감이 지나면 답장이 **정확히 한 번** 나가고, 늦게 온 결말은 두 번째 답장을 만들지 못한다★
    ///
    /// 늦은 결말을 버리는 자리는 표의 단일 소유권 지점 하나다 — 만료가 자리를 거뒀으므로 그 뒤의
    /// `complete` 는 빈손을 받는다.
    #[tokio::test]
    async fn a_deadline_answers_once_and_a_late_outcome_cannot_answer_again() {
        let clock = ManualClock::new();
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        // 봉투가 나간 것을 본 시점부터 그 왕복은 답을 기다리는 중이다.
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        clock.advance(Duration::from_secs(9));
        assert_eq!(deliveries.expire(), 0, "마감 전에는 거두지 않는다");
        clock.advance(Duration::from_secs(2));
        assert_eq!(deliveries.expire(), 1, "마감이 지나면 거둔다");
        finished(round_trip).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("마감 초과");
                assert_eq!(err.code(), ErrorCode::Timeout);
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }

        assert!(
            !deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))),
            "붙일 자리가 없다"
        );
        assert!(
            caller_inbox.try_recv().is_err(),
            "늦게 온 결말이 두 번째 답장이 되면 안 된다"
        );
    }

    // ── 끊김 정리 ────────────────────────────────────────────────────────────────

    /// 물어본 연결이 사라지면 그 연결이 물고 있던 항목이 전부 없어지고, 기다리던 배달도 끝난다.
    ///
    /// ★항목이 남는 경로가 없어야 한다★ — 남으면 아무도 안 기다리는 왕복이 마감까지 표에 앉는다.
    #[tokio::test]
    async fn a_caller_that_disconnects_takes_its_pending_round_trips_with_it() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let other = envelope("tab.create", &OwnerToken::new("whatever"));

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, other));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 운영 순서 그대로 — 명부에서 먼저 빠지고(그 연결의 출구가 사라진다) 상관 표가 뒤따른다.
        roster.detach(2);
        assert_eq!(deliveries.drop_origin(2), 1, "그 연결의 왕복을 거둔다");

        finished(round_trip).await;
        assert_eq!(deliveries.in_flight(), 0);
    }

    /// 남의 연결이 끊겨도 내 왕복은 안 건드린다 — 아니면 멀쩡한 호출자가 이유 없이 답을 잃는다.
    #[tokio::test]
    async fn another_connections_cleanup_does_not_touch_my_round_trip() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        assert_eq!(deliveries.drop_origin(7), 0, "남의 연결 정리");
        assert_eq!(deliveries.in_flight(), 1, "내 왕복은 그대로다");

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
        finished(round_trip).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
    }

    // ── 내준 사본을 왕복 너머로 들지 않는다(ADR-0148) ─────────────────────────────

    /// ★주인의 출구 사본이 왕복 동안 살아 있으면 안 된다★
    ///
    /// 대기표에 사본을 넣는 구현은 여기서 빨개진다 — 봉투를 쓴 뒤 주인이 끊겼는데도 그 사본이 남아 있어
    /// 강참조가 안 풀린다(그러면 끊긴 연결의 송신 큐가 명부보다 오래 살아 연결이 샌다).
    #[tokio::test]
    async fn the_owners_sink_is_released_as_soon_as_the_envelope_is_written() {
        let roster = CommandRoster::new();
        let (owner_sink, mut owner_inbox) = sink_with_inbox();
        let released = Arc::downgrade(&owner_sink);
        roster.attach(1, &owner_sink);
        roster
            .register(
                1,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                }],
            )
            .expect("등록");
        drop(owner_sink); // 이제 명부가 유일한 강참조다.
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 봉투는 이미 나갔고 답장은 아직이다 — 이 상태에서 주인이 끊긴다.
        roster.detach(1);
        drop(owner_inbox);

        assert!(
            released.upgrade().is_none(),
            "배달이 주인의 출구 사본을 왕복 너머로 들고 있다"
        );

        // 왕복은 여전히 마감이 거둔다 — 사본을 놓았다고 답장이 사라지지 않는다.
        assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
        finished(round_trip).await;
    }

    // ── 같은 id 가 겹칠 때(FIX 1) ────────────────────────────────────────────────
    //
    // ★여기서 재는 것은 표가 아니라 **밖으로 나간 프레임**이다★: 표의 자리 수만 보면 「자리는 하나인데
    // 답장은 둘」이라는 실제 위반이 통째로 안 보인다. 그 둘은 호출자의 pending 을 먼저 온 것으로 풀고
    // 나머지를 고아로 만든다(TRD §4-⑤).

    /// ★같은 연결의 같은 `request_id` = 재질의 → 답장은 **하나뿐**★
    ///
    /// 두 번째 요청은 진행 중인 왕복에 합쳐지고, 나중에 도착한 주인의 결말 하나가 둘을 다 답한다.
    /// 합침이 깨지면 두 번째 요청이 그 자리에서 오류를 한 장 내고, 뒤이어 진짜 결말이 같은 키로 또 한 장 나간다.
    #[tokio::test]
    async fn a_repeat_of_an_inflight_request_id_from_the_same_caller_yields_exactly_one_reply() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        // 같은 연결이 같은 키로 다시 낸다 — 이 태스크는 아무것도 내보내지 않고 끝나야 한다.
        let b = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, again));
        finished(b).await;

        assert!(
            caller_inbox.try_recv().is_err(),
            "합쳐진 요청이 자기 답장을 따로 내면 같은 키의 답장이 둘이 된다"
        );
        assert_eq!(deliveries.in_flight(), 1, "자리는 처음 하나 그대로다");
        assert!(
            owner_inbox.try_recv().is_err(),
            "봉투도 두 번 나가면 안 된다 — 주인이 같은 조작을 두 번 한다"
        );

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))));
        finished(a).await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                assert_eq!(reply.outcome, Ok(json!({ "tab": 4 })));
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert!(
            caller_inbox.try_recv().is_err(),
            "★이 왕복이 낸 답장은 정확히 하나다★"
        );
    }

    /// **다른** 연결이 쓰고 있는 id 로 물으면 그쪽은 답을 받아야 한다 — 안 그러면 매달린다.
    ///
    /// 그 답장은 먼저 열린 왕복의 답장과 **다른 연결**로 가므로 호출자별 「답장 하나」가 지켜진다.
    /// (합침을 연결 구분 없이 하면 이 호출자는 자리도 답장도 없이 영영 기다린다.)
    #[tokio::test]
    async fn another_caller_reusing_an_inflight_request_id_is_told_so_on_its_own_connection() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut first_inbox = attach_caller(&roster, 2);
        let mut second_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(ManualClock::new());
        let mine = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = mine.request_id;
        let theirs = CommandEnvelope {
            request_id,
            ..mine.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, mine));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            3,
            theirs,
        )))
        .await;

        match next_event(&mut second_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                assert_eq!(
                    reply.outcome.expect_err("남의 왕복이 쥔 키").code(),
                    ErrorCode::RequestIdConflict
                );
            }
            other => panic!("두 번째 호출자도 답을 받아야 한다: {other:?}"),
        }
        assert!(
            first_inbox.try_recv().is_err(),
            "남의 충돌이 먼저 열린 왕복의 답장 자리를 건드리면 안 된다"
        );

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
        finished(a).await;
        assert!(matches!(
            next_event(&mut first_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
        assert!(
            second_inbox.try_recv().is_err(),
            "먼저 열린 왕복의 결말이 남의 연결로 새면 안 된다"
        );
    }

    /// ★`route` 의 **다른 단계**가 만든 답장도 자리를 거쳐야 한다★ — 경합 없이 나는 결함이었다.
    ///
    /// 예전엔 자리를 2단계(전달) 안에서 열어서, 3단계(주인 부재 → `UNKNOWN_COMMAND`)와 1단계(내 표)의
    /// 답장은 표를 아예 안 보고 나갔다. 그래서 **주인이 왕복 도중 끊긴 뒤 같은 요청을 다시 물으면** 그
    /// 재질의가 3단계로 떨어져 `UNKNOWN_COMMAND` 를 그 키로 한 장 내보내고, 뒤이어 진짜 결말이 또 한 장
    /// 나갔다. 지금은 자리를 `route` **앞에서** 열므로 그 재질의는 진행 중인 왕복에 합쳐져 아무것도 안 낸다.
    #[tokio::test]
    async fn a_reply_from_another_routing_stage_cannot_bypass_an_inflight_seat() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 주인이 왕복 도중 끊긴다 — 이제 그 이름은 명부에 없고, 재질의는 3단계로 떨어질 처지다.
        roster.detach(1);
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            2,
            again,
        )))
        .await;

        assert!(
            caller_inbox.try_recv().is_err(),
            "3단계의 UNKNOWN_COMMAND 가 진행 중인 왕복의 키로 새 나가면 안 된다"
        );
        assert_eq!(deliveries.in_flight(), 1, "자리는 처음 하나 그대로다");

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))));
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.outcome, Ok(json!({ "tab": 4 })), "주인의 결말 그대로")
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert!(
            caller_inbox.try_recv().is_err(),
            "★이 왕복이 낸 답장은 정확히 하나다★"
        );
    }

    /// ★결말이 붙은 **직후** 온 재질의가 봉투를 한 번 더 보내면 안 된다★
    ///
    /// 결말이 자리를 지우고 프레임은 그 뒤에 나가던 구조에서는 그 사이가 빈 표였다: 재질의가 새 자리를 열고
    /// 같은 명령을 **다시 실행시킨다.** 그 창을 결정적으로 겨눈다 — `complete` 뒤 배달 태스크가 깨어나기
    /// **전에** 두 번째 배달의 첫 폴링을 넣는다(현재 스레드 런타임이라 그 사이에 남이 못 낀다).
    #[tokio::test]
    async fn a_repeat_arriving_before_the_reply_is_sent_does_not_forward_again() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let again = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 결말은 붙었지만 프레임은 아직 안 나갔다 — 그 태스크는 아직 깨어나지 못했다.
        assert!(deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))));
        // 상한을 둔다 — 이 회귀가 살아나면 두 번째 배달이 제 자리를 열고 오지 않을 답을 영원히 기다린다.
        tokio::time::timeout(
            Duration::from_secs(10),
            deliver(roster.clone(), deliveries.clone(), 2, again),
        )
        .await
        .expect("재질의가 제 왕복을 열어 매달렸다 — 합쳐졌어야 한다");

        assert!(
            owner_inbox.try_recv().is_err(),
            "결말과 프레임 사이에 온 재질의가 같은 조작을 한 번 더 시키면 안 된다"
        );

        finished(a).await;
        assert!(matches!(
            next_event(&mut caller_inbox).await,
            AgentEvent::CommandReply { .. }
        ));
        assert!(caller_inbox.try_recv().is_err(), "★프레임은 정확히 한 장★");
        assert_eq!(deliveries.in_flight(), 0, "프레임이 나간 뒤 자리가 풀린다");
    }

    /// ★같은 id 인데 **딴 요청**이면 합치지 않고 반려한다★(TRD §4-⑥ 셋째 다리)
    ///
    /// 합치면 그 요청은 **한 번도 전달되지 않고** 답장도 못 받은 채, 그 키로 나가는 한 장이 **앞 요청의
    /// 결과**를 실어 간다 — 호출자는 엉뚱한 payload 로 자기 promise 를 푼다.
    #[tokio::test]
    async fn a_different_command_under_an_inflight_request_id_is_refused_not_folded() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        // 같은 키, 다른 이름 — 부르는 쪽이 id 를 재사용한 **딴 조작**이다.
        let other = CommandEnvelope {
            request_id,
            name: "theme.set".to_string(),
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            2,
            other,
        )))
        .await;

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(
                    reply.outcome.expect_err("딴 요청이 같은 키를 썼다").code(),
                    ErrorCode::RequestIdConflict,
                    "합쳐서 남의 결과를 주면 안 된다 — 그 자리에서 반려한다"
                );
            }
            other => panic!("딴 요청도 답을 받아야 한다: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "반려한 요청을 주인에게 보내면 안 된다"
        );

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))));
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "먼저 열린 왕복은 제 결과를 그대로 받는다"
                )
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
    }

    /// ★마감이 지났는데 아직 안 걷힌 자리는 **다음 시도를 흡수하면 안 된다**★
    ///
    /// 수거기는 1초마다 도는데 재시도는 마감 직후에 온다. 흡수하면 그 시도는 한 번도 전달되지 않고,
    /// 호출자는 자기가 다시 묻기 **전에** 정해진 이전 시도의 `TIMEOUT` 을 답으로 받는다 — 재시도가 조용한
    /// no-op 이 된다. ★수거기를 일부러 안 띄운다★: 흡수를 막는 것이 수거 주기가 아니라 질의 경로 자신이어야
    /// 한다는 것이 이 시험의 요점이다.
    ///
    /// ★아래가 축복하는 것 — 프레임 **두 장**과 그 **순서**★: 한 시도에 한 장씩이라 두 장이 맞고, 이 시험은
    /// 옛 시도의 `TIMEOUT` 이 **재시도가 주인에게 전달된 뒤에** 같은 연결에 쓰이는 순서를 그대로 단언한다
    /// (사유 = `CommandDeliveries::open` 의 인라인 정산 주석). 즉 상관 키가 같은 두 프레임이 「옛 답 →
    /// 새 답」 순으로 도착할 수 있다는 뜻이고, 그것이 이 설계에서 정상이다.
    #[tokio::test]
    async fn a_retry_after_the_deadline_is_not_absorbed_by_the_unswept_seat() {
        let clock = ManualClock::new();
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(clock.clone());
        let first = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = first.request_id;
        let retry = CommandEnvelope {
            request_id,
            ..first.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, first));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 마감은 지났지만 아무도 안 걷었다 — 계약이 시킨 그대로의 재질의가 이 상태에서 도착한다.
        clock.advance(Duration::from_secs(11));
        let b = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, retry));

        assert!(
            matches!(
                next_event(&mut owner_inbox).await,
                AgentEvent::CommandRequest { .. }
            ),
            "재시도는 제 왕복을 얻어 주인에게 다시 가야 한다"
        );
        // 첫 시도는 결말 없이 접혔다 — 그 한 장이 먼저 나간다.
        finished(a).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(
                    reply.outcome.expect_err("첫 시도는 마감").code(),
                    ErrorCode::Timeout
                );
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({ "tab": 4 }))));
        finished(b).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(
                    reply.outcome,
                    Ok(json!({ "tab": 4 })),
                    "재시도는 제 결과를 받는다 — 앞 시도의 TIMEOUT 을 물려받지 않는다"
                )
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert_eq!(deliveries.in_flight(), 0, "남은 자리가 없어야 한다");
    }

    /// ★재접속한 호출자에게 「아무 일도 없었다」고 말하지 않는다★ — 그 봉투는 이미 갔을 수 있다.
    ///
    /// 클라이언트에는 재접속을 건너 사는 신분이 없어, 반쯤 끊겼다 다시 붙어 같은 id 로 다시 묻는 그 요청이
    /// 연결 id 로는 **남남**으로 보인다. 자리 임자가 이미 떠났으면 그 왕복의 적용 여부는 **불명**이므로
    /// 확실성도 지시도 그렇게 답한다(ADR-0153).
    #[tokio::test]
    async fn a_reconnected_caller_retrying_its_id_is_not_told_nothing_happened() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _first_inbox = attach_caller(&roster, 2);
        let mut second_inbox = attach_caller(&roster, 3);
        let deliveries = deliveries_with(ManualClock::new());
        let mine = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = mine.request_id;
        let after_reconnect = CommandEnvelope {
            request_id,
            ..mine.clone()
        };

        let a = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, mine));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 끊김만 관측됐고 정리(`drop_origin`)는 아직이다 — 그 사이에 재접속한 연결이 같은 id 로 다시 묻는다.
        roster.detach(2);
        finished(tokio::spawn(deliver(
            roster.clone(),
            deliveries.clone(),
            3,
            after_reconnect,
        )))
        .await;

        match next_event(&mut second_inbox).await {
            AgentEvent::CommandReply { reply } => {
                let err = reply
                    .outcome
                    .expect_err("남의 자리가 아직 그 키를 쥐고 있다");
                assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
                assert_eq!(
                    err.retry(),
                    engram_dashboard_command::RetryMode::Never,
                    "dedup 저장소가 없으므로 이 경로는 재시도를 지시하지 않는다"
                );
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
        finished(a).await;
    }

    /// ★종료 창에 도착한 명령도 **답을 받는다**★ — 안 그러면 프로세스가 죽을 때까지 매달린다.
    ///
    /// 표를 비우기만 하고 닫지 않으면 그 뒤 명령이 자리를 여는데, 그때 수거기는 이미 멈춰 마감을 볼 눈이
    /// 없다. 기다림에 상한을 두어 그 회귀가 hang 이 아니라 실패로 보이게 한다.
    #[tokio::test]
    async fn a_command_arriving_after_shutdown_is_answered_instead_of_hanging() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        assert_eq!(deliveries.drain(), 0, "비울 것 없이 표만 닫는다");

        tokio::time::timeout(
            Duration::from_secs(10),
            deliver(roster.clone(), deliveries.clone(), 2, env),
        )
        .await
        .expect("종료 뒤 도착한 명령이 답도 없이 매달린다");

        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("데몬이 내려가는 중");
                assert_eq!(
                    err.code(),
                    ErrorCode::OutcomeUnknown,
                    "적용 여부는 불명이다"
                );
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
        assert!(
            owner_inbox.try_recv().is_err(),
            "닫힌 표에서는 봉투가 나가지 않는다"
        );
        assert_eq!(deliveries.in_flight(), 0, "거절한 질의는 표에 앉지 않는다");
    }

    // ── 답장을 넣지 못한 경우(FIX 2) ─────────────────────────────────────────────

    /// ★유일한 답장을 못 넣었으면 **로그로라도 남는다**★
    ///
    /// 결말은 이미 자리를 거둬 갔으므로 되돌릴 길이 없다. 삼키면 호출자는 마감도 안 걸린 채 영영 답을 못
    /// 받고 서버엔 흔적이 하나도 안 남는다(로깅 컨벤션 「무로그 삼킴」 안티패턴).
    /// 큐를 1칸으로 잡고 미리 채워 포화를 만든다.
    #[test]
    fn a_reply_that_cannot_be_enqueued_is_reported_not_swallowed() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        // 용량 1 — 아래에서 한 장을 미리 채워 두면 답장이 들어갈 자리가 없다.
        let (caller_sink, _caller_inbox) = sink_with_capacity(1);
        roster.attach(2, &caller_sink);
        caller_sink
            .try_send(Frame::Text("occupied".into()))
            .expect("한 칸은 비어 있다");
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let logged = capture_warns(async {
            let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
            assert!(matches!(
                next_event(&mut owner_inbox).await,
                AgentEvent::CommandRequest { .. }
            ));
            assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
            finished(round_trip).await;
        });

        assert!(
            logged
                .iter()
                .any(|line| line.contains(&request_id.to_string())),
            "어느 왕복의 답장이 사라졌는지 남아야 한다: {logged:?}"
        );
    }

    // ── 이미 떠난 호출자(FIX 3) ─────────────────────────────────────────────────

    /// ★정리가 배달 태스크의 **첫 폴링보다 먼저** 돌아도 명령이 실행되면 안 된다★
    ///
    /// `dispatch` 는 배달을 떼어 내므로 그 태스크의 첫 폴링이 `on_disconnect` 보다 늦을 수 있다. 그 순서에서
    /// `drop_origin` 은 빈 표를 보고 지나가고, 뒤늦게 깨어난 태스크가 자리를 열어 봉투를 보내면 ① 이미 떠난
    /// 호출자를 위해 **부수효과 있는 명령이 실행되고** ② 그 자리는 아무도 안 거둬 마감까지 남는다.
    ///
    /// ★그 나쁜 순서를 결정적으로 만든다★: `deliver` 는 `async fn` 이라 **부를 때가 아니라 폴링될 때**
    /// 돈다. 그래서 future 를 만들어 두고 정리를 먼저 돌린 뒤에 await 하면 그 인터리브가 정확히 재현된다
    /// (spawn 으로는 첫 폴링 시점을 시험이 정할 수 없어 이 순서를 못 만든다).
    #[tokio::test]
    async fn a_command_from_a_caller_that_already_left_is_never_forwarded() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let _caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));

        // 아직 한 번도 폴링되지 않은 배달.
        let pending_delivery = deliver(roster.clone(), deliveries.clone(), 2, env);

        // 운영의 `on_disconnect` 순서 그대로 — 명부에서 먼저 빼고 자리를 거둔다.
        roster.detach(2);
        assert_eq!(
            deliveries.drop_origin(2),
            0,
            "아직 자리가 없다 — 이 빈손이 이 회귀의 출발점이다"
        );

        // ★기다림에 상한을 둔다★ — 이 회귀가 살아나면 봉투가 나가고 배달은 **영원히** 주인의 답을 기다린다
        //   (그 자리는 아무도 안 거두고, 이 시험엔 마감을 밀 수거기도 없다). 상한이 없으면 그 회귀가 실패가
        //   아니라 hang 으로 나타나 어느 단언이 틀렸는지조차 안 알려 준다([`next_event`] 와 같은 규약 —
        //   정상 경로는 폴링 한 번으로 끝나므로 이 값은 판정 기준이 아니다).
        tokio::time::timeout(Duration::from_secs(10), pending_delivery)
            .await
            .expect(
                "떠난 호출자의 배달이 안 끝난다 — 봉투가 나갔고 오지 않을 답을 기다리는 중이다",
            );

        assert!(
            owner_inbox.try_recv().is_err(),
            "떠난 호출자의 명령을 주인에게 배달하면 안 된다 — 부수효과가 그대로 일어난다"
        );
        assert_eq!(
            deliveries.in_flight(),
            0,
            "아무도 안 거둘 자리를 남기면 마감까지 표에 앉는다"
        );
    }

    // ── 종료(FIX 4) ─────────────────────────────────────────────────────────────

    /// ★종료는 유계다 — 남은 자리를 전부 답하고 그 답이 배달 태스크를 즉시 푼다★
    ///
    /// 비우지 않으면 배달 태스크는 자기 답장 자리를 기다리는데 그 **보내는 쪽이 이 표 안에** 있어, 마감
    /// (기본 10초)까지 깨어날 계기가 없다. 그동안 표도 태스크도 「종료했다」고 보고된 시점보다 오래 산다.
    /// 수거기를 **기다려** 그 사실을 관측한다 — 핸들을 버리면 이 단언이 설 자리가 없다.
    #[tokio::test]
    async fn shutdown_answers_every_outstanding_round_trip_and_stops_the_sweeper() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let (stop, stopped) = watch::channel(false);
        let sweeper = deliveries.spawn_sweeper(stopped);
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));
        assert_eq!(deliveries.in_flight(), 1);

        stop.send(true).expect("수거기가 살아 있다");
        tokio::time::timeout(Duration::from_secs(10), sweeper)
            .await
            .expect("수거기가 종료 신호에 안 멈춘다")
            .expect("수거기 태스크");

        assert_eq!(deliveries.in_flight(), 0, "남은 자리가 없어야 한다");
        // ★시계를 한 톱니도 안 밀었다★ — 마감이 아니라 종료가 푼 것이다.
        finished(round_trip).await;
        match next_event(&mut caller_inbox).await {
            AgentEvent::CommandReply { reply } => {
                assert_eq!(reply.request_id, request_id);
                let err = reply.outcome.expect_err("종료");
                assert_eq!(
                    err.code(),
                    ErrorCode::OutcomeUnknown,
                    "적용 여부는 불명이다"
                );
                assert_eq!(err.retry(), engram_dashboard_command::RetryMode::Never);
            }
            other => panic!("CommandReply 여야 함: {other:?}"),
        }
    }

    /// 비울 것이 없으면 조용하고, 두 번 비워도 답장이 두 번 나가지 않는다.
    #[test]
    fn draining_twice_answers_each_seat_once() {
        let deliveries = deliveries_with(ManualClock::new());
        let request_id = RequestId::new();
        let Seat::Opened { rx: _rx, .. } = deliveries.open(request_id, 1, "tab.create", &json!({}))
        else {
            panic!("자리를 얻어야 한다");
        };

        assert_eq!(deliveries.drain(), 1);
        assert_eq!(deliveries.drain(), 0, "두 번째 비움은 답할 것이 없다");
        assert_eq!(deliveries.in_flight(), 0);
    }

    /// ★비운 표는 **닫힌다** — 그 뒤 질의는 자리를 얻지 못하고 즉시 반려된다★(종료가 유계인 근거).
    ///
    /// 닫지 않으면 종료 창에 도착한 명령이 자리를 열고 답을 기다리는데, 그때 수거기는 이미 멈춰 있어
    /// 마감을 볼 눈이 없다 — 그 왕복은 답장 0장으로 프로세스가 죽을 때까지 매달린다.
    #[test]
    fn a_drained_table_refuses_new_round_trips() {
        let deliveries = deliveries_with(ManualClock::new());
        assert_eq!(deliveries.drain(), 0);

        assert!(
            matches!(
                deliveries.open(RequestId::new(), 1, "tab.create", &json!({})),
                Seat::Closed
            ),
            "닫힌 표는 자리를 내주지 않는다"
        );
        assert_eq!(deliveries.in_flight(), 0, "거절한 질의는 표에 앉지 않는다");
    }

    // ── 상관 표 자체 ─────────────────────────────────────────────────────────────

    /// 진행 중인 키를 덮지 않고, **무엇을 다시 물었나 → 누가 다시 물었나** 순으로 갈린다.
    ///
    /// 덮으면 먼저 열린 왕복이 조용히 자리를 잃는다. 그리고 갈래를 뭉치면 하나씩 깨진다 — 같은 요청에
    /// 오류를 내면 같은 키의 답장이 둘이 되고, 다른 연결에 아무것도 안 내면 그쪽이 매달리고, **딴 요청을
    /// 합치면 그 요청은 실행도 답장도 없이 남의 결과를 받는다**(근거 = [`Seat`]).
    #[test]
    fn a_second_open_of_the_same_request_id_splits_on_payload_then_on_who_asked() {
        let deliveries = deliveries_with(ManualClock::new());
        let request_id = RequestId::new();
        let args = json!({ "window": "main" });
        let Seat::Opened { rx: _first, .. } = deliveries.open(request_id, 1, "tab.create", &args)
        else {
            panic!("첫 왕복은 자리를 얻는다");
        };

        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args),
                Seat::Coalesced
            ),
            "같은 연결의 같은 요청은 합친다"
        );
        assert!(
            matches!(
                deliveries.open(request_id, 1, "theme.set", &args),
                Seat::Conflict
            ),
            "이름이 다르면 딴 요청이다 — 합치면 안 된다"
        );
        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &json!({ "window": "other" })),
                Seat::Conflict
            ),
            "인자가 다르면 딴 요청이다 — 이름만 보면 여기가 뚫린다"
        );
        assert!(
            matches!(
                deliveries.open(request_id, 2, "tab.create", &args),
                Seat::Taken { holder: 1 }
            ),
            "다른 연결이면 그쪽에 답해야 한다"
        );
        assert_eq!(deliveries.in_flight(), 1, "먼저 열린 자리는 그대로다");
    }

    /// ★자리는 결말이 붙는 순간이 아니라 **프레임이 나간 뒤**에 풀린다★(모듈 헤더 규칙 2).
    ///
    /// 결말이 붙자마자 키가 비면, 그 프레임이 나가기 전에 도착한 재질의가 빈 표를 보고 **봉투를 한 번 더**
    /// 보낸다 — 같은 조작이 두 번 적용되고 같은 키의 프레임도 둘이 된다.
    #[test]
    fn a_seat_stays_claimed_until_its_reply_has_been_sent() {
        let deliveries = deliveries_with(ManualClock::new());
        let request_id = RequestId::new();
        let args = json!({});
        let Seat::Opened { rx: _rx, token } = deliveries.open(request_id, 1, "tab.create", &args)
        else {
            panic!("자리를 얻어야 한다");
        };

        assert!(deliveries.complete(CommandReply::ok(request_id, json!({}))));
        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args),
                Seat::Coalesced
            ),
            "결말이 붙었어도 프레임 전이면 자리는 아직 그 왕복 것이다"
        );

        deliveries.release(request_id, token);
        assert!(
            matches!(
                deliveries.open(request_id, 1, "tab.create", &args),
                Seat::Opened { .. }
            ),
            "프레임이 나간 뒤에는 같은 id 로 새 왕복을 열 수 있다"
        );
    }

    /// ★늦은 `release` 는 **같은 키에 앉은 다른 왕복**을 지우지 않는다★ — 그래서 발권으로 집는다.
    ///
    /// 마감·종료가 걷어 간 뒤 호출자가 같은 id 로 다시 묻는 것은 계약이 막지 않는 정상
    /// 경로다. 키로 지우면 그 새 왕복이 이유 없이 답을 잃는다.
    #[test]
    fn a_late_release_cannot_evict_the_next_round_trip_under_the_same_id() {
        let clock = ManualClock::new();
        let deliveries = deliveries_with(clock.clone());
        let request_id = RequestId::new();
        let args = json!({});
        let Seat::Opened {
            rx: _first,
            token: stale,
        } = deliveries.open(request_id, 1, "tab.create", &args)
        else {
            panic!("첫 왕복은 자리를 얻는다");
        };

        // 마감이 그 왕복을 결말 없이 접는다 — 여기까지가 첫 시도의 끝이다.
        clock.advance(Duration::from_secs(11));
        assert_eq!(deliveries.expire(), 1);

        // 계약이 막지 않는 같은 번호의 재질의 — 새 왕복이 같은 키에 앉아야 한다.
        let Seat::Opened { rx: _second, .. } = deliveries.open(request_id, 1, "tab.create", &args)
        else {
            panic!("재질의가 새 자리를 얻는다");
        };

        // 첫 태스크가 이제서야 프레임을 내보내고 자기 자리를 놓는다.
        deliveries.release(request_id, stale);
        assert_eq!(
            deliveries.in_flight(),
            1,
            "늦은 정리가 남의 자리를 지우면 그 호출자는 답을 잃는다"
        );
    }

    /// ★**지어낸 답이 아니라 중계한 답**에도 지시 내림이 걸린다 — 그리고 그 내림이 **주인의 어휘를
    /// 지우지 않는다**★(FIX 9 의 구조 게이트가 못 보는 절반)
    ///
    /// 주인은 남의 프로세스라 이 모듈의 생성자를 안 쓴다 — 제 판단대로 `retry: same-request-id` 를 실어
    /// 보낼 수 있고, 데몬은 그 답을 **그대로 중계한다.** 릴리즈에서도 살아 있는 경로라 패닉 그물보다 이쪽이
    /// 중요하다.
    ///
    /// ★주인의 답을 **wire 에서 디코드해** 넣는 것이 이 시험의 요점이다★: in-process 로 지어낸 오류에는
    /// 원문 사본([`CommandError::received`])이 비어 있어 「어휘가 보존되나」라는 물음 자체가 공허해진다.
    /// 여기서는 **모르는 코드**(`RATE_LIMITED`)와 **계약 밖 필드**(`hint`)를 실어 보내, 나간 프레임에서
    /// 그 둘이 살아 있으면서 지시만 내려갔는지를 **원문 JSON 으로** 잰다(타입으로 돌려 읽으면 미지 코드가
    /// `INTERNAL` 로 보여 애초에 못 잰다).
    #[tokio::test]
    async fn an_error_relayed_from_the_owner_keeps_its_vocabulary_and_loses_only_its_retry_hint() {
        let (roster, mut owner_inbox) = roster_with_owner(1, "tab.create");
        let mut caller_inbox = attach_caller(&roster, 2);
        let deliveries = deliveries_with(ManualClock::new());
        let env = envelope("tab.create", &OwnerToken::new("whatever"));
        let request_id = env.request_id;

        let round_trip = tokio::spawn(deliver(roster.clone(), deliveries.clone(), 2, env));
        assert!(matches!(
            next_event(&mut owner_inbox).await,
            AgentEvent::CommandRequest { .. }
        ));

        // 주인이 보낸 바이트 그대로 — 이 데몬이 모르는 코드와 모르는 필드를 함께 실었다.
        let from_owner: CommandReply = serde_json::from_str(&format!(
            r#"{{"request_id":"{request_id}","outcome":{{"Err":{{"code":"RATE_LIMITED","message":"slow down","retry":"same-request-id","hint":"5s"}}}}}}"#
        ))
        .expect("주인의 답장을 디코드한다");
        assert!(deliveries.complete(from_owner));
        finished(round_trip).await;

        let frame = tokio::time::timeout(Duration::from_secs(10), caller_inbox.recv())
            .await
            .expect("답장이 오지 않는다")
            .expect("프레임 하나");
        let Frame::Text(text) = frame else {
            panic!("Text 여야 함: {frame:?}")
        };
        let sent: serde_json::Value =
            serde_json::from_str(&text).expect("나간 프레임을 원문으로 읽는다");
        let error = &sent["CommandReply"]["reply"]["outcome"]["Err"];

        assert_eq!(
            error["retry"], "never",
            "중계 홉이 위험한 지시를 내린다 — dedup 이 설 때까지 그 책임은 여기 있다"
        );
        assert_eq!(
            error["code"], "RATE_LIMITED",
            "★주인의 어휘를 지우지 않는다★ — 지시를 내리려고 사본을 버리면 여기가 INTERNAL 로 바뀐다"
        );
        assert_eq!(
            error["hint"], "5s",
            "계약 밖 필드도 최종 호출자까지 간다(additive 확장이 중계에서 죽으면 안 된다)"
        );
        assert_eq!(error["message"], "slow down", "문구도 주인 것 그대로다");
    }

    // ── 재시도 지시 정책(FIX 9) ─────────────────────────────────────────────────

    /// ★이 경로가 「같은 번호로 다시 물어라」를 다시 내보내면 여기서 떨어진다★
    ///
    /// 근거는 [`no_retry`] 에 있다(요약: 완료분을 재생해 줄 dedup 저장소가 없어 재질의가 **재실행**이 된다).
    /// 행동 시험으로는 그때그때 만져 본 갈래만 지킬 수 있고, 다음에 갈래가 하나 늘면 그 자리만 조용히
    /// 옛 지시로 돌아간다 — 그래서 **오류를 짓는 길 자체가 하나뿐인지**를 소스로 잰다.
    /// ★필요한 문자열은 `concat!` 로 이어 붙인다★ — 그대로 적으면 이 시험 자신이 자기 패턴에 걸린다.
    ///
    /// ★이 게이트를 실제 보장으로 읽지 말 것★ — 두 겹으로 못 본다:
    /// ① **출처**: 이 모듈이 *지은* 답만 잰다. 밖에서 들어와 이 모듈이 **통과시키는** 답(`route` 의 패닉
    ///    그물 · 주인이 보내 온 오류)은 여기 안 걸린다.
    /// ② **형태**: 두 생성자의 **철자**를 셀 뿐이라, 그것을 감싼 얇은 래퍼(`CommandError::internal` ·
    ///    `not_found` 같은 것)로 오류를 지으면 그대로 통과한다.
    /// ★이 경로의 실제 보장은 [`send_reply`] 의 출구 내림 한 줄이고**, 그것을 잰다** 자리는
    /// [`tests::an_error_relayed_from_the_owner_keeps_its_vocabulary_and_loses_only_its_retry_hint`].
    /// 이 게이트는 그 위에 얹는 **조기 경보**다 — 지시를 직접 싣는 새 자리가 생기면 출구까지 안 가서 걸린다.
    #[test]
    fn this_module_can_only_build_do_not_retry_failures() {
        let src = include_str!("command_delivery.rs");
        let derived = concat!("CommandError", "::of(");
        let typed = concat!("CommandError", "::with_retry(");

        assert_eq!(
            src.matches(derived).count(),
            0,
            "코드에서 지시를 파생시키는 생성자를 쓰면 안 된다 — 오류는 no_retry 로만 짓는다"
        );
        assert_eq!(
            src.matches(typed).count(),
            1,
            "지시를 직접 싣는 생성자는 no_retry 안의 그 한 줄뿐이어야 한다"
        );
        assert_eq!(
            no_retry(ErrorCode::OutcomeUnknown, "x").retry(),
            RetryMode::Never,
            "그 한 줄이 싣는 지시"
        );
    }

    /// 아무도 안 물은 결말은 붙을 자리가 없다.
    #[test]
    fn an_outcome_for_an_unknown_request_id_finds_no_seat() {
        let deliveries = deliveries_with(ManualClock::new());

        assert!(!deliveries.complete(CommandReply::ok(RequestId::new(), json!({}))));
    }
}
