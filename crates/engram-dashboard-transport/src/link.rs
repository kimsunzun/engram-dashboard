//! 전송 seam — 통로를 여는 것(`Dialer`) · 받는 것(`Listener`)과 열린 통로의 양 끝(`LinkTx`/`LinkRx`), 그리고 그
//! 위에서 도는 불투명 핸드셰이크.
//!
//! ★왜 제네릭이 아니라 dyn 인가★ — 한 명부 안에 **서로 다른 전송이 섞일 수 있어야** 한다(로컬 WS 상대와
//! 나중의 TLS 상대를 같은 [`crate::Registry`] 에 담는다). 제네릭 파라미터로는 그 컬렉션이 표현되지 않는다.
//! 프레임은 이미 `String`/`Vec<u8>` 값이라 프레임당 dyn 호출은 syscall 옆에서 잡음이다. (ADR-0177 결정 7)
//!
//! ★`async fn` in trait 을 쓰지 않는 이유★ — dyn 호환이 아니다. 그래서 `BoxFuture` 로 수동 박싱한다
//! (같은 저장소의 `net::frame_port` 가 같은 이유로 같은 형태다).

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;

use crate::frame::{Close, CloseCode, Frame};
use crate::mask::mask_reason;

/// 상대의 주소. ★crate 는 이 문자열을 파싱하지 않는다★ — 뜻은 [`Dialer`] 구현체만 안다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address(pub String);

impl Address {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 통로 자체가 낸 오류. ★사람이 읽는 문구뿐이고 기계 분기용 코드가 없다★ — 전송마다 오류 어휘가
/// 달라서 여기서 갈래를 못 박으면 어댑터가 자기 사유를 억지로 우리 갈래에 욱여넣는다. 분기가 필요한
/// 구분(못 붙음 / 붙었는데 거절)은 [`crate::ConnectFailure`] 가 지고, 거는 자리의 구분은 [`DialFailure`] 가 진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub message: String,
}

impl LinkError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LinkError {}

/// 통로에서 읽어 올린 것.
///
/// ★[`LinkRead::Closed`] 가 닫기 코드를 나르는 것이 계약의 핵심이다★ — 이 칸이 없으면 「상대가 나를
/// 거절했다」와 「상대가 그냥 사라졌다」를 받는 쪽이 구별할 수 없고, 그러면 ADR-0180 결정 5(종결 거절은 재연결
/// 예산을 안 쓴다)가 **코드로는 성립하지 않는다**. 종결 거절은 재시도해도 결과가 같으므로 예산을 태우면 안 되고,
/// 그 판정 재료 — 거절인가, 어느 종류인가([`RejectKind::of`]) — 가 이 코드다.
pub enum LinkRead {
    Frame(Frame),
    /// 상대가 **닫았다고 말했다**. `Some` 이면 코드와 문구를 실어서, `None` 이면 코드 없이 닫기만.
    ///
    /// ★말 없이 사라진 상대는 이 갈래로 오지 않는다★ — 실전송은 그것을 `Err(LinkError)` 로 낸다
    /// (WS 어댑터가 그 예다: 닫기 핸드셰이크 없이 버려진 소켓은 tungstenite 가
    /// `ResetWithoutClosingHandshake` 오류로 낸다). ★인메모리 하네스도 같은 모양을 진다★ — 엔드포인트를
    /// 그냥 떨어뜨리면 `testing::MemRx` 가 채널 소진을 오류로 낸다. 한때 그쪽만 `Closed(None)` 을 올려서
    /// **엔드포인트를 떨어뜨려 죽음을 흉내내는 테스트가 하네스에서만 초록**이었다 — 되돌리지 말 것.
    /// 그래도 `Closed(None)` 을 「조용히 죽었다」의 신호로 분기하지 말 것 — 그것은 「코드 없이 닫겠다고
    /// **말했다**」이고, 말 없이 사라진 것과 다르다.
    ///
    /// ★[`LinkRx::recv`] 가 한 번 실패한 뒤에는 이 갈래가 나오지 않는다★ — 그 계약이 없으면 위 문장이
    /// 「첫 읽기에서만」이라는 조건부가 된다(사유는 그쪽 rustdoc).
    Closed(Option<Close>),
}

impl fmt::Debug for LinkRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(frame) => f.debug_tuple("Frame").field(frame).finish(),
            Self::Closed(close) => f.debug_tuple("Closed").field(close).finish(),
        }
    }
}

