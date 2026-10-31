//! 상대 하나의 감독 태스크 — ★구동 층★.
//!
//! [`crate::machine`] 이 정한 것을 [`crate::link`]·[`crate::clock`] 위에서 집행한다. 순수 층이 "무엇을"
//! 이라면 이 파일은 "어떻게" 다.
//!
//! ## 이 파일이 지는 불변식
//!
//! - ★단일 writer★ — [`LinkTx`] 를 쥐는 것은 감독 태스크 하나뿐이다. 밖에서 오는 것은 전부 큐를 지난다.
//! - ★대기 슬롯 등록은 **보내기 전에**★ — 뒤집으면 빠른 답장이 슬롯을 못 찾는다(오늘 셸도 그렇다).
//! - ★연결을 버리는 **모든** 갈래가 대기자를 깨운다★ — 표에 있는 것뿐 아니라 **큐에 남은 요청과 운영
//!   단계를 떠난 뒤 도착하는 요청까지**. 하나라도 빠지면 그 슬롯은 영구 대기가 된다: 포기 상태에는
//!   시한을 돌릴 루프가 없고, 대기 중인 호출자가 핸들을 붙들고 있어 채널이 닫히지도 않는다.
//! - ★모든 쓰기에 시한이 붙는다★ — `write_deadline < ping_interval` 이라 진행 중 쓰기가 keepalive 를
//!   굶기지 못한다. 오늘 `net` 의 실물 결함이 정확히 그 굶김이다.
//! - ★핸들은 재연결에 불사★ — 세대 경계는 핸들을 죽이지 않고 [`crate::Arrival::generation`] 에 표식으로
//!   나온다.
//! - ★명령은 묶인 세대 밖으로 나가지 않는다(TRD 3 §3-0 ③)★ — 핸들이 큐에 넣는 명령마다 세대를 찍고(`*_in` 은
//!   부르는 쪽이 준 값, 나머지는 부른 순간의 지금 세대) 감독이 쓰기 직전 지금 세대와 대조해 다르면 쓰지 않는다
//!   (`Supervisor::on_cmd`). 큐는 재연결을 넘어 살아남는데 기계가 끊길 때 비우는 것은 그 순간의 큐뿐이라, 그 뒤 든
//!   것이 연결 수립 구간을 지나 다음 연결에 쓰이던 틈을 이 대조가 닫는다. 쓰지 않은 요청은 안 보냄 끊김으로 깨우고
//!   알림 · 팬아웃 · 이어받기는 센다 — 운영 단계가 아닐 때 든 명령과 같은 처분이다(`reject_command`). 핸들 입구도 같은
//!   대조를 먼저 하지만 그것은 빠른 길이고 권위는 감독 쪽이다.
//! - ★위로 올리는 것은 전부 순서 칸(감독 지역 큐)을 지나고, 버려지지 않는다★(TRD 3 §3-0 ②) — 넣는 자리
//!   (사건 · 패킷 · 시한 사건 · 유실 신고)는 동기로 넣기만 하고 줄에 자리가 있으면 그 자리에서 싣는다. 줄이
//!   차 있으면 붙들어 두고, **모든 대기 루프**(연결 수립 · 운영 · 백오프 · 포기)의 「줄에 자리 → 순서 칸
//!   비우기」 갈래가 싣는다 — 하나라도 빠지면 그 상태에 머무는 동안 붙든 것이 안 나간다. 쓰기를 미는
//!   동안에는 그 갈래가 없어 싣기가 멈춘다 — 쓰기 하나면 `write_deadline` 까지, 쓰기를 잇달아 미는
//!   자리(`Turn::Outbound` 의 큐 비우기 · `reissue_resumes`)에서는 그 쓰기 수 × `write_deadline` 까지.
//!   그래서 한 감독이 올리는 것은 낸 순서대로 실린다. 버리는 것은 받는 쪽이 닫혔을 때와 정리의
//!   상한(`CLOSE_DRAIN_LIMIT`)을 넘겼을 때뿐이다.
//! - ★줄이 차면 그 상대의 읽기를 세운다(배압 — TRD 3 §3-0 ①)★ — 순서 칸이 무엇이든 붙들고 있는 동안 운영
//!   루프는 프레임 팔을 뺀다. 그러면 앞잡이 줄([`Policy::frame_buffer`])이 차고 읽기 태스크가 통로 읽기를
//!   멈춘다. 따로, 받아 쥔 바이트(이어받기 창 몫을 뺀 것)가 읽기 몫(스트림을 안 쓰면 `BYTE_BUDGET` 전부 ·
//!   쓰면 그 반) 이상이면 읽기 태스크가 다음 읽기를 안 한다(`budget.rs` — 앞잡이 줄 · 순서 칸 · 위로 올라가는
//!   줄 · 창을 셈 하나가 덮는다). 그래서 순서 칸이
//!   쥐는 것은 줄이 찬 뒤 처리한 프레임 한 장(또는 타이머 한 바퀴)이 낳은 것과 생애 사건 몇 개, 그리고 통로를
//!   버릴 때 앞잡이 줄에서 건진 것(아래 항목)까지다 — 정리가 비울 옛 감독의 밀린 것도 그만큼이라, 비우는 받는
//!   쪽이면 `CLOSE_DRAIN_LIMIT` 안에 끝난다(그 바이트는 셈 안이다).
//!   ★상한을 넘겨 버린 것은 세지도 신고하지도 않는다★ — 정리가 넣는 끝을 놓아 수신단 끝(`None`)이 그 버림을
//!   알린다(TRD 3 §3-0 ②).
//! - ★이어받기 창이 붙든 스트림 조각은 읽기를 세우지 않는다★ — 같은 셈 안에서 창 몫으로 따로 세어 읽기
//!   문에서 빼고, 창 몫은 감독마다 창 상한(스트림을 쓰는 예산의 반)으로 잰다 — 붙들면 넘는 조각은 붙들지 않고 그 창을
//!   잘림으로 닫는다(`Supervisor::route`). 읽기를 세우면 구멍을 메울 재생분을 못 읽어, 메울 수 있던 구멍이
//!   영구 손실이 된다. 감독 하나의 바이트 천장은 `BYTE_BUDGET`(= 읽기 몫 + 창 상한) + 한 메시지다
//!   (`budget.rs` 헤더).
//! - ★붙든 동안은 침묵 판정과 이어받기 창의 시계를 멈춘다★ — 막힌 것은 우리 쪽이라 상대의 살아있음도
//!   재요청의 답도 못 읽는다. 풀리는 순간 `last_rx` 를 그때로 두고 열린 창의 청한 시각을 붙든 시간만큼
//!   민다(`Supervisor::track_pause`). 붙든 시간은 알아챈 때가 아니라 붙듦이 실제로 시작된 때부터 센다 —
//!   쓰기를 기다리던 사이 시작됐어도 그렇고, 그 사이 읽기 문이 닫혔다 다시 열려 감독이 끝내 못 본 붙듦도 센다.
//!   창은 붙든 시간보다 늦게 닫힐 수는 있어도 일찍 닫히지는 않는다(안전한 쪽 — 그 자리는 `track_pause`).
//!   우리 ping 은 그동안에도 나간다 — 상대가 우리를 침묵으로 끊지 않는다.
//!   대가로 그 상대의 답장도 못 들어온다 — 받는 쪽을 비우는 task 위에서 [`Peer::request`] 를 기다리면 그
//!   요청은 시한까지 매달린다(crate 헤더 「받는 쪽 불변식」 1).
//! - ★받는 쪽이 수신단을 놓거나 닫으면 곧바로 접는다★(crate 헤더 「받는 쪽 불변식」 5) — 모든 대기 루프(연결
//!   수립 · 운영 · 백오프 · 포기)와 쓰기(`write` · `ping`)가 수신단 닫힘(`Sender::closed`)을 지켜봐, 넣을 것이
//!   없어도(상대가 조용해도) 그 자리에서 닫기로 간다 — 다시 걸지 않는다. 걸던 통로 · 핸드셰이크는 닫힘 팔이
//!   이기는 대로 버린다 — 핸드셰이크가 막 끝나 끝남 팔이 이기면(`select!` 는 치우치지 않는다) `shake_hands` 가
//!   세대를 뽑고 운영 단계의 첫 바퀴가 닫기를 보내며 접는다(해는 없다 · 뽑기 앞에 다시 보는 검사를 두지 않는다).
//!   걸기 구간을 폴링하기 전에도 보므로 감독이 걸기를 시작하기 전에 놓인 수신단이면 통로를 안 건다. 쓰다 버린
//!   답장 없는 프레임은 다른 까닭으로 버릴 때처럼 센다(`write_or_count`). 안 지켜보는 것은 `drop_link` 의 닫기
//!   한 번뿐이다 — 닫기 코드를 끝까지 밀어 넣는 자리라 일부러 아무것도 안 듣고, 길어야 `write_deadline` 이다.
//!   정리는 실을 데가 없으니 순서 칸을 기다리지 않는다(`Closed` 도 안 실린다).
//! - ★통로를 버리기 전에 이미 읽은 것을 넘긴다★ — 운영 단계를 떠날 때(앱이 닫음 · 다시 걸기 · 쓰기 실패 ·
//!   침묵 …) 읽기 태스크를 접고 앞잡이 줄을 기다림 없이 비워 운영 중과 같은 길로 보낸다
//!   (`Supervisor::salvage_read_ahead`). 기계가 대기자를 깨우고 끊김 사건을 내기 전이라 답장은 기다리던
//!   호출자에게 가고 그 밖은 끊김 사건 앞에 선다. 디코드 실패로 끊는 정책이어도 그 뒤에 이미 읽은 것은 넘긴다.
//!   통로에 남은 것(아직 안 읽었다)은 함께 버린다. 그 줄에 읽는 쪽의 끝(상대의 닫힘 · 통로 오류 · 디코드 실패로
//!   끊음 — 맨 앞의 것)이 이미 와 있었으면 우리 쪽 끊김 까닭(쓰기 실패 · 침묵 · 넘침 …)만 그것으로 고쳐 적는다 —
//!   통로를 버리기 전에 이미 읽힌 끝이다. 앱이 닫았거나 다시 걸라고 했으면 `Explicit` 그대로다(`earlier_end`).
//! - ★붙든 스트림 조각은 처음 실어 온 연결의 세대로 올라간다★ — 이어받기 창은 다시 붙어도 살아남아 다음
//!   연결에서 닫힐 수 있지만, [`crate::Arrival::generation`] 은 그 조각을 실어 온 연결을 가리킨다.
//! - ★정리 순서 = 통로 버림 → 대기자 깨움 · 나가는 큐 닫고 남은 명령 거절 → 열린 이어받기 창 접기(붙든 것을
//!   잘림 신고와 함께 올린다) → 순서 칸 비우기 → [`TransportEvent::Closed`] 마지막 → 넣는 끝 놓기★ — 받는 쪽을
//!   기다리는 것은 순서 칸 비우기뿐이다(`teardown`). 넣는 끝은 감독 하나만 쥐므로(`PeerConfig::inbound`) 감독이
//!   끝나면 그 손잡이 수신단의 다음 `recv` 가 `None` 이다.
//!
//! ## 운영 루프의 우선순위 (되살리지 마라)
//!
//! 옛 판은 `select!` 를 `biased` 로 두고 프레임 팔을 타이머·나가는 큐보다 앞에 놓았다. `biased` 는 선언
//! 순서로 폴링하고 **먼저 준비된 팔에서 멈추므로**, 프레임이 처리 속도만큼만 들어와도 뒤의 두 팔이
//! **한 번도 폴링되지 않는다.** 그러면 ping 이 안 나가 상대가 우리를 침묵으로 끊고, 요청 시한이 안 돌아
//! 대기자가 상한을 넘겨 매달리고, 나가는 큐가 차서 기본 정책이 스스로 연결을 끊는다 — ★출력이 가장
//! 많을 때 골라서 끊어지는★ 모양이다. 지금 루프는 ① 제어를 매 바퀴 먼저 걷고 ② 시한이 이미 지났으면
//! `select` 를 건너뛰며 ③ 프레임을 [`FRAME_BURST`] 개 처리할 때마다 나가는 큐에 한 바퀴를 양보한다.
//!
//! ## 채널이 둘인 이유
//!
//! 데이터 큐(유계)와 제어 큐(무계)를 가른다. ★큐가 꽉 찬 순간에도 「닫아라」·「지금 다시 붙어라」는
//! 들어가야 한다★ — 한 채널이면 배압이 제어까지 막는다.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::budget::{Charge, Charged, Ledger};
use crate::clock::{with_deadline, Clock};
use crate::event::{Arrival, Direction, Incoming, PeerId, TransportEvent};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{Address, Dialer, Handshake, HandshakeStep, LinkError, LinkRead, LinkRx, LinkTx};
use crate::machine::{
    Action, ConnectFailure, DisconnectCause, Generation, Input, Machine, MachineEvent, PeerState,
};
use crate::pending::{Insert, Pending, RequestError};
use crate::policy::{DecodeFailurePolicy, Policy, QueueFullPolicy};
use crate::stream::{Streams, Truncation, Verdict};
use crate::wire::Wire;

/// 프레임을 이만큼 연달아 처리하면 나가는 큐에 한 바퀴를 양보한다.
const FRAME_BURST: usize = 32;

/// 정리가 순서 칸을 다 싣기를 기다리는 상한. 넘기면 남은 것을 — 마지막 [`TransportEvent::Closed`] 까지 —
/// 버리고 끝낸다(넣는 끝을 놓아 수신단 끝 `None` 이 그 버림을 알린다). 산 채 안 비우는 받는 쪽에 감독이
/// 매달리지 않게 하는 값이다.
///
/// 60 s 인 까닭 = 데몬 읽기 task 가 우선 줄에서 기다리는 최악(8칸 × kill-join 5 s)을 덮는다(TRD 3 §3-0 ②).
pub(crate) const CLOSE_DRAIN_LIMIT: Duration = Duration::from_secs(60);

/// 감독 하나가 받아 쥘 페이로드 바이트의 예산(`budget.rs` 의 B). 셈 하나가 읽기 몫과 창 상한으로 나눠 쓴다 —
/// 읽기 문은 읽기 몫으로 잰다(받아 쥔 바이트에서 이어받기 창 몫을 뺀 것이 그 이상이면 읽기 태스크가 통로를 더 안
/// 읽는다). ★스트림을 안 쓰면 읽기 몫 = 이것 전부(창 상한 0)이고, 스트림 조각을 처음 본 뒤로는 반씩이다★
/// (`budget::split` · 그 까닭은 `budget.rs` 헤더). 읽기는 「더하기 전 물높이」로 판정하므로 어느 쪽이든 감독
/// 하나의 천장은 이것 + 한 메시지다(TRD 3 §3-0 ① 「B + M」).
///
/// 16 MiB 인 까닭 = net 의 연결별 받는 줄 예산(`CONN_RX_MAX_BYTES`)과 같은 값 — 셸 · 데몬이 같은 값을 쓴다
/// (TRD 3 §3-0 ① 가안 · 메인이 정해 보고하는 내부 상수). 스트림을 안 쓰는 두 소비자(ADR-0310 결정 16)의 읽기
/// 문이 net 과 같은 16 MiB 가 되는 것이 나누기를 미루는 까닭이다. 작은 프레임 홍수는 개수 상한
/// ([`Policy::frame_buffer`] · [`Policy::inbound_queue`])이 맡는다.
pub(crate) const BYTE_BUDGET: usize = 16 << 20;

/// 운영 루프의 이번 바퀴가 무엇을 먼저 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// 시한이 이미 지났다 — `select` 를 건너뛰고 타이머부터 본다.
    Timer,
    /// 나가는 큐를 이만큼 비운다.
    Outbound(usize),
    /// 평소 — 네 팔을 공평하게 기다린다.
    Wait,
}

/// ★굶주림 방지 규칙이 사는 유일한 자리★.
///
/// 순수 함수인 것은 의도다: 이 규칙이 지키는 성질(프레임이 끊이지 않아도 타이머와 나가는 큐가
/// 돈다)은 **인메모리 하네스로 재현되지 않는다** — 그쪽에서는 읽기 태스크가 구조적 병목이라
/// 프레임 큐가 늘 비고, 그러면 `select` 가 무엇을 하든 뒤의 팔이 폴링된다(실측: `biased` 를 되살린
/// 변이가 홍수 테스트를 하나도 못 빨갛게 만들었다). 그래서 **규칙 자체를 여기서 단언**한다.
fn next_turn(
    now: Instant,
    wake: Instant,
    frames_in_row: usize,
    outbound_len: usize,
    outbound_cap: usize,
) -> Turn {
    // TODO(stream-replay-vs-timer): 시한이 프레임보다 앞이라, 창 시한이 쓰기 도중 지나고 그 사이 재생분이 앞잡이
    //   줄에 들어왔으면 그 재생분을 처리하기 전에 이어받기 창이 닫힌다 — 배압이 없어도 그렇다(crate 헤더 「알려진
    //   한계」).
    if wake <= now {
        return Turn::Timer;
    }
    if outbound_len == 0 {
        return Turn::Wait;
    }
    // ★고정 비율만으로는 부족하다★ — 32 프레임마다 한 개씩 빼면 그 비율을 넘는 생산자 앞에서
    //   큐는 여전히 자라 `QueueFull` 로 끝난다(문턱만 32배 높아진 같은 모양이다). 그래서 규칙이
    //   **깊이**를 본다: 절반을 넘으면 버스트를 기다리지 않고, 양보할 때는 쌓인 만큼 비운다.
    if outbound_len.saturating_mul(2) >= outbound_cap || frames_in_row >= FRAME_BURST {
        return Turn::Outbound(outbound_len);
    }
    Turn::Wait
}

/// 답장을 안 기다리는 것을 밀어 넣지 못했다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// 나가는 큐가 찼다.
    QueueFull,
    /// 지금 운영 단계가 아니다(연결 중·백오프 중·포기함) — 또는 묶은 연결이 지금 연결이 아니다([`Peer::notify_in`]).
    Disconnected,
    /// 감독 태스크가 사라졌다 — 이 상대는 끝났다.
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => f.write_str("outbound queue full"),
            Self::Disconnected => f.write_str("peer is not live on the bound generation"),
            Self::Closed => f.write_str("peer supervisor is gone"),
        }
    }
}

impl std::error::Error for SendError {}

/// 감독에게 가는 데이터 명령 하나 — ★부르는 쪽이 잡은 세대를 단다★(TRD 3 §3-0 ③).
///
/// 나가는 큐는 연결보다 오래 산다(핸들이 재연결에 불사). 기계가 연결을 버릴 때 큐를 비우지만(`fail_pending`)
/// 비운 뒤에 든 것은 못 보고, 연결 수립 구간(걸기 · 핸드셰이크)은 큐를 안 읽어 그것이 다음 운영 단계까지 남는다 —
/// 세대 G 에 든 명령이 G+1 의 통로에 쓰인다(셸이 ADR-0195 로 막은 결함의 transport 판). 감독은 쓰기 직전 이
/// 세대를 지금 세대와 대조해 다르면 쓰지 않는다(`Supervisor::on_cmd`) — 그 틈을 잡는 자리가 그 대조 하나다.
struct PeerCmd<W: Wire> {
    /// 이 명령이 나가도 되는 유일한 연결. 핸들의 입구가 찍는다 — `*_in` 은 부르는 쪽이 준 값, 나머지는 부른
    /// 순간의 지금 세대([`Peer::generation`]).
    generation: Generation,
    op: PeerOp<W>,
}

enum PeerOp<W: Wire> {
    Request {
        out: W::Out,
        reply: oneshot::Sender<Result<W::In, RequestError>>,
    },
    Notify {
        out: W::Out,
    },
    /// 팬아웃 — 명부가 한 번만 인코딩해 상대 수만큼 나눠 준다.
    Raw(Frame),
    Resume {
        key: W::StreamKey,
        after: Option<u64>,
    },
}

enum Ctl {
    ReconnectNow,
    Close,
}

/// 상대가 「너를 안 받는다」고 **말한** 것인가.
///
/// ★정상 종료 계열은 거절이 아니다★ — RFC 6455 의 `1000`(Normal)·`1001`(Going Away)과 우리
/// [`CloseCode::GOING_AWAY`] 는 「나 이제 간다」이지 거절이 아니다. 그것을 거절로 읽으면 데몬이 곱게
/// 종료할 때마다 클라가 **재시도 한 번 없이** 포기한다. 나머지 코드는 상대가 굳이 골라 실은 것이므로
/// 그 말을 그대로 받는다.
fn is_rejection(code: CloseCode) -> bool {
    !matches!(code.0, 1000 | 1001) && code != CloseCode::GOING_AWAY
}

/// 마지막으로 알려진 끊김 사유를 핸들 쪽에서도 읽게 하는 최소 표현.
fn cause_code(cause: DisconnectCause) -> u8 {
    match cause {
        DisconnectCause::PeerClosed => 0,
        DisconnectCause::ReadFailed => 1,
        DisconnectCause::Idle => 2,
        DisconnectCause::WriteDeadline => 3,
        DisconnectCause::QueueFull => 4,
        DisconnectCause::Explicit => 5,
        DisconnectCause::WriteFailed => 6,
    }
}

fn cause_from_code(code: u8) -> DisconnectCause {
    match code {
        0 => DisconnectCause::PeerClosed,
        1 => DisconnectCause::ReadFailed,
        2 => DisconnectCause::Idle,
        3 => DisconnectCause::WriteDeadline,
        4 => DisconnectCause::QueueFull,
        5 => DisconnectCause::Explicit,
        // 이 칸은 `cause_code` 만 적는다 — 남은 값은 6 하나다.
        _ => DisconnectCause::WriteFailed,
    }
}

struct PeerInner<W: Wire> {
    id: PeerId,
    data_tx: mpsc::Sender<PeerCmd<W>>,
    ctl_tx: mpsc::UnboundedSender<Ctl>,
    state_rx: watch::Receiver<PeerState>,
    generation: Arc<AtomicU64>,
    /// 핸들이 **큐가 차서** 못 넣은 횟수. 감독이 걷어 가 [`QueueFullPolicy`] 대로 처리한다.
    overflow: Arc<AtomicU64>,
    /// 못 넣은 명령에 찍힌 세대 중 가장 새 것(`fetch_max` — 셈보다 먼저 적는다). ★감독은 이것이 지금 세대일 때만
    /// [`QueueFullPolicy`] 를 집행한다★ — 끝난 세대의 넘침을 다음 세대가 걷어 그 새 연결을 끊지 않게(셈은 세대와
    /// 무관하게 신고한다 — `Supervisor::drain_overflow`).
    overflow_generation: Arc<AtomicU64>,
    /// 핸들이 **묶은 세대가 운영 중이 아니라서**(운영 단계가 아니거나 더 새 세대로 다시 붙었다) 버린 횟수.
    /// ★연결을 끊지 않는다★ — 묶은 연결은 이미 안 붙어 있다.
    refused: Arc<AtomicU64>,
    /// `overflow` · `refused` 를 올렸다고 감독을 깨운다 — 포기 · 백오프 대기는 그 셈만으로는 안 깨어, 안 깨우면 그
    /// 신고가 다음 「다시 연결」(포기면) · 백오프 끝(백오프면)까지 안 나온다. 남는 표는 많아야 하나라 쌓이지 않고,
    /// 남는 깨움은 해가 없다(바퀴 첫머리가 다시 걷는다). 연결 수립 구간(걸기 · 핸드셰이크)은 이것을 안 듣는다 —
    /// 그 구간에 센 것은 구간이 끝난 뒤 첫 바퀴가 신고한다.
    loss_wake: Arc<Notify>,
    /// 마지막 끊김 사유. 한 번도 안 붙었으면 [`DisconnectCause::WriteFailed`] 다 — 읽기 · 쓰기 어느 끝도 정확하지
    /// 않아, 이 값을 받는 쪽(실을 통로가 없어 못 내보낸 요청)에 가까운 나가는 쪽 끝을 골랐다.
    last_cause: Arc<AtomicU8>,
    /// 앱이 [`Peer::close`] 를 불렀다. 감독이 그것을 처리했는지와 무관하게 부른 순간 선다.
    close_requested: AtomicBool,
    #[cfg(test)]
    ledger: Arc<Ledger>,
}

/// 값싼 clone 핸들 = 감독 태스크로 가는 명령 채널.
///
/// ★재연결을 가로질러 살아남는다★ — 끊겼다고 이 핸들이 죽지 않는다. 소비자 코드가 짧아지는 대신
/// 조용한 유실이 안 생기도록 **데이터 쪽에 세대 표식**을 붙인다(ADR-0163/0164 의 idiom).
pub struct Peer<W: Wire> {
    inner: Arc<PeerInner<W>>,
}

impl<W: Wire> Clone for Peer<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W: Wire> Peer<W> {
    pub fn id(&self) -> &PeerId {
        &self.inner.id
    }

    /// 지금 상태 한 장.
    pub fn peer_state(&self) -> PeerState {
        *self.inner.state_rx.borrow()
    }

    /// 상태 변화를 지켜보는 손잡이.
    pub fn state(&self) -> watch::Receiver<PeerState> {
        self.inner.state_rx.clone()
    }

    /// 지금 연결 세대. 한 번도 안 붙었으면 명부가 준 바닥값이다.
    pub fn generation(&self) -> Generation {
        Generation(self.inner.generation.load(Ordering::Acquire))
    }

