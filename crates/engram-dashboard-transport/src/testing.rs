//! 하네스 전용 — 기능 플래그 `test-support` 뒤에 산다(ADR-0012, 선례 = `engram-dashboard-command`).
//!
//! ★여기 있는 것이 「백오프·keepalive·쓰기 시한을 **실제로 기다리지 않는다**」의 실물이다★
//! (ADR-0177 결정 8): [`ManualClock`] 이 시간을, [`MemoryNetwork`] · [`MemoryListener`] 가 전송을 대신한다.
//! ★둘은 먼 끝이 다르다★ — [`MemoryNetwork`] 의 먼 끝은 시험이 손으로 모는 대본([`MemoryEndpoint`])이고,
//! [`MemoryListener`] 의 먼 끝은 진짜 [`LinkTx`]/[`LinkRx`] 한 쌍([`Accepted`])이라 두 감독을 서로 잇는다.
//!
//! ★**[`settle`] · [`settle_until`] 만 current_thread 런타임 전용이다**★ — 협조적 양보로만 동기화하므로
//! `#[tokio::test(flavor = "multi_thread")]` 아래에서는 아무것도 보장하지 않는다. 그 위반은 조용히
//! 흔들리는 테스트가 되므로 둘이 **런타임 종류를 직접 보고 패닉한다.** 그 위에 선 시험 도우미(`registry.rs` 시험의
//! `Near::arrived` 처럼 기다리지 않고 한 번만 폴링하는 것)도 같은 전제다. 인메모리 부품([`MemoryNetwork`] ·
//! [`memory_link`] · [`ManualClock`] …)과 [`PermitPool`] 은 multi_thread 에서도 쓴다 — 끝은 실시간 시한을 건 대기로
//! 본다(`registry.rs` 의 3-0 U6b 시험).
//!
//! ★운영 코드가 이 모듈을 참조하면 안 된다★ — 기본 빌드에 없으므로 컴파일이 먼저 깨진다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::frame::{Close, Frame};
use crate::link::{
    Accepted, Address, DialFailure, Dialer, Handshake, HandshakeStep, LinkError, LinkInfo,
    LinkRead, LinkRx, LinkTx, Listener, PermitPool, RejectKind,
};
use crate::stream::StreamMark;
use crate::wire::{DecodeClass, Wire};

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
    /// 밀어 넣었지만 클라가 아직 안 읽은 수 — `MemRx` 와 함께 쥔다.
    unread: Arc<AtomicUsize>,
    from_client: Mutex<mpsc::UnboundedReceiver<ClientMsg>>,
    stall_tx: watch::Sender<bool>,
    ping_stall_tx: watch::Sender<bool>,
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
        self.send(ServerMsg::Frame(frame));
    }

    /// 밀어 넣었지만 클라가 아직 안 읽은 것(프레임 · 닫기 · 실패)의 수. ★배압이 통로까지 닿았나를 이것으로
    /// 본다★ — 감독이 앞잡이 줄을 안 비우거나 받아 쥔 바이트가 읽기 몫 이상이면 읽기 태스크가 통로를 안 읽어
    /// 여기 남는다.
    pub fn unread(&self) -> usize {
        self.unread.load(Ordering::Acquire)
    }

    /// ★보내기 전에 센다★ — 뒤에 세면 그 사이 읽은 쪽이 먼저 빼 셈이 0 밑으로 감긴다.
    fn send(&self, msg: ServerMsg) {
        self.unread.fetch_add(1, Ordering::AcqRel);
        if self.to_client.send(msg).is_err() {
            self.unread.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// ★코드 **없이** 닫는다고 말한다★ — `LinkRead::Closed(None)` 이고, 계약상 이것도 「상대가 닫았다고
    /// 말했다」다(말 없이 사라진 것은 [`MemoryEndpoint::fail`] 쪽 = `Err`). 클라는 닫힘은 알고 사유는
    /// 못 얻는다.
    ///
    /// ★이 통로를 **그냥 떨어뜨리는 것**(엔드포인트를 drop)은 이 갈래가 아니다★ — 그건 말 없이 사라진
    /// 것이라 `MemRx` 가 읽기 오류로 낸다(실소켓과 같은 모양). 「닫았다고 말했다」를 만들려면 이것이나
    /// [`MemoryEndpoint::reject`] 를 불러야 한다.
    pub fn close(&self) {
        self.send(ServerMsg::Closed(None));
    }

    /// ★코드와 문구를 실어 닫는다★ — 「붙었는데 거절당했다」를 클라가 알아볼 수 있는 유일한 경로다.
    pub fn reject(&self, close: Close) {
        self.send(ServerMsg::Closed(Some(close)));
    }

    /// 오류로 끊는다. ★읽는 쪽에만 닿는다★ — 클라의 다음 읽기가 오류다(쓰기는 그대로 나간다).
    pub fn fail(&self, why: &str) {
        self.send(ServerMsg::Fail(LinkError::new(why)));
    }

    /// ★쓰는 쪽만 끊는다★ — 클라의 다음 `send` · `ping` 이 오류로 끝나고(멈춤이 걸려 있으면 풀린 뒤) 읽기는
    /// 그대로 산다. [`MemoryEndpoint::fail`] 의 짝이다 — 엔드포인트를 떨어뜨리면 두 쪽이 함께 끊겨 어느 쪽이
    /// 먼저 알려지는지가 스케줄에 달린다. 이미 보낸 것은 [`MemoryEndpoint::take`] 로 계속 꺼낼 수 있다.
    pub fn fail_writes(&self) {
        self.from_client.lock().unwrap().close();
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

    /// 켜면 클라의 `ping` **만** 멈춘다 — [`MemoryEndpoint::stall_writes`] 와 겹쳐 걸리고, 끄면 실제로 깨어난다.
    /// ★실소켓에는 없는 모양이다★(ping 도 같은 통로로 나간다) — 「앞선 쓰기는 끝났고 그 직후 타이머 바퀴의
    /// ping 에서 선다」를 만들려고만 둔다. 둘을 한 손잡이로는 못 가른다: 멈춤을 푸는 순간 감독이 양보 없이
    /// 다음 쓰기까지 밀고 간다.
    pub fn stall_pings(&self, on: bool) {
        let _ = self.ping_stall_tx.send(on);
    }
}

struct MemTx {
    to_server: mpsc::UnboundedSender<ClientMsg>,
    stall_rx: watch::Receiver<bool>,
    ping_stall_rx: watch::Receiver<bool>,
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
            let _ = self.ping_stall_rx.wait_for(|stalled| !*stalled).await;
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
    unread: Arc<AtomicUsize>,
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
            let next = self.from_server.recv().await;
            if next.is_some() {
                self.unread.fetch_sub(1, Ordering::AcqRel);
            }
            match next {
                Some(ServerMsg::Frame(f)) => Ok(LinkRead::Frame(f)),
                Some(ServerMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★채널 소진 = 말 없이 사라졌다★ — 실소켓에서 같은 일(닫기 핸드셰이크 없이 버려진 소켓)은
                //   읽기 오류이므로 여기서도 오류다. `Ok(Closed(None))` 으로 올리던 판은
                //   [`crate::LinkRead::Closed`] 계약(「상대가 **말했다**」)을 하네스에서만 거짓으로 만들어,
                //   엔드포인트를 떨어뜨려 죽음을 흉내내는 테스트가 `PeerClosed` 로 초록이고 실소켓에서는
                //   통로 읽기 오류(지금 `ReadFailed`)로 갈리게 했다.
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
    refuse: Option<Close>,
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
                refuse: None,
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

    /// 지금부터 모든 dial 이 통로 없이 [`DialFailure::Refused`] 로 끝난다 — 거는 쪽이 이미 아는 종결 거절(로컬 버전
    /// 대조)을 흉내낸다. [`MemoryNetwork::fail_forever`] · [`MemoryNetwork::fail_next`] 보다 앞선다. `close.code` 는
    /// 종결 거절 코드여야 한다(그 갈래의 계약).
    pub fn refuse(&self, close: Close) {
        self.inner.lock().unwrap().refuse = Some(close);
    }

    /// 다시 붙을 수 있게 한다.
    pub fn allow(&self) {
        let mut st = self.inner.lock().unwrap();
        st.fail_forever = false;
        st.fail_next = 0;
        st.refuse = None;
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
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★통로를 future **안에서** 만든다★ — 밖에서 만들면 취소된 dial 이 아무도 안 쓰는 통로를
        //   명부에 남기고, 테스트가 `accept()` 로 그것을 집어 감독이 채택한 적 없는 채널에 단언한다.
        let inner = self.inner.clone();
        Box::pin(async move {
            let mut st = inner.lock().unwrap();
            st.dials += 1;
            if let Some(close) = &st.refuse {
                return Err(DialFailure::Refused {
                    code: close.code,
                    reason: close.reason.clone(),
                });
            }
            if st.fail_forever || st.fail_next > 0 {
                st.fail_next = st.fail_next.saturating_sub(1);
                return Err(LinkError::new("connection refused").into());
            }
            let (to_server, from_client) = mpsc::unbounded_channel();
            let (to_client, from_server) = mpsc::unbounded_channel();
            let (stall_tx, stall_rx) = watch::channel(false);
            let (ping_stall_tx, ping_stall_rx) = watch::channel(false);
            let unread = Arc::new(AtomicUsize::new(0));
            st.opened.push_back(Arc::new(MemoryEndpoint {
                to_client,
                unread: unread.clone(),
                from_client: Mutex::new(from_client),
                stall_tx,
                ping_stall_tx,
            }));
            let tx: Box<dyn LinkTx> = Box::new(MemTx {
                to_server,
                stall_rx,
                ping_stall_rx,
            });
            let rx: Box<dyn LinkRx> = Box::new(MemRx {
                from_server,
                unread,
                failed: false,
            });
            Ok((tx, rx))
        })
    }
}

// ── 양끝 통로 · 받는 쪽 ───────────────────────────────────────────────────────

/// 양끝 통로 위로 건너가는 것.
#[derive(Debug)]
enum PipeMsg {
    Frame(Frame),
    /// 받는 끝이 [`Frame::Keepalive`] 로 올리고 대답(`Pong`)을 돌려보낸다.
    Ping,
    Pong,
    Closed(Option<Close>),
    /// [`MemoryLinkControl::fail_reads`] 가 넣는다.
    Fail(LinkError),
}

/// 양끝 통로 한 끝의 상태 — 그 끝의 두 반쪽과 그 끝의 손잡이가 함께 쥔다.
struct EndState {
    stall: watch::Sender<bool>,
    ping_stall: watch::Sender<bool>,
    writes_failed: AtomicBool,
    /// 이 끝에 들어왔지만 아직 안 읽은 수.
    unread: AtomicUsize,
    /// 살아 있는 반쪽 수(처음 2). 0 = 이 끝이 통째로 떨어졌다.
    halves: AtomicUsize,
    /// 이 끝의 들어오는 줄 — 손잡이가 실패를 넣는 자리. ★약한 손잡이다★ — 강하면 손잡이를 쥔 시험 때문에 상대가
    /// 사라져도 줄이 안 말라, 이 끝이 「말 없이 사라졌다」(읽기 오류)를 못 본다.
    inbound: mpsc::WeakUnboundedSender<PipeMsg>,
}

impl EndState {
    fn new(inbound: mpsc::WeakUnboundedSender<PipeMsg>) -> Self {
        Self {
            stall: watch::channel(false).0,
            ping_stall: watch::channel(false).0,
            writes_failed: AtomicBool::new(false),
            unread: AtomicUsize::new(0),
            halves: AtomicUsize::new(2),
            inbound,
        }
    }
}

/// `dest` 끝의 줄에 하나 넣는다. ★넣기 전에 센다★ — 사유는 `MemoryEndpoint::send`.
fn deliver(dest: &EndState, line: &mpsc::UnboundedSender<PipeMsg>, msg: PipeMsg) -> bool {
    dest.unread.fetch_add(1, Ordering::AcqRel);
    if line.send(msg).is_err() {
        dest.unread.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    true
}

struct MemLinkTx {
    me: Arc<EndState>,
    peer: Arc<EndState>,
    to_peer: mpsc::UnboundedSender<PipeMsg>,
    stall: watch::Receiver<bool>,
    ping_stall: watch::Receiver<bool>,
}

impl MemLinkTx {
    async fn wait_writable(&mut self) {
        let _ = self.stall.wait_for(|stalled| !*stalled).await;
    }

    /// 두 멈춤이 **함께** 풀려 있을 때 돌아온다 — 깰 때마다 다른 쪽을 다시 본다. ★한 번씩 차례로 지나가게 되돌리지
    /// 말 것★ — 쓰기 멈춤을 지나 ping 멈춤에서 선 동안 쓰기 멈춤이 걸리면, ping 멈춤만 풀어도 ping 이 나갔다(ping 도
    /// 쓰기 통로로 나간다 — [`MemoryLinkControl::stall_writes`]).
    async fn wait_pingable(&mut self) {
        loop {
            self.wait_writable().await;
            let _ = self.ping_stall.wait_for(|stalled| !*stalled).await;
            if !*self.stall.borrow() {
                return;
            }
        }
    }

    /// ★상대 끝의 읽는 반쪽만 떨어졌으면 쓰기는 성공한다★ — 실소켓도 한 끝의 두 반쪽이 둘 다 떨어져야 닫힌다
    /// ([`LinkTx`] rustdoc). 그때 넣은 것은 읽을 이가 없어 사라진다.
    fn put(&self, msg: PipeMsg) -> Result<(), LinkError> {
        if self.me.writes_failed.load(Ordering::Acquire) {
            return Err(LinkError::new("writes failed"));
        }
        if deliver(&self.peer, &self.to_peer, msg) || self.peer.halves.load(Ordering::Acquire) > 0 {
            Ok(())
        } else {
            Err(LinkError::new("peer end gone"))
        }
    }
}

impl LinkTx for MemLinkTx {
    /// [`Frame::Keepalive`] 는 `ping` 과 같이 나간다 — WS 어댑터가 그것을 Ping 으로 옮기는 것과 맞춘다.
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            self.put(match frame {
                Frame::Keepalive => PipeMsg::Ping,
                other => PipeMsg::Frame(other),
            })
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_pingable().await;
            self.put(PipeMsg::Ping)
        })
    }

    /// 멈춤을 `send` · `ping` 과 같이 탄다 — 그 사유는 `MemTx::close`.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.wait_writable().await;
            let _ = self.put(PipeMsg::Closed(Some(close)));
        })
    }
}

