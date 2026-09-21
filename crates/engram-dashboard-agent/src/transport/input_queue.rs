//! 콘솔 계열 통로(PTY·stdio)의 **에이전트별 입력 큐** — `send_input` 이 OS 쓰기를 **호출 스레드에서
//! 하지 않게** 만드는 seam.
//!
//! ★왜 있나★: 옛 모양은 `send_input` 이 writer 뮤텍스를 `write_all`+`flush` 내내 쥐고 그 OS 쓰기를
//!   호출자의 스레드에서 했다. 자식이 stdin 을 안 읽으면 그 쓰기가 파이프 backpressure 로 매달리고,
//!   **부른 쪽이 함께 매달린다.** 데몬의 연결당 dispatch 소비자가 그 호출자라, 한 에이전트가 물리면
//!   그 연결의 **다른 명령 전부**가 함께 선다. 그래서 큐에 넣고 즉시 돌아오고, 전담 라이터 스레드
//!   하나가 FIFO 로 빼서 쓴다.
//!
//! ★선례는 이 저장소 안에 이미 있다 — 새로 발명한 모양이 아니다★:
//!   [`crate::backend::codex::transport`] 가 같은 문제를 같은 모양으로 풀었다(유계 큐 + `engram-codex-writer`
//!   전담 스레드 + 상한 초과는 `Err`). 그쪽은 한 칸이 **완결된 유저 턴**이라 칸 수로 세지만, 여기 한 칸은
//!   키 입력 한 글자일 수도 붙여넣은 본문 전체일 수도 있어 **칸 수가 메모리를 재지 못한다** — 그래서
//!   이쪽 상한의 축은 바이트다([`INPUT_QUEUE_MAX_BYTES`]).
//!
//! ★★이 큐가 바꾸는 계약을 정확히 적는다 — 이것이 이 파일의 가장 중요한 문장이다★★:
//!   `Ok` 의 뜻이 **「바이트가 자식에게 갔다」에서 「전량을 순서까지 확정해 받았다」로** 옮겨 갔다.
//!   받아 둔 뒤에 실패한 쓰기는 **그 호출자에게 돌아갈 길이 없다**(codex 쪽 `send_input` 의 「알려진
//!   한계」와 같은 자리). 그 실패가 남기는 것은 ⓐ `warn` 로그 ⓑ 큐가 닫히면서 **다음** `send_input` 부터
//!   돌아가는 `Err` ⓒ 그리고 대개 곧 이어지는 pump 의 종점 전이다 — 파이프가 깨졌다는 것은 reader 도
//!   곧 EOF 를 본다는 뜻이라, 사람이 보는 신호는 결국 그 종료다.
//!
//! tauri import 0.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::types::{AgentId, PtyError};