    /// 답장을 기다린다 — 부른 순간의 지금 세대에 묶는다([`Peer::request_in`] 에 [`Peer::generation`] 을 넘긴 것과
    /// 같다).
    ///
    /// 오류 조건: 시한 만료 · 끊김 · 큐 포화 · 번호 겹침 · 만료 번호 재사용 ·
    /// [`Wire::request_tag`] 가 `None`([`RequestError::NotARequest`] — `notify` 를 쓰라는 뜻).
    ///
    /// ★시한이 실제로 도달하는 경우는 「연결은 멀쩡한데 그 요청만 침묵」 하나뿐이다★ — 연결을 버리는
    /// 어느 갈래든 대기자를 즉시 깨우기 때문이다.
    ///
    /// ★재연결과 겹친 부름은 새 연결로 넘어가지 않는다★ — 세대를 먼저 읽고 운영 중인지를 그 뒤에 보므로, 그
    /// 사이 다시 붙었으면 입구에서 끝나고 그 뒤에 다시 붙었으면 감독의 쓰기 직전 대조에서 끝난다(둘 다 안 보냄
    /// 오류). 부른 순간의 연결보다 새 연결에는 쓰이지 않는다.
    ///
    /// ★`async fn` 이 아니다 — 되살리지 마라★: 세대를 부른 자리에서 읽어 돌려주는 future 에 싣는다. `async fn` 이면
    /// 본문이 첫 폴링에 돌아, future 를 만든 뒤 다시 붙고 나서 기다리면 새 세대를 읽어 새 연결에 쓰인다.
    pub fn request(&self, out: W::Out) -> impl Future<Output = Result<W::In, RequestError>> + '_ {
        let generation = self.generation();
        self.request_in(generation, out)
    }

    /// 답장을 기다린다 — ★`generation` 연결에서만 나간다★(TRD 3 §3-0 ③).
    ///
    /// 그 연결이 지금 운영 중이 아니면(끊겼다 · 다른 세대로 다시 붙었다) 보내지 않고 안 보냄 끊김
    /// ([`RequestError::Disconnected`] 의 `sent` 가 거짓)으로 끝난다. ★입구 검사는 빠른 길이고 권위는 감독의 쓰기
    /// 직전 대조다★ — 입구를 지난 뒤 쓰기 전에 세대가 바뀌면 감독이 쓰지 않고 같은 오류로 깨운다.
    ///
    /// 쓰임 = 받은 메시지에 걸린 요청을 그 메시지를 실어 온 연결로만 낼 때 그 [`crate::Arrival::generation`] 을
    /// 넘긴다. 셸이 await 하는 보내기 전에 소켓을 잡던 자리다(TRD 3 §3-0 ③). 나머지 오류 조건은
    /// [`Peer::request`] 와 같다.
    pub async fn request_in(
        &self,
        generation: Generation,
        out: W::Out,
    ) -> Result<W::In, RequestError> {
        if !self.bound_to(generation) {
            return Err(self.disconnected(generation, false));
        }
        let (tx, rx) = oneshot::channel();
        let cmd = PeerCmd {
            generation,
            op: PeerOp::Request { out, reply: tx },
        };
        match self.inner.data_tx.try_send(cmd) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.count_overflow(generation);
                return Err(RequestError::QueueFull);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(self.disconnected(generation, false))
            }
        }
        // 감독이 사라져 슬롯이 함께 떨어져도 여기서 오류로 깨어난다. ★보냄 칸은 참★ — 큐에 있었는지 대기 표에
        //   있었는지 알 길이 없어 「갔을 수 있다」 쪽으로 적는다(`sent` 칸의 「부풀 수 있다」).
        rx.await
            .unwrap_or_else(|_| Err(self.disconnected(generation, true)))
    }

    /// 답장을 안 기다린다 — 부른 순간의 지금 세대에 묶는다([`Peer::notify_in`] 에 [`Peer::generation`] 을 넘긴 것과
    /// 같다). ★await 이 없다★ — 큐에 못 넣으면 즉시 `Err`.
    ///
    /// 재연결과 겹친 부름은 [`Peer::request`] 와 같이 새 연결로 넘어가지 않는다 — 입구에서 끝나면 `Err`, 쓰기
    /// 직전 대조에서 끝나면 [`TransportEvent::Dropped`] 로 센다([`Peer::notify_in`]).
    pub fn notify(&self, out: W::Out) -> Result<(), SendError> {
        self.notify_in(self.generation(), out)
    }

    /// 답장을 안 기다린다 — ★`generation` 연결에서만 나간다★(TRD 3 §3-0 ③). ★await 이 없다★.
    ///
    /// 입구에서 그 연결이 지금 운영 중이 아니면 `Err`([`SendError::Disconnected`] — 세지 않는다, 부르는 쪽이 오류를
    /// 받았다). ★입구를 지난 뒤 쓰기 전에 세대가 바뀌면 감독이 쓰지 않고 [`TransportEvent::Dropped`]
    /// ([`crate::Direction::Outbound`])로 센다★ — 부르는 쪽은 이미 `Ok` 를 받아 갔으므로 안 세면 조용한 유실이다(큐에
    /// 든 채 끊긴 알림과 같은 규칙).
    ///
    /// 쓰임 = 받은 요청의 답을 그 요청이 온 연결로만 낼 때 그 [`crate::Arrival::generation`] 을 넘긴다 — 다른
    /// 연결로 나가면 그 상대는 자기가 안 보낸 요청의 답을 받는다(셸 `CommandOutcome` — TRD 3 §3-0 ③).
    pub fn notify_in(&self, generation: Generation, out: W::Out) -> Result<(), SendError> {
        if !self.bound_to(generation) {
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd {
            generation,
            op: PeerOp::Notify { out },
        })
    }

    /// 스트림을 이어받는다. 반환값은 이 요청이 묶인 연결 세대다 — 부른 순간의 지금 세대이고, 이 요청은 그
    /// 연결에서만 나간다([`Peer::notify`] 와 같은 대조).
    ///
    /// ★받아들여졌다는 뜻이 아니다★ — 운영 단계가 아니거나 큐가 찼거나 쓰기 전에 세대가 바뀌었으면 요청은
    /// 버려지고 [`TransportEvent::Dropped`] 로만 보인다. 반환값은 "이 세대에 묶었다" 일 뿐이다.
    /// ★쓰는 도중 끊긴 이 요청은 그것으로도 안 보인다★ — 세지도, 다음 연결에서 다시 내지도 않는다(다시
    /// 내는 것은 이 crate 가 구멍을 보고 스스로 낸 재요청뿐이다).
    pub fn resume_stream(&self, key: W::StreamKey, after: Option<u64>) -> Generation {
        let generation = self.generation();
        if self.bound_to(generation) {
            let _ = self.push(PeerCmd {
                generation,
                op: PeerOp::Resume { key, after },
            });
        } else {
            // 부르는 쪽이 오류를 못 받는 입구다(팬아웃 `send_frame` 과 둘) — 세지 않으면 조용한 유실이 된다.
            self.refuse();
        }
        generation
    }

    /// 지금 `generation` 연결이 운영 중인가 — 데이터 입구의 빠른 길이다. ★권위가 아니다★ — 이것이 참이어도 쓰기
    /// 전에 세대가 바뀔 수 있고, 그것은 감독의 쓰기 직전 대조가 잡는다(`Supervisor::on_cmd`).
    /// ★상태를 먼저 본다★ — 감독은 세대를 상태보다 먼저 내놓으므로(`Supervisor::publish`) 운영 중을 본 뒤 읽은
    /// 세대는 그 운영 단계의 것이거나 더 새 것이다. 세대를 먼저 읽으면 그 뒤 다시 붙은 운영 중을 옛 세대의 것으로
    /// 읽어, 여기서 `Err` 로 끝낼 수 있던 것을 큐에 넣는다(감독이 버리고 센다).
    fn bound_to(&self, generation: Generation) -> bool {
        self.peer_state().is_live() && self.generation() == generation
    }

    /// ★「지금 다시 해라」 입구★ — 예산을 초기화하고 즉시 시도한다(ADR-0180 결정 7). 포기 상태에서도 듣는다.
    pub fn reconnect_now(&self) {
        let _ = self.inner.ctl_tx.send(Ctl::ReconnectNow);
    }

    /// 명시 종료 — 다시 붙지 않는다.
    ///
    /// ★완료 신호가 없다★ — 돌아온 시점에 감독이 아직 통로를 닫는 중일 수 있고, 그 사이 핸드셰이크를
    /// 마쳐 **세대를 하나 더 발행할 수도** 있다. 같은 이름표를 다시 올리는 쪽은 그 겹침을
    /// **이름표별 공용 세대 계수기**로 흡수한다(`Registry::add_with_policy`) — 겹침을 없애는 것이
    /// 아니라 겹쳐도 번호가 부딪히지 않게 하는 장치다.
    pub fn close(&self) {
        self.inner.close_requested.store(true, Ordering::Release);
        let _ = self.inner.ctl_tx.send(Ctl::Close);
    }

    pub(crate) fn close_requested(&self) -> bool {
        self.inner.close_requested.load(Ordering::Acquire)
    }

    /// 시험 손잡이 — 이 감독이 지금 받아 쥔 페이로드 바이트 전부(셈의 물높이) — 앞잡이 줄 · 순서 칸 · 줄에 든
    /// 것과 이어받기 창이 붙든 조각. 그중 창 몫만은 [`Peer::window_bytes`].
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        self.inner.ledger.level()
    }

    /// 시험 손잡이 — 그중 이어받기 창이 붙든 조각의 바이트(읽기 문에서 빠지는 몫).
    #[cfg(test)]
    pub(crate) fn window_bytes(&self) -> usize {
        self.inner.ledger.held()
    }

    /// 팬아웃 한 장(`Registry::broadcast`). ★못 넣은 것은 이 상대의 감독이 세어 이 손잡이 수신단에
    /// [`TransportEvent::Dropped`] 로 신고한다★ — 명부는 수신단의 넣는 끝을 쥐지 않는다(쥐면 「`Closed` 가 마지막
    /// → 넣는 끝을 놓음 → `None`」이 깨진다 — TRD 3 §3-0 ②). 큐 포화는 넣기가 이미 센다(`push`). 감독이 사라진
    /// 상대(`SendError::Closed`)는 신고할 수신단이 이미 끝났다.
    ///
    /// ★앱이 [`Peer::close`] 를 부른 뒤 운영 단계가 아니라서 못 넣은 것은 세지 않는다★ — 앱이 닫겠다고 한 뒤의
    /// 유실 개수는 읽을 쪽이 없고 깔끔한 종료가 우선이다(사용자 결정 2026-09-26 — `lib.rs` 「알려진 한계」). 그 전에
    /// 큐에 든 것은 감독이 센다.
    ///
    /// 지금 세대에 묶는다 — 쓰기 전에 세대가 바뀌면 감독이 쓰지 않고 센다([`Peer::notify_in`] 과 같은 대조).
    pub(crate) fn send_frame(&self, frame: Frame) -> Result<(), SendError> {
        let generation = self.generation();
        if !self.bound_to(generation) {
            if !self.close_requested() {
                self.refuse();
            }
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd {
            generation,
            op: PeerOp::Raw(frame),
        })
    }

    /// 묶은 세대가 운영 중이 아니라서(운영 단계가 아니거나 더 새 세대로 다시 붙었다) 버린 것 하나를 센다 — 감독이
    /// 걷어 [`TransportEvent::Dropped`] 로 신고한다.
    fn refuse(&self) {
        self.inner.refused.fetch_add(1, Ordering::AcqRel);
        self.inner.loss_wake.notify_one();
    }

    /// `generation` 에 묶인 것이 큐가 차서 못 들어간 것 하나를 센다 — 감독이 걷어 신고하고, 그 세대가 지금 세대면
    /// [`QueueFullPolicy`] 대로 처리한다(`PeerInner::overflow_generation`). ★세대를 셈보다 먼저 적는다★ — 셈을 걷은
    /// 감독이 그 뒤에 읽는 세대가 이 넘침의 세대 이상이게. ★깨운다★ — 낡은 `Live` 를 보고 넣다 찬 셈이 감독이
    /// 큐를 다 비우고 잠든 뒤에 닿을 수 있다.
    fn count_overflow(&self, generation: Generation) {
        self.inner
            .overflow_generation
            .fetch_max(generation.0, Ordering::AcqRel);
        self.inner.overflow.fetch_add(1, Ordering::AcqRel);
        self.inner.loss_wake.notify_one();
    }

    /// 시험 손잡이 — 핸들이 묶은 세대가 운영 중이 아니라서 버리고 감독이 아직 안 걷은 수.
    #[cfg(test)]
    pub(crate) fn refused_count(&self) -> u64 {
        self.inner.refused.load(Ordering::Acquire)
    }

    /// `generation` 에 묶인 요청이 끊김으로 끝났다 — 끊김 사유는 핸들이 아는 마지막 것이다. `sent` 의 뜻은
    /// [`RequestError::Disconnected`] 그 칸.
    fn disconnected(&self, generation: Generation, sent: bool) -> RequestError {
        RequestError::Disconnected {
            generation,
            cause: cause_from_code(self.inner.last_cause.load(Ordering::Acquire)),
            sent,
        }
    }

    fn push(&self, cmd: PeerCmd<W>) -> Result<(), SendError> {
        let generation = cmd.generation;
        match self.inner.data_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.count_overflow(generation);
                Err(SendError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SendError::Closed),
        }
    }
}

/// 감독 태스크를 세우는 데 필요한 것 전부.
pub(crate) struct PeerConfig<W: Wire> {
    pub id: PeerId,
    pub addr: Address,
    pub wire: Arc<W>,
    pub dialer: Arc<dyn Dialer>,
    pub clock: Arc<dyn Clock>,
    pub handshake: Arc<dyn Handshake>,
    pub policy: Policy,
    /// ★이 손잡이 수신단의 넣는 끝 — 이 감독 하나만 쥔다★(`registry.rs` 헤더). 다른 감독과 나눠 쓰면 시끄러운
    /// 상대가 채운 줄 뒤에서 조용한 상대의 것이 순서 칸에 붙들리고, 감독이 끝나며 이것을 놓아도 수신단이 끝나지
    /// 않는다(「`Closed` 가 마지막 → 넣는 끝을 놓음 → `None`」 — TRD 3 §3-0 ②). 항목마다 그것을 넣은 감독의
    /// 바이트 몫이 실리고 받는 쪽이 꺼낼 때 떨어진다(`budget.rs`).
    pub inbound: mpsc::Sender<Charged<Incoming<W>>>,
    /// 받아 쥘 바이트 예산 — 명부는 [`BYTE_BUDGET`] 을 넣는다. 칸으로 둔 것은 시험이 작은 값을 넣으려고다.
    pub byte_budget: usize,
    /// ★이 이름표의 세대 계수기★ — 같은 이름표를 다시 올려도 **같은 Arc 를 물려준다**.
    ///
    /// 옛 감독의 마지막 세대를 읽어 바닥값으로 넘기는 방식은 경합이 있었다(D1): 옛 감독이
    /// `shake_hands` 의 `select` 안에서 핸드셰이크와 큐에 든 `Close` 를 **동시에** 준비된 채로 만나면
    /// 핸드셰이크가 이겨 세대를 하나 더 발행할 수 있는데, 명부는 그 직전 값을 이미 읽어 간 뒤다.
    /// 계수기를 공유하면 누가 이기든 같은 번호를 두 번 뽑지 않는다.
    pub generations: Arc<AtomicU64>,
}

/// 감독 태스크를 띄우고 핸들을 돌려준다.
pub(crate) fn spawn_peer<W: Wire>(cfg: PeerConfig<W>) -> Peer<W> {
    let (handle, supervisor) = new_peer(cfg);
    tokio::spawn(supervisor.run());
    handle
}

fn new_peer<W: Wire>(cfg: PeerConfig<W>) -> (Peer<W>, Supervisor<W>) {
    let (data_tx, data_rx) = mpsc::channel(cfg.policy.outbound_queue.max(1));
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    let (state_tx, state_rx) = watch::channel(PeerState::Idle);
    let generation_floor = cfg.generations.load(Ordering::Acquire);
    let generation = Arc::new(AtomicU64::new(generation_floor));
    let overflow = Arc::new(AtomicU64::new(0));
    let overflow_generation = Arc::new(AtomicU64::new(0));
    let refused = Arc::new(AtomicU64::new(0));
    let loss_wake = Arc::new(Notify::new());
    let last_cause = Arc::new(AtomicU8::new(cause_code(DisconnectCause::WriteFailed)));
    let ledger = Ledger::new(cfg.byte_budget, cfg.clock.clone());

    let handle = Peer {
        inner: Arc::new(PeerInner {
            id: cfg.id.clone(),
            data_tx,
            ctl_tx,
            state_rx,
            generation: generation.clone(),
            overflow: overflow.clone(),
            overflow_generation: overflow_generation.clone(),
            refused: refused.clone(),
            loss_wake: loss_wake.clone(),
            last_cause: last_cause.clone(),
            close_requested: AtomicBool::new(false),
            #[cfg(test)]
            ledger: ledger.clone(),
        }),
    };

    let now = cfg.clock.now();
    let supervisor = Supervisor {
        machine: Machine::new(
            cfg.policy.clone(),
            cfg.id.jitter_seed(),
            now,
            generation_floor,
        ),
        pending: Pending::new(cfg.policy.retired_tags),
        streams: Streams::new(
            cfg.policy.max_streams,
            cfg.policy.resume_buffer,
            cfg.policy.resume_timeout,
        ),
        id: cfg.id,
        addr: cfg.addr,
        wire: cfg.wire,
        dialer: cfg.dialer,
        clock: cfg.clock,
        handshake: cfg.handshake,
        policy: cfg.policy,
        inbound: cfg.inbound,
        data_rx,
        ctl_rx,
        state_tx,
        generations: cfg.generations,
        generation,
        overflow,
        overflow_generation,
        refused,
        loss_wake,
        last_cause,
        tx: None,
        rx_raw: None,
        frames_rx: None,
        reader: None,
        fresh_link: false,
        consumer_gone: AtomicBool::new(false),
        order: VecDeque::new(),
        order_since: now,
        ledger,
        paused: false,
        paused_since: now,
        last_rx: now,
        last_ping: now,
        dropped_out: 0,
    };
    (handle, supervisor)
}

enum LinkMsg {
    /// 받은 한 장과 그 바이트 몫 — 몫은 이 프레임이 낳는 항목을 따라 받는 쪽까지 간다.
    Frame(Frame, Charge),
    /// 상대가 닫았다.
    ///
    /// ★여기에는 닫기 코드를 싣지 않는다★ — 이 태스크는 **핸드셰이크가 끝난 뒤에만** 돌고, 운영 중
    /// 끊김이 나르는 것은 [`DisconnectCause`] 뿐이라 코드를 실을 칸이 없다(TRD §7 의 사건 표 그대로).
    /// 코드가 판정을 가르는 자리는 핸드셰이크이고, 그쪽은 [`LinkRx`] 를 직접 읽어 [`LinkRead::Closed`]
    /// 의 코드를 그대로 본다.
    Closed,
    /// ★통로가 준 문구를 여기서 버린다★ — [`TransportEvent::Disconnected`] 가 나르는 것은
    /// [`DisconnectCause`] 뿐이고 문구를 실을 칸이 없다(TRD §7 의 사건 표 그대로).
    Failed,
}

/// `Room` 은 [`drain_order`] 의 답이다(`Down` · `Connecting` 의 같은 칸도) — `false` = 받는 쪽이 닫혔다. 수신단
/// 닫힘을 지켜보는 팔(`Sender::closed`)도 `Room(false)` 를 낸다.
enum Wake<W: Wire> {
    Ctl(Option<Ctl>),
    Link(Option<LinkMsg>),
    Cmd(Option<PeerCmd<W>>),
    Room(bool),
    /// 읽기 문이 열렸다(`budget.rs`).
    Budget,
    Timer,
}

enum Down<W: Wire> {
    Ctl(Option<Ctl>),
    Cmd(Option<PeerCmd<W>>),
    Room(bool),
    /// 핸들이 못 넣은 것을 셌다 — 다음 바퀴 첫머리의 신고가 걷는다.
    Lost,
    Elapsed,
}

enum Connecting<T> {
    Ctl(Option<Ctl>),
    Done(T),
    Room(bool),
}

/// 받아 디코드한 메시지 하나 — 바이트 몫과 그것을 실어 온 연결의 세대를 쥐고 위로 간다. ★세대는 받은 그때
/// 찍는다★ — 이어받기 창에 붙든 채 다시 붙으면 창이 다음 연결에서 닫히는데, 그때의 세대를 찍으면
/// [`Arrival::generation`] 이 실어 온 연결을 가리키지 않는다.
struct Received<M> {
    msg: M,
    charge: Charge,
    generation: Generation,
}

struct Supervisor<W: Wire> {
    id: PeerId,
    addr: Address,
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    handshake: Arc<dyn Handshake>,
    policy: Policy,
    machine: Machine,
    pending: Pending<W::Tag, oneshot::Sender<Result<W::In, RequestError>>>,
    /// 붙든 조각도 자기 바이트 몫(창 몫으로 센다)과 실어 온 연결의 세대를 쥔 채 붙들린다.
    streams: Streams<W::StreamKey, Received<W::In>>,
    inbound: mpsc::Sender<Charged<Incoming<W>>>,
    data_rx: mpsc::Receiver<PeerCmd<W>>,
    ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    state_tx: watch::Sender<PeerState>,
    /// 이 이름표가 발행한 세대의 공용 계수기(D1).
    generations: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    overflow: Arc<AtomicU64>,
    /// 못 넣은 명령에 찍힌 세대 중 가장 새 것(`PeerInner` 의 같은 칸).
    overflow_generation: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    /// 핸들이 `overflow` · `refused` 를 올렸다 — 포기 · 백오프 대기를 깨워 신고하게 한다(`PeerInner` 의 같은 칸).
    loss_wake: Arc<Notify>,
    last_cause: Arc<AtomicU8>,
    tx: Option<Box<dyn LinkTx>>,
    /// 핸드셰이크 동안만 쥐는 읽는 쪽. 운영 단계로 들어서면 [`read_link`] 태스크로 넘어간다.
    rx_raw: Option<Box<dyn LinkRx>>,
    frames_rx: Option<mpsc::Receiver<LinkMsg>>,
    reader: Option<tokio::task::JoinHandle<()>>,
    /// 방금 붙었다 — 미결 이어받기를 다시 내보내야 한다.
    fresh_link: bool,
    /// 위로 올릴 데가 아예 닫혔다. ★다시 붙을 일이 아니라 접을 일이다★.
    consumer_gone: AtomicBool,
    /// 순서 칸 — 줄에 아직 못 실은 것을 낸 순서대로 붙든다(이 파일 헤더). 받는 쪽이 닫히면 비운다.
    order: VecDeque<Charged<Incoming<W>>>,
    /// 순서 칸이 마지막으로 붙들기 시작한 시각 — 순서 칸이 비어 있지 않을 때만 뜻이 있다(`put` 이 적는다).
    order_since: Instant,
    /// 이 감독이 받아 쥔 바이트의 셈(창 몫 포함) — 연결을 갈아도 같은 셈이다. 다시 붙은 통로는 옛 통로가 남긴
    /// 몫이 읽기 문을 열 때까지 안 읽힌다(감독 하나 = 순서 칸 하나라 그 몫도 같은 받는 쪽을 기다린다).
    ledger: Arc<Ledger>,
    /// 바퀴 첫머리에 배압으로 붙들고 있었나 — 풀리는 순간을 알아보려고 둔다(`track_pause`). ★시한 판정 ·
    /// 깨어날 시각 · 창 시계가 다 이 칸을 읽는다★ — 하나라도 그 자리의 배압을 다시 보면 서로 다른 상태로
    /// 재 진도 확인이 깨진다(`run_live` 의 `Turn::Timer`).
    paused: bool,
    /// 이번 붙듦이 시작된 시각 — 알아챈 때가 아니라 실제로 시작된 때다(`track_pause`). 붙든 동안 창 시계는 여기
    /// 멈춰 있다(`resume_clock`).
    paused_since: Instant,
    last_rx: Instant,
    last_ping: Instant,
    dropped_out: u64,
}

impl<W: Wire> Supervisor<W> {
    async fn run(mut self) {
        let mut acts = self.machine.on(self.clock.now(), Input::Start);
        loop {
            self.publish();
            let follow = self.apply(acts).await;
            self.publish();
            if self.machine.state() == PeerState::Closed {
                break;
            }
            let input = match follow {
                Some(input) => input,
                None if self.receiver_gone() => Input::Close,
                None => match self.machine.state() {
                    PeerState::Live => {
                        let input = self.run_live().await;
                        // 기계가 대기자를 깨우고 끊김 사건을 내기 전에 이미 읽은 것을 넘긴다.
                        let read_end = self.salvage_read_ahead().await;
                        earlier_end(input, read_end)
                    }
                    PeerState::GaveUp(_) | PeerState::Idle => self.park().await,
                    other => {
                        // Dialing/Backoff/Handshaking 는 `apply` 가 항상 후속 입력을 낸다.
                        // ★여기서 상대를 죽이지 않는다★ — 도달했다면 그건 우리 실수이고, 그 대가를
                        //   「이 연결을 영구히 끝냄」으로 치르는 것이 가장 파괴적인 기본값이다.
                        debug_assert!(false, "후속 입력 없이 {other:?} 로 떨어졌다");
                        Input::ReconnectNow
                    }
                },
            };
            let now = self.clock.now();
            acts = self.machine.on(now, input);
        }
        self.teardown().await;
        // 여기서 감독이 떨어지며 넣는 끝을 놓는다 — 그 손잡이 수신단의 다음 `recv` 는 `None` 이다.
    }

    fn publish(&mut self) {
        self.generation
            .store(self.machine.generation().0, Ordering::Release);
        self.state_tx.send_replace(self.machine.state());
    }

    async fn apply(&mut self, acts: Vec<Action>) -> Option<Input> {
        let mut follow = None;
        for act in acts {
            match act {
                Action::Emit(event) => self.emit_machine(event),
                Action::FailPending(cause) => self.fail_pending(cause),
                Action::DropLink { code } => self.drop_link(code).await,
                Action::Dial { deadline } => follow = Some(self.dial(deadline).await),
                Action::Handshake { deadline } => follow = Some(self.shake_hands(deadline).await),
                Action::Wait { until } => follow = Some(self.wait(until).await),
            }
        }
        follow
    }

    /// 쌓여 있는 제어 신호를 한 번에 걷는다. `Close` 가 `ReconnectNow` 를 이긴다.
    ///
    /// ★연달아 온 `ReconnectNow` 를 하나로 접는 것이 요점이다★ — 접지 않으면, 셸이 감독보다 빠르게
    /// 누를 때 매번 dial 을 **폴링되기도 전에** 취소해 「끊고 → 걸다 말고」를 무한히 되풀이한다.
    fn drain_ctl(&mut self) -> Option<Ctl> {
        let mut seen = None;
        while let Ok(signal) = self.ctl_rx.try_recv() {
            match signal {
                Ctl::Close => return Some(Ctl::Close),
                Ctl::ReconnectNow => seen = Some(Ctl::ReconnectNow),
            }
        }
        seen
    }

    // ── 연결 수립 ────────────────────────────────────────────────────────────

    /// ★두 구간(dial · 핸드셰이크) 내내 제어 신호를 듣는다★ — 안 그러면 `close()` 가 `connect_timeout`
    /// 만큼 늦게 들린다. 제어가 이기면 통로는 취소된 future 와 함께 닫힌다.
    async fn dial(&mut self, deadline: Instant) -> Input {
        // 이미 쌓여 있던 「지금 다시 붙어라」는 지금 하려는 일 그 자체이므로 삼킨다.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let dialing = dial_once(
            self.dialer.clone(),
            self.addr.clone(),
            self.clock.clone(),
            deadline,
        );
        match self.connecting(dialing).await {
            Connecting::Done(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.rx_raw = Some(rx);
                Input::Dialed
            }
            Connecting::Done(Err(failure)) => Input::ConnectFailed(failure),
            Connecting::Ctl(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Connecting::Ctl(Some(Ctl::Close)) | Connecting::Ctl(None) | Connecting::Room(_) => {
                Input::Close
            }
        }
    }