impl Drop for MemLinkTx {
    fn drop(&mut self) {
        self.me.halves.fetch_sub(1, Ordering::AcqRel);
    }
}

struct MemLinkRx {
    me: Arc<EndState>,
    peer: Arc<EndState>,
    from_peer: mpsc::UnboundedReceiver<PipeMsg>,
    /// 받은 Ping 의 대답을 돌려보내는 자리 — 실소켓에서도 대답은 읽는 쪽이 낸다(`ws.rs` 「들어온 Ping 에 대한 자동
    /// Pong」). ★그래서 이 끝이 안 읽으면 상대의 Ping 은 대답을 못 받는다★.
    to_peer: mpsc::UnboundedSender<PipeMsg>,
    /// [`LinkRx::recv`] 계약(실패 뒤에는 닫기를 내지 않는다)의 실물 — `MemRx` 와 같다.
    failed: bool,
}

impl LinkRx for MemLinkRx {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            if self.failed {
                return Err(LinkError::new("link already failed"));
            }
            let next = self.from_peer.recv().await;
            if next.is_some() {
                self.me.unread.fetch_sub(1, Ordering::AcqRel);
            }
            match next {
                Some(PipeMsg::Frame(f)) => Ok(LinkRead::Frame(f)),
                Some(PipeMsg::Ping) => {
                    // 대답은 멈춤을 안 탄다 — 실소켓에서는 혼잡한 송신 버퍼 뒤에 늦게 나간다(그 모양은 실소켓 몫).
                    if !self.me.writes_failed.load(Ordering::Acquire) {
                        deliver(&self.peer, &self.to_peer, PipeMsg::Pong);
                    }
                    Ok(LinkRead::Frame(Frame::Keepalive))
                }
                Some(PipeMsg::Pong) => Ok(LinkRead::Frame(Frame::Keepalive)),
                Some(PipeMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★줄 소진 = 상대 끝의 두 반쪽이 다 떨어졌다 = 말 없이 사라졌다★ — 닫기가 아니라 읽기 오류다
                //   (`MemRx` 의 같은 팔 · [`crate::LinkRead::Closed`] 계약).
                None => {
                    self.failed = true;
                    Err(LinkError::new("peer end gone"))
                }
                Some(PipeMsg::Fail(e)) => {
                    self.failed = true;
                    Err(e)
                }
            }
        })
    }
}

