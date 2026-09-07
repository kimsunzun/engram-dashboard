//! 하네스 전용 — 기능 플래그 `test-support` 뒤에 산다(ADR-0012, 선례 = `engram-dashboard-command`).
//!
//! ★여기 있는 것이 「백오프·keepalive·쓰기 시한을 **실제로 기다리지 않는다**」의 실물이다★
//! (ADR-0177 결정 8): [`ManualClock`] 이 시간을, [`MemoryNetwork`] 가 전송을 대신한다.
//!
//! ★**이 모듈은 current_thread 런타임 전용이다**★ — [`settle`] 이 협조적 양보로만 동기화하므로
//! `#[tokio::test(flavor = "multi_thread")]` 아래에서는 아무것도 보장하지 않는다. 그 위반은 조용히
//! 흔들리는 테스트가 되므로 [`settle`] 이 **런타임 종류를 직접 보고 패닉한다.**
//!
//! ★운영 코드가 이 모듈을 참조하면 안 된다★ — 기본 빌드에 없으므로 컴파일이 먼저 깨진다.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::frame::{Close, Frame};
use crate::link::{Address, Dialer, Handshake, HandshakeStep, LinkError, LinkRead, LinkRx, LinkTx};
use crate::stream::StreamMark;
use crate::wire::Wire;

// ── 시간 ──────────────────────────────────────────────────────────────────────

struct ManualState {
    now: Instant,
    waiters: Vec<(Instant, oneshot::Sender<()>)>,
}

impl ManualState {
    /// 받는 쪽이 사라진 잠꾸러기를 버린다.
    ///
    /// ★없으면 `waiters` 가 무한히 자란다★ — 감독의 운영 루프가 한 바퀴마다 새 `sleep` 을 만들고
    /// 그중 이긴 것 말고는 전부 그대로 버려지기 때문이다(트래픽 한 번에 한 개씩 쌓인다).
    fn prune(&mut self) {
        self.waiters.retain(|(_, tx)| !tx.is_closed());
    }
}

/// 손으로 미는 시계. ★스스로 흐르지 않는다★ — [`ManualClock::advance`] 만이 시각을 옮긴다.
#[derive(Clone)]
pub struct ManualClock {
    inner: Arc<Mutex<ManualState>>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManualState {
                now: Instant::now(),
                waiters: Vec::new(),
            })),
        }
    }

    /// `Arc<dyn Clock>` 자리에 꽂을 손잡이. 원본과 같은 시각을 본다.
    pub fn handle(&self) -> Arc<dyn Clock> {
        Arc::new(self.clone())
    }

    /// 시각을 옮기고 시한이 지난 잠꾸러기를 **시한이 이른 순서로** 깨운다.
    ///
    /// ★순서가 계약이다★ — 한 번의 `advance` 가 두 시한을 함께 넘길 때 등록 순서로 깨우면 실시간이
    /// 만들 수 없는 순서가 나오고, 그 순서에서만 통과하는 테스트가 생긴다.
    pub fn advance(&self, d: Duration) {
        let due = {
            let mut st = self.inner.lock().unwrap();
            st.prune();
            st.now += d;
            let now = st.now;
            let (mut due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut st.waiters)
                .into_iter()
                .partition(|(deadline, _)| *deadline <= now);
            st.waiters = rest;
            due.sort_by_key(|(deadline, _)| *deadline);
            due
        };
        for (_, tx) in due {
            let _ = tx.send(());
        }
    }

    /// 지금 **살아서** 잠들어 있는 수. 버려진 타이머는 세지 않는다.
    pub fn sleepers(&self) -> usize {
        let mut st = self.inner.lock().unwrap();
        st.prune();
        st.waiters.len()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.inner.lock().unwrap().now
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        if d.is_zero() {
            return Box::pin(async {});
        }
        let (tx, rx) = oneshot::channel();
        {
            let mut st = self.inner.lock().unwrap();
            st.prune();
            let deadline = st.now + d;
            st.waiters.push((deadline, tx));
        }
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}

/// [`settle`] 이 한 번에 도는 양보 횟수.
const SETTLE_YIELDS: usize = 512;