/// 아직 OS 로 나가지 못한 입력의 상한(바이트). ★넘으면 그 호출이 `Err` 로 돌아간다 — 버리지 않는다★
/// (ADR-0190 의 처분 그대로. 조용히 버리는 갈래는 그 ADR 이 이미 기각했다).
///
/// ★「아직 나가지 못한」에는 **라이터가 꺼내 들고 쓰는 중인 덩이도 든다**★ — 회계의 정본은
///   [`Inner::bytes`] 다. 그 한 덩이를 빼고 세면 이 상수가 선언하는 천장이 실제로는 두 배가 된다
///   (꺼내는 순간 0 으로 세어져 생산자가 상한만큼 또 넣을 수 있다). 물려 있는 시간이야말로 이 상한이
///   겨냥한 구간이므로, 그 구간을 안 세면 이 값이 아무것도 재지 않는다.
///
/// ★축이 바이트인 이유★: 여기 한 칸은 완결된 단위가 아니다 — 키 입력 한 글자일 수도, 붙여넣은 본문
///   전체일 수도 있다. 칸 수로 세면 「32 칸」이 32 바이트일 수도 32MiB 일 수도 있어 **무엇도 재지
///   못한다.** 이 상한이 실제로 재는 것은 **자식이 stdin 을 안 읽을 때 에이전트 하나가 물고 있을 수
///   있는 메모리**이고, 그 축에서 정직한 단위는 바이트뿐이다.
/// ★값의 근거 = 이 저장소가 이미 고른 숫자와의 대칭★: 에이전트 하나의 **출력**이 메모리에 앉을 수
///   있는 천장이 2MiB 다([`crate::output_core`] 의 replay 링). 입력 쪽에 그보다 큰 천장을 주면 「출력은
///   2MiB 로 자르면서 입력은 무제한에 가깝게 문다」가 되므로, 같은 숫자를 준다.
/// ★어긋나면 어느 쪽으로 틀리나★: 너무 작으면 정상적으로 큰 붙여넣기가 거절되고(사람이 즉시 알고
///   다시 시도할 수 있다), 너무 크면 물린 에이전트마다 그만큼의 메모리가 조용히 잠긴다. 한 번에 보내는
///   본문이 2MiB 를 넘는 것은 이 경로의 정상 사용이 아니므로(2MiB 프롬프트 ≈ 50 만 토큰) 작은 쪽의
///   대가가 실질적으로 발생하지 않는 자리를 골랐다.
pub const INPUT_QUEUE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// 라이터가 알림을 놓쳐도 **영구 침묵 대신 지연**으로 떨어지게 하는 재확인 주기.
///
/// ★정확성을 지는 것은 이 값이 아니다★ — [`InputQueue::push`]·[`InputQueue::close`] 가 **락을 쥔 채**
///   `notify` 하므로 깨움이 유실될 창이 없다. 이 타임아웃은 그 규율이 나중에 깨졌을 때 에이전트 입력이
///   영영 죽는 대신 이만큼 늦어지기만 하게 하는 백스톱이다(codex 라이터의 `SWEEP_INTERVAL` 과 같은
///   역할이되, 이쪽은 훑을 일이 없어 순수 백스톱이다).
const WAKE_BACKSTOP: Duration = Duration::from_millis(500);

struct Inner {
    pending: VecDeque<Vec<u8>>,
    /// **아직 OS 로 나가지 않은** 페이로드 바이트 합 = `pending` 이 든 것 **+ 라이터가 꺼내 들고 쓰는
    /// 중인 것**. [`InputQueue::push`] 가 더하고 [`InputQueue::mark_written`] 이 뺀다(매번 순회하지
    /// 않기 위해 합계를 들고 다닌다).
    ///
    /// ★빼는 자리가 [`InputQueue::pop`] 이 **아닌 것이 요점이다**★: 꺼낸 덩이는 그 순간부터 블로킹
    ///   OS 쓰기에 물려 있을 수 있고, 그 시간이 이 상한이 겨냥하는 바로 그 구간이다. pop 에서 빼면
    ///   물려 있는 덩이가 **0 으로 세어져** 생산자가 상한만큼 또 넣을 수 있다 — 실제 천장이 선언값의
    ///   두 배가 된다. 그래서 「받았다 → 나갔다」 사이 내내 세고, 닫힘([`InputQueue::close`])은 남은
    ///   대기분을 버리므로 거기서 0 으로 되돌린다.
    bytes: usize,
    /// `None` = 열려 있다. `Some(사유)` = 닫혔다 — 그 사유가 이후 [`InputQueue::push`] 의 `Err` 문구다.
    closed: Option<String>,
    /// 지금까지 **받은** 덩이 수. [`InputQueue::wait_drained`] 가 기다릴 목표를 여기서 찍는다.
    pushed_seq: u64,
    /// 지금까지 **실제로 OS 로 나간** 덩이 수. 라이터가 쓰기 성공마다 하나 올린다.
    ///
    /// ★둘의 차이가 「받았다」와 「나갔다」의 거리다★ — 그 거리를 밖에서 볼 수 있게 만드는 것이
    ///   이 두 칸의 존재 이유이고, 그것이 없으면 영수증이 수락을 배달로 신고한다(ADR-0088 이 가르려는
    ///   두 경우가 뒤집힌다). 덩이는 FIFO 로 나가므로 **k 번째로 받은 것이 k 번째로 나간다** — 그래서
    ///   단조 카운터 둘로 충분하고 덩이마다 표를 달 필요가 없다.
    written_seq: u64,
}