/// 거는 자리에서 난 실패 — ★재연결 예산을 쓰나 안 쓰나로 갈린다★(TRD 3 §3-0 ⑤).
///
/// `?` 로 [`LinkError`] 를 그대로 올리면 [`DialFailure::Unreachable`] 이 된다(`From`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialFailure {
    /// 통로가 안 열렸다 → 예산을 쓰고 다시 건다.
    Unreachable(LinkError),
    /// ★걸어 봐야 안 받아 줄 것을 거는 쪽이 이미 안다★(예: 로컬 데몬 정보의 버전이 다르다) → 통로 없이 「붙었는데
    /// 거절」과 같은 길로 간다 — 예산 없이 [`crate::GaveUpReason::Rejected`] 에 앉고 다시 걸지 않는다. ★늘
    /// [`RejectKind::Terminal`] 로 읽는다★ — 다시 걸고 싶은 실패면 [`DialFailure::Unreachable`] 을 돌려준다.
    ///
    /// `code` = 거는 쪽이 고른 거절 코드 — [`crate::TransportEvent::Rejected`] 에 그대로 실려 소비자가 고정 문구를
    /// 고르는 재료가 된다(그 값으로 종류를 다시 가르지 않는다). ★종결 거절 코드여야 한다★ — [`RejectKind::of`] 가
    /// [`RejectKind::Terminal`] 로 읽는 코드([`CloseCode::HANDSHAKE_REJECTED`] = 4001)다. 다시 걸 코드(예:
    /// [`CloseCode::HANDSHAKE_RETRYABLE`])를 실어도 이 갈래는 포기하므로 사건의 코드와 동작이 어긋난다(디버그
    /// 빌드는 assert 로 멈춘다). `reason` = 로컬 진단 문구 — 다른 거절 문구와 같은 가림을 거친다
    /// ([`crate::ConnectFailure::Rejected`]).
    Refused { code: CloseCode, reason: String },
}

impl From<LinkError> for DialFailure {
    fn from(err: LinkError) -> Self {
        Self::Unreachable(err)
    }
}

/// 글 칸은 [`crate::mask_reason`] 으로 가려 찍는다 — 소비자 dialer 가 상대가 준 글을 실을 수 있고, 이 값을 그대로
/// 로그에 찍는 자리가 많다.
impl fmt::Display for DialFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(err) => write!(f, "unreachable: {}", mask_reason(&err.message)),
            Self::Refused { code, reason } => {
                write!(f, "refused ({}): {}", code.0, mask_reason(reason))
            }
        }
    }
}

impl std::error::Error for DialFailure {}

/// 주소 하나로 통로를 연다.
///
/// ★시한은 부르는 쪽(감독 태스크)이 건다★ — 구현체가 자기 시한을 또 걸면 정책 값이 두 곳에 산다.
///
/// ★반환 future 가 취소될 수 있다★ — 감독이 그 사이 `close()` 를 들으면 그대로 버린다. 구현체는 통로를
/// **future 안에서** 만들어야 하고, 밖에 등록해 두면 아무도 안 쓰는 통로가 남는다.
pub trait Dialer: Send + Sync + 'static {
    fn dial(
        &self,
        addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>>;
}