fn assert_single_threaded() {
    // ★멀티스레드 런타임에서는 양보가 동기화가 아니다★ — 이 모듈의 모든 단언 패턴이 그 위에서
    //   조용히 흔들린다. 조용한 흔들림보다 시끄러운 실패가 낫다.
    let flavor = tokio::runtime::Handle::current().runtime_flavor();
    assert_eq!(
        flavor,
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "engram-dashboard-transport 의 하네스는 current_thread 런타임 전용이다 \
         (#[tokio::test] 기본값). multi_thread 아래에서는 settle() 이 아무것도 보장하지 않는다"
    );
}

/// 수동 시계 아래에서 감독·reader 태스크가 갈 데까지 가게 한다.
///
/// ★실시간을 재우지 않는다★ — `yield_now` 만 반복한다. 조건을 알고 있다면 [`settle_until`] 이 낫다:
/// 이 함수는 **안 끝났다는 것을 알아채지 못하고** 그 실패가 엉뚱한 단언 자리에서 튀어나온다.
pub async fn settle() {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        tokio::task::yield_now().await;
    }
}

/// 조건이 설 때까지 양보한다. ★한도를 넘기면 그 자리에서 패닉한다★ — 「안 끝났다」를 「단언이 틀렸다」로
/// 둔갑시키지 않으려는 것이 이 함수의 존재 이유다.
pub async fn settle_until(what: &str, mut done: impl FnMut() -> bool) {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    if !done() {
        panic!("{SETTLE_YIELDS}번 양보하고도 정착하지 않았다: {what}");
    }
}

// ── 전송 ──────────────────────────────────────────────────────────────────────

/// 클라가 통로로 보낸 것. 프레임뿐 아니라 **살아있음 확인과 닫기**도 관측된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Frame(Frame),
    Ping,
    Close(Close),
}

#[derive(Debug)]
enum ServerMsg {
    Frame(Frame),
    Closed(Option<Close>),
    Fail(LinkError),
}

/// 통로의 상대편. 테스트가 이것으로 프레임을 밀어 넣고 클라가 무엇을 보냈는지 본다.
pub struct MemoryEndpoint {
    to_client: mpsc::UnboundedSender<ServerMsg>,
    from_client: Mutex<mpsc::UnboundedReceiver<ClientMsg>>,
    stall_tx: watch::Sender<bool>,
}

impl MemoryEndpoint {
    /// 클라가 보낸 것 하나. 없으면 `None`.
    pub fn take(&self) -> Option<ClientMsg> {
        self.from_client.lock().unwrap().try_recv().ok()
    }