    async fn shake_hands(&mut self, deadline: Instant) -> Input {
        // ★`dial` 의 같은 줄과 짝이다★ — 쌓인 「지금 다시 붙어라」를 여기서도 접는다.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let (Some(tx), Some(rx)) = (self.tx.take(), self.rx_raw.take()) else {
            return Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(
                "no link to hand shake on",
            )));
        };
        let shaking = handshake_once(tx, rx, self.handshake.clone(), self.clock.clone(), deadline);
        match self.connecting(shaking).await {
            Connecting::Done(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.attach_reader(rx);
                // 번호를 여기서 뽑는다 — 이 자리가 「이 통로가 운영 단계에 들어섰다」의 유일한 지점이다.
                let minted = Generation(self.generations.fetch_add(1, Ordering::AcqRel) + 1);
                Input::Connected(minted)
            }
            Connecting::Done(Err(failure)) => Input::ConnectFailed(failure),
            Connecting::Ctl(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Connecting::Ctl(Some(Ctl::Close)) | Connecting::Ctl(None) | Connecting::Room(_) => {
                Input::Close
            }
        }
    }

    /// 연결 수립의 한 구간을 끝까지 돌리면서 제어를 듣고 순서 칸을 싣고 수신단 닫힘을 지켜본다. 돌려주는
    /// `Room` 은 늘 `false`(받는 쪽이 닫혔다) — 실은 바퀴는 구간을 이어 간다.
    ///
    /// ★구간 future 를 바퀴 밖에 고정해 둔다★ — 순서 칸 갈래가 이길 때마다 그것을 떨어뜨리면 걸던 통로와
    /// 핸드셰이크가 처음부터 다시 시작한다.
    async fn connecting<T>(&mut self, step: impl Future<Output = T>) -> Connecting<T> {
        // ★구간을 폴링하기 전에 본다★ — 안 보면 받는 쪽이 이미 사라졌는데도 통로를 거는 future 를 한 번은
        //   폴링한다(아래 `select!` 는 준비된 팔 중 무엇을 먼저 볼지 모른다). 첫 사건을 넣기도 전에 놓인 수신단이면
        //   통로를 아예 안 건다.
        if self.receiver_gone() {
            return Connecting::Room(false);
        }
        tokio::pin!(step);
        loop {
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                // ★수신단이 닫히면 깨운다(`closed`)★ — 넣을 것이 없어도 곧바로 접는다(crate 헤더 「받는 쪽 불변식」
                //   5). 취소 안전이다 — 다시 걸면 닫혔나부터 본다(외부 사실 — tokio `Sender::closed`).
                tokio::select! {
                    signal = ctl.recv() => Connecting::Ctl(signal),
                    out = &mut step => Connecting::Done(out),
                    open = drain_order(lane, order), if backlog => Connecting::Room(open),
                    () = lane.closed() => Connecting::Room(false),
                }
            };
            match woken {
                Connecting::Room(true) => {}
                Connecting::Room(false) => {
                    self.consumer_gone.store(true, Ordering::Release);
                    return Connecting::Room(false);
                }
                other => return other,
            }
        }
    }

    /// 읽기를 따로 태스크로 뗀다 — 그 사유는 [`read_link`] 헤더에 있다.
    fn attach_reader(&mut self, rx: Box<dyn LinkRx>) {
        let (frames_tx, frames_rx) = mpsc::channel(self.policy.frame_buffer.max(1));
        self.reader = Some(tokio::spawn(read_link(rx, frames_tx, self.ledger.clone())));
        self.frames_rx = Some(frames_rx);
        self.fresh_link = true;
        let now = self.clock.now();
        self.last_rx = now;
        self.last_ping = now;
    }

    /// 백오프 대기. ★기다리는 동안에도 큐에 들어오는 요청을 즉시 실패시킨다★ — 안 그러면 그 슬롯이
    /// 다음 운영 단계까지(또는 포기 상태에서는 영원히) 매달린다.
    async fn wait(&mut self, until: Instant) -> Input {
        loop {
            // ★여기서도 신고한다★ — 안 그러면 백오프·포기 중에 쌓인 유실이 사용자가 「다시 연결」을
            //   누를 때까지 아무 데도 안 나온다.
            let _ = self.drain_overflow();
            self.report_drops();
            // 받는 쪽이 사라졌으면 다시 붙지 않고 접는다.
            if self.receiver_gone() {
                return Input::Close;
            }
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let delay = until.saturating_duration_since(self.clock.now());
            if delay.is_zero() {
                return Input::BackoffElapsed;
            }
            let sleep = self.clock.sleep(delay);
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                let lost = &self.loss_wake;
                // 수신단이 닫히면 깨운다 — `connecting` 의 같은 팔.
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Down::Room(open),
                    () = lane.closed() => Down::Room(false),
                    () = lost.notified() => Down::Lost,
                    _ = sleep => Down::Elapsed,
                }
            };
            match woken {
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Room(open) => self.note_room(open),
                Down::Lost => {}
                Down::Elapsed => return Input::BackoffElapsed,
            }
        }
    }

    /// 포기한 채(또는 아직 시작 전) 제어만 기다린다. 대기 요청은 여기서도 즉시 실패시킨다.
    async fn park(&mut self) -> Input {
        loop {
            let _ = self.drain_overflow();
            self.report_drops();
            // `wait` 의 같은 줄.
            if self.receiver_gone() {
                return Input::Close;
            }
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let backlog = !self.order.is_empty();
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let (lane, order) = (&self.inbound, &mut self.order);
                let lost = &self.loss_wake;
                // 수신단이 닫히면 깨운다 — `connecting` 의 같은 팔.
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Down::Room(open),
                    () = lane.closed() => Down::Room(false),
                    () = lost.notified() => Down::Lost,
                }
            };
            match woken {
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Room(open) => self.note_room(open),
                Down::Lost => {}
                Down::Elapsed => return Input::Close,
            }
        }
    }

    fn note_room(&mut self, open: bool) {
        if !open {
            self.consumer_gone.store(true, Ordering::Release);
        }
    }

    /// 받는 쪽이 사라졌나 — 넣다가 닫힌 것을 이미 봤거나 지금 수신단이 닫혀 있다(놓였거나
    /// [`crate::Inbox::close`]). 닫힌 것을 보면 적어 둔다 — 그 뒤로는 넣지 않는다(`put`).
    fn receiver_gone(&self) -> bool {
        if self.inbound.is_closed() {
            self.consumer_gone.store(true, Ordering::Release);
        }
        self.consumer_gone.load(Ordering::Acquire)
    }

    // ── 운영 단계 ────────────────────────────────────────────────────────────

    async fn run_live(&mut self) -> Input {
        if std::mem::take(&mut self.fresh_link) {
            if let Some(input) = self.reissue_resumes().await {
                return input;
            }
        }
        let mut frames_in_row = 0usize;
        loop {
            // ★붙듦은 유실 신고보다 먼저 본다★ — 신고는 순서 칸에 넣으며 줄에 실을 수 있는 만큼 싣는다(`put`).
            //   쓰기를 기다리던 사이 받는 쪽이 줄을 비웠으면 그 신고가 순서 칸을 다 비워, 쓰기 도중 시작된 붙듦을
            //   끝내 못 본다 — 시한을 넘긴 창이 앞잡이 줄의 재생분보다 먼저 닫혀 재생분이 지나간 번호로 버려진다.
            let now = self.clock.now();
            self.track_pause(now);
            let overflow = self.drain_overflow();
            self.report_drops();
            // `wait` 의 같은 줄. 넘침 판정보다 앞이다 — 접을 상대를 백오프로 돌리지 않는다.
            if self.receiver_gone() {
                return Input::Close;
            }
            if let Some(input) = overflow {
                return input;
            }
            // 핸드셰이크가 운영 단계를 열 때 두 절반이 함께 선다 — 빠졌으면 빠진 절반 쪽의 끝으로 적는다.
            if self.tx.is_none() {
                return Input::LinkLost(DisconnectCause::WriteFailed);
            }
            if self.frames_rx.is_none() {
                return Input::LinkLost(DisconnectCause::ReadFailed);
            }
            // ① 제어는 매 바퀴 먼저 — 프레임이 끊이지 않아도 즉시 듣는다.
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }

            let wake = self.wake_at();
            // ②③ 굶주림 방지 규칙. 판정은 `next_turn` 이 하고 여기서는 집행만 한다.
            match next_turn(
                now,
                wake,
                frames_in_row,
                self.data_rx.len(),
                self.policy.outbound_queue.max(1),
            ) {
                Turn::Timer => {
                    frames_in_row = 0;
                    if let Some(input) = self.on_timer().await {
                        return input;
                    }
                    // ★진도를 확인한다★ — 어떤 타이머가 자기 시한을 못 밀면 이 루프는 `await` 를 한 번도
                    //   지나지 않고 영원히 돈다. current_thread 런타임에서는 그 순간 **다른 태스크가
                    //   아무것도 못 하고 프로세스가 멎는다**(실측: `expire_resumes` 를 무력화한 변이가
                    //   테스트를 실패시키는 대신 10분 넘게 매달렸다).
                    if self.wake_at() <= now {
                        debug_assert!(
                            false,
                            "타이머가 자기 시한을 못 밀었다 — 운영 루프가 await 없이 돈다"
                        );
                        tokio::task::yield_now().await;
                    }
                    continue;
                }
                Turn::Outbound(budget) => {
                    frames_in_row = 0;
                    for _ in 0..budget {
                        let Ok(cmd) = self.data_rx.try_recv() else {
                            break;
                        };
                        if let Some(input) = self.on_cmd(cmd).await {
                            return input;
                        }
                    }
                    continue;
                }
                Turn::Wait => {}
            }

            let sleep = self.clock.sleep(wake.saturating_duration_since(now));
            let backlog = !self.order.is_empty();
            // 읽기 문이 열리면 깨운다. ★바퀴 첫머리에 읽기 문 때문에 붙든 것으로 봤으면(`paused` · 순서 칸 빔)
            //   지금 이미 풀렸어도 이 팔을 세운다★ — 곧바로 깨어 다음 바퀴가 풀림을 적는다. 안 세우면 깨어날
            //   시각이 `paused` 를 따라 침묵 · 창 시한을 뺀 채라, 상대가 조용하면 ping 때까지 풀림이 안 적힌다.
            //   첫머리 뒤 신고(`report_drops`)가 순서 칸을 비웠으면 읽기 문이 열려 있어도 이 팔이 서지만, 곧바로
            //   깨어 다음 바퀴가 풀림을 적는다 — 한 번 더 깰 뿐 헛돌지 않는다.
            let over = self.ledger.over() || (self.paused && !backlog);
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let frames = self.frames_rx.as_mut().expect("checked above");
                let (lane, order) = (&self.inbound, &mut self.order);
                let ledger = &self.ledger;
                // ★`biased` 를 다시 붙이지 말 것★ — 선언 순서 폴링이 프레임 홍수 아래에서 뒤 팔을
                //   통째로 굶긴다(이 파일 헤더의 「운영 루프의 우선순위」).
                // ★순서 칸이 붙들고 있으면 프레임 팔을 뺀다(배압)★ — 그 프레임이 낳을 것도 줄에 못 실리니
                //   통로에 두는 편이 낫다. 문턱을 하나로 둔 것은 순서 칸이 동기로 넣는 자리(사건 · 시한 사건)를
                //   위한 것이지 프레임의 둘째 버퍼가 아니라서다. 그 대신 비우기 팔이 늘 함께 서 있다.
                // ★수신단이 닫히면 깨운다★ — 조용한 상대 앞에서 넣을 것이 없어도 곧바로 접는다(`connecting` 의 같은 팔).
                tokio::select! {
                    signal = ctl.recv() => Wake::Ctl(signal),
                    msg = frames.recv(), if !backlog => Wake::Link(msg),
                    cmd = data.recv() => Wake::Cmd(cmd),
                    open = drain_order(lane, order), if backlog => Wake::Room(open),
                    () = lane.closed() => Wake::Room(false),
                    () = ledger.holder_room(), if over => Wake::Budget,
                    _ = sleep => Wake::Timer,
                }
            };

            let outcome = match woken {
                Wake::Ctl(Some(Ctl::ReconnectNow)) => Some(Input::ReconnectNow),
                Wake::Ctl(Some(Ctl::Close)) | Wake::Ctl(None) => Some(Input::Close),
                Wake::Link(None) | Wake::Link(Some(LinkMsg::Failed)) => {
                    Some(Input::LinkLost(DisconnectCause::ReadFailed))
                }
                Wake::Link(Some(LinkMsg::Closed)) => {
                    Some(Input::LinkLost(DisconnectCause::PeerClosed))
                }
                Wake::Link(Some(LinkMsg::Frame(frame, charge))) => {
                    frames_in_row += 1;
                    match self.on_frame(frame, charge) {
                        Ok(Some(resume)) => self.write(resume).await.err(),
                        Ok(None) => None,
                        Err(input) => Some(input),
                    }
                }
                Wake::Cmd(None) => Some(Input::Close),
                Wake::Cmd(Some(cmd)) => {
                    frames_in_row = 0;
                    self.on_cmd(cmd).await
                }
                Wake::Room(open) => {
                    self.note_room(open);
                    None
                }
                // 다음 바퀴의 `track_pause` 가 풀림을 본다.
                Wake::Budget => None,
                Wake::Timer => {
                    frames_in_row = 0;
                    self.on_timer().await
                }
            };
            if let Some(input) = outcome {
                return input;
            }
        }
    }

    /// 끊긴 채 남아 있던 이어받기를 다시 내보낸다.
    ///
    /// ★이것이 없으면 구멍이 영구히 남는다★ — 끊긴 순간 나가던 재요청은 상대에게 닿지 않았을 수 있고,
    /// 그 답을 기다리는 상태만 남아 다음 조각들이 전부 보류되다 잘림으로 접힌다.
    async fn reissue_resumes(&mut self) -> Option<Input> {
        let outstanding = self.streams.pending_resumes();
        if outstanding.is_empty() {
            return None;
        }
        for (key, after) in outstanding {
            let out = self.wire.resume_request(&key, Some(after));
            let frame = self.wire.encode(&out);
            if let Err(input) = self.write(frame).await {
                return Some(input);
            }
        }
        // 창 시계로 감는다 — 앞 통로에서 붙든 채 끊겼으면 그 시계가 아직 멈춰 있고, 풀릴 때 붙든 시간만큼
        //   밀린다(`track_pause`). 지금 시각으로 감으면 그 밀기가 끊겨 있던 시간까지 한 번 더 민다.
        let now = self.clock.now();
        self.streams.rearm_resumes(self.resume_clock(now));
        None
    }

    /// 다음에 깨어날 시각. ★모든 타이머 원천이 여기 한 곳에 모인다★ — 새 원천을 더하면서 이 함수를
    /// 안 고치면 그 시한은 아무도 깨우지 않고, `Turn::Timer` 의 진도 확인도 그것을 못 본다.
    ///
    /// ★붙든 동안 멈추는 시한(침묵 · 이어받기 창)은 `paused` 로 가린다★ — `on_timer` 가 그 칸으로 판정하므로,
    /// 그 자리의 배압을 다시 보면 둘이 어긋나 진도 확인이 깨진다(풀림이 첫머리 뒤에 끼면 낡은 `last_rx` 로 깰
    /// 시각을 잡고, `on_timer` 는 아직 붙든 것으로 보고 안 끊는다).
    fn wake_at(&self) -> Instant {
        let mut wake = self.last_ping + self.policy.ping_interval;
        if !self.paused {
            wake = wake.min(self.last_rx + self.policy.idle_timeout);
        }
        if let Some(deadline) = self.pending.next_deadline() {
            wake = wake.min(deadline);
        }
        // ★없으면 아무도 재요청 창을 못 닫는다★ — 잘림 판정이 조각 도착에만 매달려 있으면,
        //   구멍 뒤에 스트림이 조용해진 평범한 경우(턴이 끝났다)에 그 창이 영원히 열려 있다.
        if !self.paused {
            if let Some(deadline) = self.streams.next_resume_deadline() {
                wake = wake.min(deadline);
            }
        }
        wake
    }

    /// 배압이 풀린 순간 `last_rx` 를 그때로 두고 열린 이어받기 창의 청한 시각을 붙든 시간만큼 민다(TRD 3 §3-0
    /// ① 불변식 2). 붙든 동안은 상대의 살아있음도 재요청의 답도 못 읽으니, 그 시간을 침묵 · 창 시한으로 세면
    /// 우리가 막아 놓고 상대를 끊거나 메울 수 있던 구멍을 잘림으로 접는다 — 늦게 온 재생분은 지나간 번호로
    /// 버려진다. 붙듦 = 순서 칸이 붙들고 있다(프레임 팔을 뺐다) 또는 읽기 문이 닫혔다(읽기 태스크가 통로를 안
    /// 읽는다).
    ///
    /// ★붙든 시간은 알아챈 때가 아니라 붙듦이 시작된 때부터 센다★ — 감독은 쓰기를 기다리는 동안 바퀴 첫머리에
    /// 못 온다. 그 사이 붙듦이 시작됐는데 알아챈 때부터 세면, 쓰기를 기다린 시간이 창 시한으로 세져 풀리자마자
    /// 창이 닫힌다. ★그 사이 읽기 문이 닫혔다 다시 열렸으면 감독은 그 붙듦을 끝내 못 본다★ — 셈이 그 구간을
    /// 모아 두었다가 넘기고([`Ledger::take_shut`]) 여기서 그만큼 민다. 안 밀면 시한을 넘긴 창이 앞잡이 줄의
    /// 재생분보다 먼저 닫혀 낮은 번호가 지나간 번호로 버려진다.
    ///
    /// ★한 시간을 두 번 밀지 않는다★ — 붙든 줄 알고 있던 동안(`paused`) 닫혔다 열린 구간은 다 그 붙듦이 시작된
    /// 뒤라 풀릴 때 미는 시간 안에 들고, 못 본 구간이 지금 새로 알아챈 붙듦에 걸쳐 있으면(순서 칸이 먼저
    /// 붙들었다) 따로 밀지 않고 그 첫 구간이 닫힌 때부터 이 붙듦이 시작된 것으로 본다.
    ///
    /// 창이 붙든 시간보다 늦게 닫히는 쪽(잃는 것이 없다)으로만 어긋난다 — 풀린 때는 알아챈 때로 두고, 걸친
    /// 구간 사이의 안 붙든 틈도 붙든 것으로 세고, 창을 열기 전의 구간도 그 창을 민다([`Self::resume_clock`]).
    /// 침묵 판정은 풀림을 알아챈 때를 마지막 수신으로 삼으므로 시작 시각과 무관하다.
    ///
    /// `on_timer` 첫머리에서도 부른다 — 잠든 사이 풀렸으면(풀림 팔과 시한 팔이 함께 준비돼 시한 팔이 이겼다)
    /// 그 판정이 바퀴 첫머리에 본 낡은 「붙듦」 대신 지금 상태를 쓰게 한다. ★침묵 판정은 이 칸(`paused`)으로
    /// 한다 — 붙듦을 그 자리에서 다시 보면 풀린 순간 옛 `last_rx` 로 재서 50 s 넘게 붙든 상대를 곧바로 `Idle`
    /// 로 끊는다★.
    fn track_pause(&mut self, now: Instant) {
        let shut = self.ledger.take_shut();
        let order = (!self.order.is_empty()).then_some(self.order_since);
        let began = match (order, shut.since) {
            (Some(order), Some(gate)) => Some(order.min(gate)),
            (order, gate) => order.or(gate),
        };
        let closed = shut.closed;
        match (self.paused, began) {
            // 그 사이 닫혔다 열린 구간은 붙든 줄 알고 있던 시간 안이다 — 풀릴 때 함께 민다.
            (true, None) => {
                self.last_rx = now;
                self.streams
                    .shift_resumes(now.saturating_duration_since(self.paused_since));
            }
            (true, Some(_)) => {}
            (false, Some(began)) if closed.until.is_some_and(|until| until > began) => {
                self.paused_since = closed.from.map_or(began, |from| from.min(began)).min(now);
            }
            (false, began) => {
                // 못 본 붙듦이 풀린 것도 풀림이다 — 알아챈 지금을 마지막 수신으로 삼는다.
                if closed.until.is_some() {
                    self.last_rx = now;
                    self.streams.shift_resumes(closed.total);
                }
                if let Some(began) = began {
                    self.paused_since = began.min(now);
                }
            }
        }
        self.paused = began.is_some();
    }

    /// 이어받기 창이 읽는 시계 — 붙든 동안은 붙들기 시작한 시각에 멈춰 있다. 그 사이 연 창도 그 시각에 연
    /// 것으로 적혀, 풀릴 때 민 만큼(`track_pause`) 풀린 시각부터 센다.
    /// ★민 시간이 창을 넘치게 늦출 수는 있다(안전한 쪽)★ — 창을 열기 전에 이미 닫혔다 열렸거나 닫혀 있던 읽기
    /// 문의 시간도 열린 창 전부를 민다.
    fn resume_clock(&self, now: Instant) -> Instant {
        if self.paused {
            self.paused_since
        } else {
            now
        }
    }

    async fn on_timer(&mut self) -> Option<Input> {
        let now = self.clock.now();
        self.track_pause(now);
        if !self.paused && now.saturating_duration_since(self.last_rx) >= self.policy.idle_timeout {
            return Some(Input::LinkLost(DisconnectCause::Idle));
        }
        let after = self.policy.request_timeout;
        for (tag, slot) in self.pending.expire(now) {
            let _ = slot.send(Err(RequestError::TimedOut { after }));
            self.emit(TransportEvent::RequestTimedOut {
                peer: self.id.clone(),
                tag,
                after,
            });
        }
        // 붙든 동안은 창을 안 닫는다 — 그 시계는 멈춰 있다(`resume_clock`).
        if !self.paused {
            let expired = self.streams.expire_resumes(now);
            self.hand_over_truncations(expired);
        }
        if now.saturating_duration_since(self.last_ping) >= self.policy.ping_interval {
            self.last_ping = now;
            if let Err(input) = self.ping().await {
                return Some(input);
            }
        }
        None
    }

    /// 운영 중에 꺼낸 명령 하나를 처리한다.
    ///
    /// ★쓰기 직전 세대 대조(TRD 3 §3-0 ③)★ — 찍힌 세대가 지금 세대가 아니면 아무것도 하지 않고 운영 단계가 아닐 때
    /// 도착한 명령처럼 끝낸다(`reject_command` — 요청은 안 보냄으로 깨우고 나머지는 센다). 대조와 쓰기 사이에는 세대가
    /// 바뀔 틈이 없다 — 세대를 바꾸는 것은 이 감독 하나이고 둘 다 이 함수 안이다. 대조는 다른 어떤 부수효과(대기 표
    /// 등록 · 인코딩)보다 앞이다 — 낡은 요청을 표에 올리면 지금 연결의 같은 번호와 겹친다.
    async fn on_cmd(&mut self, cmd: PeerCmd<W>) -> Option<Input> {
        if cmd.generation != self.machine.generation() {
            self.reject_command(cmd);
            return None;
        }
        match cmd.op {
            PeerOp::Raw(frame) => self.write_or_count(frame).await,
            PeerOp::Notify { out } => {
                let frame = self.wire.encode(&out);
                self.write_or_count(frame).await
            }
            PeerOp::Resume { key, after } => {
                let out = self.wire.resume_request(&key, after);
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
            PeerOp::Request { out, reply } => {
                let Some(tag) = self.wire.request_tag(&out) else {
                    let _ = reply.send(Err(RequestError::NotARequest));
                    return None;
                };
                let deadline = self.clock.now() + self.policy.request_timeout;
                match self.pending.insert(tag, reply, deadline) {
                    Insert::Retired(reply) => {
                        let _ = reply.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Collided {
                        displaced,
                        rejected,
                    } => {
                        let _ = displaced.send(Err(RequestError::Superseded));
                        let _ = rejected.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Placed => {}
                }
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
        }
    }

    /// 받은 한 장을 처리한다. ★기다리지 않는다★ — 통로를 버리기 전에 앞잡이 줄을 비우는 자리
    /// ([`Self::salvage_read_ahead`])도 이것을 부른다. 써야 할 재요청이 생기면 그 프레임을 돌려주고 쓰기는
    /// 부르는 쪽 몫이다. `Err` = 이 통로를 끊는다(디코드 실패 정책).
    fn on_frame(&mut self, frame: Frame, charge: Charge) -> Result<Option<Frame>, Input> {
        let now = self.clock.now();
        if matches!(frame, Frame::Keepalive) {
            self.last_rx = now;
            return Ok(None);
        }
        // ★쓰레기는 살아있음의 증거가 아니다★ — 디코드 전에 되감으면 못 읽을 것만 보내는 상대가
        //   침묵 판정에 영원히 안 걸리고, 기본 정책이 프레임만 버리므로 끊는 밸브가 어디에도 없다.
        let msg = match self.wire.decode(frame) {
            Ok(msg) => {
                self.last_rx = now;
                msg
            }
            Err(err) => {
                let reason = err.to_string();
                let generation = self.machine.generation();
                self.emit(TransportEvent::DecodeFailed {
                    peer: self.id.clone(),
                    generation,
                    reason,
                });
                // 통로는 멀쩡해도 읽은 것을 못 읽어 끊는다 — 읽는 쪽의 끝으로 적는다(통로 오류와 같은 값이다).
                return match self.policy.decode_failure {
                    DecodeFailurePolicy::DropFrame => Ok(None),
                    DecodeFailurePolicy::Disconnect => {
                        Err(Input::LinkLost(DisconnectCause::ReadFailed))
                    }
                };
            }
        };
        Ok(self.route(msg, charge))
    }

    /// 답장이면 몫을 여기서 놓는다 — 답장은 줄이 아니라 기다리던 호출자에게 간다. 돌려주는 것 = 써야 할
    /// 재요청([`Self::on_frame`]).
    fn route(&mut self, msg: W::In, charge: Charge) -> Option<Frame> {
        let reply = self.wire.reply_tag(&msg);
        let mark = self.wire.stream_mark(&msg);
        if reply.is_some() && mark.is_some() {
            self.emit(TransportEvent::AmbiguousRole {
                peer: self.id.clone(),
            });
        }
        // 순서가 계약이다 — 답장 · 스트림 조각 · 그 외.
        if let Some(tag) = reply {
            if let Some(slot) = self.pending.take(&tag) {
                let _ = slot.send(Ok(msg));
                return None;
            }
            // 만료·겹침으로 은퇴한 번호의 늦은 답장은 설계대로 조용히 버린다(ADR-0181 결정 4).
            //   그 밖의 것은 우리가 청한 적 없는 번호라 신고한다.
            if !self.pending.is_retired(&tag) {
                self.emit(TransportEvent::UnmatchedReply {
                    peer: self.id.clone(),
                    tag,
                });
            }
            return None;
        }
        let msg = Received {
            msg,
            charge,
            generation: self.machine.generation(),
        };
        let Some(mark) = mark else {
            self.deliver(msg);
            return None;
        };
        // 이 감독이 스트림을 쓴다 — 예산을 읽기 몫과 창 상한으로 나눈다(`budget.rs` 헤더). 처음 보는 스트림의 첫
        //   조각은 늘 배달이라(`Streams::observe_capped`) 나눔은 어느 붙듦보다도 앞선다 — 아래 창 상한 재기와의
        //   앞뒤는 상관없다.
        self.ledger.use_streams();

        // ★창 상한은 붙들기 전에 잰다★ — 이 조각까지 붙들면 넘는다면 붙들 자리가 없는 것으로 보고 개수 상한이 찬
        //   것과 같은 잘림으로 보낸다(`Streams::observe_capped`). 붙든 뒤에 자르면 그 사이 상한을 넘는다.
        let over_cap = !self.ledger.window_fits(msg.charge.bytes());
        let now = self.resume_clock(self.clock.now());
        let verdict = self.streams.observe_capped(&mark, msg, now, over_cap);
        if matches!(verdict, Verdict::Gap { .. } | Verdict::Held { .. }) {
            // 방금 붙든 조각의 몫을 창 몫으로 센다 — 읽기 문에서 빠진다(`budget.rs` 헤더). 창에서 나갈 때는
            //   `deliver` 가 되돌리고, 창 안에서 버려지면(답이 왔다 · 화신이 갈렸다) 몫이 떨어지며 둘 다에서 뺀다.
            if let Some(held) = self.streams.held_mut(&mark.key, mark.seq) {
                held.charge.hold();
            }
        }
        let (event, resume_after, delivered) = match verdict {
            Verdict::Deliver(msg) => (None, None, vec![msg]),
            Verdict::Duplicate | Verdict::Held { .. } => (None, None, Vec::new()),
            Verdict::GenerationChanged {
                from,
                to,
                msg,
                discarded,
            } => (
                Some(TransportEvent::StreamGenerationChanged {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    from,
                    to,
                    discarded: discarded as u64,
                }),
                None,
                vec![msg],
            ),
            Verdict::Gap {
                expected,
                got,
                resume_from,
            } => (
                Some(TransportEvent::StreamGap {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    expected,
                    got,
                    resumed_from: Some(resume_from),
                }),
                Some(resume_from),
                Vec::new(),
            ),
            Verdict::Truncated {
                oldest_valid,
                flushed,
            } => (
                Some(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    oldest_valid,
                }),
                None,
                flushed,
            ),
        };
        if let Some(event) = event {
            self.emit(event);
        }
        for msg in delivered {
            self.deliver(msg);
        }
        self.flush_evictions();
        resume_after.map(|after| {
            let out = self.wire.resume_request(&mark.key, Some(after));
            self.wire.encode(&out)
        })
    }

    /// 자리를 만들려고 잊은 스트림을 신고하고, 그것이 붙들고 있던 조각을 내보낸다.
    ///
    /// ★축출이 미결 재요청을 통째로 삼키면 그 구멍은 영영 안 메워진다★ — `pending_resumes` 에서
    /// 사라지므로 재연결이 다시 청하지도 못하고, 잘림 사건도 안 난다.
    fn flush_evictions(&mut self) {
        for evicted in self.streams.drain_evicted() {
            self.emit(TransportEvent::StreamForgotten {
                peer: self.id.clone(),
                stream: evicted.key.clone(),
                generation: evicted.generation,
                last_seq: evicted.last_seq,
            });
            if let Some(oldest_valid) = evicted.oldest_valid {
                self.emit(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: evicted.key,
                    generation: evicted.generation,
                    oldest_valid,
                });
            }
            for msg in evicted.flushed {
                self.deliver(msg);
            }
        }
    }

    /// 닫힌 이어받기 창을 신고하고 붙들었던 것을 순서대로 올린다.
    fn hand_over_truncations(&mut self, collapsed: Vec<Truncation<W::StreamKey, Received<W::In>>>) {
        for collapsed in collapsed {
            self.emit(TransportEvent::StreamTruncated {
                peer: self.id.clone(),
                stream: collapsed.key,
                generation: collapsed.generation,
                oldest_valid: collapsed.oldest_valid,
            });
            for msg in collapsed.flushed {
                self.deliver(msg);
            }
        }
    }

    // ── 쓰기 ────────────────────────────────────────────────────────────────

    /// ★쓰는 동안에도 제어와 수신단 닫힘을 듣는다★ — 안 들으면 `close()` · 받는 쪽의 사라짐이 `write_deadline`
    /// 만큼(큐를 잇달아 비우는 자리면 그 쓰기 수만큼 곱절로) 늦게 들린다. 그쪽이 이기면 쓰던 future 를 떨어뜨리고
    /// 통로를 버린다(machine 이 `DropLink` 를 낸다).
    /// ★그 프레임이 어디까지 갔는지는 모른다★ — 한 번도 폴링되지 않았을 수도, 어댑터 버퍼에 들어가
    /// `drop_link` 의 닫기 앞에 실려 나갈 수도 있다. 알림·팬아웃이면 [`Self::write_or_count`] 가 센다.
    async fn write(&mut self, frame: Frame) -> Result<(), Input> {
        let clock = self.clock.clone();
        let deadline = self.policy.write_deadline;
        let ctl = &mut self.ctl_rx;
        let lane = &self.inbound;
        let Some(tx) = self.tx.as_mut() else {
            return Err(Input::LinkLost(DisconnectCause::WriteFailed));
        };
        tokio::select! {
            signal = ctl.recv() => Err(match signal {
                Some(Ctl::ReconnectNow) => Input::ReconnectNow,
                Some(Ctl::Close) | None => Input::Close,
            }),
            () = lane.closed() => Err(Input::Close),
            result = with_deadline(&*clock, deadline, tx.send(frame)) => match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(Input::LinkLost(DisconnectCause::WriteFailed)),
                Err(_) => Err(Input::LinkLost(DisconnectCause::WriteDeadline)),
            },
        }
    }

    /// 답장 없는 프레임을 쓴다. ★`Ok` 로 안 끝나면 사유를 가리지 않고 센다★ — 핸들은 큐에 넣을 때 이미
    /// `Ok` 를 받아 갔으므로 여기서 안 세면 조용한 유실이다(`fail_pending` 의 그 규칙). 쓰다 만 프레임은
    /// 실제로는 나갔을 수 있어 부풀 수 있다([`Direction::Outbound`] rustdoc).
    async fn write_or_count(&mut self, frame: Frame) -> Option<Input> {
        let result = self.write(frame).await;
        if result.is_err() {
            self.dropped_out += 1;
        }
        result.err()
    }

    async fn ping(&mut self) -> Result<(), Input> {
        let clock = self.clock.clone();
        let deadline = self.policy.write_deadline;
        let ctl = &mut self.ctl_rx;
        let lane = &self.inbound;
        let Some(tx) = self.tx.as_mut() else {
            return Err(Input::LinkLost(DisconnectCause::WriteFailed));
        };
        // `write` 의 같은 팔들.
        tokio::select! {
            signal = ctl.recv() => Err(match signal {
                Some(Ctl::ReconnectNow) => Input::ReconnectNow,
                Some(Ctl::Close) | None => Input::Close,
            }),
            () = lane.closed() => Err(Input::Close),
            result = with_deadline(&*clock, deadline, tx.ping()) => match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(Input::LinkLost(DisconnectCause::WriteFailed)),
                Err(_) => Err(Input::LinkLost(DisconnectCause::WriteDeadline)),
            },
        }
    }

    // ── 정리 ────────────────────────────────────────────────────────────────

    /// 통로를 버린다 — 닫기 코드를 실어 보내려 하고, 읽는 태스크를 접는다.
    ///
    /// ★여기서는 제어를 듣지 않는다 — 닫기를 끝까지 밀어 넣는다★. `select!` 는 `biased` 가 없으면
    /// 폴링마다 시작 팔을 난수로 고르는데(외부 사실), 제어 팔을 걸어 두면 **큐에 신호가 한 칸이라도
    /// 남아 있는 동안** 그 팔이 곧바로 `Ready` 라서 닫기 future 가 한 번도 폴링되지 않은 채 버려진다.
    /// 실측 2026-09-07: `reconnect_now()` 를 두 번 부르면(둘째 신호가 큐에 남는다) 혼잡이 하나도 없는데도
    /// 100회 중 41·54·49·49회 goodbye 가 안 나갔다. ★되돌리지 말 것★ — 그 손실은 문구가 아니라 소비자가
    /// 분기하는 값을 뒤집는다: 닫기 프레임이 나가면 상대는 「말하고 갔다」로 읽고, 안 나가면 tungstenite 가
    /// `ResetWithoutClosingHandshake` 를 올려 「통로 오류」로 읽는다.
    ///
    /// ★제어 팔이 벌던 것은 지연뿐이고 그 지연은 이미 유계였다★ — 걸린 닫기를 제어가 잘라도 닫기 코드는
    /// 어차피 안 나가고, 안 자르면 `write_deadline` 에서 잘린다. 즉 그 팔은 **한 프레임도 더 배달하지
    /// 않으면서** 배달되는 프레임을 절반 버리고 있었다.
    ///
    /// ★혼잡하면 코드는 여전히 잃는다★ — `write_deadline` 이 이 대기를 자른다(`lib.rs` 「닫기 코드가
    /// 상대에게 닿는 자리는 좁다」).
    ///
    /// ★알려진 한계 — 이 대기는 시계에 매인다★. 자기 시계를 손으로 미는 하네스에서 쓰기가 막힌 채
    /// 통로가 버려지면, 시계를 `write_deadline` 만큼 밀거나 멈춤을 풀 때까지 감독이 여기서 선다
    /// (증상: `close()` 를 불렀는데 상대가 접히지 않는다). 운영 시계는 항상 흐르므로 그쪽에서는
    /// `write_deadline` 이 상한이다. 하네스 쪽 계산법의 정본 = `testing::MemoryEndpoint::stall_writes`.
    async fn drop_link(&mut self, code: CloseCode) {
        if let Some(mut tx) = self.tx.take() {
            let clock = self.clock.clone();
            let deadline = self.policy.write_deadline;
            let _ = with_deadline(&*clock, deadline, tx.close(Close::new(code, "closing"))).await;
        }
        self.rx_raw = None;
        // 운영 단계를 떠난 갈래면 `salvage_read_ahead` 가 이미 비우고 접었다 — 여기 남은 것은 받는 쪽이
        //   사라져 넘길 데가 없는 경우뿐이다.
        self.frames_rx = None;
        self.fresh_link = false;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            // ★취소를 기다린다★ — 읽는 태스크가 통로의 나머지 절반을 쥐고 있어서, 그것이 떨어지기 전에는
            //   소켓이 안 닫힌다(`ws::split_link` 의 그 계약). 안 기다리면 닫힘 시점이 런타임의 회수
            //   시점이 되고, 상대가 붙은 통로를 세면 한동안 둘로 보인다. `read_link` 는 항상 `await`
            //   자리에서 멈춰 있어 이 대기가 한 바퀴를 넘지 않는다.
            let _ = reader.await;
        }
    }

    /// ★통로를 버리기 전에 이미 읽은 것을 넘긴다★ — 운영 단계를 떠나는 모든 갈래(앱이 닫음 · 다시 걸기 · 쓰기
    /// 실패 · 침묵 · 넘침 …)에서 기계에 입력을 먹이기 전에 부른다(`run`). 그 자리라 기계가 낼 대기자 깨움 ·
    /// 끊김 사건보다 앞이다: 답장은 기다리던 호출자에게 가고, 그 밖은 끊김 사건 앞에 순서 칸에 선다. 세대
    /// 표식도 운영 중과 같은 지금 세대다(새 세대는 다음 핸드셰이크가 뽑는다). 배압 아래에서는 앞잡이 줄이 차
    /// 있을 수 있어, 이것이 없으면 그 줄([`Policy::frame_buffer`] 장)을 말없이 버린다.
    ///
    /// ★읽기 태스크를 먼저 접는다★ — 그 태스크는 앞잡이 줄 자리를 잡은 뒤에만 읽으므로(`read_link`) 접힌 뒤
    /// 줄에 든 것이 읽은 것 전부다. 안 접고 비우면 비운 자리로 통로를 더 읽는다.
    /// ★그 취소를 한 바퀴 기다리는 것 말고는 기다리지 않는다★(`drop_link` 의 같은 대기를 앞당긴 것 — TRD 3
    /// §3-0 ② 「여기까지 기다림 없이」). 재요청도 안 쓴다 — 그 창은 열린 채 남아 다음 통로에서 다시 나간다
    /// (`reissue_resumes`).
    ///
    /// 돌려주는 것 = 앞잡이 줄에 이미 와 있던 맨 앞의 읽는 쪽 끝(디코드 실패로 끊는 한 장 · 상대의 닫힘 · 통로
    /// 오류) — 끊김 원인을 그것으로 고쳐 적을지는 [`earlier_end`] 가 정한다.
    async fn salvage_read_ahead(&mut self) -> Option<DisconnectCause> {
        let mut frames = self.frames_rx.take()?;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        // 맨 앞의 읽기 끝 — 디코드 실패로 끊는 한 장이 그 뒤 닫힘보다 앞이다(운영 중에 그 한 장에서 끊었을 때와
        //   같은 원인).
        let mut cut = None;
        while let Ok(msg) = frames.try_recv() {
            match msg {
                // ★디코드 실패로 끊는 정책이어도 멈추지 않고 그 뒤까지 넘긴다★ — 운영 중에 그 정책으로 끊으면
                //   `run_live` 가 그 한 장에서 돌아오고 그 뒤에 이미 읽은 것은 이 비우기가 넘긴다. 여기서만 멈추면
                //   같은 앞잡이 줄이 끊은 자리가 운영 중이었나 비우는 중이었나에 따라 넘어가기도 버려지기도 한다.
                //   끊김 원인만 그 한 장의 것으로 적어 둔다. 돌려받는 재요청은 안 쓴다(위 rustdoc).
                LinkMsg::Frame(frame, charge) => {
                    if let Err(Input::LinkLost(cause)) = self.on_frame(frame, charge) {
                        cut.get_or_insert(cause);
                    }
                }
                // 그 뒤에는 아무것도 없다(읽기 태스크가 거기서 끝났다).
                LinkMsg::Closed => return cut.or(Some(DisconnectCause::PeerClosed)),
                LinkMsg::Failed => return cut.or(Some(DisconnectCause::ReadFailed)),
            }
        }
        cut
    }

    fn fail_pending(&mut self, cause: DisconnectCause) {
        self.last_cause.store(cause_code(cause), Ordering::Release);
        let generation = self.machine.generation();
        // ★대기 표에 있는 것은 보냄이다★ — 표에 오르는 것은 쓰기 직전이라(`on_cmd`) 상대에게 갔을 수 있다. 찍힌
        //   세대 대조를 지나야 오르므로 그 세대가 곧 지금 세대다.
        for (_, slot) in self.pending.drain() {
            let _ = slot.send(Err(RequestError::Disconnected {
                generation,
                cause,
                sent: true,
            }));
        }
        // ★큐에서 잠든 것까지 걷는다★ — 요청은 안 보냄으로 깨우고, 답장 없는 것은 **센다**(핸들은 이미 `Ok` 를
        //   받아 갔으므로 여기서 안 세면 그대로 조용한 유실이다).
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
    }

    /// 쓰지 않을 명령 하나를 끝낸다 — 운영 단계가 아닐 때 도착했거나 찍힌 세대가 지금 세대가 아니다(`on_cmd`).
    /// 요청은 그것이 묶인 세대(찍힌 값)와 감독이 아는 마지막 끊김 사유로 깨운다 — 그 사유는 찍힌 세대가 끝난
    /// 까닭이 아니라 그 뒤의 끊김 것일 수 있다(그 사이 여러 번 다시 붙었으면). 답장 없는 것은 센다.
    fn reject_command(&mut self, cmd: PeerCmd<W>) {
        match cmd.op {
            PeerOp::Request { reply, .. } => {
                let _ = reply.send(Err(RequestError::Disconnected {
                    generation: cmd.generation,
                    cause: cause_from_code(self.last_cause.load(Ordering::Acquire)),
                    sent: false,
                }));
            }
            // ★`_` 로 접지 마라★ — 새 갈래(예: 닫기 전 비우기의 표지)가 버려질 때 세나 · 깨우나 · 따로 끝내나를
            //   그 자리에서 정하게 한다.
            PeerOp::Notify { .. } | PeerOp::Raw(_) | PeerOp::Resume { .. } => self.dropped_out += 1,
        }
    }

    /// ★순서 칸을 비우는 것이 맨 뒤다★ — 받는 쪽을 기다리는 것은 거기뿐이라, 그 앞(대기자 깨움 · 나가는
    /// 큐 닫기 · 남은 명령 거절)을 받는 쪽 속도에 묶지 않는다. 앞에 두면 그 사이 큐에 든 요청 · 알림이 받는
    /// 쪽이 비울 때까지(최대 [`CLOSE_DRAIN_LIMIT`]) 매달린다.
    async fn teardown(&mut self) {
        let cause = DisconnectCause::Explicit;
        self.drop_link(CloseCode::GOING_AWAY).await;
        self.fail_pending(cause);
        self.data_rx.close();
        // 남은 것도 마저 깨운다 — 닫은 뒤에 들어온 것은 채널이 `Closed` 로 거절한다.
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
        // ★열린 이어받기 창을 접는다★ — 장부는 이 감독과 함께 사라지므로, 안 접으면 붙든 조각이 배달도 잘림
        //   신고도 없이 없어진다. 창이 시한에 닫힐 때와 같은 신고로 내보낸다.
        let open = self.streams.close_resumes();
        self.hand_over_truncations(open);
        self.report_drops();
        self.emit(TransportEvent::Closed {
            peer: self.id.clone(),
            generation: self.machine.generation(),
            cause,
        });
        self.hand_over_held().await;
    }

    async fn hand_over_held(&mut self) {
        if self.order.is_empty() {
            return;
        }
        let clock = self.clock.clone();
        let (lane, order) = (&self.inbound, &mut self.order);
        let all = async move {
            while !order.is_empty() {
                if !drain_order(lane, order).await {
                    return false;
                }
            }
            true
        };
        match with_deadline(&*clock, CLOSE_DRAIN_LIMIT, all).await {
            Ok(open) => self.note_room(open),
            Err(_) => self.order.clear(),
        }
    }

    // ── 위로 올리기 ──────────────────────────────────────────────────────────

    fn emit_machine(&mut self, event: MachineEvent) {
        let peer = self.id.clone();
        let event = match event {
            MachineEvent::Connected { generation } => {
                TransportEvent::Connected { peer, generation }
            }
            MachineEvent::Disconnected { generation, cause } => {
                self.last_cause.store(cause_code(cause), Ordering::Release);
                TransportEvent::Disconnected {
                    peer,
                    generation,
                    cause,
                }
            }
            MachineEvent::ConnectFailed {
                attempt,
                of,
                cause,
                retry_in,
            } => TransportEvent::ConnectFailed {
                peer,
                attempt,
                of,
                cause,
                retry_in,
            },
            MachineEvent::Rejected { code, reason } => {
                TransportEvent::Rejected { peer, code, reason }
            }
            MachineEvent::GaveUp { reason } => TransportEvent::GaveUp { peer, reason },
        };
        self.emit(event);
    }

    fn emit(&mut self, event: TransportEvent<W>) {
        self.put(Charged::bare(Incoming::Event(event)));
    }

    fn deliver(&mut self, msg: Received<W::In>) {
        let Received {
            msg,
            mut charge,
            generation,
        } = msg;
        // ★올라가는 것은 전부 읽기 문에 선다★ — 창에서 나온 조각이면 창 몫에서 뺀다(`budget.rs` 헤더). 창 몫에
        //   남겨 두면 받는 쪽이 느린 것만으로 창 몫이 차 다음 구멍을 못 붙든다.
        charge.unhold();
        let arrival = Arrival {
            peer: self.id.clone(),
            generation,
            msg,
        };
        self.put(Charged {
            item: Incoming::Message(arrival),
            charge,
        });
    }

    /// 순서 칸 끝에 넣고 줄에 실을 수 있는 만큼 싣는다. ★기다리지 않는다★ — 넣는 자리(사건 · 패킷 · 시한
    /// 사건 · 신고)가 전부 동기라서다. 거기서 기다리면 제어와 `fail_pending` 이 서고 정리가 매달린다.
    ///
    /// ★줄이 찼다고 이 연결을 끊지도 버리지도 않는다★ — 그 줄은 이 손잡이의 것이라(`PeerConfig::inbound`)
    /// 찼다는 것은 소비자가 이 상대를 못 따라간다는 뜻이지 연결의 잘못이 아니다(ADR-0310 결정 17). 대신 순서
    /// 칸이 붙든 동안 운영 루프가 그 상대의 읽기를 세운다(이 파일 헤더 「배압」).
    fn put(&mut self, item: Charged<Incoming<W>>) {
        if self.consumer_gone.load(Ordering::Acquire) {
            return;
        }
        let was_empty = self.order.is_empty();
        self.order.push_back(item);
        let open = flush(&self.inbound, &mut self.order);
        // 순서 칸이 지금 붙들기 시작했다 — 이 상대의 배압이 시작된 시각이다(`track_pause`).
        if was_empty && !self.order.is_empty() {
            self.order_since = self.clock.now();
        }
        self.note_room(open);
    }

    /// 세어 둔 나가는 쪽 유실을 신고한다. ★정책과 무관하게 신고한다★ — 세는 것을 그만두면
    /// `Disconnect` 보다 나쁜 선택이 된다(policy.rs 의 그 문장).
    fn report_drops(&mut self) {
        let count = std::mem::take(&mut self.dropped_out);
        if count == 0 {
            return;
        }
        // 아직 못 실은 신고가 순서 칸 끝에 있으면 거기 더한다 — 줄이 밀린 동안 바퀴마다 한 건씩 쌓이지 않게.
        if let Some(Charged {
            item: Incoming::Event(TransportEvent::Dropped { count: held, .. }),
            ..
        }) = self.order.back_mut()
        {
            *held += count;
            return;
        }
        self.emit(TransportEvent::Dropped {
            peer: self.id.clone(),
            direction: Direction::Outbound,
            count,
        });
    }

    /// 핸들이 못 넣은 것을 걷어 처리한다.
    ///
    /// ★나가는 큐 포화만 연결을 끊는다★ — 「내 큐가 찼다」는 이 연결의 문제이고, 「위로 올라가는 줄이
    /// 찼다」는 소비자의 속도다(`put`). 둘을 한 셈 칸으로 합치면 소비자가 느린 것만으로 연결이 끊긴다.
    ///
    /// ★그 넘침이 지금 세대의 것일 때만 끊는다★ — 셈은 세대를 넘어 남는다: 세대 G 에서 센 넘침은 그 세대가 끝난
    /// 뒤 연결 수립 구간을 지나(그 구간은 이것을 안 부른다) G+1 의 첫 바퀴가 걷는다. 거기서 정책을 집행하면 방금
    /// 붙은 새 연결을 옛 연결의 넘침으로 끊는다. 셈은 세대와 무관하게 신고한다.
    #[must_use = "운영 중에는 이 판정이 연결을 끊는다"]
    fn drain_overflow(&mut self) -> Option<Input> {
        let refused = self.refused.swap(0, Ordering::AcqRel);
        if refused > 0 {
            self.dropped_out += refused;
        }
        let full = self.overflow.swap(0, Ordering::AcqRel);
        if full == 0 {
            return None;
        }
        self.dropped_out += full;
        self.report_drops();
        // 셈을 걷은 뒤 읽는다 — 걷은 넘침의 세대 이상이 보인다(`Peer::count_overflow`).
        let latest = Generation(self.overflow_generation.load(Ordering::Acquire));
        if latest != self.machine.generation() {
            return None;
        }
        match self.policy.full_queue {
            QueueFullPolicy::Disconnect => Some(Input::LinkLost(DisconnectCause::QueueFull)),
            QueueFullPolicy::DropAndReport => None,
        }
    }
}