/// 통로의 나가는 쪽. ★감독 태스크 하나만 이것을 쥔다(단일 writer)★.
///
/// ★양 끝이 **둘 다** 떨어져야 통로가 닫힌다★ — 한 끝만 놓고 다른 끝을 쥐고 있으면 상대는 끊김을 못
/// 보고, 그건 소켓 누수다. 두 끝은 [`Dialer::dial`] 이 한 쌍으로 주고 한 쌍으로 버려야 한다. 구현체가
/// 하나의 하위 스트림을 공유하는 것이 보통이라(WS 어댑터는 `BiLock` 이다) **부르는 쪽이 이 의무를 진다.**
pub trait LinkTx: Send + 'static {
    /// ★반환 future 가 취소된 뒤 부르는 쪽이 이 끝에 하는 일은 많아야 [`LinkTx::close`] 한 번이다★ —
    /// 그 뒤 양 끝을 버리고 `send`·`ping` 은 다시 부르지 않는다. 운영 단계에서는 `Supervisor::drop_link`
    /// 가 닫기를 보낸다. 핸드셰이크 도중에는 닫기 없이 버린다 — 시한이 자르면(거절 직전 프레임 포함)
    /// `handshake_once` 가, 제어가 이기면 `shake_hands` 가 핸드셰이크 future 째 떨어뜨린다. 그래서
    /// 구현체는 **취소된 `send` 뒤의 `close`** 를 견뎌야 하고, 잘린 프레임의 나머지가 그 닫기 앞에 실려
    /// 나갈 수 있다(WS 어댑터가 그렇다 — `src/ws.rs` 「통로 성질」).
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;

    /// 살아있나 물어본다. ★"언제" 는 이 crate 가 정하고 "어떻게" 만 구현체가 안다★ — 그래서 WS 처럼
    /// 전송 계층이 자체 Ping 을 가진 경우 프레임을 새로 짜지 않아도 된다.
    ///
    /// ★취소는 [`LinkTx::send`] 와 같은 계약이다★ — 잘린 뒤 이 끝이 받는 것은 많아야 `close` 한 번이다.
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>>;

    /// 코드와 문구를 실어 닫는다. 실패해도 알릴 데가 없으므로 반환값이 없다.
    ///
    /// ★[`Close::reason`] 은 유계이고 넘치면 잘려서 나간다★ — 전송이 그것을 제어 프레임에 실으면
    /// 상한이 있다. ★상한의 단위는 글자가 아니라 바이트★라 한글·이모지는 세 배 이상 빨리 찬다.
    /// ★그 **수**는 이 파일에 없다★ — 전송마다 다르고, 여기 베끼면 정본이 둘이 된다. WS 의 값은
    /// 어댑터의 `MAX_CLOSE_REASON_BYTES`(`src/ws.rs`)가 정본이고 그 상수 rustdoc 이 근거(RFC 6455 §5.5)
    /// 를 든다. 자르는 것이 안 자르는 것보다 나은 이유는 상한을 넘긴 닫기 프레임을 **받는 쪽이 읽기
    /// 오류로 끊어서**, 「붙었는데 거절」이 「못 붙었다」로 도착하고 재연결 예산까지 태우기 때문이다.
    /// 문구에 판정 재료를 담지 말 것 — 분기는 [`Close::code`] 로만 한다.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()>;
}

/// 통로의 들어오는 쪽.
///
/// ★[`LinkTx`] 와 한 쌍이다 — 양 끝이 둘 다 떨어져야 통로가 닫힌다★(그쪽 rustdoc 이 사유를 든다).
pub trait LinkRx: Send + 'static {
    /// keepalive 응답은 [`Frame::Keepalive`] 로 올라온다.
    ///
    /// ★한 번 `Err` 를 낸 통로는 계속 `Err` 를 낸다★ — 구현체는 실패 뒤에 [`LinkRead::Closed`] 를
    /// **내지 않는다**. 실패 뒤 더 읽는 것을 금지하는 쪽이 아니라 이쪽을 계약으로 삼은 이유는, 금지는
    /// 지키지 않아도 아무 신호가 없고 어기면 「상대가 닫았다고 말했다」가 거짓이 되기 때문이다(전송
    /// 계층이 자기 종료 표식으로 소진을 알리는 것이 보통이다 — WS 어댑터 rustdoc 이 그 실물을 든다).
    /// 그래서 이 계약 위에서 [`LinkRead::Closed`] 는 **조건 없이** 「상대가 말했다」를 뜻한다.
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>>;
}

// ── 받는 쪽 ──────────────────────────────────────────────────────────────────

/// 들어오는 통로를 받는다 — [`Dialer`] 의 짝(TRD 3 §3-1 ① · D1).
///
/// ★수락 루프는 소비자가 돈다★ — `accept` 를 언제 부르고 받은 것을 붙일지(연결 수 상한 · 속도)는 소비자 정책이다
/// (ADR-0177 결정 4). 명부는 청취기를 쥐지 않는다.
///
/// ★받는 쪽은 하나다(`&mut self`)★ — 한 청취기를 두 task 가 나눠 받는 모양을 두지 않는다.
///
/// ★청취기를 떨어뜨리면 아직 안 받아 간 통로는 닫힌다★ — 거는 쪽은 그것을 통로 손실로 본다. 붙들어 두면 아무도
/// 안 받을 소켓이 업그레이드 칸을 쥔 채 남는다.
pub trait Listener: Send + 'static {
    /// 다음 받은 통로를 기다린다.
    ///
    /// ★취소 안전★ — 반환 future 를 끝나기 전에 떨어뜨려도 받은 통로를 잃지 않고 다음 `accept` 가 그것을 준다
    /// (`select!` 팔에 걸어 종료 신호와 겨뤄도 된다).
    ///
    /// ★`Err` 는 종결이다★ — 청취기가 더는 받을 수 없다. 한 번 `Err` 를 낸 청취기는 계속 `Err` 를 낸다
    /// ([`LinkRx::recv`] 와 같은 계약). ★상대 하나의 실패(업그레이드 실패 · 칸 넘침 · 시한)는 `Err` 가 아니다★ — 그
    /// 통로는 청취기가 스스로 닫고 다음 것을 기다린다. 그것을 `Err` 로 내면 상대 하나가 수락 루프를 끝낸다.
    ///
    /// ★그런 거절은 이 trait 밖에서 구현마다 알린다★(TRD 3 §5 18) — `ws` 는 `bind` 가 청취기와 함께 거절 수신기를
    /// 돌려준다(3-0 U7). `accept` 의 `Ok` 는 받은 통로([`Accepted`]) 하나뿐이고 거절 갈래를 싣지 않는다.
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>>;
}