    /// 클라가 보낸 것 전부.
    pub fn drain(&self) -> Vec<ClientMsg> {
        let mut rx = self.from_client.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    /// 클라가 보낸 **프레임**만.
    pub fn frames(&self) -> Vec<Frame> {
        self.drain()
            .into_iter()
            .filter_map(|m| match m {
                ClientMsg::Frame(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// 클라에게 한 덩어리 보낸다.
    pub fn push(&self, frame: Frame) {
        let _ = self.to_client.send(ServerMsg::Frame(frame));
    }

    /// ★코드 **없이** 닫는다고 말한다★ — `LinkRead::Closed(None)` 이고, 계약상 이것도 「상대가 닫았다고
    /// 말했다」다(말 없이 사라진 것은 [`MemoryEndpoint::fail`] 쪽 = `Err`). 클라는 닫힘은 알고 사유는
    /// 못 얻는다.
    ///
    /// ★이 통로를 **그냥 떨어뜨리는 것**(엔드포인트를 drop)은 이 갈래가 아니다★ — 그건 말 없이 사라진
    /// 것이라 `MemRx` 가 읽기 오류로 낸다(실소켓과 같은 모양). 「닫았다고 말했다」를 만들려면 이것이나
    /// [`MemoryEndpoint::reject`] 를 불러야 한다.
    pub fn close(&self) {
        let _ = self.to_client.send(ServerMsg::Closed(None));
    }

    /// ★코드와 문구를 실어 닫는다★ — 「붙었는데 거절당했다」를 클라가 알아볼 수 있는 유일한 경로다.
    pub fn reject(&self, close: Close) {
        let _ = self.to_client.send(ServerMsg::Closed(Some(close)));
    }

    /// 오류로 끊는다.
    pub fn fail(&self, why: &str) {
        let _ = self.to_client.send(ServerMsg::Fail(LinkError::new(why)));
    }

    /// 켜면 클라의 쓰기가 멈춘다 — `send`·`ping`·`close` **셋 다**다(실소켓에서 그 셋이 한 통로를
    /// 쓰므로). ★끄면 **실제로 깨어난다**★ — 깨울 길 없는 `pending()` 으로 막으면 그 멈춤은 장식이고,
    /// 그 위에서 통과한 테스트는 아무것도 재지 않은 것이다.
    ///
    /// ★막힌 채로 통로가 버려지는 테스트는 걸린 닫기까지 계산해야 한다★ — `drop_link` 의 닫기가
    /// 여기서 멈추면 그 뒤 단계(백오프 대기·유실 신고)가 시작되지 않는다. 푸는 길은 둘이다: 이것을
    /// 끄거나, 시계를 `write_deadline` 만큼 더 밀어 그 닫기를 시한으로 자르거나.
    ///
    /// ★커널 버퍼가 있어야 재는 「상대가 안 읽을 때」 는 이것으로 못 잰다★ — 그건 실소켓 몫이고,
    /// 여기서 재는 것은 **우리 쪽 시한·큐 로직**이다.
    pub fn stall_writes(&self, on: bool) {
        let _ = self.stall_tx.send(on);
    }

    /// 지금 쓰기가 막혀 있나.
    pub fn writes_stalled(&self) -> bool {
        *self.stall_tx.borrow()
    }
}

struct MemTx {
    to_server: mpsc::UnboundedSender<ClientMsg>,
    stall_rx: watch::Receiver<bool>,
}

impl MemTx {
    fn put(&self, msg: ClientMsg) -> Result<(), LinkError> {
        self.to_server
            .send(msg)
            .map_err(|_| LinkError::new("peer endpoint gone"))
    }

    async fn wait_writable(&mut self) {
        let _ = self.stall_rx.wait_for(|stalled| !*stalled).await;
    }
}

impl LinkTx for MemTx {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            self.put(ClientMsg::Frame(frame))
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            self.put(ClientMsg::Ping)
        })
    }

    /// ★닫기도 `send`/`ping` 과 **같은 통로**로 나가므로 같이 막힌다★ — 실소켓에서는 닫기 프레임이
    /// 잘린 프레임의 나머지 뒤에 줄을 서고, 그것이 감독의 `drop_link` 가 `write_deadline` 만큼 먹는
    /// 자리다. ★이 `wait_writable` 을 지우지 말 것★ — 없던 동안 하네스는 「이미 막힌 쓰기 뒤에 닫기가
    /// 걸린다」를 표현할 수 없었고, 그래서 `drop_link` 가 그 동안 제어를 못 듣는 결함이 무검으로
    /// 남아 있었다(2026-09-07).
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.wait_writable().await;
            let _ = self.put(ClientMsg::Close(close));
        })
    }
}

struct MemRx {
    from_server: mpsc::UnboundedReceiver<ServerMsg>,
    /// 이미 읽기가 실패했다 — [`LinkRx::recv`] 계약(실패 뒤에는 닫기를 내지 않는다)의 실물.
    failed: bool,
}

impl LinkRx for MemRx {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★실 어댑터와 같은 계약을 진다★ — 하네스가 실패 뒤에 `Closed(None)` 을 내면, 그 값으로
            //   분기하는 코드가 여기서는 초록이고 실소켓에서는 안 탄다(그 함정의 정본 = `link.rs` 의
            //   [`crate::LinkRead::Closed`] rustdoc).
            if self.failed {
                return Err(LinkError::new("link already failed"));
            }
            match self.from_server.recv().await {
                Some(ServerMsg::Frame(f)) => Ok(LinkRead::Frame(f)),
                Some(ServerMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★채널 소진 = 말 없이 사라졌다★ — 실소켓에서 같은 일(닫기 핸드셰이크 없이 버려진 소켓)은
                //   읽기 오류이므로 여기서도 오류다. `Ok(Closed(None))` 으로 올리던 판은
                //   [`crate::LinkRead::Closed`] 계약(「상대가 **말했다**」)을 하네스에서만 거짓으로 만들어,
                //   엔드포인트를 떨어뜨려 죽음을 흉내내는 테스트가 `PeerClosed` 로 초록이고 실소켓에서는
                //   `LinkError` 로 갈리게 했다.
                None => {
                    self.failed = true;
                    Err(LinkError::new("peer endpoint gone"))
                }
                Some(ServerMsg::Fail(e)) => {
                    self.failed = true;
                    Err(e)
                }
            }
        })
    }
}