/// 한 에이전트의 입력 대기열. 생산자 = `send_input` 을 부르는 아무 스레드, 소비자 = 전담 라이터 하나.
pub(crate) struct InputQueue {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl InputQueue {
    pub(crate) fn new() -> Self {
        InputQueue {
            inner: Mutex::new(Inner {
                pending: VecDeque::new(),
                bytes: 0,
                closed: None,
                pushed_seq: 0,
                written_seq: 0,
            }),
            wake: Condvar::new(),
        }
    }

    /// ★전량을 받거나 하나도 안 받는다★ — 부분 수용을 `Ok` 로 축소 보고하지 않는다.
    ///
    /// `Err` 가 되는 자리는 둘뿐이고 **둘 다 이 호출이 돌아오기 전에 결정된다**: ① 큐가 닫혔다
    /// ② 상한을 넘는다. 받아 둔 뒤의 실패는 이 반환값이 아니라 다음 호출의 `Err` 로 나타난다
    /// (모듈 헤더의 계약 문단).
    pub(crate) fn push(&self, bytes: Vec<u8>) -> Result<(), PtyError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(reason) = &inner.closed {
            return Err(PtyError::WriteFailed(reason.clone()));
        }
        // ★단일 페이로드가 홀로 상한을 넘는 경우도 여기서 걸린다 — 의도다★: 그것을 통과시키면 상한이
        //   재려던 바로 그 메모리를 한 번의 호출이 무한정 가져간다.
        let after = inner.bytes.saturating_add(bytes.len());
        if after > INPUT_QUEUE_MAX_BYTES {
            return Err(PtyError::WriteFailed(format!(
                "입력 대기열 상한({INPUT_QUEUE_MAX_BYTES} bytes)을 넘는다 — 대기 {} + 요청 {} — 이 입력은 받지 않았다(버린 것이 아니다)",
                inner.bytes,
                bytes.len()
            )));
        }
        inner.bytes = after;
        inner.pending.push_back(bytes);
        inner.pushed_seq += 1;
        // ★락을 쥔 채 알린다★ — 이것이 [`WAKE_BACKSTOP`] 이 백스톱에 그치는 이유다.
        self.wake.notify_one();
        Ok(())
    }