/// 받은 통로에 붙은 사실 — 소비자가 그 통로의 핸드셰이크 값을 지을 때 읽는다(TRD 3 §3-1 ①).
///
/// ★`None` = 이 전송이 그것을 모른다★ — 인메모리 통로에는 원격 주소가 없고 Origin 은 WS 업그레이드에만 있다.
/// ★`#[non_exhaustive]`★ — 나중 전송이 칸을 더해도 소비자가 안 깨진다. crate 밖에서는 [`LinkInfo::default`] 에서
/// 칸을 채워 짓는다.
///
/// `Debug` 는 `remote` · `origin` 을 [`crate::mask_reason`] 으로 가리고 `peer_key` 는 길이만 찍는다.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LinkInfo {
    /// 받은 쪽에서 본 상대 끝 — 진단 · 로그 재료다. ★[`Address`] 가 아니다★ — 연결마다 바뀌는 임시 포트 같은 것이라
    /// 그쪽으로 걸 수 있는 주소가 아니다.
    /// ★전송이 지은 값이다 — 상대 글을 넣지 말 것★.
    pub remote: Option<String>,
    /// WS 업그레이드 요청의 `Origin` 머리 값. ★상대가 지어 보낸 글이다★ — 판정은 수락 정책 몫이고(3-0 U7) 그대로
    /// 찍지 않는다.
    pub origin: Option<String>,
    /// 전송이 확인한 상대 공개 키(TLS 키 고정 · iroh 노드 id — TRD 3 §3-5 · §4). ★3단계 전송은 늘 `None` 이다★ — 칸만
    /// 미리 박는다.
    pub peer_key: Option<Vec<u8>>,
}

impl fmt::Debug for LinkInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkInfo")
            .field("remote", &self.remote.as_deref().map(mask_reason))
            .field("origin", &self.origin.as_deref().map(mask_reason))
            .field(
                "peer_key",
                &self.peer_key.as_ref().map(|k| format!("{} bytes", k.len())),
            )
            .finish()
    }
}

/// [`Listener::accept`] 가 내놓는 받은 통로 하나 — 양 끝 · [`LinkInfo`] · 업그레이드 칸 · 업그레이드 끝 시각.
///
/// ★양 끝은 crate 밖으로 꺼내지 않는다★ — 받은 상대로 얹는 자리([`crate::Registry::attach`])만 쓴다. 떨어뜨리면 두
/// 끝이 함께 떨어진다([`LinkTx`] 의 「양 끝이 둘 다」 의무가 여기서 저절로 선다).
///
/// ★칸은 업그레이드 대기 칸이다★ — 전송이 통로를 받은 순간(TCP 수락)에 쥐고, 받는 쪽이 다음 칸(인증 대기)을 쥔
/// 뒤에야 놓는다(TRD 3 §3-1 · §5 14). 그래서 소비자가 `accept` 를 안 부르는 동안 준비 큐에 앉은 통로도 센다. 인증 대기
/// 칸이 아직 없어 얹은 상대는 운영 단계에 들 때까지 이 칸을 쥔다([`crate::Registry::attach_with_policy`]).
///
/// ★인증 시한은 `upgraded_at` 부터 잰다★(TRD 3 §5 3) — 준비 큐에 앉은 시간도 시한에 든다. 그 시각을 찍은 시계와
/// 시한을 재는 시계가 같아야 하므로 전송 구현에 명부와 같은 [`crate::Clock`] 을 준다.
pub struct Accepted {
    pub(crate) tx: Box<dyn LinkTx>,
    pub(crate) rx: Box<dyn LinkRx>,
    pub(crate) info: LinkInfo,
    pub(crate) upgraded_at: Instant,
    pub(crate) permit: ConnPermit,
}