struct NetState {
    dials: usize,
    fail_next: usize,
    fail_forever: bool,
    opened: VecDeque<Arc<MemoryEndpoint>>,
}

/// 인메모리 전송. 한 [`Dialer`] 뒤에서 통로를 열고, 열린 통로의 상대편을 테스트에 넘긴다.
#[derive(Clone)]
pub struct MemoryNetwork {
    inner: Arc<Mutex<NetState>>,
}

impl Default for MemoryNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryNetwork {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(NetState {
                dials: 0,
                fail_next: 0,
                fail_forever: false,
                opened: VecDeque::new(),
            })),
        }
    }

    pub fn dialer(&self) -> Arc<dyn Dialer> {
        Arc::new(self.clone())
    }

    /// 다음 `n` 번의 dial 이 실패한다.
    pub fn fail_next(&self, n: usize) {
        self.inner.lock().unwrap().fail_next = n;
    }

    /// 지금부터 모든 dial 이 실패한다.
    pub fn fail_forever(&self) {
        self.inner.lock().unwrap().fail_forever = true;
    }

    /// 다시 붙을 수 있게 한다.
    pub fn allow(&self) {
        let mut st = self.inner.lock().unwrap();
        st.fail_forever = false;
        st.fail_next = 0;
    }

    /// 지금까지 **실제로 폴링된** dial 수. 취소돼 시작조차 못 한 dial 은 세지 않는다.
    pub fn dials(&self) -> usize {
        self.inner.lock().unwrap().dials
    }

    /// 가장 오래된 「아직 안 가져간」 통로의 상대편.
    pub fn accept(&self) -> Option<Arc<MemoryEndpoint>> {
        self.inner.lock().unwrap().opened.pop_front()
    }

    /// 마지막으로 열린 통로의 상대편(앞의 것은 버린다).
    pub fn latest(&self) -> Option<Arc<MemoryEndpoint>> {
        let mut st = self.inner.lock().unwrap();
        let last = st.opened.pop_back();
        st.opened.clear();
        last
    }

    /// 아직 아무도 안 가져간 통로 수.
    pub fn open_endpoints(&self) -> usize {
        self.inner.lock().unwrap().opened.len()
    }
}

impl Dialer for MemoryNetwork {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError>> {
        // ★통로를 future **안에서** 만든다★ — 밖에서 만들면 취소된 dial 이 아무도 안 쓰는 통로를
        //   명부에 남기고, 테스트가 `accept()` 로 그것을 집어 감독이 채택한 적 없는 채널에 단언한다.
        let inner = self.inner.clone();
        Box::pin(async move {
            let mut st = inner.lock().unwrap();
            st.dials += 1;
            if st.fail_forever || st.fail_next > 0 {
                st.fail_next = st.fail_next.saturating_sub(1);
                return Err(LinkError::new("connection refused"));
            }
            let (to_server, from_client) = mpsc::unbounded_channel();
            let (to_client, from_server) = mpsc::unbounded_channel();
            let (stall_tx, stall_rx) = watch::channel(false);
            st.opened.push_back(Arc::new(MemoryEndpoint {
                to_client,
                from_client: Mutex::new(from_client),
                stall_tx,
            }));
            let tx: Box<dyn LinkTx> = Box::new(MemTx {
                to_server,
                stall_rx,
            });
            let rx: Box<dyn LinkRx> = Box::new(MemRx {
                from_server,
                failed: false,
            });
            Ok((tx, rx))
        })
    }
}

// ── 핸드셰이크 ────────────────────────────────────────────────────────────────

/// 왕복 없이 곧장 운영 단계로.
pub struct ImmediateHandshake;

impl Handshake for ImmediateHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Done
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Done
    }
}

/// 두 걸음 — `hello` 를 보내고 `welcome` 을 받으면 운영 단계로, 아니면 거절.
pub struct HelloHandshake;

impl HelloHandshake {
    pub const HELLO: &'static str = "hello";
    pub const WELCOME: &'static str = "welcome";
}

impl Handshake for HelloHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Send(Frame::Text(Self::HELLO.into()))
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        match frame {
            Frame::Text(t) if t == Self::WELCOME => HandshakeStep::Done,
            other => HandshakeStep::Reject {
                last: None,
                reason: format!("unexpected handshake frame: {other:?}"),
            },
        }
    }
}