/// 운영 단계를 떠날 때 기계에 먹일 입력 — 통로를 버리며 건진 앞잡이 줄([`Supervisor::salvage_read_ahead`])에
/// 읽는 쪽의 끝이 이미 와 있었으면(맨 앞의 것 — 줄 가운데의 디코드 실패로 끊는 한 장일 수 있다) 우리가 떠난
/// 까닭(쓰기 실패 · 침묵 · 넘침 …) 대신 그 끝을 끊김 원인으로 적는다 — 통로를 버리기 전에 이미 읽힌 끝이다. 배압으로 프레임 팔을 뺀 동안 상대가 닫으면 감독은 그 닫힘을
/// 그 탓에 실패한 쓰기로 먼저 알아챌 수 있다(crate 헤더 「알려진 한계」의 줄 항목).
///
/// ★고쳐 적지 않는 것★ — 앱이 닫았거나(`Input::Close`) 다시 걸라고 했으면(`Input::ReconnectNow`) 그대로다: 둘 다
/// 끊김 원인이 [`DisconnectCause::Explicit`] 이고 부른 쪽이 그 까닭을 안다. 다시 걸기를 끊김으로 바꾸면 예산을
/// 되채우고 곧바로 거는 그 입구의 약속(ADR-0180 결정 7)이 사라진다. 우리 까닭이 이미 읽는 쪽의 끝(디코드 실패로
/// 끊음)이면 그것도 그대로다 — 줄 안에서 그 한 장이 끝보다 앞이다.
fn earlier_end(input: Input, read_end: Option<DisconnectCause>) -> Input {
    match (input, read_end) {
        (Input::LinkLost(ours), Some(theirs)) if !is_read_end(ours) => Input::LinkLost(theirs),
        (input, _) => input,
    }
}

/// 읽는 쪽의 끝인가 — 상대의 닫힘 · 통로 읽기 오류(디코드 실패로 끊음 포함).
fn is_read_end(cause: DisconnectCause) -> bool {
    matches!(
        cause,
        DisconnectCause::PeerClosed | DisconnectCause::ReadFailed
    )
}

/// 순서 칸을 앞에서부터 줄에 실을 수 있는 만큼 싣는다 — 기다리지 않는다. `false` = 받는 쪽이 닫혔다(순서
/// 칸은 비운다 — 실을 데가 없다).
fn flush<T>(lane: &mpsc::Sender<T>, order: &mut VecDeque<T>) -> bool {
    while let Some(item) = order.pop_front() {
        match lane.try_send(item) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(item)) => {
                order.push_front(item);
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                order.clear();
                return false;
            }
        }
    }
    true
}

/// 줄에 자리가 날 때까지 기다렸다가 순서 칸을 실을 수 있는 만큼 싣는다. `false` = 받는 쪽이 닫혔다.
///
/// ★취소 안전 — `select!` 팔에 건다★: 자리를 기다리는 동안 떨어뜨리면 순서 칸은 그대로이고, 자리를 잡은
/// 뒤의 꺼냄 · 싣기는 `await` 없이 한 번에 끝난다. 자리는 줄에 넣는 쪽들 사이에서 기다린 순서대로
/// 돌아간다(외부 사실 — tokio 유계 채널의 자리 잡기는 FIFO).
async fn drain_order<T>(lane: &mpsc::Sender<T>, order: &mut VecDeque<T>) -> bool {
    let Ok(permit) = lane.reserve().await else {
        order.clear();
        return false;
    };
    if let Some(item) = order.pop_front() {
        permit.send(item);
    }
    flush(lane, order)
}

/// 통로만 연다. ★시한은 dial 과 핸드셰이크가 **함께** 쓴다★ — 현행 `HANDSHAKE_TIMEOUT` 이 같은 모양이라,
/// dial 이 오래 끌면 핸드셰이크에 남는 시간이 그만큼 줄어든다.
async fn dial_once(
    dialer: Arc<dyn Dialer>,
    addr: Address,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), ConnectFailure> {
    let left = deadline.saturating_duration_since(clock.now());
    match with_deadline(&*clock, left, dialer.dial(&addr)).await {
        Ok(Ok(pair)) => Ok(pair),
        Ok(Err(err)) => Err(ConnectFailure::Unreachable(err)),
        Err(_) => Err(ConnectFailure::Unreachable(LinkError::new(
            "connect timed out",
        ))),
    }
}

/// 열린 통로 위에서 핸드셰이크를 끝까지 돌린다.
async fn handshake_once(
    mut tx: Box<dyn LinkTx>,
    mut rx: Box<dyn LinkRx>,
    handshake: Arc<dyn Handshake>,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), ConnectFailure> {
    let left = |clock: &dyn Clock| deadline.saturating_duration_since(clock.now());
    let mut step = handshake.start();
    loop {
        match step {
            HandshakeStep::Done => return Ok((tx, rx)),
            HandshakeStep::Reject { last, reason } => {
                let mut link_usable = true;
                if let Some(frame) = last {
                    // 절반 나간 프레임 뒤에 더 쓰지 않는다 — 통로를 그냥 버린다.
                    link_usable = matches!(
                        with_deadline(&*clock, left(&*clock), tx.send(frame)).await,
                        Ok(Ok(()))
                    );
                }
                if link_usable {
                    let close = Close::new(CloseCode::HANDSHAKE_REJECTED, reason.clone());
                    let _ = with_deadline(&*clock, left(&*clock), tx.close(close)).await;
                }
                return Err(ConnectFailure::Rejected {
                    code: Some(CloseCode::HANDSHAKE_REJECTED),
                    reason,
                });
            }
            HandshakeStep::Send(frame) => {
                match with_deadline(&*clock, left(&*clock), tx.send(frame)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => return Err(ConnectFailure::Unreachable(err)),
                    Err(_) => {
                        return Err(ConnectFailure::Unreachable(LinkError::new(
                            "handshake write timed out",
                        )))
                    }
                }
                step = HandshakeStep::Await;
            }
            HandshakeStep::Await => match with_deadline(&*clock, left(&*clock), rx.recv()).await {
                Ok(Ok(LinkRead::Frame(Frame::Keepalive))) => step = HandshakeStep::Await,
                Ok(Ok(LinkRead::Frame(frame))) => step = handshake.on_frame(&frame),
                // ★상대가 코드를 실어 닫았다 = 거절이다★. 그 판정이 재연결 예산을 가르므로
                //   (ADR-0180 결정 5) 이 코드가 없으면 「거절은 예산을 안 쓴다」가 코드로는 성립하지 않는다.
                Ok(Ok(LinkRead::Closed(Some(close)))) if is_rejection(close.code) => {
                    return Err(ConnectFailure::Rejected {
                        code: Some(close.code),
                        reason: close.reason,
                    })
                }
                // ★맨 종료는 「거절」이 아니다★ — 상대가 그렇게 **말했을** 때만 거절이고, 말 없이
                //   끊긴 것은 「못 붙었다」라 예산을 쓴다(데몬 재시작 중이면 다음 시도에 붙는다).
                Ok(Ok(LinkRead::Closed(_))) => {
                    return Err(ConnectFailure::Unreachable(LinkError::new(
                        "peer closed during handshake",
                    )))
                }
                Ok(Err(err)) => return Err(ConnectFailure::Unreachable(err)),
                Err(_) => {
                    return Err(ConnectFailure::Unreachable(LinkError::new(
                        "handshake timed out",
                    )))
                }
            },
        }
    }
}