impl Drop for MemLinkRx {
    fn drop(&mut self) {
        self.me.halves.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 양끝 통로의 한 끝 — 두 반쪽과 그 끝의 손잡이.
pub struct MemoryLinkEnd {
    pub tx: Box<dyn LinkTx>,
    pub rx: Box<dyn LinkRx>,
    pub control: MemoryLinkControl,
}

/// 양끝 통로의 한 끝을 조작하는 손잡이 — ★제 끝에만 작용한다★. [`MemoryEndpoint`] 의 손잡이는 먼 끝이 클라 끝에
/// 거는 것이라 이름이 갈린다: 그쪽 `fail` = 여기 [`MemoryLinkControl::fail_reads`]. 나머지는 이름이 같다.
///
/// 손잡이를 쥐고 있어도 그 끝이 살아 있게 되지는 않는다 — 두 반쪽이 다 떨어지면 상대는 읽기 오류를 본다.
#[derive(Clone)]
pub struct MemoryLinkControl {
    state: Arc<EndState>,
}

impl MemoryLinkControl {
    /// 켜면 이 끝의 `send` · `ping` · `close` 가 멈추고 끄면 실제로 깨어난다([`MemoryEndpoint::stall_writes`] 와 같은
    /// 계약 — 그쪽 rustdoc 의 주의도 같다).
    pub fn stall_writes(&self, on: bool) {
        self.state.stall.send_replace(on);
    }

    pub fn writes_stalled(&self) -> bool {
        *self.state.stall.borrow()
    }

    /// 켜면 이 끝의 `ping` **만** 멈춘다([`MemoryEndpoint::stall_pings`] 와 같다 — 실소켓에는 없는 모양이다).
    pub fn stall_pings(&self, on: bool) {
        self.state.ping_stall.send_replace(on);
    }

    /// 이 끝의 읽기를 오류로 끊는다 — 이미 들어와 있는 것 뒤에 줄 선다. 쓰기는 그대로 나간다.
    pub fn fail_reads(&self, why: &str) {
        if let Some(line) = self.state.inbound.upgrade() {
            deliver(&self.state, &line, PipeMsg::Fail(LinkError::new(why)));
        }
    }

    /// 이 끝의 다음 `send` · `ping` 이 오류로 끝난다(멈춤이 걸려 있으면 풀린 뒤). `close` 는 아무것도 안 보낸다.
    /// 읽기는 그대로 산다.
    pub fn fail_writes(&self) {
        self.state.writes_failed.store(true, Ordering::Release);
    }

    /// 이 끝에 들어왔지만 아직 안 읽은 것(프레임 · 대답 · 닫기 · 실패)의 수.
    pub fn unread(&self) -> usize {
        self.state.unread.load(Ordering::Acquire)
    }

    /// 이 끝의 두 반쪽이 다 떨어졌나.
    pub fn dropped(&self) -> bool {
        self.state.halves.load(Ordering::Acquire) == 0
    }
}

/// 서로 이어진 두 끝을 짓는다 — 한 끝이 보낸 것을 다른 끝이 읽는다.
///
/// ★상한 없는 줄이다★ — 안 읽는 상대 앞에서 쓰기가 서는 모양(커널 버퍼)은 없다. 그건 실소켓 몫이다.
pub fn memory_link() -> (MemoryLinkEnd, MemoryLinkEnd) {
    let (into_a, a_from) = mpsc::unbounded_channel();
    let (into_b, b_from) = mpsc::unbounded_channel();
    let a = Arc::new(EndState::new(into_a.downgrade()));
    let b = Arc::new(EndState::new(into_b.downgrade()));
    let end = |me: &Arc<EndState>,
               peer: &Arc<EndState>,
               from_peer: mpsc::UnboundedReceiver<PipeMsg>,
               to_peer: mpsc::UnboundedSender<PipeMsg>| MemoryLinkEnd {
        tx: Box::new(MemLinkTx {
            me: me.clone(),
            peer: peer.clone(),
            to_peer: to_peer.clone(),
            stall: me.stall.subscribe(),
            ping_stall: me.ping_stall.subscribe(),
        }),
        rx: Box::new(MemLinkRx {
            me: me.clone(),
            peer: peer.clone(),
            from_peer,
            to_peer,
            failed: false,
        }),
        control: MemoryLinkControl { state: me.clone() },
    };
    (end(&a, &b, a_from, into_b), end(&b, &a, b_from, into_a))
}

/// [`MemoryConnector`] 가 연 통로 하나의 두 손잡이.
#[derive(Clone)]
pub struct MemoryLinkControls {
    /// 거는 쪽(감독이 쥔 끝).
    pub dialer: MemoryLinkControl,
    /// 받는 쪽([`Accepted`] 에 실린 끝).
    pub acceptor: MemoryLinkControl,
}

struct ListenLog {
    dials: usize,
    links: Vec<MemoryLinkControls>,
}

struct ListenShared {
    clock: Arc<dyn Clock>,
    permits: PermitPool,
    log: Mutex<ListenLog>,
}

/// 인메모리 청취기 — [`MemoryConnector`] 로 건 통로를 [`Accepted`] 로 내놓는다.
///
/// ★건 순간이 업그레이드 끝이다★ — 인메모리 통로에는 업그레이드가 없어서, 걸기가 끝난 순간 그 통로가 준비 큐에
/// 앉고 [`Accepted::upgraded_at`] 이 그 시각이다. 칸([`MemoryListener::permits`])은 그 순간 쥐어 [`Accepted`] 와 함께
/// 간다 — 칸이 다 찼으면 그 걸기는 실패한다(넘친 새것을 닫는다 — TRD 3 §5 14).
///
/// ★[`Listener::accept`] 의 종결 `Err` = 모든 [`MemoryConnector`] 가 사라지고 준비 큐가 비었다★ — 더 걸 길이 없다.
/// 청취기를 떨어뜨리면 준비 큐에 앉은 통로가 닫히고(거는 쪽은 읽기 오류) 그 뒤의 걸기는 실패한다.
pub struct MemoryListener {
    ready: mpsc::UnboundedReceiver<Accepted>,
    permits: PermitPool,
}

impl MemoryListener {
    /// `clock` = 받은 상대를 얹을 명부와 같은 시계. `max_pending_upgrades` = 업그레이드 칸 수(준비 큐 + 받아 갔지만
    /// 아직 칸을 쥔 것) — TRD 3 §5 14 의 업그레이드 대기 상한이다. ★TCP listen backlog 가 아니다★ — 커널 수락 큐가
    /// 아니라 이 crate 가 세는 칸이다.
    pub fn new(clock: Arc<dyn Clock>, max_pending_upgrades: usize) -> (Self, MemoryConnector) {
        let (ready_tx, ready) = mpsc::unbounded_channel();
        let permits = PermitPool::new(max_pending_upgrades);
        let connector = MemoryConnector {
            ready: ready_tx,
            shared: Arc::new(ListenShared {
                clock,
                permits: permits.clone(),
                log: Mutex::new(ListenLog {
                    dials: 0,
                    links: Vec::new(),
                }),
            }),
        };
        (Self { ready, permits }, connector)
    }

    /// 업그레이드 칸 묶음 — 청취기를 떨어뜨린 뒤에도 셈을 읽을 수 있다.
    pub fn permits(&self) -> PermitPool {
        self.permits.clone()
    }

    /// 준비 큐에 앉아 아직 안 받아 간 수.
    pub fn queued(&self) -> usize {
        self.ready.len()
    }
}

impl Listener for MemoryListener {
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>> {
        Box::pin(async move {
            self.ready
                .recv()
                .await
                .ok_or_else(|| LinkError::new("every connector is gone"))
        })
    }
}

/// [`MemoryListener`] 로 거는 [`Dialer`]. clone 은 같은 청취기로 건다. 주소는 보지 않는다.
#[derive(Clone)]
pub struct MemoryConnector {
    ready: mpsc::UnboundedSender<Accepted>,
    shared: Arc<ListenShared>,
}

impl MemoryConnector {
    pub fn dialer(&self) -> Arc<dyn Dialer> {
        Arc::new(self.clone())
    }

    /// 지금까지 **실제로 폴링된** 걸기 수(실패 포함).
    pub fn dials(&self) -> usize {
        self.shared.log.lock().unwrap().dials
    }

    /// 준비 큐에 닿은 통로 수.
    pub fn links(&self) -> usize {
        self.shared.log.lock().unwrap().links.len()
    }

    /// `n` 번째(1 부터) 통로의 손잡이 — 그 [`Accepted`] 의 [`LinkInfo::remote`] 가 `mem:<n>` 이다.
    pub fn link(&self, n: usize) -> Option<MemoryLinkControls> {
        let log = self.shared.log.lock().unwrap();
        n.checked_sub(1).and_then(|i| log.links.get(i).cloned())
    }

    /// 마지막 통로의 손잡이.
    pub fn latest(&self) -> Option<MemoryLinkControls> {
        self.shared.log.lock().unwrap().links.last().cloned()
    }
}

impl Dialer for MemoryConnector {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★통로를 future **안에서** 만든다★ — [`Dialer`] 계약(취소된 걸기는 아무것도 남기지 않는다).
        let ready = self.ready.clone();
        let shared = self.shared.clone();
        Box::pin(async move {
            // ★주입된 시계는 장부 자물쇠 밖에서 읽는다★ — 시계의 `now` 가 이 연결기의 장부(`links` · `dials`)를 읽으면
            //   쥔 자물쇠를 다시 잡아 교착한다. 자물쇠 안에서는 주입받은 것을 부르지 않는다.
            let now = shared.clock.now();
            let mut log = shared.log.lock().unwrap();
            log.dials += 1;
            if ready.is_closed() {
                return Err(LinkError::new("listener gone").into());
            }
            let Some(permit) = shared.permits.try_take() else {
                return Err(LinkError::new("upgrade permits full").into());
            };
            let n = log.links.len() + 1;
            let (near, far) = memory_link();
            let info = LinkInfo {
                remote: Some(format!("mem:{n}")),
                ..LinkInfo::default()
            };
            let accepted = Accepted::new(far.tx, far.rx, info, now, permit);
            // 실패하면 돌려받은 `Accepted` 가 여기서 떨어져 칸 · 먼 끝이 함께 돌아간다.
            if ready.send(accepted).is_err() {
                return Err(LinkError::new("listener gone").into());
            }
            log.links.push(MemoryLinkControls {
                dialer: near.control,
                acceptor: far.control,
            });
            Ok((near.tx, near.rx))
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
                kind: RejectKind::Terminal,
                last: None,
                reason: format!("unexpected handshake frame: {other:?}"),
            },
        }
    }
}

/// 붙자마자 거절 — [`RejectingHandshake::new`] 는 종결(★예산을 안 쓰는 경로★), [`RejectingHandshake::retryable`] 은
/// 다시 거는 거절(예산을 쓰는 경로)을 재는 데 쓴다.
pub struct RejectingHandshake {
    pub kind: RejectKind,
    pub reason: String,
}

impl RejectingHandshake {
    pub fn new(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            kind: RejectKind::Terminal,
            reason: reason.into(),
        })
    }

    pub fn retryable(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            kind: RejectKind::Retryable,
            reason: reason.into(),
        })
    }

    fn reject(&self) -> HandshakeStep {
        HandshakeStep::Reject {
            kind: self.kind,
            last: None,
            reason: self.reason.clone(),
        }
    }
}