/// 붙자마자 거절 — ★예산을 안 쓰는 경로★를 재는 데 쓴다.
pub struct RejectingHandshake {
    pub reason: String,
}

impl RejectingHandshake {
    pub fn new(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            reason: reason.into(),
        })
    }
}

impl Handshake for RejectingHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Reject {
            last: None,
            reason: self.reason.clone(),
        }
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Reject {
            last: None,
            reason: self.reason.clone(),
        }
    }
}

/// 아무것도 안 보내고 상대의 첫 프레임을 기다린다 — 상대가 **닫기로** 말하는 경로를 재는 데 쓴다.
pub struct ListeningHandshake;

impl Handshake for ListeningHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Await
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Done
    }
}

// ── 가짜 패킷 어휘 ────────────────────────────────────────────────────────────

/// 하네스가 보내는 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOut {
    Request { tag: u64, body: String },
    Notice(String),
    Resume { key: u32, after: Option<u64> },
    Blob(Vec<u8>),
}

/// 하네스가 받는 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestIn {
    Reply {
        tag: u64,
        body: String,
    },
    Chunk {
        key: u32,
        generation: u32,
        seq: u64,
        body: String,
    },
    Notice(String),
    /// 답장이면서 스트림 조각이라고 신고한다 — [`Wire`] 계약 위반 관측용.
    Ambiguous {
        tag: u64,
        key: u32,
        generation: u32,
        seq: u64,
    },
    Blob(Vec<u8>),
}

/// 최소 패킷 어휘. `|` 로 칸을 나눈 텍스트라 serde 없이 왕복한다.
pub struct TestWire;

impl TestWire {
    /// 상대편이 클라에게 밀어 넣을 프레임을 짓는다.
    pub fn encode_in(inn: &TestIn) -> Frame {
        match inn {
            TestIn::Reply { tag, body } => Frame::Text(format!("rep|{tag}|{body}")),
            TestIn::Chunk {
                key,
                generation,
                seq,
                body,
            } => Frame::Text(format!("chk|{key}|{generation}|{seq}|{body}")),
            TestIn::Notice(body) => Frame::Text(format!("not|{body}")),
            TestIn::Ambiguous {
                tag,
                key,
                generation,
                seq,
            } => Frame::Text(format!("amb|{tag}|{key}|{generation}|{seq}")),
            TestIn::Blob(bytes) => Frame::Binary(bytes.clone()),
        }
    }

    /// 스트림 조각 하나를 만드는 지름길.
    pub fn chunk(key: u32, generation: u32, seq: u64) -> Frame {
        Self::encode_in(&TestIn::Chunk {
            key,
            generation,
            seq,
            body: format!("s{seq}"),
        })
    }

    /// 답장 하나를 만드는 지름길.
    pub fn reply(tag: u64) -> Frame {
        Self::encode_in(&TestIn::Reply {
            tag,
            body: "ok".into(),
        })
    }
}

impl Wire for TestWire {
    type Out = TestOut;
    type In = TestIn;
    type Tag = u64;
    type StreamKey = u32;
    type DecodeError = String;

    fn encode(&self, out: &Self::Out) -> Frame {
        match out {
            TestOut::Request { tag, body } => Frame::Text(format!("req|{tag}|{body}")),
            TestOut::Notice(body) => Frame::Text(format!("not|{body}")),
            TestOut::Resume { key, after } => {
                let after = after.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
                Frame::Text(format!("res|{key}|{after}"))
            }
            TestOut::Blob(bytes) => Frame::Binary(bytes.clone()),
        }
    }