/// 통로에서 읽어 감독에게 넘긴다.
///
/// ★따로 태스크인 이유★ — `select!` 팔에 `recv()` 를 직접 걸면 다른 팔이 이길 때마다 그 future 가
/// 취소되고, 취소 안전하지 않은 전송에서는 그 순간 프레임이 사라진다.
///
/// ★읽기 문이 닫혀 있으면 다음 읽기를 안 한다★(「더하기 전 물높이」 — `budget.rs`) — 앞잡이 줄의 칸 수만으로는
/// 그 줄이 칸 수 × 한 메시지 상한까지 쥔다. 몫은 읽은 그 자리에서 더한다.
///
/// ★앞잡이 줄 자리를 읽기 전에 잡는다★ — 읽은 한 장을 손에 든 채 자리를 기다리지 않는다. 그래서 감독이 이
/// 태스크를 접으면 이미 읽은 것은 전부 앞잡이 줄에 있다(`Supervisor::salvage_read_ahead`). 손에 든 채
/// 접히면 그 한 장은 통로에서 이미 꺼냈는데 아무 데도 안 간다.
async fn read_link(mut rx: Box<dyn LinkRx>, out: mpsc::Sender<LinkMsg>, ledger: Arc<Ledger>) {
    loop {
        let Ok(slot) = out.reserve().await else {
            return;
        };
        ledger.reader_room().await;
        let (msg, last) = match rx.recv().await {
            Ok(LinkRead::Frame(frame)) => {
                let charge = ledger.charge(frame.payload_len());
                (LinkMsg::Frame(frame, charge), false)
            }
            Ok(LinkRead::Closed(_)) => (LinkMsg::Closed, true),
            Err(_) => (LinkMsg::Failed, true),
        };
        slot.send(msg);
        if last {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::split;
    use crate::policy::ReconnectPolicy;
    use crate::testing::{
        settle, settle_until, ClientMsg, HelloHandshake, ImmediateHandshake, ListeningHandshake,
        ManualClock, MemoryEndpoint, MemoryNetwork, RejectingHandshake, TestIn, TestOut, TestWire,
    };
    use futures_util::future::poll_immediate;
    use std::time::Duration;

    struct Harness {
        peer: Peer<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
        inbound: mpsc::Receiver<Charged<Incoming<TestWire>>>,
        /// 울타리를 보려고 미리 꺼내 온 것들. 뒤이은 단언이 이것부터 읽는다.
        buffered: Vec<Incoming<TestWire>>,
        /// 감독 태스크. ★채널 · 시계로는 「끝났다」와 「패닉했다」를 못 가른다★ — 패닉도 감독을 떨어뜨려
        /// 채널을 다 닫고 잠든 시계를 다 놓는다.
        task: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn events(&mut self) -> Vec<TransportEvent<TestWire>> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Event(event) => Some(event),
                    Incoming::Message(_) => None,
                })
                .collect()
        }

        fn kinds(&mut self) -> Vec<&'static str> {
            self.events().iter().map(|e| e.kind()).collect()
        }

        fn messages(&mut self) -> Vec<TestIn> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Message(arrival) => Some(arrival.msg),
                    Incoming::Event(_) => None,
                })
                .collect()
        }

        fn drain(&mut self) -> Vec<Incoming<TestWire>> {
            let mut out = std::mem::take(&mut self.buffered);
            while let Ok(item) = self.inbound.try_recv() {
                out.push(item.into_item());
            }
            out
        }

        /// 이 문구를 실은 알림이 올라왔나. ★꺼내 온 것을 버리지 않는다★ — 뒤이은 단언이 그것을 본다.
        fn seen_notice(&mut self, body: &str) -> bool {
            let mut hit = false;
            let mut kept = Vec::new();
            while let Ok(item) = self.inbound.try_recv() {
                let item = item.into_item();
                if let Some(TestIn::Notice(text)) = item.as_message().map(|a| &a.msg) {
                    if text == body {
                        hit = true;
                    }
                }
                kept.push(item);
            }
            self.buffered.extend(kept);
            hit
        }

        fn disconnect_cause(&mut self) -> Option<DisconnectCause> {
            self.events().iter().find_map(|e| match e {
                TransportEvent::Disconnected { cause, .. } => Some(*cause),
                _ => None,
            })
        }

        /// 정리가 나가는 큐를 닫았나. ★줄에 자리가 있었으면 보이는 순간 마지막 유실 신고와 `Closed` 까지
        /// 올라와 있다★ — `teardown` 이 큐를 닫은 뒤 `await` 없이 곧장 넣고 싣으므로, current_thread
        /// 런타임에서는 그 사이에 이 줄이 끼어들 수 없다. 줄이 차 있었으면 정리는 아직 순서 칸에서 기다린다.
        fn supervisor_done(&self) -> bool {
            self.peer.inner.data_tx.is_closed()
        }

        /// 감독이 나가는 큐를 다 꺼내 갔나. ★「통로로 아무것도 안 나갔다」만으로는 「쓰는 중」이 안
        /// 선다★ — 큐에 남아 있어도 참이고, 큐에 남은 것은 `fail_pending` 이 따로 세거나 깨운다. 그러면
        /// 쓰는 도중의 계수를 재는 단언이 쓰기와 무관하게 맞는다.
        fn queue_drained(&self) -> bool {
            let tx = &self.peer.inner.data_tx;
            tx.capacity() == tx.max_capacity()
        }

        /// 감독 태스크가 지금 끝나 있고 패닉하지 않았나. 끝난 핸들을 다시 폴링하면 tokio 가 패닉하므로
        /// 하네스를 먹는다 — 마지막 단언으로 쓴다.
        async fn exited_cleanly(self) -> bool {
            matches!(poll_immediate(self.task).await, Some(Ok(())))
        }
    }

    fn no_jitter(policy: Policy) -> Policy {
        Policy {
            reconnect: ReconnectPolicy {
                jitter: 0.0,
                ..policy.reconnect
            },
            ..policy
        }
    }

    fn build_with(policy: Policy, handshake: Arc<dyn Handshake>, inbound_cap: usize) -> Harness {
        build_budgeted(policy, handshake, inbound_cap, BYTE_BUDGET)
    }

    fn build_budgeted(
        policy: Policy,
        handshake: Arc<dyn Handshake>,
        inbound_cap: usize,
        byte_budget: usize,
    ) -> Harness {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, inbound) = mpsc::channel(inbound_cap);
        let (peer, supervisor) = new_peer(PeerConfig {
            id: PeerId::new("d1"),
            addr: Address::new("mem://d1"),
            wire: Arc::new(TestWire),
            dialer: net.dialer(),
            clock: clock.handle(),
            handshake,
            policy: no_jitter(policy),
            inbound: inbound_tx,
            byte_budget,
            generations: Arc::new(AtomicU64::new(0)),
        });
        Harness {
            peer,
            net,
            clock,
            inbound,
            buffered: Vec::new(),
            task: tokio::spawn(supervisor.run()),
        }
    }

    fn build(policy: Policy, handshake: Arc<dyn Handshake>) -> Harness {
        build_with(policy, handshake, 4096)
    }

    async fn live() -> (Harness, Arc<MemoryEndpoint>) {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.kinds(), vec!["Connected"]);
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        (h, endpoint)
    }

    /// 프레임이 **끊이지 않는** 상태를 만든다.
    ///
    /// ★유한한 다발로는 굶주림을 못 만든다★ — 감독이 그것을 다 비우고 나면 프레임 팔이 `Pending` 이
    /// 되어 뒤의 팔들이 그때 폴링된다. 즉 「1000개를 밀어 넣고 settle」 하는 테스트는 **`biased` 든
    /// 아니든 똑같이 통과한다**(실측 — 그 형태로 쓴 첫 판이 변이를 하나도 못 잡았다). 굶주림은
    /// **채워지는 속도 ≥ 비우는 속도**일 때만 생기므로 공급을 계속 돌린다.
    fn flood(endpoint: &Arc<MemoryEndpoint>) -> tokio::task::JoinHandle<()> {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let mut seq = 1u64;
            loop {
                for _ in 0..256 {
                    endpoint.push(TestWire::chunk(1, 1, seq));
                    seq += 1;
                }
                tokio::task::yield_now().await;
            }
        })
    }

    /// 홍수 아래에서 감독을 몇 바퀴 돌린다. [`settle`] 의 512 바퀴는 여기서 과하다(공급이 그만큼 쌓인다).
    async fn pump(rounds: usize) {
        for _ in 0..rounds {
            tokio::task::yield_now().await;
        }
    }

    /// ★결함이 되살아나면 **매달리는 대신 깨끗이 실패한다**★ — libtest 에는 테스트별 시한이 없어서,
    /// 영구 대기가 되는 회귀는 이 그물이 없으면 CI 를 통째로 멈춘다. 실시간 5초는 **실패 경로에서만**
    /// 흐른다(같은 그물의 선례가 셸 패키지의 `lib_unit` 이다).
    async fn joined(
        task: tokio::task::JoinHandle<Result<TestIn, RequestError>>,
    ) -> Result<TestIn, RequestError> {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("요청이 안 깨어났다 — 영구 대기 회귀다")
            .expect("요청 태스크가 패닉했다")
    }

    fn spawn_request(
        peer: &Peer<TestWire>,
        tag: u64,
    ) -> tokio::task::JoinHandle<Result<TestIn, RequestError>> {
        let peer = peer.clone();
        tokio::spawn(async move {
            peer.request(TestOut::Request {
                tag,
                body: format!("body{tag}"),
            })
            .await
        })
    }

    // ── C2/F1: 굶주림 방지 규칙 ──
    //
    // ★이 여덟이 C2·F1 의 회귀망이다★ — 아래 홍수 테스트가 아니다. 홍수 쪽은 「부하 아래에서 살아
    //   있나」를 보는 것이고, **굶주림 자체는 이 하네스로 못 만든다**(읽기 태스크가 구조적 병목이라
    //   프레임 큐가 늘 빈다 — `next_turn` 주석의 실측).
    const CAP: usize = 512;

    #[test]
    fn a_due_deadline_beats_a_ready_frame_however_many_are_queued() {
        let t0 = Instant::now();
        for frames_in_row in [0usize, 1, FRAME_BURST, 10_000] {
            for outbound in [0usize, 1, CAP] {
                assert_eq!(
                    next_turn(t0, t0, frames_in_row, outbound, CAP),
                    Turn::Timer,
                    "frames={frames_in_row} outbound={outbound}"
                );
                assert_eq!(
                    next_turn(
                        t0 + Duration::from_secs(1),
                        t0,
                        frames_in_row,
                        outbound,
                        CAP
                    ),
                    Turn::Timer,
                    "이미 지난 시한은 더 급하다"
                );
            }
        }
    }

    #[test]
    fn a_frame_burst_yields_a_turn_to_the_outbound_queue() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(
            next_turn(t0, future, FRAME_BURST, 1, CAP),
            Turn::Outbound(1)
        );
        assert_eq!(
            next_turn(t0, future, FRAME_BURST + 1, 3, CAP),
            Turn::Outbound(3)
        );
    }

    #[test]
    fn below_the_burst_and_before_the_deadline_the_loop_just_waits() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, 0, 1, CAP), Turn::Wait);
        assert_eq!(next_turn(t0, future, FRAME_BURST - 1, 1, CAP), Turn::Wait);
    }

    #[test]
    fn an_empty_outbound_queue_never_takes_a_turn() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, FRAME_BURST * 10, 0, CAP), Turn::Wait);
    }

    #[test]
    fn the_deadline_outranks_the_burst_when_both_are_due() {
        let t0 = Instant::now();
        assert_eq!(next_turn(t0, t0, FRAME_BURST, 5, CAP), Turn::Timer);
    }

    #[test]
    fn the_burst_bound_is_finite_so_the_outbound_queue_cannot_be_starved() {
        assert!(FRAME_BURST > 0);
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        // ★상한을 두고 찾는다★ — 무한 탐색으로 두면 규칙이 깨졌을 때 매달린다.
        let first_yield = (0..FRAME_BURST * 4)
            .find(|n| matches!(next_turn(t0, future, *n, 1, CAP), Turn::Outbound(_)))
            .expect("연달아 처리하는 프레임 수에 상한이 없으면 나가는 큐가 영원히 굶는다");
        assert_eq!(first_yield, FRAME_BURST);
    }

    // ── F1: 고정 비율만으로는 부족하다 ──
    #[test]
    fn a_backlog_past_half_capacity_does_not_wait_for_a_frame_burst() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        let half = CAP / 2;
        for outbound in [half, half + 1, CAP] {
            assert_eq!(
                next_turn(t0, future, 0, outbound, CAP),
                Turn::Outbound(outbound),
                "★깊이를 안 보면 비율을 넘는 생산자 앞에서 큐가 계속 자라 QueueFull 로 끝난다★"
            );
        }
        assert_eq!(
            next_turn(t0, future, 0, half - 1, CAP),
            Turn::Wait,
            "절반 아래에서는 평소대로 기다린다"
        );
    }

    #[test]
    fn a_yield_drains_the_whole_backlog_not_one_item() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        for outbound in [1usize, 7, 64, CAP] {
            assert_eq!(
                next_turn(t0, future, FRAME_BURST, outbound, CAP),
                Turn::Outbound(outbound),
                "한 바퀴에 한 개씩만 빼면 문턱만 32배 높아진 같은 결함이다"
            );
        }
    }

    // ── 연결 수립 ──
    #[tokio::test]
    async fn a_peer_dials_and_reaches_live_on_its_own() {
        let (h, _endpoint) = live().await;
        assert_eq!(h.peer.generation(), Generation(1));
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_two_step_handshake_round_trips_before_going_live() {
        let h = build(Policy::default(), Arc::new(HelloHandshake));
        settle_until("핸드셰이크 진입", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().expect("통로");
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text(HelloHandshake::HELLO.into())]
        );
        endpoint.push(Frame::Text(HelloHandshake::WELCOME.into()));
        settle_until("운영 진입", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── D1: 세대는 이름표별 공용 계수기에서 나온다 ──
    #[tokio::test]
    async fn the_supervisor_mints_its_generation_from_the_shared_counter() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(64);
        let counter = Arc::new(AtomicU64::new(41));
        let peer = spawn_peer(PeerConfig {
            id: PeerId::new("d1"),
            addr: Address::new("mem://d1"),
            wire: Arc::new(TestWire),
            dialer: net.dialer(),
            clock: clock.handle(),
            handshake: Arc::new(ImmediateHandshake),
            policy: Policy::default(),
            inbound: inbound_tx,
            byte_budget: BYTE_BUDGET,
            generations: counter.clone(),
        });
        settle_until("연결", || peer.peer_state() == PeerState::Live).await;
        assert_eq!(peer.generation(), Generation(42));
        assert_eq!(
            counter.load(Ordering::Acquire),
            42,
            "계수기가 함께 올라간다"
        );
    }

    #[tokio::test]
    async fn two_supervisors_sharing_a_counter_never_mint_the_same_generation() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(256);
        let counter = Arc::new(AtomicU64::new(0));
        let spawn = |tag: &str| {
            spawn_peer(PeerConfig {
                id: PeerId::new(tag),
                addr: Address::new("mem://d1"),
                wire: Arc::new(TestWire),
                dialer: net.dialer(),
                clock: clock.handle(),
                handshake: Arc::new(ImmediateHandshake),
                policy: Policy::default(),
                inbound: inbound_tx.clone(),
                byte_budget: BYTE_BUDGET,
                generations: counter.clone(),
            })
        };
        // 옛 감독이 아직 살아 있는 채로 새 감독이 붙는 상황(같은 이름표의 승계 경합).
        let first = spawn("d1");
        let second = spawn("d1");
        settle_until("둘 다 연결", || {
            first.peer_state() == PeerState::Live && second.peer_state() == PeerState::Live
        })
        .await;
        assert_ne!(
            first.generation(),
            second.generation(),
            "★같은 번호를 두 감독이 발행하면 소비자가 어느 소켓의 것인지 못 가린다★"
        );
    }

    // ── 요청/응답 ──
    #[tokio::test]
    async fn a_request_is_registered_before_it_is_written() {
        let (h, endpoint) = live().await;
        let task = spawn_request(&h.peer, 5);
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("req|5|body5".into())]);
        endpoint.push(TestWire::reply(5));
        settle().await;
        assert_eq!(
            joined(task).await,
            Ok(TestIn::Reply {
                tag: 5,
                body: "ok".into()
            })
        );
    }

    #[tokio::test]
    async fn a_request_that_takes_no_reply_fails_at_once() {
        let (h, _endpoint) = live().await;
        let out = h.peer.request(TestOut::Notice("hi".into())).await;
        assert_eq!(out, Err(RequestError::NotARequest));
    }

    #[tokio::test]
    async fn only_the_silent_request_times_out() {
        let (mut h, endpoint) = live().await;
        let a = spawn_request(&h.peer, 1);
        settle().await;
        let b = spawn_request(&h.peer, 2);
        settle().await;
        endpoint.push(TestWire::reply(2));
        settle().await;
        assert!(joined(b).await.is_ok());

        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert_eq!(
            joined(a).await,
            Err(RequestError::TimedOut {
                after: Duration::from_secs(8)
            })
        );
        assert!(h.kinds().contains(&"RequestTimedOut"));
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "시한 만료는 연결을 건드리지 않는다"
        );
    }

    #[tokio::test]
    async fn a_reply_that_arrives_after_the_timeout_is_dropped_and_the_tag_is_retired() {
        let (mut h, endpoint) = live().await;
        let first = spawn_request(&h.peer, 9);
        settle().await;
        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert!(matches!(
            joined(first).await,
            Err(RequestError::TimedOut { .. })
        ));
        h.drain();

        endpoint.push(TestWire::reply(9));
        settle().await;
        assert!(h.messages().is_empty(), "늦은 답장은 조용히 버려진다");
        assert!(
            !h.kinds().contains(&"UnmatchedReply"),
            "설계된 늦은 답장은 신고 대상이 아니다"
        );

        let again = h.peer.request(TestOut::Request {
            tag: 9,
            body: "b".into(),
        });
        assert_eq!(again.await, Err(RequestError::TagRetired));
    }

    // ── M1: 겹친 번호는 배달을 못 가르므로 양쪽을 다 실패시킨다 ──
    #[tokio::test]
    async fn a_colliding_tag_wakes_the_older_waiter_and_refuses_the_newer_one() {
        let (h, endpoint) = live().await;
        let old = spawn_request(&h.peer, 3);
        settle().await;
        let new = spawn_request(&h.peer, 3);
        settle().await;
        assert_eq!(joined(old).await, Err(RequestError::Superseded));
        assert_eq!(joined(new).await, Err(RequestError::TagRetired));

        // 먼저 나간 요청의 늦은 답장이 도착해도 아무에게도 배달되지 않는다.
        endpoint.push(TestWire::reply(3));
        settle().await;
    }

    #[tokio::test]
    async fn a_reply_nobody_asked_for_is_reported() {
        let (mut h, endpoint) = live().await;
        h.drain();
        endpoint.push(TestWire::reply(77));
        settle().await;
        assert!(h.kinds().contains(&"UnmatchedReply"));
    }

    // ── C1: 연결을 버리는 어느 갈래도 대기자를 매달아 두지 않는다 ──
    #[tokio::test]
    async fn a_pending_request_is_woken_when_the_shell_asks_to_reconnect() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 11);
        settle().await;
        h.peer.reconnect_now();
        settle().await;
        assert!(
            matches!(
                joined(pending).await,
                Err(RequestError::Disconnected { .. })
            ),
            "reconnect_now 만 FailPending 을 안 내면 이 슬롯은 영구 대기가 된다"
        );
    }

    #[tokio::test]
    async fn a_pending_request_does_not_hang_when_reconnect_then_exhausts_the_budget() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 12);
        settle().await;

        h.net.fail_forever();
        h.peer.reconnect_now();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
    }

    #[tokio::test]
    async fn a_request_that_lands_after_the_link_is_gone_is_refused_not_parked() {
        let (h, endpoint) = live().await;
        // 큐에는 들어가지만 감독은 이미 운영 단계를 떠난 뒤인 경합을 흉내낸다.
        let late = {
            let peer = h.peer.clone();
            tokio::spawn(async move {
                peer.request(TestOut::Request {
                    tag: 21,
                    body: "x".into(),
                })
                .await
            })
        };
        endpoint.close();
        settle().await;
        let outcome = joined(late).await;
        assert!(
            matches!(outcome, Err(RequestError::Disconnected { .. })),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn losing_the_link_wakes_every_waiter() {
        let (mut h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 1);
        settle().await;
        endpoint.close();
        settle().await;
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
        assert!(h.kinds().contains(&"Disconnected"));
    }

    // ── 세대 도장 · 보냄 칸(TRD 3 §3-0 ③ · D12 — 셸 ADR-0195 시험 둘의 transport 판) ──

    /// 상대가 받은 데이터 프레임을 글로 — 하네스 어휘는 전부 `Frame::Text` 다.
    fn texts(frames: Vec<Frame>) -> Vec<String> {
        frames
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    fn spawn_request_in(
        peer: &Peer<TestWire>,
        generation: Generation,
        tag: u64,
    ) -> tokio::task::JoinHandle<Result<TestIn, RequestError>> {
        let peer = peer.clone();
        tokio::spawn(async move {
            peer.request_in(
                generation,
                TestOut::Request {
                    tag,
                    body: format!("body{tag}"),
                },
            )
            .await
        })
    }

    // 셸 `buffered_command_on_disconnect_is_drained_not_executed_post_reconnect` 의 transport 판 — 답을 기다리던
    //   요청과 큐에 든 채였던 요청이 끊김에 둘 다 깨어나고, 다음 연결에서 실행되지 않는다. ★갈래를 결정론으로
    //   못 박는다★: 앞선 알림의 쓰기를 막아 감독을 그 위에 세우면 둘째 요청은 큐에 남는다(쓰기는 데이터 큐를 안
    //   읽는다). 그래서 둘이 보냄 칸의 두 갈래(D12)를 하나씩 맡는다.
    #[tokio::test]
    async fn a_waiting_request_ends_as_sent_a_queued_one_as_not_sent_and_neither_runs_on_the_next_link(
    ) {
        let (h, first) = live().await;
        let g1 = h.peer.generation();
        let waiting = spawn_request(&h.peer, 1);
        settle().await;
        assert_eq!(
            texts(first.frames()),
            vec!["req|1|body1"],
            "첫 요청이 나가 답을 기다린다"
        );

        first.stall_writes(true);
        h.peer
            .notify(TestOut::Notice("stuck".into()))
            .expect("운영 중이다");
        settle().await;
        let queued = spawn_request(&h.peer, 2);
        settle().await;
        assert!(
            !h.queue_drained(),
            "감독이 막힌 쓰기 위에 서 있어 둘째 요청은 큐에 있다"
        );

        // 통로가 끊긴다 — 막힌 쓰기가 실패로 끝난다.
        first.fail_writes();
        first.stall_writes(false);
        settle().await;
        let waiting = joined(waiting).await;
        assert!(
            matches!(
                waiting,
                Err(RequestError::Disconnected {
                    generation,
                    sent: true,
                    ..
                }) if generation == g1
            ),
            "★답을 기다리던 요청은 보냄이다★ — 상대가 처리했을 수 있다: {waiting:?}"
        );
        let queued = joined(queued).await;
        assert!(
            matches!(
                queued,
                Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                }) if generation == g1
            ),
            "★큐에 든 채였던 요청은 안 보냄이다★ — 다시 보내도 두 번 적용되지 않는다: {queued:?}"
        );

        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let next = h.net.accept().expect("둘째 통로");
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "끊긴 명령이 다음 연결에서 실행되면 안 된다(ADR-0195)"
        );
    }

    // 셸 `held_clone_enqueued_after_disconnect_is_rejected_and_never_runs` 의 transport 판 — ★감독의 쓰기 직전
    //   대조를 잰다★. 기계는 끊길 때 큐를 비우지만 그 뒤에 든 것은 못 보고, 연결 수립 구간은 큐를 안 읽어 그것이
    //   다음 운영 단계까지 남는다. 그 틈을 만들려고 큐에 직접 넣는다 — 핸들 입구는 지금 운영 중이 아니라
    //   거절하므로, 입구를 지난 직후 다시 걸린 경합(셸에선 창구 clone 을 쥐고 있다 넣는 길)을 흉내 낸다.
    #[tokio::test]
    async fn a_command_stamped_before_a_reconnect_is_not_written_on_the_next_link() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        // ★상대편을 쥐고 있는다★ — 떨어뜨리면 말 없이 사라진 통로라 읽기 오류로 끊긴다(`MemoryEndpoint::close`).
        let first = h.net.accept().expect("첫 통로");
        first.push(Frame::Text("go".into()));
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let g1 = h.peer.generation();

        h.peer.reconnect_now();
        settle_until("다음 핸드셰이크 대기", || {
            h.net.dials() == 2 && h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.drain();
        let (reply, mut woken) = oneshot::channel();
        let stale = |op| PeerCmd { generation: g1, op };
        let data = &h.peer.inner.data_tx;
        assert!(data
            .try_send(stale(PeerOp::Request {
                out: TestOut::Request {
                    tag: 5,
                    body: "stale".into(),
                },
                reply,
            }))
            .is_ok());
        assert!(data
            .try_send(stale(PeerOp::Notify {
                out: TestOut::Notice("stale".into()),
            }))
            .is_ok());
        // 팬아웃 · 이어받기도 같은 대조를 지난다 — 갈래마다 따로 빠지지 않게 함께 넣는다.
        assert!(data
            .try_send(stale(PeerOp::Raw(Frame::Text("raw|stale".into()))))
            .is_ok());
        assert!(data
            .try_send(stale(PeerOp::Resume {
                key: 7,
                after: Some(1),
            }))
            .is_ok());
        settle().await;
        assert!(
            matches!(woken.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "★틈이 실재한다★ — 연결 수립 구간은 큐를 안 읽어 낡은 명령이 다음 운영 단계까지 남는다"
        );

        let next = h.net.accept().expect("둘째 통로");
        next.push(Frame::Text("go".into()));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        settle().await;
        assert_ne!(h.peer.generation(), g1);
        let woken = woken.try_recv();
        assert!(
            matches!(
                woken,
                Ok(Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                })) if generation == g1
            ),
            "낡은 요청은 안 보냄으로 깨어난다: {woken:?}"
        );
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "★옛 세대에 묶인 명령은 다음 연결에 안 쓰인다★"
        );
        // 신고가 몇 건으로 갈리는지는 감독이 한 바퀴에 몇 개를 꺼내는지에 달려 있어 합만 본다.
        assert_eq!(
            outbound_dropped(&h.events()),
            3,
            "★부르는 쪽이 `Ok` 를 받아 간 알림 · 팬아웃 · 이어받기는 버린 만큼 센다★ — 안 세면 조용한 유실이다"
        );
    }

    // `request` 는 세대를 부른 자리에서 잡는다 — future 를 만든 뒤 다시 붙고 나서 처음 폴링해도 새 연결로 안 간다.
    #[tokio::test]
    async fn a_request_future_made_before_a_reconnect_stays_bound_to_the_old_generation() {
        let (h, first) = live().await;
        let g1 = h.peer.generation();
        let peer = h.peer.clone();
        let mut asked = Box::pin(peer.request(TestOut::Request {
            tag: 1,
            body: "early".into(),
        }));
        first.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("둘째 통로");
        assert_ne!(h.peer.generation(), g1);

        let first_poll = poll_immediate(&mut asked).await;
        assert!(
            matches!(
                first_poll,
                Some(Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                })) if generation == g1
            ),
            "★처음 폴링이 다시 붙은 뒤여도 만든 때의 세대에 묶인다★: {first_poll:?}"
        );
        settle().await;
        assert_eq!(
            texts(next.frames()),
            Vec::<String>::new(),
            "만든 때의 연결이 끝났으니 다음 연결에 안 쓰인다"
        );
    }

    // 끝난 세대에서 찬 큐는 신고만 하고 다음 연결을 끊지 않는다(`Supervisor::drain_overflow`). 「지금 다시 붙어라」는
    //   백오프 대기를 거치지 않고 곧장 걸어, 그 넘침 셈을 처음 걷는 것이 새 연결의 첫 바퀴다.
    #[tokio::test]
    async fn a_queue_that_overflowed_on_the_old_generation_does_not_cut_the_next_link() {
        let mut h = build(
            Policy {
                outbound_queue: 2,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let first = h.net.accept().expect("첫 통로");
        let g1 = h.peer.generation();

        // 감독을 막힌 쓰기 위에 세우고 큐를 채운다.
        first.stall_writes(true);
        let notice = |body: &str| TestOut::Notice(body.into());
        assert_eq!(h.peer.notify(notice("n1")), Ok(()));
        settle().await;
        assert_eq!(h.peer.notify(notice("n2")), Ok(()));
        assert_eq!(h.peer.notify(notice("n3")), Ok(()));
        assert_eq!(h.peer.notify(notice("n4")), Err(SendError::QueueFull));

        h.peer.reconnect_now();
        settle().await;
        // 통로를 버리는 닫기가 막힌 쓰기에 서 있다 — 풀어 준다.
        first.stall_writes(false);
        settle_until("재연결", || {
            h.peer.peer_state() == PeerState::Live && h.peer.generation() != g1
        })
        .await;
        let _next = h.net.accept().expect("둘째 통로");
        settle().await;

        let g2 = h.peer.generation();
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "★옛 세대의 넘침이 새 연결을 끊으면 안 된다★"
        );
        let events = h.events();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                TransportEvent::Disconnected { generation, .. } if *generation == g2
            )),
            "새 연결은 끊기지 않았다: {:?}",
            events.iter().map(|e| e.kind()).collect::<Vec<_>>()
        );
        assert_eq!(
            outbound_dropped(&events),
            4,
            "쓰다 버린 n1 · 큐에 든 채 버린 n2 · n3 · 넘친 n4 — 신고는 한다"
        );
    }

    // 셸 `CommandOutcome` 소켓 대조의 transport 판 — 받은 연결의 세대를 잡아 답을 내면 그 연결에서만 나간다.
    #[tokio::test]
    async fn notify_in_and_request_in_go_out_only_on_the_generation_they_name() {
        let (mut h, first) = live().await;
        let g1 = h.peer.generation();
        first.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        let next = h.net.accept().expect("둘째 통로");
        let g2 = h.peer.generation();
        assert_ne!(g1, g2);
        h.drain();

        assert_eq!(
            h.peer.notify_in(g1, TestOut::Notice("old".into())),
            Err(SendError::Disconnected),
            "옛 연결의 답은 입구에서 거절된다"
        );
        let old = joined(spawn_request_in(&h.peer, g1, 1)).await;
        assert!(
            matches!(
                old,
                Err(RequestError::Disconnected {
                    generation,
                    sent: false,
                    ..
                }) if generation == g1
            ),
            "{old:?}"
        );
        settle().await;
        assert_eq!(texts(next.frames()), Vec::<String>::new());
        assert!(
            !h.kinds().contains(&"Dropped"),
            "입구의 거절은 부르는 쪽이 오류로 받았으니 세지 않는다"
        );
        assert_eq!(h.peer.refused_count(), 0);

        assert_eq!(h.peer.notify_in(g2, TestOut::Notice("new".into())), Ok(()));
        let asked = spawn_request_in(&h.peer, g2, 2);
        settle().await;
        assert_eq!(texts(next.frames()), vec!["not|new", "req|2|body2"]);
        next.push(TestWire::reply(2));
        assert_eq!(
            joined(asked).await,
            Ok(TestIn::Reply {
                tag: 2,
                body: "ok".into()
            })
        );
    }

    // ── keepalive · 시한 ──
    #[tokio::test]
    async fn silence_past_the_idle_timeout_cuts_the_link() {
        let (mut h, _endpoint) = live().await;
        h.clock.advance(Duration::from_secs(50));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::Idle));
    }

    #[tokio::test]
    async fn keepalive_goes_out_on_its_own_schedule() {
        let (h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        endpoint.push(Frame::Keepalive);
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn an_inbound_keepalive_keeps_the_idle_timer_from_firing() {
        let (h, endpoint) = live().await;
        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(40));
            endpoint.push(Frame::Keepalive);
            settle().await;
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── C2: 프레임 홍수가 타이머·나가는 큐를 굶기지 않는다 ──
    // ── 부하 아래 생존 (★C2 회귀망이 아니다 — 위 `next_turn` 다섯이 그것이다★) ──
    #[tokio::test]
    async fn keepalive_still_goes_out_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(20));
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "★출력이 가장 많을 때 ping 이 멎으면 상대가 우리를 침묵으로 끊는다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_outbound_queue_still_drains_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.peer.notify(TestOut::Notice("let-me-out".into())).unwrap();
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint
                .frames()
                .contains(&Frame::Text("not|let-me-out".into())),
            "★프레임 팔이 계속 준비돼 있어도 나가는 큐가 굶으면 안 된다★"
        );
    }

    #[tokio::test]
    async fn the_request_deadline_still_fires_under_load() {
        let (h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 31);
        settle().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(8));
        pump(64).await;
        feeder.abort();
        assert!(
            matches!(joined(pending).await, Err(RequestError::TimedOut { .. })),
            "★시한이 안 돌면 요청이 상한을 넘겨 매달린다★"
        );
    }

    // ── 스트림 ──
    #[tokio::test]
    async fn a_stream_gap_is_reported_and_resumed_from_the_last_good_seq() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 1);

        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        let gap = h.events().iter().find_map(|e| match e {
            TransportEvent::StreamGap {
                expected,
                got,
                resumed_from,
                stream,
                ..
            } => Some((*stream, *expected, *got, *resumed_from)),
            _ => None,
        });
        assert_eq!(gap, Some((1, 2, 5, Some(1))));
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text("res|1|1".into())],
            "구멍을 만나면 그 지점부터 다시 청한다"
        );
    }

    // ── C3: 재요청의 답을 잘림으로 오독하지 않는다 ──
    #[tokio::test]
    async fn chunks_in_flight_before_the_resume_do_not_look_like_truncation() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // 상대가 재요청을 받기 전에 이미 띄운 조각. ★그 뒤에 울타리를 하나 세운다★ — 「없다」를
        //   `settle()` 뒤에 그냥 단언하면 아직 처리가 안 됐을 때도 통과해, 결함을 축복하는 테스트가 된다.
        endpoint.push(TestWire::chunk(1, 4, 6));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fence".into())));
        settle_until("6번 조각과 울타리가 처리될 때까지", || {
            h.seen_notice("fence")
        })
        .await;
        assert!(
            !h.kinds().contains(&"StreamTruncated"),
            "★상대가 못 준다★ 가 아니라 ★아직 안 왔다★ 다"
        );

        // 상대의 재생분이 순서대로 올라온다.
        for seq in 2..=6 {
            endpoint.push(TestWire::chunk(1, 4, seq));
        }
        settle().await;
        let seqs: Vec<u64> = h
            .messages()
            .into_iter()
            .filter_map(|m| match m {
                TestIn::Chunk { seq, .. } => Some(seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![2, 3, 4, 5, 6], "잃은 구간이 실제로 돌아온다");
    }

    #[tokio::test]
    async fn a_resume_window_closes_without_any_further_arrival() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // ★조각이 더 안 와도 창이 닫혀야 한다★ — 턴이 끝나 스트림이 조용해지는 것이 평범한 경우이고,
        //   도착에만 매달린 판정은 그때 영원히 열려 있다.
        h.clock.advance(Duration::from_secs(2));
        settle().await;
        let items = h.drain();
        let oldest = items.iter().find_map(|i| match i.as_event() {
            Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
            _ => None,
        });
        assert_eq!(oldest, Some(5), "★붙들고 있던 첫 번호가 하한이다★");
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(
            seqs,
            vec![5],
            "★신고한 하한부터 실제로 배달해야 그 값이 참이다★"
        );
    }

    #[tokio::test]
    async fn an_outstanding_resume_is_sent_again_after_a_reconnect() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);
        h.drain();

        endpoint.close();
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let reconnected = h.net.accept().expect("새 통로");
        assert_eq!(
            reconnected.frames(),
            vec![Frame::Text("res|1|1".into())],
            "★재연결 뒤 다시 청하지 않으면 그 구멍은 영영 안 메워진다★"
        );
    }

    // ── B3: 축출이 미결 재요청을 삼키지 않는다 ──
    #[tokio::test]
    async fn forgetting_a_stream_is_reported_and_gives_back_what_it_held() {
        let mut h = build(
            Policy {
                max_streams: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();

        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // 자리가 하나뿐이라 새 스트림이 옛 스트림을 밀어낸다.
        endpoint.push(TestWire::chunk(2, 4, 1));
        settle().await;
        let items = h.drain();
        let kinds: Vec<&str> = items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert!(
            kinds.contains(&"StreamForgotten"),
            "★무엇을 잊었는지 말하지 않으면 그 스트림의 장부가 조용히 처음부터 다시 시작된다★"
        );
        assert!(
            kinds.contains(&"StreamTruncated"),
            "붙들고 있던 조각이 축출과 함께 사라지면 안 된다"
        );
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { key: 1, seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![5], "붙들고 있던 5 를 돌려받는다");
    }

    #[tokio::test]
    async fn a_duplicate_chunk_never_reaches_the_consumer() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 2);
    }

    #[tokio::test]
    async fn a_generation_change_is_announced_and_restarts_the_ledger() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 100));
        settle().await;
        h.drain();
        endpoint.push(TestWire::chunk(1, 5, 1));
        settle().await;
        assert!(h.kinds().contains(&"StreamGenerationChanged"));
    }

    #[tokio::test]
    async fn a_message_claiming_both_roles_is_reported_and_treated_as_a_reply() {
        let (mut h, endpoint) = live().await;
        let waiting = spawn_request(&h.peer, 8);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Ambiguous {
            tag: 8,
            key: 1,
            generation: 1,
            seq: 1,
        }));
        settle().await;
        assert!(h.kinds().contains(&"AmbiguousRole"));
        assert!(joined(waiting).await.is_ok(), "답장으로 먼저 처리한다");
    }

    #[tokio::test]
    async fn resume_stream_writes_the_consumers_own_envelope() {
        let (h, endpoint) = live().await;
        let generation = h.peer.resume_stream(7, Some(42));
        settle().await;
        assert_eq!(generation, Generation(1));
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|7|42".into())]);
    }

    // ── M2: 운영 단계가 아닐 때의 resume 은 조용히 사라지지 않는다 ──
    #[tokio::test]
    async fn a_resume_asked_while_down_is_counted_and_reported() {
        let (mut h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        assert_ne!(h.peer.peer_state(), PeerState::Live);
        h.drain();

        h.peer.resume_stream(7, Some(1));
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n >= 1));
    }

    // ── 디코드 ──
    #[tokio::test]
    async fn a_frame_the_wire_cannot_read_is_reported_without_cutting_the_link() {
        let (mut h, endpoint) = live().await;
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_disconnect_on_decode_failure_policy_cuts_the_link() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::ReadFailed),
            "읽은 것을 못 읽어 끊었다 — 읽는 쪽의 끝이다"
        );
    }

    // ── 재연결 예산 ──
    #[tokio::test]
    async fn being_unreachable_burns_the_budget_and_then_gives_up() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle_until("첫 실패", || h.peer.peer_state() == PeerState::Backoff).await;

        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert_eq!(h.net.dials(), 4, "첫 시도 + 재시도 3회");
        let kinds = h.kinds();
        assert_eq!(kinds.iter().filter(|k| **k == "ConnectFailed").count(), 4);
        assert!(kinds.contains(&"GaveUp"));
    }

    #[tokio::test]
    async fn a_rejection_spends_no_budget_and_dials_only_once() {
        let mut h = build(Policy::default(), RejectingHandshake::new("version 3 != 4"));
        settle_until("거절", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected)
        );
        assert_eq!(h.net.dials(), 1, "붙었는데 거절 = 재시도하지 않는다");
        let events = h.events();
        let reason = events.iter().find_map(|e| match e {
            TransportEvent::Rejected { reason, .. } => Some(reason.clone()),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("version 3 != 4"));
        assert!(events.iter().any(|e| e.kind() == "GaveUp"));
        assert!(
            !events.iter().any(|e| e.kind() == "ConnectFailed"),
            "예산을 쓴 적이 없으므로 시도 실패 사건도 없다"
        );
    }

    // ── N2: 상대가 코드를 실어 닫으면 그것이 거절이다 ──
    #[tokio::test]
    async fn a_peer_that_closes_with_a_code_is_a_rejection_not_a_retry() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.reject(Close::new(CloseCode::HANDSHAKE_REJECTED, "protocol 3 != 4"));
        settle_until("거절 판정", || h.peer.peer_state().is_terminal()).await;

        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected),
            "★코드를 읽지 못하면 여기가 BudgetExhausted 가 된다★"
        );
        assert_eq!(h.net.dials(), 1, "거절은 예산을 안 쓴다");
        let reported = h.events().into_iter().find_map(|e| match e {
            TransportEvent::Rejected { code, reason, .. } => Some((code, reason)),
            _ => None,
        });
        assert_eq!(
            reported,
            Some((
                Some(CloseCode::HANDSHAKE_REJECTED),
                "protocol 3 != 4".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_peer_that_closes_without_a_code_is_treated_as_unreachable() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net.accept().unwrap().close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
    }

    #[tokio::test]
    async fn reconnect_now_refills_the_budget_and_dials_again() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(h.peer.peer_state().is_terminal());
        h.drain();

        h.net.allow();
        h.peer.reconnect_now();
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert!(h.kinds().contains(&"Connected"));
    }

    #[tokio::test]
    async fn a_burst_of_reconnect_requests_still_lets_a_dial_happen() {
        let (h, _endpoint) = live().await;
        for _ in 0..50 {
            h.peer.reconnect_now();
        }
        settle_until(
            "쌓인 신호를 접지 않으면 dial 이 폴링되기도 전에 매번 취소된다",
            || h.net.dials() >= 2,
        )
        .await;
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    #[tokio::test]
    async fn a_lost_link_comes_back_on_the_first_backoff_step() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.peer.generation(), Generation(2), "세대가 하나 올랐다");
    }

    // ── N8: 붙자마자 끊기는 상대도 결국 포기에 닿는다(팝업 자격) ──
    #[tokio::test]
    async fn a_flapping_peer_reaches_gave_up_so_the_popup_can_fire() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        let mut backoffs = [500u64, 1000, 2000].into_iter();
        for _ in 0..4 {
            settle().await;
            if h.peer.peer_state().is_terminal() {
                break;
            }
            if let Some(endpoint) = h.net.accept() {
                // 붙자마자 끊는다 — dwell 을 한 번도 못 채운다.
                endpoint.close();
            }
            settle().await;
            if let Some(delay) = backoffs.next() {
                h.clock.advance(Duration::from_millis(delay));
            }
        }
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted),
            "★여기 못 닿으면 ADR-0180 결정 7 의 팝업이 영영 안 뜬다★"
        );
    }

    #[tokio::test]
    async fn a_connection_that_holds_long_enough_gets_its_budget_back() {
        let (h, endpoint) = live().await;
        h.clock.advance(Policy::default().live_dwell);
        settle().await;
        endpoint.push(Frame::Keepalive);
        settle().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── 큐 ──
    #[tokio::test]
    async fn requests_made_while_down_fail_without_waiting() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        let out = h
            .peer
            .request(TestOut::Request {
                tag: 1,
                body: "x".into(),
            })
            .await;
        assert!(matches!(out, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            h.peer.notify(TestOut::Notice("x".into())),
            Err(SendError::Disconnected)
        );
    }

    #[tokio::test]
    async fn a_full_outbound_queue_cuts_the_link_under_the_default_policy() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);

        let mut refused = 0;
        for n in 0..8 {
            if h.peer.notify(TestOut::Notice(format!("n{n}"))).is_err() {
                refused += 1;
            }
            settle().await;
        }
        assert!(refused > 0, "유계 큐가 실제로 찼어야 한다");
        assert!(
            endpoint.frames().is_empty(),
            "막힌 동안에는 한 프레임도 안 나간다"
        );

        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::QueueFull));
    }

    #[tokio::test]
    async fn the_drop_and_report_policy_keeps_the_link_and_counts_the_loss() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                full_queue: QueueFullPolicy::DropAndReport,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);
        for n in 0..8 {
            let _ = h.peer.notify(TestOut::Notice(format!("n{n}")));
            settle().await;
        }
        assert!(endpoint.frames().is_empty());
        endpoint.stall_writes(false);
        settle().await;
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n > 0));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── N6a: 큐에서 걷어 버린 것도 센다 ──
    #[tokio::test]
    async fn commands_discarded_from_the_queue_are_reported_not_dropped_silently() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        assert!(endpoint.frames().is_empty(), "감독이 첫 쓰기에서 멈췄다");

        // 이것들은 큐에 남은 채로 버려지는 자리다 — 핸들은 이미 `Ok` 를 받아 갔다.
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★막힌 통로에서는 `drop_link` 의 닫기도 함께 걸린다★ — 그것을 풀지 않으면 백오프 대기가
        //   시작되지 않아 아래 재연결이 오지 않는다(`stall_writes` rustdoc 의 그 계산).
        endpoint.stall_writes(false);
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        // 4 = 쓰다 잘린 첫 것 1 + 큐에 남은 것 3.
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "핸들이 이미 Ok 를 받아 갔으므로 여기서 안 세면 조용한 유실이다: {dropped:?}"
        );
    }

    // ── N4: 위로 올릴 데가 닫히면 다시 붙지 않고 접는다 ──
    #[tokio::test]
    async fn a_closed_consumer_ends_the_peer_instead_of_triggering_reconnects() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let dials_before = h.net.dials();

        // 소비자가 사라진다.
        h.inbound.close();
        while h.inbound.try_recv().is_ok() {}
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));

        endpoint.push(TestWire::encode_in(&TestIn::Notice("x".into())));
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.net.dials(),
            dials_before,
            "소비자 부재는 재연결할 일이 아니다"
        );
    }

    // ── F6: 곱게 끄는 것은 거절이 아니다 ──
    #[tokio::test]
    async fn a_peer_that_shuts_down_gracefully_is_not_a_rejection() {
        for code in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "shutting down"));
            settle_until("판정", || h.peer.peer_state() != PeerState::Handshaking).await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Backoff,
                "★{code:?} 는 「나 이제 간다」이지 「너를 안 받는다」가 아니다 —                  거절로 읽으면 데몬이 곱게 종료할 때마다 재시도 없이 포기한다★"
            );
            assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
        }
    }

    // ── F5: 쓰레기는 살아있음의 증거가 아니다 ──
    #[tokio::test]
    async fn a_peer_sending_only_garbage_is_eventually_cut_by_the_idle_timer() {
        let (mut h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(25));
        for n in 0..4 {
            endpoint.push(Frame::Text(format!("garbage{n}")));
        }
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));

        // 연결 시점부터 idle_timeout(50s)이 지났다. 쓰레기가 시계를 되감았다면 안 끊긴다.
        h.clock.advance(Duration::from_secs(26));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "★못 읽을 것만 보내는 상대를 끊는 밸브가 이것뿐이다★"
        );
    }

    // ── F4: 내려가 있는 동안에도 유실을 신고한다 ──
    #[tokio::test]
    async fn losses_are_reported_while_the_peer_is_still_down() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★막힌 통로에서는 `drop_link` 의 닫기도 함께 걸린다★ — 유실 신고는 그 다음 단계(백오프 대기)에서
        //   나오므로, 풀지 않으면 아래 신고가 아직 안 나온 채로 단언을 만난다. 시계로 자르지 않는 이유는
        //   그러면 백오프까지 지나가 「아직 안 붙었다」가 깨지기 때문이다.
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Backoff,
            "아직 다시 붙지 않은 상태여야 이 테스트가 뜻을 갖는다"
        );
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        // 4 = 쓰다 잘린 첫 것 1 + 큐에 남은 것 3.
        assert!(
            matches!(dropped, Some((Direction::Outbound, 4))),
            "★다시 붙을 때까지 기다리면 사용자가 「다시 연결」을 누를 때까지 아무 데도 안 나온다★: {dropped:?}"
        );
    }

    // ── 3-0 U1d: 큐 포화 셈도 잠든 감독을 깨운다 ──
    //
    // 핸들이 낡은 `Live` 를 보고 넣다 찼는데, 그 셈이 감독이 큐를 다 비우고 포기 대기에 잠든 뒤에 닿은 경합의 끝
    //   모양이다 — 큐는 비었고 셈만 남았다. 안 깨우면 다음 제어까지 신고가 안 나온다.
    #[tokio::test]
    async fn a_queue_full_count_wakes_a_parked_supervisor() {
        let mut h = build(Policy::default(), RejectingHandshake::new("nope"));
        settle_until("포기", || {
            matches!(h.peer.peer_state(), PeerState::GaveUp(_))
        })
        .await;
        h.drain();

        h.peer.count_overflow(h.peer.generation());
        settle().await;
        assert_eq!(
            outbound_dropped(&h.events()),
            1,
            "★잠든 감독이 스스로 깨어 큐 포화 셈을 신고한다★"
        );
    }

    // ── 쓰는 도중 끊긴 답장 없는 프레임도 센다 ──
    //
    // ★핸들은 큐에 넣을 때 이미 `Ok` 를 받아 갔다★ — 쓰기가 `Ok` 로 안 끝난 알림·팬아웃 프레임을 안 세면
    //   그대로 조용한 유실이다(`fail_pending` 의 그 규칙). 끊는 사유(제어 둘 · 통로 오류 · 쓰기 시한)마다
    //   재고, 세면 안 되는 둘(요청 · keepalive)도 함께 잰다.

    /// 나가는 쪽 유실 신고의 합 — 신고가 여러 번에 나뉘어 올라와도 합한다.
    fn outbound_dropped(events: &[TransportEvent<TestWire>]) -> u64 {
        events
            .iter()
            .map(|e| match e {
                TransportEvent::Dropped {
                    direction: Direction::Outbound,
                    count,
                    ..
                } => *count,
                _ => 0,
            })
            .sum()
    }

    type Push = fn(&Peer<TestWire>) -> Result<(), SendError>;

    /// 답장 없는 프레임의 두 입구 — 알림과 팬아웃.
    fn reply_less_pushes() -> [(&'static str, Push); 2] {
        [
            ("알림", |p| p.notify(TestOut::Notice("in flight".into()))),
            ("팬아웃", |p| {
                p.send_frame(Frame::Text("in flight".into()))
            }),
        ]
    }

    /// 쓰기를 막고 하나를 밀어 넣어 감독을 그 쓰기 위에 세운다. 올라와 있던 사건은 비운다.
    async fn park_in_flight(h: &mut Harness, endpoint: &MemoryEndpoint, push: Push) {
        endpoint.stall_writes(true);
        push(&h.peer).unwrap();
        settle().await;
        assert!(h.queue_drained(), "감독이 그것을 큐에서 꺼냈어야 한다");
        assert!(
            endpoint.drain().is_empty(),
            "감독이 그 쓰기 위에 서 있어야 한다"
        );
        h.drain();
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_close_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.close();
            settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
            // 걸린 goodbye 를 푼다 — 안 풀면 `teardown` 의 신고까지 못 간다.
            endpoint.stall_writes(false);
            settle_until("감독이 끝났다", || h.supervisor_done()).await;
            let events = h.events();
            assert_eq!(outbound_dropped(&events), 1, "{what}");
            let tail: Vec<&str> = events.iter().rev().take(2).map(|e| e.kind()).collect();
            assert_eq!(
                tail,
                vec!["Closed", "Dropped"],
                "{what}: 정리의 유실 신고도 Closed 앞이다"
            );
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_mid_write_by_reconnect_now_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.peer.reconnect_now();
            // ★제어가 이긴 뒤에 푼다★ — 먼저 풀면 걸린 쓰기가 제어와 함께 준비돼 그대로 나갈 수 있다.
            settle_until("제어가 이겼다", || {
                h.peer.peer_state() == PeerState::Dialing
            })
            .await;
            endpoint.stall_writes(false);
            settle_until("다시 붙었다", || {
                h.net.dials() == 2 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            settle().await;
            assert_eq!(outbound_dropped(&h.events()), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_whose_write_fails_on_a_dead_link_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            // ★쓰는 쪽만 끊고 멈춤을 풀면 걸린 쓰기가 통로 오류로 끝난다★ — 받을 쪽이 닫혀 실패한다
            //   (`testing::MemTx`). `fail` 로는 이 갈래가 안 선다: 그것은 읽는 쪽에만 닿고, 쓰는 동안 감독은 읽는
            //   쪽을 안 본다. 엔드포인트를 떨어뜨리는 것도 아니다 — 읽는 쪽이 함께 끊겨, 그 끝이 앞잡이 줄에 먼저
            //   들면 끊김 원인이 읽기 끝으로 나온다(`salvage_read_ahead`).
            endpoint.fail_writes();
            endpoint.stall_writes(false);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            settle().await;
            let events = h.events();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    TransportEvent::Disconnected {
                        cause: DisconnectCause::WriteFailed,
                        ..
                    }
                )),
                "{what}: 통로 오류 갈래를 재야 한다"
            );
            assert_eq!(outbound_dropped(&events), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_reply_less_frame_cut_by_the_write_deadline_is_counted() {
        for (what, push) in reply_less_pushes() {
            let (mut h, endpoint) = live().await;
            park_in_flight(&mut h, &endpoint, push).await;
            h.clock.advance(Policy::default().write_deadline);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            // 걸린 goodbye 를 푼다 — 유실 신고는 그 다음 단계(백오프 대기)에서 나온다.
            endpoint.stall_writes(false);
            settle().await;
            let events = h.events();
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    TransportEvent::Disconnected {
                        cause: DisconnectCause::WriteDeadline,
                        ..
                    }
                )),
                "{what}: 쓰기 시한 갈래를 재야 한다"
            );
            assert_eq!(outbound_dropped(&events), 1, "{what}");
        }
    }

    #[tokio::test]
    async fn a_request_cut_mid_write_is_woken_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        // ★별 태스크로 띄우지 않고 이 자리에서 한 번 폴링한다★ — 띄우면 그 태스크가 아직 안 돈 순간
        //   아래 「큐에서 꺼냈다」가 빈 큐로 참이 된다. 첫 폴링이 큐에 넣는 데까지를 동기로 한다.
        let peer = h.peer.clone();
        let mut waiting = Box::pin(peer.request(TestOut::Request {
            tag: 1,
            body: "x".into(),
        }));
        assert!(poll_immediate(&mut waiting).await.is_none());
        assert!(!h.queue_drained(), "요청이 큐에 들어가 있어야 한다");
        settle().await;
        assert!(h.queue_drained(), "감독이 그것을 큐에서 꺼냈어야 한다");
        // ★아직 안 깨어났어야 「쓰는 중」이 선다★ — 쓰기 전에 거절된 요청도 아래에서 같은 오류로
        //   깨어나므로, 이것 없이는 마지막 단언이 쓰기와 무관하게 맞는다.
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "쓰기 전에 끝났다"
        );
        assert!(
            endpoint.drain().is_empty(),
            "감독이 그 쓰기 위에 서 있어야 한다"
        );
        h.drain();
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.supervisor_done()).await;
        let woken = poll_immediate(&mut waiting)
            .await
            .expect("감독이 끝났는데 요청이 안 깨어났다");
        assert!(matches!(woken, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "요청은 오류로 깨어나므로 유실로 또 세지 않는다"
        );
    }

    #[tokio::test]
    async fn a_keepalive_cut_mid_write_is_not_counted() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "ping 이 걸려 있어야 한다");
        h.drain();
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.supervisor_done()).await;
        assert_eq!(
            outbound_dropped(&h.events()),
            0,
            "keepalive 는 소비자가 넣은 프레임이 아니다"
        );
    }

    // ── 종료 ──
    #[tokio::test]
    async fn closing_is_final() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        h.peer.reconnect_now();
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Closed);
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_write_that_never_completes_is_cut_by_the_write_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        assert!(endpoint.frames().is_empty(), "실제로 막혀 있어야 한다");
        h.clock.advance(Duration::from_secs(5));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::WriteDeadline));
    }

    // ── M4: 쓰기 도중에도 제어를 듣는다 ──
    //
    // ★발행된 상태가 `Closed` 인 것으로는 감독이 접혔다는 뜻이 안 된다★ — `run` 이 행동을 **적용하기
    //   전에** 상태를 발행하므로(그 루프의 두 `publish` 사이), 아래 `settle_until` 은 정리가 아직 걸린
    //   닫기 위에 멈춰 있는 동안에도 통과한다. 그래서 멈춤을 풀고 **그 뒤까지** 잰다.
    #[tokio::test]
    async fn close_is_heard_while_a_write_is_parked() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;

        endpoint.stall_writes(false);
        let mut seen = Vec::new();
        settle_until("걸려 있던 goodbye 가 나갔다", || {
            seen.extend(endpoint.drain());
            seen.iter()
                .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
        })
        .await;
        // ★이 단언은 하네스의 성질이지 [`LinkTx::send`] 계약이 아니다★ — `testing::MemTx` 는 막힌 쓰기를
        //   넘기기 전에 세우므로 잘리면 아무것도 안 남는다. WS 어댑터에서는 잘린 프레임이 goodbye 앞에
        //   실려 나갈 수 있다(`ws.rs` 「통로 성질」).
        assert!(
            !seen.iter().any(|m| matches!(m, ClientMsg::Frame(_))),
            "취소된 쓰기가 뒤늦게 나갔다: {seen:?}"
        );
    }

    // ── 종료: goodbye 코드가 **실제로** 통로로 나간다 ──
    //
    // ★마지막 핸들까지 놓는 모양인 것은 의도★ — `Registry::remove` 가 그 모양이고(닫으라 한 뒤 핸들을
    //   버린다), 그 상태에서 제어 채널은 곧바로 `Ready(None)` 을 낸다. `drop_link` 에 제어 팔이 있던
    //   판에서는 `select!` 의 난수 팔 선택이 그 `None` 을 승자로 쳐 닫기 future 가 폴링되기도 전에
    //   버려졌다 — 실측으로 약 절반이었다(2026-09-07).
    // ★한 바퀴로 재지 말 것★ — 그래서 이 그물은 한 번에 절반만 잡는다. 아래 반복이 그것을 결정적으로
    //   만든다.
    #[tokio::test]
    async fn closing_speaks_the_goodbye_code_on_the_wire() {
        for round in 0..24 {
            let (h, endpoint) = live().await;
            h.peer.close();
            drop(h);
            let what = format!("{round}번째 바퀴: goodbye 가 통로로 나갔다");
            let mut seen = Vec::new();
            settle_until(&what, || {
                seen.extend(endpoint.drain());
                seen.iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
            })
            .await;
        }
    }

    // ── 걸린 닫기는 제어가 아니라 **시한**이 자른다 ──
    //
    // ★쓰기가 이미 막힌 통로에서 닫기도 함께 막힌다★ — 하네스의 `stall_writes` 가 `send`·`ping`·`close`
    //   셋 다 막으므로(`testing::MemoryEndpoint::stall_writes`) 실소켓의 그 모양이 여기서 재진다.
    // ★시계를 미는 줄을 빼지 말 것★ — `drop_link` 가 더는 제어로 그 대기를 자르지 않으므로(그 자리
    //   rustdoc 의 「알려진 한계」), 빼면 감독이 걸린 닫기 위에 선 채로 `settle_until` 이 한도에서 죽는다.
    #[tokio::test]
    async fn a_parked_closing_frame_is_cut_by_the_write_deadline_not_by_control() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        endpoint.fail("link died");
        settle_until("통로가 죽어 백오프로 내려갔다", || {
            h.peer.peer_state() == PeerState::Backoff
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "★닫기가 실제로 걸려 있어야 이 테스트가 무언가를 잰다★ — 나갔으면 아래 단언은 공짜다"
        );
        h.peer.close();
        // ★제목의 「제어가 아니라」를 재는 것은 이 단언이다★ — 아래 단언들은 제어 팔이 걸린 닫기를
        //   자르던 판에서도 초록이다. 시계를 밀기 전에는 닫기 요청만으로 접히면 안 된다.
        settle().await;
        assert_ne!(
            h.peer.peer_state(),
            PeerState::Closed,
            "시계를 밀기 전에 접혔다 — 걸린 닫기를 제어가 잘랐다"
        );
        // 걸린 닫기를 자르는 것은 이 줄이다. 제어는 큐에 남아 그 다음에 읽힌다.
        h.clock.advance(Duration::from_secs(5));
        settle_until("걸린 닫기 뒤에도 제어가 살아 있다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "혼잡해서 시한에 잘리면 닫기 코드는 못 나간다 — `lib.rs` 가 인정한 그 한계다"
        );
    }

    // ── F1 회귀: 신호가 큐에 남아 있어도 goodbye 를 잃지 않는다 ──
    //
    // ★`reconnect_now()` 를 **두 번** 부르는 것이 요점이다★ — 감독은 한 바퀴에 신호를 하나만 집으므로
    //   둘째가 큐에 남고, 그 뒤 `drop_link` 가 제어 팔을 걸면 그 팔이 곧바로 `Ready` 라 닫기 future 가
    //   폴링조차 안 된 채 버려진다. 혼잡은 하나도 없다 — 실측 2026-09-07 로 100회 중 41·54·49·49회가
    //   그렇게 goodbye 를 잃었고, 고친 뒤 0회다.
    // ★한 바퀴로 재지 말 것★ — 회귀가 되살아나도 절반은 통과하므로, 세 번째 재발인 이 결함의 그물이
    //   동전 던지기가 된다. 아래 반복이 그것을 결정적으로 만든다.
    #[tokio::test]
    async fn a_queued_control_signal_does_not_swallow_the_goodbye_code() {
        let (mut h, mut endpoint) = live().await;
        for round in 0..24 {
            let dials = h.net.dials();
            h.peer.reconnect_now();
            h.peer.reconnect_now();
            settle_until("다시 붙었다", || {
                h.net.dials() == dials + 1 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            assert!(
                endpoint
                    .drain()
                    .iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY)),
                "{round}번째 바퀴에서 goodbye 가 통로로 안 나갔다"
            );
            h.drain();
            endpoint = h.net.accept().expect("새 통로");
        }
    }

    // ── 3-0 U1b: 순서 칸 ──
    //
    // ★줄 길이 1 로 세운다★ — 첫 `Connected` 하나로 줄이 차서, 그 뒤 올라오는 것은 전부 감독의 순서 칸을
    //   지난다. 줄에 자리가 있으면 순서 칸은 넣는 그 자리에서 비므로 아무것도 재지 않는다.

    /// 감독을 돌리며 받는 쪽을 비운다 — 한 바퀴에 아무것도 안 올라올 때까지.
    async fn drain_until_quiet(h: &mut Harness) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        loop {
            settle().await;
            let batch = h.drain();
            if batch.is_empty() {
                return out;
            }
            out.extend(batch);
        }
    }

    fn label(item: &Incoming<TestWire>) -> String {
        match item {
            Incoming::Message(arrival) => match &arrival.msg {
                TestIn::Notice(body) => format!("msg:{body}"),
                other => format!("msg:{other:?}"),
            },
            Incoming::Event(TransportEvent::Connected { generation, .. }) => {
                format!("Connected:{}", generation.0)
            }
            Incoming::Event(TransportEvent::Disconnected {
                generation, cause, ..
            }) => format!("Disconnected:{}:{cause:?}", generation.0),
            Incoming::Event(TransportEvent::Closed {
                generation, cause, ..
            }) => format!("Closed:{}:{cause:?}", generation.0),
            Incoming::Event(other) => other.kind().to_string(),
        }
    }

    fn labels(items: &[Incoming<TestWire>]) -> Vec<String> {
        items.iter().map(label).collect()
    }

    async fn live_on_a_full_line() -> (Harness, Arc<MemoryEndpoint>) {
        let h = build_with(Policy::default(), Arc::new(ImmediateHandshake), 1);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        (h, endpoint)
    }

    #[tokio::test]
    async fn lifecycle_events_are_never_lost_while_the_line_is_full() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:PeerClosed",
                "ConnectFailed",
                "ConnectFailed",
                "ConnectFailed",
                "GaveUp",
            ],
            "★받는 쪽이 밀려 있어도 생애 사건은 하나도 안 버려지고 낸 순서대로 온다★"
        );
    }

    #[tokio::test]
    async fn a_message_then_an_immediate_close_arrives_message_first_then_the_disconnect() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("last words".into())));
        endpoint.close();
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "★닫힘은 붙든 말 뒤에 줄 서 있다★ — 받는 쪽이 그 말을 꺼내기 전에는 감독이 닫힘을 읽지 않는다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:last words", "Disconnected:1:PeerClosed"],
            "★상대가 보내고 곧바로 닫아도 그 말은 안 버려지고 끊김보다 먼저 온다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Backoff);
    }

    #[tokio::test]
    async fn held_items_keep_flowing_while_the_supervisor_is_mid_handshake() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // 상대의 닫힘은 붙든 n0 뒤에 줄 서 안 읽히므로(배압) 제어로 다시 걸게 한다.
        h.peer.reconnect_now();
        settle_until("둘째 핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let _second = h.net.accept().unwrap();

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:n0", "Disconnected:1:Explicit"],
            "★핸드셰이크가 상대를 기다리는 동안에도 받는 쪽이 비우는 대로 붙든 것이 실린다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Handshaking);
        assert_eq!(
            h.net.dials(),
            2,
            "싣는 동안 걸던 통로를 버리고 다시 걸지 않았다"
        );
    }

    #[tokio::test]
    async fn closing_hands_over_everything_held_and_ends_with_closed() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        for n in 0..3 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            h.peer.held_bytes(),
            3 * "not|n0".len(),
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에 있다 — 순서 칸이 붙든 동안 프레임 팔을 뺐다"
        );
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ],
            "★붙든 것을 다 실은 뒤 Closed 가 맨 끝이다★ — 앞잡이 줄의 n1 · n2 도 통로를 버리기 전에 건져 \
             끊김 앞에 실린다"
        );
        assert_eq!(h.peer.held_bytes(), 0, "꺼낸 몫은 셈에서 빠졌다");
        assert!(
            matches!(
                h.inbound.try_recv(),
                Err(mpsc::error::TryRecvError::Disconnected)
            ),
            "Closed 뒤에는 감독이 넣는 끝을 놓는다"
        );
    }

    // ★정리 안의 순서(대기자 깨움 · 큐 닫기 → 순서 칸 비우기)를 잰다★. 접힘은 통로를 버리기 전에 핸들에
    //   보이고(`run` 이 `apply` 앞에서 낸다) 기계의 `FailPending` 이 그보다 먼저 대기 슬롯과 큐를 비우므로,
    //   정리의 `fail_pending` 에 걸리는 요청은 핸들의 입구 검사(묶은 세대가 운영 중인가)를 지난 직후 접힌 경합에서만
    //   생긴다. 그 요청을 큐에 직접 넣어 만들고, 쓰기를 막아 감독을 기계의 `DropLink` 위에 세운 채 끼운다.
    #[tokio::test]
    async fn requests_end_at_once_while_the_teardown_waits_on_the_held_items() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        assert!(
            matches!(endpoint.take(), Some(ClientMsg::Frame(_))),
            "요청이 나가 답을 기다리는 중이어야 한다"
        );
        endpoint.stall_writes(true);
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert!(
            matches!(joined(asked).await, Err(RequestError::Disconnected { .. })),
            "★받는 쪽이 안 비워도 대기 요청은 곧바로 깨어난다★"
        );

        let (reply, mut raced) = oneshot::channel();
        h.peer
            .inner
            .data_tx
            .try_send(PeerCmd {
                generation: h.peer.generation(),
                op: PeerOp::Request {
                    out: TestOut::Request {
                        tag: 9,
                        body: "raced".into(),
                    },
                    reply,
                },
            })
            .unwrap_or_else(|_| panic!("나가는 큐에 자리가 있어야 한다"));
        settle().await;
        assert!(
            matches!(raced.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "감독이 막힌 닫기 위에 서 있어야 한다 — 이 요청은 정리의 몫이다"
        );
        endpoint.stall_writes(false);
        settle().await;
        assert!(h.supervisor_done(), "정리가 나가는 큐를 닫았다");
        assert!(
            matches!(raced.try_recv(), Ok(Err(RequestError::Disconnected { .. }))),
            "★정리에 걸린 요청은 순서 칸 비우기를 기다리지 않고 곧바로 끝난다★"
        );
        assert!(
            matches!(
                joined(spawn_request(&h.peer, 8)).await,
                Err(RequestError::Disconnected { .. })
            ),
            "정리 중에 낸 요청도 곧바로 끝난다"
        );
        assert!(
            h.peer.notify(TestOut::Notice("late".into())).is_err(),
            "정리 중에 든 알림은 곧바로 거절된다"
        );
        assert!(
            !h.inbound.is_closed(),
            "정리가 아직 순서 칸에서 기다린다 — 감독이 넣는 끝을 놓지 않았다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:held",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit",
            ]
        );
    }

    #[tokio::test]
    async fn the_teardown_gives_up_on_a_line_nobody_drains_only_at_the_limit() {
        for (waited, expected) in [
            (
                CLOSE_DRAIN_LIMIT - Duration::from_secs(1),
                vec![
                    "Connected:1",
                    "msg:held",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit",
                ],
            ),
            (CLOSE_DRAIN_LIMIT, vec!["Connected:1"]),
        ] {
            let (mut h, endpoint) = live_on_a_full_line().await;
            endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
            settle().await;
            h.peer.close();
            settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
            h.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut h).await;
            assert_eq!(labels(&items), expected, "{waited:?} 기다린 뒤");
            assert!(
                matches!(
                    h.inbound.try_recv(),
                    Err(mpsc::error::TryRecvError::Disconnected)
                ),
                "{waited:?}: 감독이 끝났다"
            );
        }
    }

    #[tokio::test]
    async fn a_teardown_ends_at_once_when_the_consumer_is_gone() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.clock.sleepers(), 1, "정리가 상한 시계를 걸고 기다린다");

        drop_the_consumer(&mut h);
        settle_until(
            "받는 쪽이 사라지면 정리가 기다리지 않는다",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(h.clock.sleepers(), 0, "상한 시계를 놓았다");
        assert!(h.exited_cleanly().await, "감독이 패닉 없이 끝났다");
    }

    fn drop_the_consumer(h: &mut Harness) {
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));
    }

    // ── 3-0 U1b: 받는 쪽이 사라지면 어느 대기 루프에서든 시계를 기다리지 않고 접는다 ──
    //
    // ★시계를 밀지 않는다★ — 밀면 백오프가 끝나거나 시한이 나 대기 루프가 그 판정 없이도 빠져나온다.

    #[tokio::test]
    async fn a_backing_off_peer_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        // 붙드는 것은 끊김 사건이다 — 그 앞에 알림을 두면 닫힘이 그 뒤에 줄 서 안 읽힌다(배압).
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_peer_that_gave_up_holding_items_folds_when_the_consumer_goes() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        h.net.fail_forever();
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_holding_items_folds_when_the_consumer_goes() {
        let mut h = build_with(Policy::default(), Arc::new(ListeningHandshake), 1);
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let first = h.net.accept().unwrap();
        first.push(Frame::Text("welcome".into()));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        first.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        settle().await;
        // `held_items_keep_flowing_while_the_supervisor_is_mid_handshake` 의 같은 줄 — 제어로 다시 걸게 한다.
        h.peer.reconnect_now();
        settle_until("둘째 핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(
            h.inbound.capacity(),
            0,
            "줄이 차 있어 순서 칸이 붙들고 있다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.peer.generation(),
            Generation(1),
            "둘째 세대를 뽑지 않았다"
        );
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: 붙든 것이 없어도 받는 쪽이 사라지면 그 자리에서 접는다 ──
    //
    // 위 묶음은 순서 칸이 붙들고 있어 비우기 팔이 그 사라짐을 본다. 여기는 붙든 것이 없어 대기 루프마다 둔 수신단
    // 닫힘 팔(`Sender::closed`)만이 본다 — 그 팔이 없으면 백오프 · 포기 · 핸드셰이크 · 조용한 운영이 시계나 다음
    // 사건까지 그대로 머문다. 시계는 여기서도 밀지 않는다.

    #[tokio::test]
    async fn a_waiting_peer_with_nothing_held_folds_when_the_consumer_goes() {
        for gave_up in [false, true] {
            let (mut h, endpoint) = live().await;
            if gave_up {
                h.net.fail_forever();
            }
            endpoint.close();
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            if gave_up {
                for delay in [500u64, 1000, 2000] {
                    h.clock.advance(Duration::from_millis(delay));
                    settle().await;
                }
                assert!(matches!(h.peer.peer_state(), PeerState::GaveUp(_)));
            }
            assert!(
                h.inbound.capacity() > 0,
                "gave_up={gave_up}: 줄에 자리가 있다 — 순서 칸은 붙든 것이 없다"
            );
            let dials = h.net.dials();
            drop_the_consumer(&mut h);
            settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
            assert_eq!(h.net.dials(), dials, "gave_up={gave_up}: 다시 걸지 않았다");
            settle_until("감독이 끝났다", || h.task.is_finished()).await;
            assert!(h.exited_cleanly().await, "gave_up={gave_up}");
        }
    }

    #[tokio::test]
    async fn a_mid_handshake_peer_with_nothing_held_folds_when_the_consumer_goes() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert_eq!(h.peer.generation(), Generation(0), "세대를 뽑지 않았다");
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_silent_live_peer_folds_without_another_frame_when_the_consumer_goes() {
        let (mut h, endpoint) = live().await;
        drop_the_consumer(&mut h);
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        settle_until(
            "감독이 끝났다 — 상한을 기다리지 않았다",
            || h.task.is_finished(),
        )
        .await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★상대는 조용했고 우리가 보낸 것은 닫기뿐이다★"
        );
        assert_eq!(h.net.dials(), 1, "다시 걸지 않았다");
        assert!(h.exited_cleanly().await);
    }

    // 받는 쪽이 사라진 것과 「지금 다시 붙어라」가 함께 준비돼도 — 어느 팔이 먼저 깨든 — 다시 걸지 않는다.
    #[tokio::test]
    async fn a_reconnect_racing_the_consumers_exit_does_not_dial() {
        let (mut h, _endpoint) = live().await;
        drop_the_consumer(&mut h);
        h.peer.reconnect_now();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(h.net.dials(), 1, "아무도 안 들을 데몬에 다시 걸지 않았다");
        assert_eq!(h.peer.generation(), Generation(1));
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1d: 끝 원인 — 읽기 끝과 쓰기 끝을 가른다 ──
    //
    // 데몬 어댑터가 이 값으로 읽기 끝(남은 명령을 마저 돌린다)과 쓰기 끝(곧바로 끊는다)을 가른다(TRD 3 §3-0 ①).
    // 엔드포인트를 떨어뜨리지 않는다 — 두 쪽이 함께 끊겨 어느 쪽이 먼저 알려지는지가 스케줄에 달린다.

    type Cut = fn(&Harness, &MemoryEndpoint);
    type ReadEnd = fn(&MemoryEndpoint);
    type AppAct = fn(&Harness);

    #[tokio::test]
    async fn a_read_failure_and_a_write_failure_arrive_as_different_causes() {
        let cases: [(&str, Cut, DisconnectCause); 3] = [
            (
                "읽기 오류",
                |_, e| e.fail("boom"),
                DisconnectCause::ReadFailed,
            ),
            (
                "알림 쓰기 실패",
                |h, e| {
                    e.fail_writes();
                    h.peer.notify(TestOut::Notice("x".into())).unwrap();
                },
                DisconnectCause::WriteFailed,
            ),
            (
                "keepalive 쓰기 실패",
                |h, e| {
                    e.fail_writes();
                    h.clock.advance(Policy::default().ping_interval);
                },
                DisconnectCause::WriteFailed,
            ),
        ];
        for (what, cut, expected) in cases {
            let (mut h, endpoint) = live().await;
            cut(&h, &endpoint);
            settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
            assert_eq!(h.disconnect_cause(), Some(expected), "{what}");
        }
    }

    // ── 3-0 U1d: 통로를 버릴 때 앞잡이 줄에 이미 와 있던 읽기 끝이 끊김 원인이 된다 ──
    //
    // 줄 길이 1 — 첫 알림이 순서 칸에 붙들려 운영 루프가 프레임 팔을 뺀다(배압). 그 뒤 온 상대의 닫힘 · 읽기
    // 오류는 앞잡이 줄에 든 채 안 읽히고, 감독은 자기 까닭(쓰기 실패 · 쓰기 시한 · 넘침 …)으로 먼저 떠난다.

    /// 읽기 끝 하나를 앞잡이 줄에 넣어 둔 채 감독을 운영 단계에 세운다.
    async fn a_read_end_behind_a_held_line(
        policy: Policy,
        end: ReadEnd,
    ) -> (Harness, Arc<MemoryEndpoint>) {
        let h = build_with(policy, Arc::new(ImmediateHandshake), 1);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        end(&endpoint);
        settle().await;
        assert_eq!(endpoint.unread(), 0, "그 끝은 앞잡이 줄에 들었다");
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "프레임 팔을 뺀 감독은 그 끝을 아직 못 읽었다"
        );
        (h, endpoint)
    }

    fn disconnects(items: &[Incoming<TestWire>]) -> Vec<String> {
        labels(items)
            .into_iter()
            .filter(|label| label.starts_with("Disconnected"))
            .collect()
    }

    /// 감독이 운영 단계를 떠나는 우리 쪽 까닭 하나 — 정책과, 사이사이 감독을 돌리며 밟는 걸음들.
    type Leave = (&'static str, fn() -> Policy, &'static [Cut]);

    const LEAVES: [Leave; 4] = [
        (
            "알림 쓰기 실패",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.peer.notify(TestOut::Notice("x".into())).unwrap();
            }],
        ),
        (
            "keepalive 쓰기 실패",
            Policy::default,
            &[|h, e| {
                e.fail_writes();
                h.clock.advance(Policy::default().ping_interval);
            }],
        ),
        // 걸린 쓰기를 시한이 자른 뒤 멈춤을 푼다 — 통로를 버리는 닫기가 나가야 백오프에 든다.
        (
            "쓰기 시한",
            Policy::default,
            &[
                |h, e| {
                    e.stall_writes(true);
                    h.peer.notify(TestOut::Notice("x".into())).unwrap();
                },
                |h, _| h.clock.advance(Policy::default().write_deadline),
                |_, e| e.stall_writes(false),
            ],
        ),
        // 나가는 큐 한 칸 — 둘째 알림이 못 들어 세어지고, 감독이 첫째를 쓴 뒤 바퀴 첫머리에서 끊는다(기본 정책).
        (
            "나가는 큐 넘침",
            || Policy {
                outbound_queue: 1,
                ..Policy::default()
            },
            &[|h, _| {
                h.peer.notify(TestOut::Notice("x".into())).unwrap();
                let _ = h.peer.notify(TestOut::Notice("y".into()));
            }],
        ),
    ];

    #[tokio::test]
    async fn a_read_end_already_read_ahead_outranks_our_own_reason_for_leaving() {
        let ends: [(&str, ReadEnd, &str); 2] = [
            ("상대의 닫힘", |e| e.close(), "Disconnected:1:PeerClosed"),
            ("읽기 오류", |e| e.fail("boom"), "Disconnected:1:ReadFailed"),
        ];
        for (end_what, end, expected) in ends {
            for (leave_what, policy, steps) in LEAVES {
                let (mut h, endpoint) = a_read_end_behind_a_held_line(policy(), end).await;
                for step in steps {
                    step(&h, &endpoint);
                    settle().await;
                }
                settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
                let items = drain_until_quiet(&mut h).await;
                assert_eq!(
                    disconnects(&items),
                    vec![expected.to_string()],
                    "{end_what} 뒤 {leave_what}: ★먼저 난 읽기 끝이 원인이다★"
                );
            }
        }
    }

    // 앱이 닫았거나 다시 걸라고 했으면 그 까닭(`Explicit`)이 남는다 — 부른 쪽이 안다.
    #[tokio::test]
    async fn an_app_close_or_reconnect_stays_explicit_with_a_read_end_read_ahead() {
        let acts: [(&str, AppAct); 2] = [
            ("닫기", |h| h.peer.close()),
            ("다시 걸기", |h| h.peer.reconnect_now()),
        ];
        for (what, act) in acts {
            let (mut h, _endpoint) =
                a_read_end_behind_a_held_line(Policy::default(), |e| e.close()).await;
            act(&h);
            settle_until("접었거나 다시 걸었다", || {
                h.peer.peer_state() == PeerState::Closed || h.net.dials() == 2
            })
            .await;
            let items = drain_until_quiet(&mut h).await;
            assert_eq!(
                disconnects(&items),
                vec!["Disconnected:1:Explicit"],
                "{what}"
            );
        }
    }

    // 우리 까닭이 이미 읽는 쪽의 끝이면(디코드 실패로 끊음) 그대로다 — 줄 안에서 그 한 장이 뒤따른 닫힘보다 앞이다.
    #[tokio::test]
    async fn a_decode_failure_cut_keeps_its_cause_over_a_close_read_behind_it() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert_eq!(endpoint.unread(), 0, "닫힘도 읽혀 앞잡이 줄에 들었다");
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::ReadFailed));
    }

    // 통로를 버리며 건진 줄 안에서도 같다 — 디코드 실패로 끊는 한 장이 뒤따른 닫힘보다 앞이라 그 원인이 남는다.
    //   그 뒤에 이미 읽은 것은 그래도 넘긴다(`salvaging_carries_on_past_a_frame_it_cannot_decode`).
    #[tokio::test]
    async fn a_decode_cut_salvaged_from_the_read_ahead_outranks_a_close_behind_it() {
        let (mut h, endpoint) = a_read_end_behind_a_held_line(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            |e| {
                e.push(Frame::Text("garbage".into()));
                e.push(TestWire::encode_in(&TestIn::Notice("after".into())));
                e.close();
            },
        )
        .await;
        endpoint.fail_writes();
        h.peer.notify(TestOut::Notice("x".into())).unwrap();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        let items = drain_until_quiet(&mut h).await;
        let seen = labels(&items);
        assert!(
            seen.contains(&"msg:after".to_string()),
            "끊는 한 장 뒤에 이미 읽은 것도 넘겼다: {seen:?}"
        );
        assert_eq!(disconnects(&items), vec!["Disconnected:1:ReadFailed"]);
    }

    // ── 3-0 U1d: 쓰는 도중에 받는 쪽이 사라져도 그 쓰기를 기다리지 않는다 ──
    //
    // 큐를 잇달아 비우는 자리(`Turn::Outbound`)에서 걸린 쓰기 하나 · 걸린 keepalive 하나 — 둘 다 시계를 밀지 않고
    // 접혀야 한다. 쓰기 팔이 없으면 걸린 쓰기마다 `write_deadline` 까지 선다.

    #[tokio::test]
    async fn a_consumer_leaving_mid_batch_folds_without_waiting_out_the_stalled_write() {
        let mut h = build(
            Policy {
                outbound_queue: 4,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        // keepalive 위에 세워 두고 큐를 반 넘게 채운 뒤, 쓰기를 막고 keepalive 를 푼다 — 다음 바퀴가 큐를 잇달아
        //   비우다 첫 쓰기에서 선다.
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        for n in 0..3 {
            h.peer
                .notify(TestOut::Notice(format!("n{n}")))
                .expect("나가는 큐");
        }
        endpoint.stall_writes(true);
        endpoint.stall_pings(false);
        settle().await;
        assert_eq!(endpoint.drain(), vec![ClientMsg::Ping]);
        assert_eq!(
            h.peer.inner.data_tx.capacity(),
            2,
            "감독이 하나를 꺼내 쓰다 섰고 둘이 큐에 남았다"
        );

        drop_the_consumer(&mut h);
        settle_until("접힘 — 시계를 밀지 않았다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        // 통로를 버리는 닫기는 쓰기 막힘에 걸린다 — 그 하나는 일부러 안 듣는 자리라 풀어 준다(`drop_link`).
        endpoint.stall_writes(false);
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))],
            "★걸린 쓰기도 남은 둘도 안 나갔다★"
        );
        assert!(h.exited_cleanly().await);
    }

    #[tokio::test]
    async fn a_consumer_leaving_mid_keepalive_folds_without_waiting_out_the_stalled_ping() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_pings(true);
        h.clock.advance(Policy::default().ping_interval);
        settle().await;
        assert!(endpoint.drain().is_empty(), "keepalive 위에 서 있다");

        drop_the_consumer(&mut h);
        settle_until("접힘 — 시계를 밀지 않았다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        settle_until("감독이 끝났다", || h.task.is_finished()).await;
        assert_eq!(
            endpoint.drain(),
            vec![ClientMsg::Close(Close::new(
                CloseCode::GOING_AWAY,
                "closing"
            ))]
        );
        assert!(h.exited_cleanly().await);
    }

    // ── 3-0 U1c: 배압 · 바이트 셈 · 침묵 판정 멈춤 ──

    /// 페이로드가 정확히 `len` 바이트인 한 장.
    fn blob(tag: u8, len: usize) -> Frame {
        Frame::Binary(vec![tag; len])
    }

    fn blob_tags(items: &[Incoming<TestWire>]) -> Vec<u8> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Blob(bytes)) => Some(bytes[0]),
                _ => None,
            })
            .collect()
    }

    fn notices(items: &[Incoming<TestWire>]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Notice(body)) => Some(body.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_held_line_stops_reading_the_link_instead_of_growing_the_order_slot() {
        let frame_buffer = 4;
        let mut h = build_with(
            Policy {
                frame_buffer,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        for n in 0..20 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n:02}"))));
        }
        settle().await;

        // n00 = 순서 칸 · n01~n04 = 앞잡이 줄. 읽기 태스크는 자리를 잡은 뒤에만 읽어 손에 든 한 장이 없다.
        let read = 1 + frame_buffer;
        assert_eq!(
            endpoint.unread(),
            20 - read,
            "★순서 칸이 붙든 동안 프레임을 더 안 꺼내 통로가 밀린다★ — 순서 칸이 자라지 않는다"
        );
        assert_eq!(h.peer.held_bytes(), read * "not|n00".len());
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "밀린 것은 끊을 일이 아니다"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            notices(&items),
            (0..20).map(|n| format!("n{n:02}")).collect::<Vec<_>>(),
            "★배압 아래에서도 하나도 안 버려지고 온 순서대로 나온다★"
        );
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(h.peer.held_bytes(), 0, "꺼낸 몫은 셈에서 빠진다");
    }

    #[tokio::test]
    async fn a_frame_past_the_byte_budget_stops_the_reads_and_loses_nothing() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        for tag in 0..5u8 {
            endpoint.push(blob(tag, 150));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            4,
            "★빈 셈에는 예산보다 큰 한 장도 들어가고, 그 뒤로는 통로를 안 읽는다★(더하기 전 물높이)"
        );
        assert_eq!(h.peer.held_bytes(), 150);

        let mut tags = Vec::new();
        for _ in 0..5 {
            let taken = h.drain();
            assert_eq!(blob_tags(&taken).len(), 1, "한 번에 한 장씩만 읽힌다");
            tags.extend(blob_tags(&taken));
            settle().await;
        }
        assert_eq!(tags, vec![0, 1, 2, 3, 4], "★손실 0 · 순서 그대로★");
        assert_eq!(endpoint.unread(), 0);
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★줄 길이 1 로 세 층을 다 쓴다★ — 첫 장은 순서 칸에, 둘째 장은 앞잡이 줄에 선다. 읽기 몫(스트림을 안 쓰니
    //   예산 전부) 바로 밑에서 한 장을 더 읽는 것이 최악이라 한 장을 읽기 몫보다 1 작게 둔다. 스트림을 쓰며 창
    //   몫까지 채운 천장은 아래 `holding_a_window_full_and_a_read_share_full_still_fits_the_budget_plus_one_message`.
    #[tokio::test]
    async fn what_a_peer_holds_never_exceeds_the_budget_plus_one_message() {
        let budget = 1000;
        let message = budget - 1;
        let mut h = build_budgeted(Policy::default(), Arc::new(ImmediateHandshake), 1, budget);
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let sent = 10u8;
        for tag in 0..sent {
            endpoint.push(blob(tag, message));
            settle().await;
            assert!(
                h.peer.held_bytes() <= budget + message,
                "{tag}번째를 보낸 뒤 {} 바이트를 쥐었다 — 앞잡이 줄 · 순서 칸 · 줄의 합이 B + M 을 넘었다",
                h.peer.held_bytes()
            );
        }
        assert_eq!(
            h.peer.held_bytes(),
            2 * message,
            "★스트림을 안 쓰는 감독은 예산 전부를 읽기 몫으로 쓴다★ — 그 밑에서 한 장 더 읽고 거기서 멈췄다"
        );
        assert_eq!(endpoint.unread(), usize::from(sent) - 2);

        let mut tags = Vec::new();
        loop {
            settle().await;
            assert!(h.peer.held_bytes() <= budget + message);
            let taken = h.drain();
            if taken.is_empty() {
                break;
            }
            tags.extend(blob_tags(&taken));
        }
        assert_eq!(
            tags,
            (0..sent).collect::<Vec<_>>(),
            "★손실 0 · 순서 그대로★"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn keepalive_goes_out_and_close_is_heard_while_the_line_is_held() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        for n in 0..8 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        endpoint.push(Frame::Keepalive);
        settle().await;
        assert!(h.peer.held_bytes() > 0, "순서 칸이 붙들고 있다");

        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(20));
            settle().await;
            assert!(
                endpoint.drain().contains(&ClientMsg::Ping),
                "★붙든 동안에도 우리 ping 은 나간다★ — 안 나가면 상대가 우리를 침묵으로 끊는다"
            );
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        h.peer.close();
        settle_until("붙든 채로도 닫기가 들린다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items).last().map(String::as_str),
            Some("Closed:1:Explicit")
        );
    }

    /// 50 s 넘게 붙든 뒤 풀어도 `Idle` 이 아니고, 침묵 판정은 풀린 그 순간부터 다시 센다.
    async fn idle_restarts_from_the_release(mut h: Harness, what: &str) {
        h.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "{what}: ★막은 것은 우리 쪽이다★ — 붙든 시간을 상대의 침묵으로 세면 안 된다"
        );

        drain_until_quiet(&mut h).await;
        h.clock.advance(Duration::from_secs(49));
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "{what}: ★풀린 순간이 `last_rx` 다★ — 그 앞의 마지막 수신부터 세면 곧바로 끊긴다"
        );
        h.clock.advance(Duration::from_secs(1));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "{what}: 풀린 뒤에는 침묵 판정이 다시 돈다"
        );
    }

    #[tokio::test]
    async fn holding_past_the_idle_timeout_on_a_full_line_is_not_silence() {
        let (h, endpoint) = live_on_a_full_line().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        settle().await;
        idle_restarts_from_the_release(h, "순서 칸").await;
    }

    #[tokio::test]
    async fn holding_past_the_idle_timeout_over_the_byte_budget_is_not_silence() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        // ★뒤에 살아있음 확인을 두지 않는다★ — 풀리자마자 그것이 읽혀 `last_rx` 를 되감으면 풀림의 되감기를
        //   재지 못한다.
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(
            h.peer.held_bytes() >= budget,
            "줄에 든 한 장이 예산을 넘겼다"
        );
        idle_restarts_from_the_release(h, "바이트 예산").await;
    }

    // ★풀림이 진도 확인이 따르는 타이머 바퀴(`Turn::Timer`) 안에 끼게 한다★ — `on_timer` 는 첫머리의 「붙듦」으로
    //   침묵 판정을 건너뛰었는데, 깨어날 시각을 그 자리의 배압으로 다시 재면 낡은 `last_rx` 의 침묵 시한(이미
    //   지났다)으로 잡혀 진도 확인(`debug_assert`)이 감독을 패닉시킨다.
    //   그 바퀴로 가려면 감독이 `select` 밖에 있는 동안 ping 시각이 지나야 한다 — 막힌 알림 쓰기로 세운다. 그 쓰기를
    //   풀면 감독은 양보 없이 그 바퀴의 ping 까지 가므로, ping 만 따로 막아 그 안에서 받는 쪽이 비우게 한다.
    #[tokio::test]
    async fn a_release_inside_a_timer_round_does_not_trip_the_progress_check() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(blob(0, 2 * budget));
        settle().await;
        assert!(h.peer.held_bytes() >= budget, "예산만으로 붙든다");
        // 침묵 시한(50 s)을 넘긴다 — 붙든 동안이라 끊지 않고, ping 은 20 s 마다 나간다.
        for _ in 0..2 {
            h.clock.advance(Duration::from_secs(20));
            settle().await;
        }
        h.clock.advance(Duration::from_secs(16));
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        // 쓰기에 선 채 다음 ping 시각(60 s)을 넘긴다 — 쓰기 시한(5 s) 안이다.
        h.clock.advance(Duration::from_millis(4500));
        settle().await;
        endpoint.stall_pings(true);
        endpoint.stall_writes(false);
        settle().await;
        assert!(!h.task.is_finished(), "타이머 바퀴의 ping 에서 기다린다");
        h.drain();
        endpoint.stall_pings(false);
        settle().await;
        assert!(
            !h.task.is_finished(),
            "★붙든 채 들어간 타이머 바퀴 안에서 풀려도 진도 확인이 안 터진다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
    }

    // ── 3-0 U1c: 이어받기 창 · 배압 ──

    /// 페이로드가 정확히 `len` 바이트인 스트림 조각(키 1 · 화신 1).
    fn sized_chunk(seq: u64, len: usize) -> Frame {
        let head = format!("chk|1|1|{seq}|");
        assert!(len >= head.len(), "{len} 바이트로는 머리도 못 싣는다");
        Frame::Text(format!("{head}{}", "x".repeat(len - head.len())))
    }

    fn chunk_seqs(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect()
    }

    fn truncations(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item.as_event() {
                Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
                _ => None,
            })
            .collect()
    }

    /// 1번 조각을 받아 꺼내 둔 감독 — 다음 조각부터 구멍 판정이 선다.
    async fn streaming(budget: usize, inbound_cap: usize) -> (Harness, Arc<MemoryEndpoint>) {
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            inbound_cap,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::chunk(1, 1, 1));
        assert_eq!(chunk_seqs(&drain_until_quiet(&mut h).await), vec![1]);
        (h, endpoint)
    }

    // ★붙든 조각을 창 상한만큼 키운다★ — 창 상한이 읽기 몫 이상이라, 그 몫이 읽기 문에 들어가면 문이 닫혀
    //   재생분이 통로에 남아 구멍을 못 메우고, 창이 시한에 닫힌 뒤 늦게 온 재생분은 지나간 번호로 버려진다.
    #[tokio::test]
    async fn a_held_chunk_never_stops_the_reads_that_fill_its_gap() {
        let budget = 100;
        let (read_share, window_cap) = split(budget);
        assert!(window_cap >= read_share);
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(3, window_cap));
        settle().await;
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text("res|1|1".into())],
            "구멍을 보고 재요청을 냈다"
        );
        assert_eq!(
            h.peer.window_bytes(),
            window_cap,
            "붙든 조각은 창 몫에 선다"
        );

        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "★붙든 조각이 창 상한만큼이어도 재생분은 읽힌다★"
        );
        let items = h.drain();
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★구멍이 메워져 순서대로 온다★"
        );
        assert_eq!(truncations(&items), Vec::<u64>::new(), "잘림이 아니다");
        assert_eq!(h.peer.window_bytes(), 0, "답이 와 붙든 것을 버렸다");
    }

    // ★창을 연 뒤 받는 쪽이 멈춘다★ — 줄 길이 1 을 알림 하나로 채워 구멍 사건이 순서 칸에 서고, 그동안 프레임
    //   팔이 빠져 재생분이 앞잡이 줄에 머문다. 창 시한을 그대로 세면 그 사이 창이 닫혀 재생분이 버려진다.
    #[tokio::test]
    async fn a_stalled_consumer_does_not_run_the_resume_window_out() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 1).await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fill".into())));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert!(
            h.peer.held_bytes() > h.peer.window_bytes(),
            "재생분이 앞잡이 줄에 있다"
        );

        // 창 시한(2 s)을 넘기고, ping 시각(20 s)도 넘겨 붙든 채 타이머가 한 번 돌게 한다.
        h.clock.advance(Duration::from_secs(3));
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "붙든 채 타이머가 돌았다"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★막아 둔 것은 우리 쪽이다★ — 붙든 시간을 창 시한으로 세면 메울 수 있던 구멍을 잘림으로 접는다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★풀린 뒤 재생분이 구멍을 메운다★"
        );
    }

    // 예산 100 = 창 상한 50 — 30 두 장이면 둘째에서 넘는다.
    #[tokio::test]
    async fn holding_past_the_window_cap_takes_the_truncation_path() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(3, 30));
        settle().await;
        assert_eq!(h.peer.window_bytes(), 30);

        endpoint.push(sized_chunk(4, 30));
        settle().await;
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (60, 0),
            "★창에서 나온 조각의 몫은 읽기 문으로 돌아간다★ — 창 몫에 남으면 받는 쪽이 느린 것만으로 다음 구멍을 \
             못 붙든다"
        );
        let items = h.drain();
        assert_eq!(
            truncations(&items),
            vec![3],
            "★창 상한을 넘기면 말없이 버리지 않고 잘림으로 신고한다★ — 붙든 첫 번호가 하한이다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![3, 4],
            "붙든 것을 순서대로 내보낸다"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn held_bytes_counts_a_held_chunk_until_its_window_resolves() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        let held = TestWire::chunk(1, 1, 3).payload_len();
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.drain();
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (held, held),
            "★창이 붙든 조각도 감독이 쥔 바이트다★"
        );

        let answer = TestWire::chunk(1, 1, 2).payload_len();
        endpoint.push(TestWire::chunk(1, 1, 2));
        settle().await;
        assert_eq!(
            (h.peer.held_bytes(), h.peer.window_bytes()),
            (answer, 0),
            "답이 와 창이 닫히면 붙든 몫이 빠지고 올라간 답만 남는다"
        );
        h.drain();
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ── 3-0 U1c: 통로를 버릴 때 앞잡이 줄을 건진다 ──
    //
    // ★줄 길이 1 로 세운다★ — n0 은 순서 칸에 서고 n1 · n2 는 프레임 팔이 빠져 앞잡이 줄에 남는다.

    async fn three_notices_held(h: &Harness, endpoint: &MemoryEndpoint) {
        for n in 0..3 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            h.peer.held_bytes(),
            3 * "not|n0".len(),
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에 있다"
        );
        assert_eq!(endpoint.unread(), 0);
    }

    // ★앞잡이 줄을 꽉 채운다(2칸)★ — 읽기 태스크가 다음 자리를 기다리는 동안 손에 든 한 장이 없어야, 건진 것과
    //   통로에 남은 것을 합쳐 보낸 것 전부가 된다.
    #[tokio::test]
    async fn reconnecting_hands_over_what_was_already_read_before_the_new_link() {
        let mut h = build_with(
            Policy {
                frame_buffer: 2,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        for n in 0..5 {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(format!("n{n}"))));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "n0 은 순서 칸에, n1 · n2 는 앞잡이 줄에"
        );
        h.peer.reconnect_now();
        settle_until("새 통로", || h.net.dials() == 2).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:Explicit",
                "Connected:2",
            ],
            "★다시 걸어도 이미 읽은 것은 버리지 않고 옛 세대로 끊김 앞에 싣는다★"
        );
        assert_eq!(
            endpoint.unread(),
            2,
            "n3 · n4 는 옛 통로에 남았다 — 읽고서 버린 것은 없다"
        );
        assert_eq!(h.peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn a_write_failure_hands_over_what_was_already_read_before_the_disconnect() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        three_notices_held(&h, &endpoint).await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("stuck".into())).unwrap();
        settle().await;
        h.clock.advance(Policy::default().write_deadline);
        settle().await;
        // 버리는 통로의 닫기도 같은 멈춤에 걸린다 — 풀어 정리를 끝낸다(`stall_writes` rustdoc).
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "msg:n1",
                "msg:n2",
                "Disconnected:1:WriteDeadline",
                "Dropped",
            ],
            "★쓰기가 실패해도 이미 읽은 것은 끊김 앞에 온다★ — 못 나간 알림의 신고는 백오프가 낸다"
        );
    }

    // ★다시 붙은 통로도 같은 셈을 쓴다★ — 옛 통로가 남긴 몫이 읽기 몫 밑으로 내려오기 전에는 새 통로를 안 읽는다.
    #[tokio::test]
    async fn a_new_link_waits_for_what_the_old_link_left_to_drop_below_the_budget() {
        let budget = 100;
        let mut h = build_budgeted(
            Policy::default(),
            Arc::new(ImmediateHandshake),
            4096,
            budget,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let first = h.net.accept().unwrap();
        first.push(blob(0, 2 * budget));
        settle().await;
        assert_eq!(h.peer.held_bytes(), 2 * budget, "줄에 남겨 둔다");

        h.peer.reconnect_now();
        settle_until("다시 붙음", || h.net.dials() == 2).await;
        settle_until("운영", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        second.push(blob(1, 10));
        settle().await;
        assert_eq!(
            second.unread(),
            1,
            "★옛 통로의 몫이 읽기 몫 이상이면 새 통로를 안 읽는다★"
        );

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(blob_tags(&items), vec![0, 1]);
        assert_eq!(second.unread(), 0, "옛 몫이 빠지자 새 통로를 읽었다");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★답장이 앞잡이 줄에 있는 채 다시 걸어도 기다리던 호출자가 받는다★ — 건지기가 기계의 대기자 깨움보다 앞이다.
    #[tokio::test]
    async fn a_reply_already_read_reaches_its_caller_when_reconnecting() {
        let (mut h, endpoint) = live_on_a_full_line().await;
        let asked = spawn_request(&h.peer, 7);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("held".into())));
        endpoint.push(TestWire::reply(7));
        settle().await;
        assert_eq!(endpoint.unread(), 0);
        assert!(!asked.is_finished(), "답장은 앞잡이 줄에 있다");

        h.peer.reconnect_now();
        assert_eq!(
            joined(asked).await,
            Ok(TestIn::Reply {
                tag: 7,
                body: "ok".into(),
            }),
            "★이미 읽은 답장은 끊김으로 실패하지 않는다★"
        );
        drain_until_quiet(&mut h).await;
    }

    // ── 3-0 U1c: 예산 하나가 읽기 몫과 창 상한을 함께 덮는다(B + M) ──

    // ★창을 채운 채 읽기 몫까지 채운다★ — 창 상한이 예산이면(셈이 둘이면) 창에 넷(96)을 붙들고 읽기로 둘(98)을
    //   더 쥐어 B + M(149)을 넘는다. 창 상한이 반이면 셋째 조각에서 창이 잘림으로 닫히고 그 몫이 읽기 문을 닫는다.
    #[tokio::test]
    async fn holding_a_window_full_and_a_read_share_full_still_fits_the_budget_plus_one_message() {
        let budget = 100;
        let message = 49;
        let (mut h, endpoint) = streaming(budget, 1).await;
        let pushes: Vec<Frame> = (3..=6)
            .map(|seq| sized_chunk(seq, 24))
            .chain((0..3).map(|tag| blob(tag, message)))
            .collect();
        for frame in pushes {
            endpoint.push(frame);
            settle().await;
            assert!(
                h.peer.held_bytes() <= budget + message,
                "{} 바이트를 쥐었다 — 셈 전체가 B + M 을 넘었다",
                h.peer.held_bytes()
            );
            assert!(h.peer.window_bytes() <= split(budget).1);
        }

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(chunk_seqs(&items), vec![3, 4, 5, 6]);
        assert_eq!(truncations(&items), vec![3]);
        assert_eq!(blob_tags(&items), vec![0, 1, 2], "★손실 0 · 순서 그대로★");
        assert_eq!(h.peer.held_bytes(), 0);
    }

    // ★스트림 조각을 본 감독만 읽기 문을 반으로 줄인다★ — 같은 세 장(30 · 30 · 30)을 예산 100 에 보낸다. 스트림을
    //   안 쓴 감독은 셋째도 읽고(60 < 100), 조각을 하나 본 감독은 둘째에서 읽기 몫(50)에 닿아 셋째를 통로에 둔다.
    #[tokio::test]
    async fn only_a_peer_that_has_seen_a_stream_chunk_splits_its_read_gate() {
        let budget = 100;
        for streams in [false, true] {
            let mut h = build_budgeted(
                Policy::default(),
                Arc::new(ImmediateHandshake),
                4096,
                budget,
            );
            settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
            let endpoint = h.net.accept().unwrap();
            if streams {
                endpoint.push(TestWire::chunk(1, 1, 1));
            }
            drain_until_quiet(&mut h).await;
            for tag in 0..3u8 {
                endpoint.push(blob(tag, 30));
            }
            settle().await;
            assert_eq!(
                endpoint.unread(),
                usize::from(streams),
                "스트림 조각을 봤나 = {streams}"
            );
            assert_eq!(
                blob_tags(&drain_until_quiet(&mut h).await),
                vec![0, 1, 2],
                "★손실 0 · 순서 그대로★"
            );
        }
    }

    // ── 3-0 U1c: 붙든 조각의 진도 · 시계 · 세대 · 정리 ──

    // ★바이트 상한에 닿게 한 조각이 붙든 것 중 가장 큰 번호가 아니다★ — 진도를 그 조각 번호로 두면 재생분 4 · 5 가
    //   5 뒤에 다시 올라간다(순서 역행 · 겹침).
    #[tokio::test]
    async fn a_descending_arrival_past_the_window_cap_never_rewinds_the_stream() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(sized_chunk(5, 30));
        endpoint.push(sized_chunk(3, 30));
        settle().await;
        for seq in 4..=6 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), vec![3]);
        assert_eq!(
            chunk_seqs(&items),
            vec![3, 5, 6],
            "★늘 오름차순 · 겹침 없음★"
        );
    }

    // ★붙듦이 쓰기를 기다리던 사이 시작된다★ — 감독은 그 쓰기가 끝나야 바퀴 첫머리에 와서 붙듦을 알아챈다. 알아챈
    //   때부터 세면 쓰기에 선 3 s 가 창 시한(2 s)으로 세져, 풀리자마자 창이 닫히고 재생분은 지나간 번호로 버려진다.
    #[tokio::test]
    async fn backpressure_that_begins_during_a_stalled_write_still_stops_the_resume_clock() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        endpoint.push(blob(0, 2 * budget));
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "쓰기에 선 사이 읽기 문이 닫혀 재생분이 통로에 남았다"
        );
        let resume_timeout = Policy::default().resume_timeout;
        let stalled = resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★붙듦은 시작된 때부터 센다★ — 알아챈 때부터 세면 쓰기를 기다린 시간이 창 시한을 다 먹는다"
        );
        assert_eq!(
            chunk_seqs(&items),
            vec![2, 3],
            "★풀린 뒤 재생분이 구멍을 메운다★"
        );
        assert_eq!(blob_tags(&items), vec![0]);
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // 위 시험의 순서 칸 판 — 구멍 사건이 찬 줄 때문에 순서 칸에 선 그 자리에서 재요청 쓰기가 막힌다. 순서 칸이
    //   붙들기 시작한 때부터 세야 쓰기에 선 시간이 창 시한으로 안 세진다.
    #[tokio::test]
    async fn a_held_line_that_begins_before_a_stalled_resume_write_still_stops_the_resume_clock() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 1).await;
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fill".into())));
        settle().await;
        endpoint.stall_writes(true);
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), Vec::<u64>::new());
        assert_eq!(chunk_seqs(&items), vec![2, 3]);
    }

    // 위 시험의 유실 신고 판 — 쓰기에 선 사이 받는 쪽이 줄을 비우고 나가는 큐가 넘친다. 쓰기가 끝난 바퀴 첫머리의
    //   신고가 구멍 사건까지 줄에 실어 순서 칸을 비운다 — 붙듦을 그 신고보다 먼저 보지 않으면 쓰기 도중 시작된
    //   붙듦을 잃어, 시한이 앞잡이 줄의 재생분보다 먼저 와 창을 닫는다.
    #[tokio::test]
    async fn a_drop_report_that_empties_the_held_line_does_not_hide_the_pause() {
        let mut h = build_budgeted(
            Policy {
                outbound_queue: 1,
                full_queue: QueueFullPolicy::DropAndReport,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            2,
            BYTE_BUDGET,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::chunk(1, 1, 1));
        assert_eq!(chunk_seqs(&drain_until_quiet(&mut h).await), vec![1]);
        for body in ["fill0", "fill1"] {
            endpoint.push(TestWire::encode_in(&TestIn::Notice(body.into())));
        }
        settle().await;
        endpoint.stall_writes(true);
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        h.peer.notify(TestOut::Notice("queued".into())).unwrap();
        assert_eq!(
            h.peer.notify(TestOut::Notice("lost".into())),
            Err(SendError::QueueFull)
        );
        assert_eq!(
            labels(&h.drain()),
            vec!["msg:fill0", "msg:fill1"],
            "쓰기에 선 사이 받는 쪽이 줄을 비웠다 — 구멍 사건은 아직 순서 칸에 있다"
        );
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(truncations(&items), Vec::<u64>::new());
        assert_eq!(chunk_seqs(&items), vec![2, 3]);
        assert!(labels(&items).contains(&"Dropped".to_string()));
    }

    // ★읽기 문이 쓰기를 기다리던 사이 닫혔다가 그 쓰기가 끝나기 전에 다시 열린다★ — 감독은 바퀴 첫머리에서 열린
    //   문만 보고 붙듦을 끝내 못 본다. 그 시간을 안 밀면 쓰기에 선 3 s 가 창 시한(2 s)을 넘겨, 시한이 앞잡이 줄의
    //   재생분보다 먼저 와 창을 닫고 재생분 2 는 지나간 번호로 버려진다.
    #[tokio::test]
    async fn a_gate_that_shuts_and_reopens_inside_a_stalled_write_still_stops_the_resume_clock() {
        let budget = 100;
        let (mut h, endpoint) = streaming(budget, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        endpoint.push(blob(0, 40));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);

        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("slow".into())).unwrap();
        settle().await;
        endpoint.push(blob(1, 20));
        endpoint.push(TestWire::chunk(1, 1, 2));
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            2,
            "쓰기에 선 사이 읽기 문이 닫혀 재생분이 통로에 남았다"
        );
        let stalled = Policy::default().resume_timeout + Duration::from_secs(1);
        assert!(stalled < Policy::default().write_deadline);
        h.clock.advance(stalled);
        settle().await;
        assert_eq!(blob_tags(&h.drain()), vec![0]);
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "받는 쪽이 비워 읽기 문이 다시 열렸다 — 쓰기는 아직 서 있다"
        );
        endpoint.stall_writes(false);

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            truncations(&items),
            Vec::<u64>::new(),
            "★끝내 못 본 붙듦도 창 시한에서 뺀다★"
        );
        assert_eq!(chunk_seqs(&items), vec![2, 3], "★재생분이 구멍을 메운다★");
        assert_eq!(blob_tags(&items), vec![1]);
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── 3-0 U1c: 붙든 시간 세기(`track_pause`) — 돌리지 않은 감독에 칸을 손으로 놓는다 ──

    fn idle_supervisor() -> (Supervisor<TestWire>, ManualClock) {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound, _) = mpsc::channel(1);
        let (_, supervisor) = new_peer(PeerConfig {
            id: PeerId::new("d1"),
            addr: Address::new("mem://d1"),
            wire: Arc::new(TestWire),
            dialer: net.dialer(),
            clock: clock.handle(),
            handshake: Arc::new(ImmediateHandshake),
            policy: Policy::default(),
            inbound,
            byte_budget: 100,
            generations: Arc::new(AtomicU64::new(0)),
        });
        (supervisor, clock)
    }

    /// 1번을 받고 3번에서 구멍을 연 창 — 지금 청한 것으로 적힌다. 돌려주는 것 = 그 창의 시한.
    fn open_window(sup: &mut Supervisor<TestWire>) -> Instant {
        let now = sup.clock.now();
        for seq in [1, 3] {
            let msg = sup.wire.decode(TestWire::chunk(1, 1, seq)).unwrap();
            let mark = sup.wire.stream_mark(&msg).unwrap();
            let held = Received {
                msg,
                charge: Charge::none(),
                generation: Generation(1),
            };
            let _ = sup.streams.observe_capped(&mark, held, now, false);
        }
        sup.streams.next_resume_deadline().expect("창이 열렸다")
    }

    /// 읽기 문을 `secs` 초 닫았다가 연다 — 예산 100 · 스트림을 안 쓰니 100 바이트면 닫힌다.
    fn shut_for(sup: &Supervisor<TestWire>, clock: &ManualClock, secs: u64) {
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(secs));
        drop(gate);
    }

    #[test]
    fn unseen_shut_intervals_shift_the_window_by_exactly_their_length() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 2);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 3);
        sup.track_pause(clock.now());
        assert!(!sup.paused);
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(5)),
            "★두 구간을 다 민다★ — 그 사이 열려 있던 1 s 는 안 민다"
        );
        assert_eq!(sup.last_rx, clock.now(), "못 본 붙듦이 풀린 것도 풀림이다");
    }

    // ★못 본 구간이 순서 칸의 붙듦에 걸쳐 있으면 그 첫 구간부터 붙든 것으로 본다★ — 순서 칸이 붙들기 시작한 때만
    //   보면 그 앞에 닫혀 있던 1 s 를 잃고, 둘을 따로 밀면 겹친 1 s 를 두 번 민다.
    #[test]
    fn an_unseen_interval_reaching_into_a_held_line_starts_the_pause_at_its_own_start() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(1));
        sup.order.push_back(Charged::bare(Incoming::Event(
            TransportEvent::AmbiguousRole {
                peer: PeerId::new("d1"),
            },
        )));
        sup.order_since = clock.now();
        clock.advance(Duration::from_secs(1));
        drop(gate);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(sup.paused);

        sup.order.clear();
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "읽기 문이 닫힌 때(1 s)부터 풀림을 알아챈 때(5 s)까지다"
        );
    }

    // ★못 본 구간이 지금 붙듦보다 앞에서 끝났으면 따로 민다★ — 지금 붙듦의 시작을 그 구간의 시작으로 당기면
    //   그 사이 열려 있던 시간까지 민다.
    #[test]
    fn an_unseen_interval_that_ended_before_the_current_pause_is_shifted_on_its_own() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 2);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(sup.paused);

        drop(gate);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "못 본 2 s + 지금 붙듦(4 s 에 닫혀 6 s 에 알아챘다) 2 s — 그 사이 열린 1 s 는 안 민다"
        );
    }

    // ★붙든 줄 알고 있던 동안 닫혔다 열린 구간을 셈이 따로 넘겨도 다시 밀지 않는다★ — 그 구간은 다 붙듦이
    //   시작된 뒤라 풀릴 때 미는 시간 안에 이미 든다.
    #[test]
    fn intervals_inside_a_known_pause_are_not_shifted_twice() {
        let (mut sup, clock) = idle_supervisor();
        let deadline = open_window(&mut sup);
        clock.advance(Duration::from_secs(1));
        let gate = sup.ledger.charge(100);
        sup.track_pause(clock.now());
        assert!(sup.paused);
        clock.advance(Duration::from_secs(1));
        drop(gate);
        clock.advance(Duration::from_secs(1));
        shut_for(&sup, &clock, 1);
        clock.advance(Duration::from_secs(1));
        sup.track_pause(clock.now());
        assert!(!sup.paused);
        assert_eq!(
            sup.streams.next_resume_deadline(),
            Some(deadline + Duration::from_secs(4)),
            "붙듦이 시작된 1 s 부터 풀림을 알아챈 5 s 까지 한 번만"
        );
    }

    // ★디코드 실패로 끊는 정책이어도 건지기는 그 뒤까지 넘긴다★ — 운영 중에 그 한 장에서 끊어도 그 뒤는 건지기가
    //   넘기므로, 건지는 중에 멈추면 같은 앞잡이 줄이 끊은 자리에 따라 넘어가기도 버려지기도 한다.
    #[tokio::test]
    async fn salvaging_carries_on_past_a_frame_it_cannot_decode() {
        let mut h = build_with(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
            1,
        );
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n0".into())));
        endpoint.push(Frame::Text("garbage".into()));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("n1".into())));
        settle().await;
        assert_eq!(
            endpoint.unread(),
            0,
            "n0 은 순서 칸에, 나머지는 앞잡이 줄에"
        );
        h.peer.reconnect_now();
        settle_until("새 통로", || h.net.dials() == 2).await;

        let items = drain_until_quiet(&mut h).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "msg:n0",
                "DecodeFailed",
                "msg:n1",
                "Disconnected:1:Explicit",
                "Connected:2",
            ]
        );
    }

    #[tokio::test]
    async fn a_chunk_held_across_a_reconnect_keeps_the_generation_that_carried_it() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.peer.reconnect_now();
        settle_until("다시 붙음", || h.net.dials() == 2).await;
        settle_until("운영", || h.peer.peer_state() == PeerState::Live).await;
        let second = h.net.accept().unwrap();
        settle().await;
        assert_eq!(
            second.frames(),
            vec![Frame::Text("res|1|1".into())],
            "창이 살아남아 새 통로에서 다시 청했다"
        );

        h.clock.advance(Policy::default().resume_timeout);
        let items = drain_until_quiet(&mut h).await;
        let held: Vec<(u64, u64)> = items
            .iter()
            .filter_map(Incoming::as_message)
            .filter_map(|arrival| match &arrival.msg {
                TestIn::Chunk { seq, .. } => Some((*seq, arrival.generation.0)),
                _ => None,
            })
            .collect();
        assert_eq!(truncations(&items), vec![3], "둘째 통로에서 창이 닫혔다");
        assert_eq!(
            held,
            vec![(3, 1)],
            "★실어 온 연결의 세대다★ — 창이 닫힌 연결(2)의 세대를 찍으면 `Arrival::generation` 계약이 깨진다"
        );
    }

    // ★정리가 장부를 버리기 전에 열린 창을 접는다★ — 안 접으면 붙든 조각이 배달도 잘림 신고도 없이 사라진다.
    #[tokio::test]
    async fn closing_hands_over_a_held_chunk_as_a_truncation_before_closed() {
        let (mut h, endpoint) = streaming(BYTE_BUDGET, 4096).await;
        endpoint.push(TestWire::chunk(1, 1, 3));
        settle().await;
        h.peer.close();
        settle_until("접힘", || h.peer.peer_state() == PeerState::Closed).await;

        let items = drain_until_quiet(&mut h).await;
        let seen: Vec<String> = items
            .iter()
            .map(|item| match item.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => format!("chunk:{seq}"),
                _ => label(item),
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                "StreamGap",
                "Disconnected:1:Explicit",
                "StreamTruncated",
                "chunk:3",
                "Closed:1:Explicit",
            ]
        );
        assert_eq!(truncations(&items), vec![3], "붙든 첫 번호가 하한이다");
        assert_eq!(h.peer.held_bytes(), 0);
    }
}