impl Accepted {
    /// 전송 구현이 짓는다. `tx` · `rx` 는 같은 통로의 한 쌍이다. `upgraded_at` = 통로가 프레임을 나를 수 있게 된 순간
    /// (WS 면 업그레이드 응답을 보낸 뒤).
    pub fn new(
        tx: Box<dyn LinkTx>,
        rx: Box<dyn LinkRx>,
        info: LinkInfo,
        upgraded_at: Instant,
        permit: ConnPermit,
    ) -> Self {
        Self {
            tx,
            rx,
            info,
            upgraded_at,
            permit,
        }
    }

    pub fn info(&self) -> &LinkInfo {
        &self.info
    }

    pub fn upgraded_at(&self) -> Instant {
        self.upgraded_at
    }
}

impl fmt::Debug for Accepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Accepted")
            .field("info", &self.info)
            .field("upgraded_at", &self.upgraded_at)
            .field("permit", &self.permit)
            .finish_non_exhaustive()
    }
}

/// 동시에 쥘 수 있는 [`ConnPermit`] 수의 상한 — 연결 수 상한의 셈(TRD 3 §5 14). clone 은 같은 묶음이다.
///
/// ★사건으로 세지 않고 쥔 손잡이로 센다★(TRD 3 §3-0 ②) — 세는 대상의 수명에 [`ConnPermit`] 을 묶으면 그것이 어느 길로
/// 끝나든(오류 · 패닉 · 취소 포함) 떨어질 때 칸이 돌아간다. 사건으로 세면 사건 하나를 놓친 길마다 셈이 샌다.
#[derive(Clone)]
pub struct PermitPool {
    count: Arc<PermitCount>,
}

struct PermitCount {
    limit: usize,
    in_use: AtomicUsize,
}

impl PermitPool {
    /// `limit` = 0 이면 칸을 하나도 안 준다.
    pub fn new(limit: usize) -> Self {
        Self {
            count: Arc::new(PermitCount {
                limit,
                in_use: AtomicUsize::new(0),
            }),
        }
    }

    /// 칸 하나. `None` = 다 찼다 — 기다리지 않는다(넘친 것은 새것을 닫는다 — TRD 3 §5 14).
    pub fn try_take(&self) -> Option<ConnPermit> {
        let limit = self.count.limit;
        self.count
            .in_use
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .ok()
            .map(|_| ConnPermit {
                count: self.count.clone(),
            })
    }

    pub fn in_use(&self) -> usize {
        self.count.in_use.load(Ordering::Acquire)
    }

    pub fn limit(&self) -> usize {
        self.count.limit
    }
}

impl fmt::Debug for PermitPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermitPool")
            .field("in_use", &self.in_use())
            .field("limit", &self.limit())
            .finish()
    }
}

/// [`PermitPool`] 의 칸 하나 — 떨어뜨리면 돌아간다.
#[must_use = "떨어뜨리는 순간 칸이 돌아간다 — 세는 대상의 수명에 묶어 둔다"]
pub struct ConnPermit {
    count: Arc<PermitCount>,
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        self.count.in_use.fetch_sub(1, Ordering::AcqRel);
    }
}

impl fmt::Debug for ConnPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnPermit")
            .field("limit", &self.count.limit)
            .finish_non_exhaustive()
    }
}

/// 불투명 핸드셰이크. ★왕복 횟수를 crate 가 세지 않는다★ — [`HandshakeStep::Send`]/[`HandshakeStep::Await`]
/// 를 몇 번 돌려주든 받는다.
///
/// ★방향 대칭★ — 받는 쪽(데몬)은 `start()` 가 [`HandshakeStep::Await`] 를 주고 `on_frame` 에서 판정한다.
/// 그래서 버전·능력 협상 어휘는 **구현체 안에만** 있고 crate 는 그것을 모른다(ADR-0178).
pub trait Handshake: Send + Sync + 'static {
    fn start(&self) -> HandshakeStep;
    fn on_frame(&self, frame: &Frame) -> HandshakeStep;
}