    fn decode(&self, frame: Frame) -> Result<Self::In, Self::DecodeError> {
        let text = match frame {
            Frame::Binary(bytes) => return Ok(TestIn::Blob(bytes)),
            Frame::Keepalive => return Err("keepalive must not reach decode".into()),
            Frame::Text(t) => t,
        };
        let parts: Vec<&str> = text.splitn(5, '|').collect();
        let num = |s: &str| -> Result<u64, String> {
            s.parse::<u64>()
                .map_err(|e| format!("bad number {s:?}: {e}"))
        };
        match parts.as_slice() {
            ["rep", tag, body] => Ok(TestIn::Reply {
                tag: num(tag)?,
                body: (*body).to_string(),
            }),
            ["chk", key, generation, seq, body] => Ok(TestIn::Chunk {
                key: num(key)? as u32,
                generation: num(generation)? as u32,
                seq: num(seq)?,
                body: (*body).to_string(),
            }),
            ["not", body] => Ok(TestIn::Notice((*body).to_string())),
            ["amb", tag, key, generation, seq] => Ok(TestIn::Ambiguous {
                tag: num(tag)?,
                key: num(key)? as u32,
                generation: num(generation)? as u32,
                seq: num(seq)?,
            }),
            _ => Err(format!("unknown frame: {text:?}")),
        }
    }

    fn request_tag(&self, out: &Self::Out) -> Option<Self::Tag> {
        match out {
            TestOut::Request { tag, .. } => Some(*tag),
            _ => None,
        }
    }

    fn reply_tag(&self, inn: &Self::In) -> Option<Self::Tag> {
        match inn {
            TestIn::Reply { tag, .. } | TestIn::Ambiguous { tag, .. } => Some(*tag),
            _ => None,
        }
    }

    fn stream_mark(&self, inn: &Self::In) -> Option<StreamMark<Self::StreamKey>> {
        match inn {
            TestIn::Chunk {
                key,
                generation,
                seq,
                ..
            }
            | TestIn::Ambiguous {
                key,
                generation,
                seq,
                ..
            } => Some(StreamMark {
                key: *key,
                generation: *generation,
                seq: *seq,
            }),
            _ => None,
        }
    }