    /// 다음 덩이를 꺼낸다. 큐가 닫혔으면 `None` — 라이터는 그것을 보고 루프를 끝낸다.
    ///
    /// ★닫힘이 남은 대기분보다 **먼저** 판정된다 — 즉 닫는 순간 아직 못 나간 입력은 사라진다★.
    ///   닫히는 자리가 전부 「이 자식은 더 이상 읽지 않는다」(kill·스트림 종료·쓰기 실패)라, 남은 것을
    ///   마저 쓰려 해도 전부 실패한다. codex 라이터의 처분과 같다.
    /// ★꺼내도 [`Inner::bytes`] 는 **줄지 않는다**★ — 꺼낸 덩이는 아직 나간 것이 아니다. 그 회계를
    ///   푸는 것은 [`mark_written`](Self::mark_written) 하나뿐이고, 근거는 그 필드 doc.
    pub(crate) fn pop(&self) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if inner.closed.is_some() {
                return None;
            }
            if let Some(chunk) = inner.pending.pop_front() {
                return Some(chunk);
            }
            let (guard, _timed_out) = self
                .wake
                .wait_timeout(inner, WAKE_BACKSTOP)
                .unwrap_or_else(|p| p.into_inner());
            inner = guard;
        }
    }

    /// 큐를 닫는다(멱등 — **첫 사유가 이긴다**). 라이터를 깨워 끝내고, 이후 `push` 는 그 사유로 `Err`.
    ///
    /// ★여기서 잡는 것은 이 큐의 락뿐이다★ — 블로킹 쓰기에 매달린 라이터는 이 락을 **놓고** 쓰므로
    ///   ([`drain`] 이 `pop` 반환 뒤에 쓴다) 이 호출은 그 쓰기 뒤에 매달릴 수 없다. `shutdown()` 이
    ///   이것을 kill 보다 먼저 불러도 안전한 근거가 그것이고, 순서를 뒤집어도 안전하지만 **먼저 닫는
    ///   쪽**이 죽어 가는 자식에게 한 줄이라도 덜 먹인다.
    /// ★첫 사유가 이기는 이유★: 실제 원인은 언제나 먼저 온 쪽이다(쓰기 실패 → 그 뒤 종료). 나중 사유로
    ///   덮으면 진단이 「에이전트를 종료했다」로 뭉개진다.
    pub(crate) fn close(&self, reason: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.closed.is_none() {
            inner.closed = Some(reason.to_string());
        }
        inner.pending.clear();
        inner.bytes = 0;
        self.wake.notify_all();
    }

    /// 덩이 하나가 **실제로 OS 로 나갔다**. 라이터만 부른다([`drain`]).
    ///
    /// ★`len` 은 그 덩이의 바이트 수★ — 여기가 [`Inner::bytes`] 회계를 푸는 **유일한** 자리다.
    ///   닫힘이 먼저 0 으로 되돌렸을 수 있으므로 `saturating_sub` 로 뺀다(그 경우 이미 0 이 맞다).
    fn mark_written(&self, len: usize) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.bytes = inner.bytes.saturating_sub(len);
        inner.written_seq += 1;
        // 대기자([`wait_drained`])를 깨운다. `pop` 도 같은 조건변수에서 자지만 그쪽은 루프라 무해하다.
        self.wake.notify_all();
    }

    /// ★이 호출 시점까지 받아 둔 것이 **전부 나갈 때까지** 기다린다★ — `Ok` = 나갔다.
    ///
    /// ★왜 있나 — 이것이 영수증의 뜻을 되돌리는 자리다★: 큐가 생기면서 `send_input` 의 `Ok` 는
    ///   「받았다」로 내려앉았고, 그것을 그대로 배달 영수증에 실으면 **아직 안 나간 본문이 「배달 성공」으로
    ///   기록된다**(ADR-0088 이 가르려는 "전송 실패" 와 "모델이 무시" 가 뒤집힌다). 배달 동사만 이 확인을
    ///   지나면 그 뜻이 다시 「나갔다」가 된다. ★키 입력 경로는 이것을 부르지 않는다★ — 거기서 기다리면
    ///   이 큐가 없애려던 바로 그 head-of-line blocking 이 되돌아온다.
    /// ★목표를 락 안에서 찍는다★: 그래서 「내가 넣은 것까지」가 확실히 포함된다. 그 사이 남이 더 넣으면
    ///   그것까지 기다리게 되지만(초과 대기) 부족하지는 않다 — 틀리는 방향이 안전한 쪽이다.
    /// ★시한을 넘겨도 **큐에 든 것을 취소하지 않는다**★: 그래서 「확인 실패」와 「안 나갔다」는 같지 않다 —
    ///   늦게 나가는 갈래가 남는다. 그 잔여의 처분(재시도가 중복 배달이 될 수 있다)은 이 층이 아니라
    ///   부르는 쪽의 몫이고, 호출부가 그 사실을 적는다.
    pub(crate) fn wait_drained(&self, timeout: Duration) -> Result<(), PtyError> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let target = inner.pushed_seq;
        loop {
            if inner.written_seq >= target {
                return Ok(());
            }
            // ★닫힘이 먼저다★ — 닫히면 남은 대기분은 버려지므로([`pop`]) 목표에 영영 도달하지 않는다.
            //   그 사유가 곧 「왜 못 나갔나」라서 그대로 싣는다(쓰기 실패였다면 그 문구가 여기 온다).
            if let Some(reason) = &inner.closed {
                return Err(PtyError::WriteFailed(format!(
                    "쓰기 확인 실패 — 통로가 닫혔다: {reason}"
                )));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(PtyError::WriteFailed(format!(
                    "쓰기 확인 시한({timeout:?})을 넘겼다 — {} 덩이가 아직 안 나갔다(취소하지는 않았다)",
                    target - inner.written_seq
                )));
            }
            let (guard, _timed_out) = self
                .wake
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(|p| p.into_inner());
            inner = guard;
        }
    }

    /// 대기 중인 페이로드 바이트 — 시험대가 상한 판정을 재는 창.
    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).bytes
    }
}