impl Handshake for RejectingHandshake {
    fn start(&self) -> HandshakeStep {
        self.reject()
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        self.reject()
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

/// [`TestWire`] 의 해독 실패. ★값을 실은 갈래는 받은 글을 그대로 인용한다★ — 사건에 그 글이 안 새는지를 재는
/// 재료다([`DecodeClass`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestDecodeError {
    /// [`Frame::Keepalive`] 가 해독기에 왔다 — crate 계약 위반.
    Keepalive,
    /// 숫자 칸을 못 읽었다.
    BadNumber(String),
    /// 모르는 모양.
    Unknown(String),
}

impl DecodeClass for TestDecodeError {
    fn class(&self) -> &'static str {
        match self {
            TestDecodeError::Keepalive => "keepalive",
            TestDecodeError::BadNumber(_) => "bad number",
            TestDecodeError::Unknown(_) => "unknown frame",
        }
    }
}

impl Wire for TestWire {
    type Out = TestOut;
    type In = TestIn;
    type Tag = u64;
    type StreamKey = u32;
    type DecodeError = TestDecodeError;

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
            Frame::Keepalive => return Err(TestDecodeError::Keepalive),
            Frame::Text(t) => t,
        };
        let parts: Vec<&str> = text.splitn(5, '|').collect();
        let num = |s: &str| -> Result<u64, TestDecodeError> {
            s.parse::<u64>()
                .map_err(|e| TestDecodeError::BadNumber(format!("{s:?}: {e}")))
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
            _ => Err(TestDecodeError::Unknown(text)),
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
    use crate::event::{Incoming, PeerId, TransportEvent};
    use crate::frame::CloseCode;
    use crate::machine::PeerState;
    use crate::policy::Policy;
    use crate::registry::Registry;
    use futures_util::future::poll_immediate;

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

    // ── H6: 엔드포인트를 떨어뜨린 것은 닫기가 아니라 읽기 오류다 ──
    //
    // ★`MemRx::recv` 의 채널 소진 팔을 지키는 것은 이 테스트뿐이다★ — H5 는 `fail` 로 `Fail` 팔만 타고,
    //   채널 소진 팔을 `Ok(Closed(None))` 으로 되돌려도 나머지 스위트는 전부 초록이었다(실측 2026-09-25).
    #[tokio::test]
    async fn dropping_the_endpoint_is_a_read_error_not_a_close() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        // `accept` 가 명부에서 꺼내 주므로 이 `Arc` 가 송신단의 유일한 소유자다.
        assert_eq!(Arc::strong_count(&endpoint), 1);
        drop(endpoint);
        match rx.recv().await {
            Err(_) => {}
            Ok(other) => panic!("말 없이 사라진 통로가 닫기로 올라왔다: {other:?}"),
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

    // ── H7: 양끝 통로 — 두 끝이 다 진짜 통로다 ──
    #[tokio::test]
    async fn a_two_ended_link_carries_frames_both_ways_and_a_spoken_close() {
        let (mut a, mut b) = memory_link();
        a.tx.send(Frame::Text("a to b".into())).await.unwrap();
        b.tx.send(Frame::Binary(vec![1, 2])).await.unwrap();
        assert_eq!((a.control.unread(), b.control.unread()), (1, 1));
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Text(t)) if t == "a to b")
        );
        assert!(
            matches!(a.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Binary(v)) if v == [1, 2])
        );
        assert_eq!((a.control.unread(), b.control.unread()), (0, 0));
        a.tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, "no"))
            .await;
        match b.rx.recv().await.unwrap() {
            LinkRead::Closed(Some(close)) => {
                assert_eq!(close.code, CloseCode::HANDSHAKE_REJECTED);
                assert_eq!(close.reason, "no");
            }
            other => panic!("{other:?}"),
        }
    }

    // 실소켓과 같다 — 한 끝의 두 반쪽이 둘 다 떨어져야 닫힌다(`LinkTx` rustdoc). 다 떨어진 끝은 상대에게 닫기가 아니라
    // 읽기 오류다(H6 의 양끝 판).
    #[tokio::test]
    async fn one_end_closes_only_once_both_of_its_halves_are_gone() {
        let (mut a, b) = memory_link();
        let MemoryLinkEnd {
            tx: b_tx,
            rx: b_rx,
            control: b_control,
        } = b;
        drop(b_rx);
        a.tx.send(Frame::Text("into the void".into()))
            .await
            .expect("상대 끝의 쓰는 반쪽이 살아 있으면 통로는 열려 있다");
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "아직 안 닫혔다"
        );
        assert!(!b_control.dropped());
        drop(b_tx);
        assert!(b_control.dropped());
        assert!(
            a.rx.recv().await.is_err(),
            "말 없이 사라진 끝은 읽기 오류다"
        );
        assert!(a.rx.recv().await.is_err(), "실패 뒤에도 오류다");
        assert!(a.tx.send(Frame::Text("x".into())).await.is_err());
    }

    #[tokio::test]
    async fn a_ping_is_answered_only_once_the_far_end_reads_it() {
        let (mut a, mut b) = memory_link();
        a.tx.ping().await.unwrap();
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "상대가 읽기 전에는 대답이 없다"
        );
        assert!(matches!(
            b.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
        assert!(matches!(
            a.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
        assert_eq!((a.control.unread(), b.control.unread()), (0, 0));
    }

    // H1 의 양끝 판 — 멈춘 쓰기를 다른 태스크에 두는 사유도 H1 과 같다.
    #[tokio::test]
    async fn clearing_a_two_ended_stall_actually_wakes_the_parked_write() {
        let (a, mut b) = memory_link();
        a.control.stall_writes(true);
        assert!(a.control.writes_stalled());
        let mut tx = a.tx;
        let parked = tokio::spawn(async move { tx.send(Frame::Text("late".into())).await });
        settle().await;
        assert!(!parked.is_finished(), "막혀 있어야 한다");
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 쓰기가 안 깨어났다")
            .expect("쓰기 태스크가 패닉했다")
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Text(t)) if t == "late")
        );
    }

    // ping 멈춤에서 선 동안 쓰기 멈춤이 걸리면, ping 멈춤만 풀어서는 안 나가고 두 멈춤이 다 풀려야 나간다.
    #[tokio::test]
    async fn a_ping_parked_on_its_own_stall_still_waits_for_a_write_stall_set_meanwhile() {
        let (a, mut b) = memory_link();
        a.control.stall_pings(true);
        let mut tx = a.tx;
        let parked = tokio::spawn(async move { tx.ping().await });
        settle().await;
        assert!(!parked.is_finished(), "ping 멈춤에서 서 있어야 한다");
        a.control.stall_writes(true);
        a.control.stall_pings(false);
        settle().await;
        assert!(
            !parked.is_finished(),
            "쓰기 멈춤이 걸려 있는데 ping 이 나갔다"
        );
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("두 멈춤을 다 풀었는데도 ping 이 안 깨어났다")
            .expect("ping 태스크가 패닉했다")
            .unwrap();
        assert!(matches!(
            b.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
    }

    #[tokio::test]
    async fn the_fail_knobs_act_on_their_own_end_and_one_direction_only() {
        let (mut a, mut b) = memory_link();
        b.tx.send(Frame::Text("before".into())).await.unwrap();
        a.control.fail_reads("boom");
        assert!(
            matches!(a.rx.recv().await.unwrap(), LinkRead::Frame(_)),
            "이미 들어온 것이 먼저다"
        );
        assert_eq!(a.rx.recv().await.unwrap_err().message, "boom");
        assert!(a.rx.recv().await.is_err());
        a.tx.send(Frame::Text("writes still go".into()))
            .await
            .expect("읽기 실패는 쓰기를 안 끊는다");
        assert!(matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)));

        b.control.fail_writes();
        assert!(b.tx.send(Frame::Text("x".into())).await.is_err());
        assert!(b.tx.ping().await.is_err());
        b.tx.close(Close::going_away()).await;
        assert_eq!(
            a.control.unread(),
            0,
            "쓰기가 끊긴 끝의 닫기는 아무것도 안 보낸다"
        );
        a.tx.send(Frame::Text("still readable".into()))
            .await
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)),
            "쓰기 실패는 읽기를 안 끊는다"
        );
    }

    // ── H8: MemoryListener — 받는 쪽 ──
    fn listener(max_pending_upgrades: usize) -> (ManualClock, MemoryListener, MemoryConnector) {
        let clock = ManualClock::new();
        let (listener, connector) = MemoryListener::new(clock.handle(), max_pending_upgrades);
        (clock, listener, connector)
    }

    fn mem_addr() -> Address {
        Address::new("mem://listener")
    }

    // 받은 끝을 감독 없이 손으로 몬다(명부에 얹는 길은 `Registry::attach` — 그 시험은 `registry.rs`) — 거는 쪽 감독과
    //   진짜 통로로 이어지는지가 요점이다.
    #[tokio::test]
    async fn a_dialer_peer_handshakes_and_round_trips_a_request_with_a_raw_accepted_end() {
        let (clock, mut listener, connector) = listener(4);
        let registry = Registry::new(
            Arc::new(TestWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (peer, mut inbox) =
            registry.add(PeerId::new("d"), mem_addr(), Arc::new(HelloHandshake));
        settle_until("걸기가 준비 큐에 닿았다", || {
            listener.queued() == 1
        })
        .await;
        let mut accepted = listener.accept().await.expect("받은 통로");
        assert_eq!(accepted.info().remote.as_deref(), Some("mem:1"));
        assert!(matches!(
            accepted.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Text(t)) if t == HelloHandshake::HELLO
        ));
        accepted
            .tx
            .send(Frame::Text(HelloHandshake::WELCOME.into()))
            .await
            .unwrap();
        settle_until("운영 진입", || peer.peer_state() == PeerState::Live).await;
        assert!(matches!(
            inbox.recv().await,
            Some(Incoming::Event(TransportEvent::Connected { .. }))
        ));

        let asking = peer.clone();
        let reply = tokio::spawn(async move {
            asking
                .request(TestOut::Request {
                    tag: 7,
                    body: "q".into(),
                })
                .await
        });
        match accepted.rx.recv().await.unwrap() {
            LinkRead::Frame(frame) => assert_eq!(frame, Frame::Text("req|7|q".into())),
            other => panic!("{other:?}"),
        }
        accepted.tx.send(TestWire::reply(7)).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), reply)
            .await
            .expect("답장이 안 왔다")
            .expect("요청 태스크가 패닉했다")
            .expect("요청이 실패했다");
        assert_eq!(
            got,
            TestIn::Reply {
                tag: 7,
                body: "ok".into()
            }
        );
        assert_eq!((connector.dials(), connector.links()), (1, 1));
    }

    // 깨운 뒤 버린 future 가 요점이다 — 기다리던 중에 통로가 닿아 깨어난 accept 를 폴링 없이 버려도 통로는 남는다.
    #[tokio::test]
    async fn accept_is_cancel_safe() {
        let (_clock, mut listener, connector) = listener(4);
        let mut waiting = listener.accept();
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "아직 아무도 안 걸었다"
        );
        let (mut tx, _rx) = connector.dial(&mem_addr()).await.unwrap();
        tx.send(Frame::Text("first words".into())).await.unwrap();
        drop(waiting);
        assert_eq!(
            listener.queued(),
            1,
            "버린 accept 가 통로를 가져가지 않았다"
        );
        let mut accepted = listener.accept().await.unwrap();
        assert_eq!(accepted.info().remote.as_deref(), Some("mem:1"));
        assert!(matches!(
            accepted.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Text(t)) if t == "first words"
        ));
    }

    #[tokio::test]
    async fn dropping_the_listener_closes_what_it_has_not_handed_out() {
        let (_clock, listener, connector) = listener(4);
        let permits = listener.permits();
        let (mut tx, mut rx) = connector.dial(&mem_addr()).await.unwrap();
        assert_eq!(permits.in_use(), 1, "준비 큐에 앉은 통로가 칸을 쥔다");
        drop(listener);
        assert_eq!(permits.in_use(), 0);
        assert!(
            connector.link(1).unwrap().acceptor.dropped(),
            "안 받아 간 먼 끝이 닫혔다"
        );
        assert!(
            rx.recv().await.is_err(),
            "거는 쪽은 통로 손실(읽기 오류)로 본다"
        );
        assert!(tx.send(Frame::Text("x".into())).await.is_err());
        assert!(
            matches!(
                connector.dial(&mem_addr()).await,
                Err(DialFailure::Unreachable(_))
            ),
            "청취기가 없으면 못 붙는다"
        );
        assert_eq!((connector.dials(), connector.links()), (2, 1));
    }

    #[tokio::test]
    async fn a_dialer_peer_sees_a_dropped_listener_as_link_loss_and_cannot_land_again() {
        let (clock, listener, connector) = listener(4);
        let registry = Registry::new(
            Arc::new(TestWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (peer, _inbox) = registry.add(PeerId::new("d"), mem_addr(), Arc::new(HelloHandshake));
        settle_until("핸드셰이크 진입", || {
            peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(listener.queued(), 1);
        drop(listener);
        settle_until("통로 손실로 물러남", || {
            peer.peer_state() == PeerState::Backoff
        })
        .await;
        clock.advance(Duration::from_secs(11));
        settle_until("다시 걸었다", || connector.dials() == 2).await;
        assert_eq!(connector.links(), 1, "닿은 통로는 처음 하나뿐이다");
    }

    // 인증 시한은 업그레이드 끝부터 잰다(TRD 3 §5 3) — 받아 간 순간으로 찍으면 준비 큐에 앉은 시간이 시한에서 빠진다.
    #[tokio::test]
    async fn upgraded_at_is_when_the_link_landed_not_when_it_was_accepted() {
        let (clock, mut listener, connector) = listener(4);
        let landed = clock.now();
        let _near = connector.dial(&mem_addr()).await.unwrap();
        clock.advance(Duration::from_secs(3));
        let accepted = listener.accept().await.unwrap();
        assert_eq!(accepted.upgraded_at(), landed);
        assert_eq!(clock.now() - accepted.upgraded_at(), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn the_upgrade_permit_rides_in_the_accepted_link_and_full_permits_refuse_the_dial() {
        let (_clock, mut listener, connector) = listener(1);
        let permits = listener.permits();
        let _first = connector.dial(&mem_addr()).await.unwrap();
        assert!(matches!(
            connector.dial(&mem_addr()).await,
            Err(DialFailure::Unreachable(e)) if e.message == "upgrade permits full"
        ));
        let accepted = listener.accept().await.unwrap();
        assert_eq!(permits.in_use(), 1, "받아 가도 칸은 그 통로와 함께 간다");
        assert!(
            connector.dial(&mem_addr()).await.is_err(),
            "아직 칸이 안 돌아왔다"
        );
        drop(accepted);
        assert_eq!(permits.in_use(), 0);
        assert!(connector.dial(&mem_addr()).await.is_ok());
        assert_eq!((connector.dials(), connector.links()), (4, 2));
    }

    #[tokio::test]
    async fn accept_ends_for_good_once_every_connector_is_gone_but_hands_out_what_landed_first() {
        let (_clock, mut listener, connector) = listener(4);
        let _near = connector.dial(&mem_addr()).await.unwrap();
        let other = connector.clone();
        drop((connector, other));
        assert!(
            listener.accept().await.is_ok(),
            "닿아 있던 것은 먼저 내준다"
        );
        assert!(listener.accept().await.is_err());
        assert!(listener.accept().await.is_err(), "종결은 계속 종결이다");
    }

    #[tokio::test]
    async fn a_connector_dial_that_is_never_polled_lands_nothing() {
        let (_clock, listener, connector) = listener(1);
        drop(connector.dial(&mem_addr()));
        assert_eq!(
            (
                connector.dials(),
                listener.queued(),
                listener.permits().in_use()
            ),
            (0, 0, 0)
        );
    }

    /// `now` 안에서 연결기 장부를 읽는 시계. 시계 ↔ 연결기가 서로를 쥐어 새지만 시험 하나 동안뿐이다.
    struct LogReadingClock {
        connector: std::sync::OnceLock<MemoryConnector>,
        reads: AtomicUsize,
    }

    impl Clock for LogReadingClock {
        fn now(&self) -> Instant {
            if let Some(connector) = self.connector.get() {
                let _ = (connector.links(), connector.dials());
                self.reads.fetch_add(1, Ordering::AcqRel);
            }
            Instant::now()
        }

        fn sleep(&self, _d: Duration) -> BoxFuture<'static, ()> {
            Box::pin(std::future::pending())
        }
    }

    // 걸기를 다른 스레드에서 돌린다 — 교착이 시험을 매달지 않고 시한 실패로 드러나게.
    #[test]
    fn a_clock_that_reads_the_connector_log_does_not_deadlock_the_dial() {
        use futures_util::FutureExt;

        let clock = Arc::new(LogReadingClock {
            connector: std::sync::OnceLock::new(),
            reads: AtomicUsize::new(0),
        });
        let (listener, connector) = MemoryListener::new(clock.clone(), 1);
        assert!(clock.connector.set(connector.clone()).is_ok());
        let (done_tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let landed = connector
                .dial(&mem_addr())
                .now_or_never()
                .map(|r| r.is_ok());
            let _ = done_tx.send(landed);
        });
        let landed = done
            .recv_timeout(Duration::from_secs(5))
            .expect("걸기가 교착했다 — 시계를 장부 자물쇠 안에서 읽었다");
        assert_eq!(landed, Some(true));
        assert_eq!(clock.reads.load(Ordering::Acquire), 1);
        assert_eq!(listener.queued(), 1);
    }
}
