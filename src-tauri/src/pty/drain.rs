//! drain thread — PTY reader에서 출력을 읽어 replay 저장 + 구독자에게 전달.
//!
//! OS thread(std::thread)를 쓴다. blocking read와 자연스럽게 맞고, tokio runtime을
//! PTY I/O로 점유하지 않기 위함이다. tauri import 0, unsafe 0.
//!
//! ★핵심 불변식 (LLD §10 규칙3)★
//! - `sink.send()` 호출 시 어떤 session lock도 보유하지 않는다.
//!   subscribers를 clone()으로 스냅샷 뜨고 lock을 즉시 해제한 뒤, 복사본을 돌며 send.
//! - replay lock과 subscribers lock을 동시에 보유하지 않는다(각각 짧게).
//!   두 lock 동시 취득은 subscribe 함수 단독 예외이며 drain은 절대 금지.

use std::io::Read;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use base64::Engine as _;

use crate::pty::session::PtySession;
use crate::pty::types::{AgentStatus, OutputChunk, PtyEvent, StatusSink};

/// drain thread 기동. reader는 master.try_clone_reader() 결과(spawn 직후 호출).
/// status_sink/done_tx는 PtySession에 없으므로 manager가 보유분을 넘긴다.
pub fn spawn_drain_thread(
    session: Arc<PtySession>,
    reader: Box<dyn Read + Send>,
    status_sink: Arc<dyn StatusSink>,
    done_tx: Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        drain_loop(&session, reader);

        // 루프 탈출(EOF/Err/shutdown 무엇이든) → terminal 전이 1회 + 알림.
        let new_status = transition(&session);
        status_sink.status_changed(session.id, new_status, session.epoch);

        // G-1: 완료 신호. kill_agent의 recv_timeout(5s)가 이걸 받는다.
        // 수신측이 이미 사라졌어도(타임아웃 후 detach) 무시.
        let _ = done_tx.send(());
    })
}

fn drain_loop(session: &Arc<PtySession>, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; 4096];

    loop {
        // 1. blocking read — read 자체가 자연 배칭 역할. EOF(master drop) 또는 Err로 깨면 종료.
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };

        // 2. shutdown 보조 확인. 보통 master drop으로 인한 EOF가 먼저 깨우지만,
        //    드물게 read가 데이터를 막 반환한 직후 kill이 걸린 경우를 위한 안전망.
        if session.shutdown.load(Ordering::Relaxed) {
            break;
        }

        // 3. seq 발급 + 이벤트 구성 (C2: 즉시 send — partial batch 정체 없음).
        let seq = session.seq.fetch_add(1, Ordering::Relaxed);
        let data = buf[..n].to_vec();
        let event = PtyEvent {
            agent_id: session.id,
            seq,
            data_b64: base64::engine::general_purpose::STANDARD.encode(&data),
        };

        // 4. replay 저장 — brief lock. 여기서 subscribers lock은 잡지 않는다(불변식 2).
        session
            .replay
            .lock()
            .expect("replay poisoned")
            .push(OutputChunk { seq, data });

        // 5. ★불변식 1★ subscribers를 clone으로 스냅샷 뜨고 즉시 lock 해제 → lock 밖에서 send.
        //    send는 blocking 가능(IPC). lock을 쥔 채 send하면 subscribe/다른 send와 교착·정체.
        let sinks = session
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();

        let mut dead = Vec::new();
        for sink in sinks {
            if sink.send(event.clone()).is_err() {
                dead.push(sink.sink_id());
            }
        }

        // 6. 죽은 구독자 제거 — 다시 짧게 lock. (clone 시점 이후 새로 붙은 sink는 건드리지 않음)
        if !dead.is_empty() {
            session
                .subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }
}

/// terminal 상태 전이 (LLD §9, M5: 전이 주체는 drain thread 단독).
/// shutdown flag가 켜져 있으면 Killed, 아니면 child exit code로 Exited{code}.
///
/// **race 방지(중요):** shutdown 판정과 상태 기록을 같은 status lock 구간 안에서 한다.
/// exit code는 lock 밖에서 미리 취득하고(child lock과 status lock을 겹쳐 잡지 않기 위함),
/// status lock을 잡은 뒤 shutdown을 Acquire로 읽어 Killed/Exited를 정한다.
/// 또한 이미 terminal(Exited/Killed/Failed)이면 덮어쓰지 않는다 — kill_agent가 Exiting을
/// 쓰는 경로와 경합해 terminal이 Exiting으로 고착되는 것을 막는다.
fn transition(session: &Arc<PtySession>) -> AgentStatus {
    // child exit code는 status lock 밖에서 먼저 취득(두 lock 겹침 회피).
    // Killed 판정 시엔 안 쓰지만, lock 안에서 판정이 갈리므로 미리 확보해 둔다.
    let code = {
        let mut child = session.child.lock().expect("child poisoned");
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            _ => None,
        }
    };

    let mut status = session.status.lock().expect("status poisoned");

    // 이미 terminal이면 그대로 둔다(idempotent — 중복/경합 전이 방지).
    if matches!(
        *status,
        AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
    ) {
        return status.clone();
    }

    // shutdown store(Release)와 페어링되도록 Acquire로 읽는다.
    let new_status = if session.shutdown.load(Ordering::Acquire) {
        AgentStatus::Killed
    } else {
        AgentStatus::Exited { code }
    };

    *status = new_status.clone();
    new_status
}