/// 거절의 종류 — ★재연결 예산을 가른다★. 통로 위에서는 닫기 코드로 건너간다(TRD 3 §3-0 ⑤ · D20 ①).
///
/// 거절하는 쪽의 [`HandshakeStep::Reject`] 가 종류를 고르면 crate 가 그 코드([`RejectKind::close_code`])를 실어
/// 닫고, 거는 쪽은 받은 코드로 종류를 정한다([`RejectKind::of`]). ★상대가 준 글로는 가르지 않는다★ — 글은
/// 사람용 진단이라 예고 없이 바뀐다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectKind {
    /// 다시 걸어도 결과가 같다(예: 버전 불일치) → 예산 없이 [`crate::GaveUpReason::Rejected`] 에 앉는다(ADR-0180
    /// 결정 5). 코드 = [`CloseCode::HANDSHAKE_REJECTED`].
    Terminal,
    /// 다시 걸면 붙을 수 있다(예: 제자리 다시 쓴 데몬 정보에서 낡은 자격을 읽었다) → 못 붙음과 같이 예산을 쓰고 다시
    /// 건다. 코드 = [`CloseCode::HANDSHAKE_RETRYABLE`].
    Retryable,
}

impl RejectKind {
    /// 이 종류로 거절할 때 실을 닫기 코드.
    pub fn close_code(self) -> CloseCode {
        match self {
            RejectKind::Terminal => CloseCode::HANDSHAKE_REJECTED,
            RejectKind::Retryable => CloseCode::HANDSHAKE_RETRYABLE,
        }
    }

    /// 핸드셰이크 중 상대가 실어 닫은 코드를 읽는다. `None` = 거절이 아니다(못 붙음 — 예산을 쓴다).
    ///
    /// ★정상 종료 계열은 거절이 아니다★ — RFC 6455 의 `1000`(Normal) · `1001`(Going Away)과
    /// [`CloseCode::GOING_AWAY`] 는 「나 이제 간다」다. 그것을 거절로 읽으면 데몬이 곱게 종료할 때마다 거는 쪽이
    /// 재시도 한 번 없이 포기한다. ★[`CloseCode::HANDSHAKE_REJECTED`] 만 [`RejectKind::Terminal`] 이다★ — 나머지는
    /// 상대가 굳이 골라 실은 것이라 거절로 받되, 모르는 코드로 포기하면 사용자가 손대기 전엔 안 풀리므로 다시
    /// 건다(예산을 쓴다 — 끝없이 걸지는 않는다).
    pub fn of(code: CloseCode) -> Option<Self> {
        if matches!(code.0, 1000 | 1001) || code == CloseCode::GOING_AWAY {
            return None;
        }
        Some(if code == CloseCode::HANDSHAKE_REJECTED {
            RejectKind::Terminal
        } else {
            RejectKind::Retryable
        })
    }
}

/// 핸드셰이크의 다음 걸음.
///
/// ★[`HandshakeStep::Send`] 는 "보내고 **상대의 다음 프레임을 기다린다**" 는 뜻이다★ — trait 에 프레임
/// 없이 다음 걸음을 묻는 입구가 없으므로(`start`/`on_frame` 둘뿐) 보낸 뒤 갈 곳은 대기밖에 없다. 그래서
/// **연속 두 프레임을 보내는 핸드셰이크는 이 모양으로 표현되지 않는다**(TRD §3-5 가 든 예 두 걸음은
/// 표현된다: `start() = Send(auth)` → 상대 Hello → `on_frame = Done`).
///
/// ★[`Frame::Keepalive`] 는 `on_frame` 에 전달되지 않는다★ — 핸드셰이크 중 전송 계층 keepalive 가
/// 섞여 들어오면 판정이 흔들린다.
pub enum HandshakeStep {
    /// 보내고 상대의 다음 프레임을 기다린다.
    Send(Frame),
    /// 아무것도 안 보내고 상대의 다음 프레임을 기다린다.
    ///
    /// ★상대가 거절을 알리는 프레임(오류 문구)을 보냈으면 여기서 [`HandshakeStep::Reject`] 하지 말고 이것으로
    /// 닫기를 기다린다★ — 거절의 종류는 그 뒤 상대가 실어 닫는 코드가 정한다([`RejectKind::of`] · TRD 3 §3-0 ⑤).
    /// 코드 없이 닫히면 못 붙음이다. 그 프레임의 글은 소비자의 진단 · 로그 몫이다.
    Await,
    /// 운영 단계로.
    Done,
    /// ★붙었는데 거절★ — `last` 를 보내고 `kind` 의 코드([`RejectKind::close_code`])로 닫는다. 단 `last` 가
    /// 핸드셰이크 시한 안에 못 나가면 닫기는 건너뛰고 통로만 버린다(crate 헤더 「닫기 코드가 상대에게 닿는 자리는
    /// 좁다」). 거절한 이쪽이 건 상대면 `kind` 대로 간다 — [`RejectKind::Terminal`] 은 예산 없이 포기(ADR-0180
    /// 결정 5) · [`RejectKind::Retryable`] 은 예산을 쓰고 다시 건다.
    ///
    /// ★`reason` 은 유계다★ — 이것이 닫기 프레임의 문구로 나가므로 전송의 상한에 걸리고 넘치면
    /// **잘린다**(★글자가 아니라 바이트★ — 수는 어댑터 상수가 정본이다). 상세와 그 사유는
    /// [`LinkTx::close`]. 길게 남길 진단은 소비자 쪽 로그로 보내고 여기엔 상대가 읽을 한 줄만 담는다.
    Reject {
        kind: RejectKind,
        last: Option<Frame>,
        reason: String,
    },
}