    fn resume_request(&self, key: &Self::StreamKey, after: Option<u64>) -> Self::Out {
        TestOut::Resume { key: *key, after }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── H1: 멈춘 쓰기는 실제로 깨어나야 한다 ──
    //
    // ★멈춘 작업을 **다른 태스크**에 둔다★ — 같은 태스크에서 손으로 poll 하면 멈춤을 푼 뒤 우리가 직접
    //   다시 poll 하게 되고, 그러면 **waker 를 등록하지 않는 구현도 통과한다**. 그 형태로 쓴 옛 판은
    //   반례 변이(멈춤 중 `Pending` 을 내고 waker 는 등록하지 않는 `poll_fn`)를 하나도 못 잡았다
    //   (실측 2026-09-07). 태스크에 두면 깨우는 것 말고는 완료로 가는 길이 없다.
    #[tokio::test]
    async fn clearing_the_stall_actually_wakes_the_parked_write() {
        let net = MemoryNetwork::new();
        let (mut tx, _rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.stall_writes(true);

        let parked = tokio::spawn(async move { tx.send(Frame::Text("payload".into())).await });
        settle().await;
        assert!(!parked.is_finished(), "막혀 있어야 한다");
        assert!(endpoint.frames().is_empty());

        // ★멈춤을 푸는 것은 잠든 태스크가 아니라 이쪽이다★ — 저쪽을 깨우는 유일한 수단이 waker 다.
        endpoint.stall_writes(false);
        // ★유계 대기★ — 깨울 길 없는 멈춤으로 되돌아가면 매달리는 대신 깨끗이 실패해야 한다.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 쓰기가 안 깨어났다")
            .expect("쓰기 태스크가 패닉했다")
            .unwrap();
        assert_eq!(endpoint.frames(), vec![Frame::Text("payload".into())]);
    }

    // ── H1b: 멈춘 **닫기**도 실제로 깨어나야 한다 ──
    #[tokio::test]
    async fn clearing_the_stall_actually_wakes_the_parked_close() {
        let net = MemoryNetwork::new();
        let (mut tx, _rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.stall_writes(true);

        let parked = tokio::spawn(async move { tx.close(Close::going_away()).await });
        settle().await;
        assert!(
            !parked.is_finished(),
            "★닫기도 막혀야 한다★ — 안 막히면 하네스가 실소켓의 모양을 표현하지 못한다"
        );
        assert!(endpoint.drain().is_empty());

        endpoint.stall_writes(false);
        // ★유계 대기★ — 깨울 길 없는 멈춤으로 되돌아가면 매달리는 대신 깨끗이 실패해야 한다.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 닫기가 안 깨어났다")
            .expect("닫기 태스크가 패닉했다");
        assert!(matches!(
            endpoint.drain().as_slice(),
            [ClientMsg::Close(c)] if c.code == crate::frame::CloseCode::GOING_AWAY
        ));
    }

    // ── H2: 버려진 타이머가 쌓이지 않고, 깨우는 순서가 시한 순이다 ──
    #[tokio::test]
    async fn abandoned_timers_do_not_pile_up() {
        let clock = ManualClock::new();
        for _ in 0..1000 {
            drop(clock.sleep(Duration::from_secs(60)));
        }
        assert_eq!(clock.sleepers(), 0);

        let kept = clock.sleep(Duration::from_secs(60));
        assert_eq!(clock.sleepers(), 1);
        drop(kept);
        assert_eq!(clock.sleepers(), 0);
    }

    #[tokio::test]
    async fn one_advance_wakes_in_deadline_order_not_registration_order() {
        let clock = ManualClock::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        // 늦은 시한을 먼저 등록한다 — 등록 순서로 깨우면 뒤집힌다.
        for (label, secs) in [("late", 9u64), ("early", 1), ("middle", 5)] {
            let sleep = clock.sleep(Duration::from_secs(secs));
            let order = order.clone();
            tokio::spawn(async move {
                sleep.await;
                order.lock().unwrap().push(label);
            });
        }
        settle().await;
        clock.advance(Duration::from_secs(10));
        settle().await;
        assert_eq!(*order.lock().unwrap(), vec!["early", "middle", "late"]);
    }

    // ── H3: 정착하지 않으면 시끄럽게 실패한다 ──
    #[tokio::test]
    #[should_panic(expected = "정착하지 않았다")]
    async fn settle_until_says_so_instead_of_failing_somewhere_else() {
        settle_until("절대 참이 안 되는 조건", || false).await;
    }

    #[tokio::test]
    async fn settle_until_returns_as_soon_as_the_condition_holds() {
        let flag = Arc::new(Mutex::new(false));
        let setter = flag.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            *setter.lock().unwrap() = true;
        });
        settle_until("깃발", || *flag.lock().unwrap()).await;
        assert!(*flag.lock().unwrap());
    }

    // ── H4: 취소된 dial 은 통로를 남기지 않는다 ──
    #[tokio::test]
    async fn a_dial_that_is_never_polled_opens_nothing() {
        let net = MemoryNetwork::new();
        let never_polled = net.dial(&Address::new("mem://x"));
        drop(never_polled);
        assert_eq!(net.open_endpoints(), 0);
        assert_eq!(net.dials(), 0, "폴링조차 안 된 dial 은 시도가 아니다");
    }

    #[tokio::test]
    async fn a_completed_dial_registers_exactly_one_endpoint() {
        let net = MemoryNetwork::new();
        let _link = net.dial(&Address::new("mem://x")).await.unwrap();
        assert_eq!(net.open_endpoints(), 1);
        assert_eq!(net.dials(), 1);
    }

    // ── H5: 하네스도 「실패 뒤에는 닫기를 내지 않는다」 걸쇠를 진다 ──
    //
    // ★실패 **뒤에 닫기를 줄 세우는** 것이 요점이다★ — 걸쇠가 없으면 둘째 읽기가 큐에 남은 그것을
    //   「상대가 말했다」로 올려, [`crate::LinkRead::Closed`] 계약이 하네스에서만 거짓이 된다.
    //   실 어댑터 쪽 짝 = `tests/ws_reject.rs` 의
    //   `a_read_after_a_failed_read_is_still_an_error_never_a_spoken_close`.
    #[tokio::test]
    async fn a_read_after_a_failed_read_is_still_an_error_in_the_harness_too() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.fail("boom");
        endpoint.close();
        assert!(rx.recv().await.is_err(), "첫 읽기는 실패다");
        match rx.recv().await {
            Err(_) => {}
            Ok(other) => panic!("실패 뒤에 닫기가 나왔다: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_peer_can_close_with_a_code_and_the_reader_sees_it() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.reject(Close::new(
            crate::frame::CloseCode::HANDSHAKE_REJECTED,
            "no",
        ));
        match rx.recv().await.unwrap() {
            LinkRead::Closed(Some(close)) => {
                assert_eq!(close.code, crate::frame::CloseCode::HANDSHAKE_REJECTED);
                assert_eq!(close.reason, "no");
            }
            other => panic!("{other:?}"),
        }
    }
}