/// 전담 라이터 스레드의 **본체**. 큐가 닫힐 때까지 FIFO 로 빼서 `write` 에 넘긴다.
///
/// ★`impl FnMut` 로 쓰기 대상을 추상화한 이유★: PTY 는 라이터 스레드가 `Box<dyn Write + Send>` 를
///   **소유**해야 하고(portable-pty 의 writer 는 한 번만 take 할 수 있으며 `Sync` 가 아니다), stdio 는
///   `shutdown()` 의 `try_lock` 규율 때문에 `Mutex<Option<ChildStdin>>` 를 **공유**해야 한다 — 두 쓰기
///   대상의 모양이 달라 공통 타입이 없다. 클로저로 받으면 두 통로가 같은 루프를 쓰면서 각자의 소유
///   규율을 지키고, 시험대는 실 프로세스 없이 이 루프 자체를 몰 수 있다(ADR-0012 의 단독 검증).
/// ★쓰기 실패는 큐를 닫고 **끝낸다**★ — 파이프가 깨진 뒤의 재시도는 전부 실패하고, 계속 돌면 큐에
///   쌓이는 입력이 영영 안 나가면서 상한까지 메모리만 문다. 닫으면 다음 `send_input` 이 사유를 들고
///   `Err` 로 돌아가 부른 쪽이 재시도할지 포기할지 정할 수 있다.
/// ★★panic 도 같은 처분을 받는다 — `catch_unwind` 가 그것을 위해 있다★★: 쓰기 중 panic 하면 큐 락은
///   **poison 되지 않는다**(쓰기는 락을 놓고 한다). 그래서 이 갈래를 안 잡으면 라이터만 죽고 큐는
///   **열린 채** 남아, 이후 `send_input` 이 계속 `Ok` 를 돌려주다가 상한에 닿는 순간부터 **틀린
///   사유**(「상한 초과」)로 거절한다 — 진짜 원인은 그 로그 어디에도 없다. 그 사이 바이트는 하나도
///   안 나간다. `spawn` 실패 갈래(`pty.rs`·`stdio.rs`)가 이미 고른 규율과 같은 자리다: **라이터가
///   없으면 큐를 닫아 그 순간부터 정직하게 거절한다.**
/// ★`agent` 는 로그 귀속용★ — 이 경고들은 「받아 둔 뒤 실패한 쓰기」의 **유일한 흔적**이라, 에이전트가
///   여럿이면 누구 것인지가 없으면 흔적이 아니다(`docs/reference/logging-conventions.md` 의 형식 규약).
pub(crate) fn drain(
    queue: &InputQueue,
    label: &str,
    agent: AgentId,
    mut write: impl FnMut(&[u8]) -> std::io::Result<()>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while let Some(chunk) = queue.pop() {
            match write(&chunk) {
                // ★성공을 세는 이 한 줄이 [`InputQueue::wait_drained`] 의 유일한 진행 신호다★.
                //   같은 줄이 [`Inner::bytes`] 회계도 푼다 — 「나갔다」가 두 뜻을 함께 갖는다.
                Ok(()) => queue.mark_written(chunk.len()),
                Err(e) => {
                    // ★이 실패는 그 바이트를 보낸 호출자에게 돌아갈 길이 없다★ — 그 호출은 이미 `Ok` 를
                    //   받고 떠났다. 여기서 할 수 있는 것은 사유를 남기고 통로를 닫아 **다음** 호출부터
                    //   정직해지는 것뿐이다(모듈 헤더의 계약 문단).
                    let reason = format!("{label} 입력 쓰기 실패: {e}");
                    tracing::warn!(agent = %agent, "{reason}");
                    queue.close(&reason);
                    return;
                }
            }
        }
    }));

    if let Err(payload) = outcome {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let reason = format!("{label} 입력 라이터가 panic 했다: {msg}");
        // 레벨 = error. 이 에이전트의 입력 경로가 **복구 불가**로 죽었고 사람이 봐야 한다
        //   (`docs/reference/logging-conventions.md` 의 레벨 표 — panic 은 error).
        tracing::error!(agent = %agent, "{reason}");
        queue.close(&reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    /// 시험대의 에이전트 표식 — 이 층은 값을 쓰지 않고 로그 필드로만 싣는다.
    fn test_agent() -> AgentId {
        AgentId::nil()
    }

    /// FIFO — 넣은 순서 그대로, 덩이 경계도 그대로 나온다.
    #[test]
    fn drain_writes_chunks_in_fifo_order() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");
        queue.close("시험 종료");

        // 닫힘이 대기분보다 먼저 판정되므로(위 `pop` 계약) 닫은 뒤 drain 하면 아무것도 안 나온다 —
        // 그래서 순서를 재는 이 항목은 닫기 **전에** 라이터를 돌린다. 아래 본 항목이 그 모양이다.
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        drain(&queue, "시험", test_agent(), |b| {
            written.lock().unwrap().push(b.to_vec());
            Ok(())
        });
        assert!(
            written.lock().unwrap().is_empty(),
            "닫힌 뒤에는 남은 대기분을 쓰지 않는다(계약)"
        );

        // 본 항목 — 라이터를 먼저 띄우고, 쓴 뒤에 닫는다.
        let queue = Arc::new(InputQueue::new());
        let written: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = written.clone();
        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |b| {
                seen.lock().unwrap().push(b.to_vec());
                Ok(())
            });
        });

        queue.push(b"first".to_vec()).expect("push 1");
        queue.push(b"second".to_vec()).expect("push 2");
        queue.push(b"third".to_vec()).expect("push 3");

        let deadline = Instant::now() + Duration::from_secs(5);
        while written.lock().unwrap().len() < 3 {
            assert!(Instant::now() < deadline, "5s 안에 3 덩이가 안 나갔다");
            std::thread::sleep(Duration::from_millis(5));
        }
        queue.close("시험 종료");
        handle.join().expect("라이터 스레드 panic");

        let got = written.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
            "FIFO 순서·덩이 경계 보존"
        );
    }

    /// 상한 초과는 **거절**이지 유실이 아니다 — 큐 내용이 그대로 남는다.
    #[test]
    fn push_over_the_bound_is_refused_not_dropped() {
        let queue = InputQueue::new();
        let half = INPUT_QUEUE_MAX_BYTES / 2;
        queue.push(vec![b'a'; half]).expect("절반은 들어간다");
        queue
            .push(vec![b'b'; half])
            .expect("나머지 절반도 들어간다");
        assert_eq!(queue.queued_bytes(), INPUT_QUEUE_MAX_BYTES);

        let err = queue
            .push(vec![b'c'; 1])
            .expect_err("상한을 1 바이트라도 넘으면 거절");
        match err {
            PtyError::WriteFailed(msg) => {
                assert!(msg.contains("상한"), "사유가 상한임을 말해야: {msg}");
                assert!(
                    msg.contains("받지 않았다"),
                    "거절이지 유실이 아님을 말해야: {msg}"
                );
            }
            other => panic!("WriteFailed 여야: {other:?}"),
        }
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "거절이 앞서 받아 둔 것을 건드리지 않는다"
        );

        // 하나가 **실제로 나가면** 그만큼 다시 받는다 — 상한이 영구 사망 선고가 아님.
        // ★꺼내는 것만으로는 자리가 안 난다★ — 그 회계의 정본과 회귀망은 아래
        //   `in_flight_bytes_still_count_against_the_bound`.
        let popped = queue.pop().expect("하나 꺼내기");
        assert_eq!(popped.len(), half);
        queue.mark_written(popped.len());
        queue.push(vec![b'c'; 1]).expect("자리가 나면 다시 받는다");
    }

    /// ★꺼냈지만 아직 안 나간 덩이도 상한에 **센다**★ — 안 세면 천장이 선언값의 두 배가 된다.
    ///
    /// ★이것이 재는 실제 사고★: 라이터가 덩이 하나를 꺼내 블로킹 OS 쓰기에 물린다(자식이 stdin 을 안
    ///   읽는 그 상황이 이 큐의 존재 이유다). 그 동안 그 덩이를 0 으로 세면 생산자가 상한만큼 **또**
    ///   넣을 수 있고, 한 에이전트가 무는 메모리가 `INPUT_QUEUE_MAX_BYTES` 가 아니라 그 두 배가 된다.
    /// ★시한·스레드가 없다★ — `pop` 과 `push` 만으로 그 창을 만들 수 있어 결정적이다.
    #[test]
    fn in_flight_bytes_still_count_against_the_bound() {
        let queue = InputQueue::new();
        queue
            .push(vec![b'a'; INPUT_QUEUE_MAX_BYTES])
            .expect("상한 전량은 들어간다");

        let in_flight = queue.pop().expect("라이터가 하나 꺼낸다");
        assert_eq!(in_flight.len(), INPUT_QUEUE_MAX_BYTES);
        assert_eq!(
            queue.queued_bytes(),
            INPUT_QUEUE_MAX_BYTES,
            "꺼냈다고 회계가 풀리면 안 된다 — 아직 OS 로 나가지 않았다"
        );

        let err = queue
            .push(vec![b'b'; 1])
            .expect_err("쓰는 중인 덩이를 0 으로 세면 여기서 상한이 두 배가 된다");
        match err {
            PtyError::WriteFailed(msg) => assert!(msg.contains("상한"), "{msg}"),
            other => panic!("WriteFailed 여야: {other:?}"),
        }

        // 실제로 나간 뒤에야 자리가 난다.
        queue.mark_written(in_flight.len());
        assert_eq!(queue.queued_bytes(), 0);
        queue.push(vec![b'b'; 1]).expect("나간 뒤엔 받는다");
    }

    /// ★라이터가 panic 해도 큐는 **닫힌다**★ — 안 닫으면 그 뒤 `send_input` 이 계속 `Ok` 를 돌려주다가
    /// 상한에 닿는 순간부터 **틀린 사유**(「상한 초과」)로 거절한다. 진짜 원인은 어디에도 안 남는다.
    ///
    /// ★락 poison 이 대신해 주지 않는다★: 쓰기는 큐 락을 **놓고** 하므로(`pop` 반환 뒤) panic 해도
    ///   `Mutex` 는 멀쩡하다 — 그래서 `catch_unwind` 없이는 이 전이가 일어날 자리가 없다.
    #[test]
    fn a_panicking_writer_closes_the_queue_with_the_panic_as_the_reason() {
        let queue = Arc::new(InputQueue::new());
        queue.push(b"doomed".to_vec()).expect("push");

        let writer_queue = queue.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |_| {
                panic!("라이터 폭발");
            });
        });
        handle
            .join()
            .expect("drain 이 panic 을 흡수해 라이터 스레드가 정상 종료해야");

        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("panic"), "사유가 panic 임을 말해야: {msg}");
                assert!(msg.contains("라이터 폭발"), "원 payload 보존: {msg}");
                assert!(
                    !msg.contains("상한"),
                    "상한 초과로 둔갑하면 진단이 반대 방향으로 간다: {msg}"
                );
            }
            other => panic!("panic 뒤 push 는 닫힌 큐의 WriteFailed 여야: {other:?}"),
        }
    }

    /// 단일 페이로드가 홀로 상한을 넘어도 같은 처분(빈 큐라도 통과시키지 않는다).
    #[test]
    fn single_oversized_payload_is_refused_on_an_empty_queue() {
        let queue = InputQueue::new();
        assert!(queue.push(vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1]).is_err());
        assert_eq!(queue.queued_bytes(), 0, "거절된 것은 쌓이지 않는다");
    }

    /// ★라이터 종료 계약★: `close` 가 `drain` 을 끝낸다 — 대기 중이든 쓰는 중이든.
    /// (`shutdown()`·pump 종료가 부르는 그 동사다.)
    #[test]
    fn close_terminates_the_drain_loop() {
        let queue = Arc::new(InputQueue::new());
        let finished = Arc::new(AtomicBool::new(false));

        let writer_queue = queue.clone();
        let writer_finished = finished.clone();
        let handle = std::thread::spawn(move || {
            drain(&writer_queue, "시험", test_agent(), |_| Ok(()));
            writer_finished.store(true, Ordering::Release);
        });

        // 라이터가 확실히 대기에 들어가도록 잠깐 양보.
        std::thread::sleep(Duration::from_millis(50));
        assert!(!finished.load(Ordering::Acquire), "닫기 전엔 안 끝난다");

        queue.close("시험 종료");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "close 후 5s 안에 라이터가 안 끝났다 — shutdown 이 스레드를 못 거둔다"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.join().expect("라이터 스레드 panic");
        assert!(finished.load(Ordering::Acquire));
    }

    /// 닫힌 뒤의 `push` 는 **닫은 사유를 들고** `Err` 로 돌아간다.
    #[test]
    fn push_after_close_reports_the_close_reason() {
        let queue = InputQueue::new();
        queue.close("에이전트를 종료했다");
        match queue.push(b"late".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("에이전트를 종료했다"), "사유 전달: {msg}")
            }
            other => panic!("닫힌 큐는 WriteFailed 여야: {other:?}"),
        }
    }

    /// 쓰기 실패는 큐를 닫고 라이터를 끝낸다 — 그 뒤 `send_input` 은 그 사유로 거절된다.
    #[test]
    fn write_failure_closes_the_queue_with_its_reason() {
        let queue = InputQueue::new();
        queue.push(b"doomed".to_vec()).expect("push");
        drain(&queue, "시험", test_agent(), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "파이프가 끊겼다",
            ))
        });
        match queue.push(b"after".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => {
                assert!(msg.contains("입력 쓰기 실패"), "사유 전달: {msg}");
                assert!(msg.contains("파이프가 끊겼다"), "원 오류 보존: {msg}");
            }
            other => panic!("쓰기 실패 뒤 push 는 WriteFailed 여야: {other:?}"),
        }
    }

    /// 첫 사유가 이긴다 — 나중 닫기가 진단을 덮어쓰지 않는다.
    #[test]
    fn first_close_reason_wins() {
        let queue = InputQueue::new();
        queue.close("진짜 원인");
        queue.close("나중에 온 종료");
        match queue.push(b"x".to_vec()) {
            Err(PtyError::WriteFailed(msg)) => assert!(msg.contains("진짜 원인"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }
}