/// 실은 프레임은 [`Frame`] 의 `Debug`(갈래 + 바이트 길이)로만 찍는다 — `Send` 가 인증 토큰을 싣는다(TRD 3 §5 5).
/// `reason` 은 상대에게 닫기 문구로 나가는 한 줄이라 값 그대로 찍는다 — 길이는 줄을 탈 때만 묶인다(ws 어댑터 `clamp_close_reason`).
impl fmt::Debug for HandshakeStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(frame) => f.debug_tuple("Send").field(frame).finish(),
            Self::Await => f.write_str("Await"),
            Self::Done => f.write_str("Done"),
            Self::Reject { kind, last, reason } => f
                .debug_struct("Reject")
                .field("kind", kind)
                .field("last", last)
                .field("reason", reason)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::CloseCode;

    #[test]
    fn address_is_opaque() {
        let a = Address::new("ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.as_str(), "ws://127.0.0.1:1/path?x=1");
        assert_eq!(a.to_string(), "ws://127.0.0.1:1/path?x=1");
    }

    #[test]
    fn link_error_displays_its_message() {
        let e = LinkError::new("refused");
        assert_eq!(e.to_string(), "refused");
    }

    #[test]
    fn a_close_can_carry_a_code_the_reader_can_branch_on() {
        let spoken = LinkRead::Closed(Some(Close::new(
            CloseCode::HANDSHAKE_REJECTED,
            "version 3 != 4",
        )));
        let silent = LinkRead::Closed(None);
        match (&spoken, &silent) {
            (LinkRead::Closed(Some(c)), LinkRead::Closed(None)) => {
                assert_eq!(c.code, CloseCode::HANDSHAKE_REJECTED);
                assert!(c.reason.contains("version"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn handshake_step_and_link_read_debug_carry_no_frame_body() {
        let secret = r#"{"auth":"tok-SECRET"}"#;
        let n = secret.len();
        let shown = [
            (
                format!("{:?}", HandshakeStep::Send(Frame::Text(secret.into()))),
                format!("Send(Text {{ bytes: {n} }})"),
            ),
            (
                format!(
                    "{:?}",
                    HandshakeStep::Send(Frame::Binary(secret.as_bytes().to_vec()))
                ),
                format!("Send(Binary {{ bytes: {n} }})"),
            ),
            (
                format!(
                    "{:?}",
                    HandshakeStep::Reject {
                        kind: RejectKind::Retryable,
                        last: Some(Frame::Text(secret.into())),
                        reason: "stale credential".into(),
                    }
                ),
                format!(
                    "Reject {{ kind: Retryable, last: Some(Text {{ bytes: {n} }}), reason: \"stale credential\" }}"
                ),
            ),
            (
                format!("{:?}", LinkRead::Frame(Frame::Text(secret.into()))),
                format!("Frame(Text {{ bytes: {n} }})"),
            ),
        ];
        for (got, expected) in shown {
            assert!(!got.contains("SECRET"), "{got}");
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn handshake_step_debug_names_each_arm() {
        assert_eq!(format!("{:?}", HandshakeStep::Await), "Await");
        assert_eq!(format!("{:?}", HandshakeStep::Done), "Done");
        assert!(format!("{:?}", HandshakeStep::Send(Frame::Keepalive)).starts_with("Send"));
        assert!(format!(
            "{:?}",
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "bad version".into()
            }
        )
        .contains("bad version"));
    }

    #[test]
    fn each_reject_kind_travels_as_its_own_close_code_and_comes_back_as_itself() {
        for kind in [RejectKind::Terminal, RejectKind::Retryable] {
            assert_eq!(RejectKind::of(kind.close_code()), Some(kind), "{kind:?}");
        }
        assert_ne!(
            RejectKind::Terminal.close_code(),
            RejectKind::Retryable.close_code()
        );
    }

    #[test]
    fn only_the_terminal_code_gives_up_and_a_graceful_close_is_no_rejection() {
        for graceful in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            assert_eq!(RejectKind::of(graceful), None, "{graceful:?}");
        }
        assert_eq!(
            RejectKind::of(CloseCode::HANDSHAKE_REJECTED),
            Some(RejectKind::Terminal)
        );
        // 모르는 코드 · crate 의 다른 코드 · 소비자 구간은 다시 건다.
        for other in [
            CloseCode(1008),
            CloseCode::KEEPALIVE_TIMEOUT,
            CloseCode::PROTOCOL_ERROR,
            CloseCode(CloseCode::CONSUMER_BASE),
            CloseCode(4999),
        ] {
            assert_eq!(
                RejectKind::of(other),
                Some(RejectKind::Retryable),
                "{other:?}"
            );
        }
    }

    #[test]
    fn a_dial_failure_displays_its_text_masked() {
        let hostile = "a\r\n\u{2028}b\u{202e}c\u{e0041}d\u{200b}";
        assert_eq!(
            DialFailure::Unreachable(LinkError::new(hostile)).to_string(),
            "unreachable: abcd"
        );
        assert_eq!(
            DialFailure::Refused {
                code: CloseCode::HANDSHAKE_REJECTED,
                reason: hostile.into(),
            }
            .to_string(),
            "refused (4001): abcd"
        );
    }

    // ── 받는 쪽 타입 ──
    #[test]
    fn a_permit_pool_never_hands_out_more_than_its_limit_and_takes_back_on_drop() {
        let pool = PermitPool::new(2);
        let a = pool.try_take().expect("첫 칸");
        let b = pool.try_take().expect("둘째 칸");
        assert!(pool.try_take().is_none(), "상한을 넘겼다");
        assert_eq!(pool.in_use(), 2);
        drop(a);
        assert_eq!(pool.in_use(), 1);
        let c = pool
            .clone()
            .try_take()
            .expect("돌아온 칸 — clone 은 같은 묶음이다");
        assert!(pool.try_take().is_none());
        drop((b, c));
        assert_eq!(pool.in_use(), 0);
        assert!(PermitPool::new(0).try_take().is_none());
    }

    #[test]
    fn link_info_debug_masks_the_texts_and_shows_only_the_key_length() {
        let info = LinkInfo {
            remote: Some("127.0.0.1:5\u{1b}[2J".into()),
            origin: Some("http://evil\r\n\u{202e}x".into()),
            peer_key: Some(vec![7; 32]),
        };
        assert_eq!(
            format!("{info:?}"),
            r#"LinkInfo { remote: Some("127.0.0.1:5[2J"), origin: Some("http://evilx"), peer_key: Some("32 bytes") }"#
        );
        assert_eq!(
            format!("{:?}", LinkInfo::default()),
            "LinkInfo { remote: None, origin: None, peer_key: None }"
        );
    }

    #[test]
    fn accepted_debug_carries_the_facts_but_no_link() {
        let end = crate::testing::memory_link().0;
        let permit = PermitPool::new(1).try_take().unwrap();
        let info = LinkInfo {
            origin: Some("o".into()),
            ..LinkInfo::default()
        };
        let accepted = Accepted::new(end.tx, end.rx, info.clone(), Instant::now(), permit);
        assert_eq!(accepted.info(), &info);
        let shown = format!("{accepted:?}");
        assert!(shown.starts_with("Accepted { info: LinkInfo {"), "{shown}");
        assert!(
            shown.ends_with("permit: ConnPermit { limit: 1, .. }, .. }"),
            "{shown}"
        );
    }

    #[test]
    fn a_link_error_raised_with_question_mark_is_unreachable() {
        fn dial() -> Result<(), DialFailure> {
            Err(LinkError::new("refused"))?;
            Ok(())
        }
        assert_eq!(
            dial(),
            Err(DialFailure::Unreachable(LinkError::new("refused")))
        );
    }
}
