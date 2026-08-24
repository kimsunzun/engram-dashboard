//! 데몬 클라이언트의 pending 슬롯 승계(`daemon_client::connection::register_pending`) 통합 테스트 —
//! 소켓·데몬·Tauri 0.
//!
//! ★이 파일이 `tests/`(통합 타깃)에 있는 이유★: 재는 대상이 **실 소켓을 요구하는 배선**이라, 그 자리를
//! 포트로 끊어 세우는 통합 하네스가 제자리다(그래서 하네스 자신은 소켓·데몬·Tauri 를 하나도 안 쓴다).
//! ★"요구한다"이지 "못 세운다"가 아니다★ — `src/daemon_client/tests.rs` 가 루프백 WS 로 그 배선을 실제로
//! 돌린다. 여기서 사는 것은 소켓 없이 얻는 결정론과 속도이지 유일한 가능성이 아니다.
//! 순수 단위 단언은 모듈 옆 `#[cfg(test)]` 로 가고 그쪽은 `--test lib_unit` 으로 돈다
//! (그 타깃을 세운 결정 = ADR-0174 · 현황 = CLAUDE.md 「빌드·검증 명령」).
//! 실행: `cargo test -p engram-dashboard --test daemon_client_pending`(자식 프로세스를 하나도 안
//! 띄우므로 `-- --test-threads=4` 를 붙이지 않는다 — 판정 규칙 정본 = CLAUDE.md 「빌드·검증 명령」).
//! ★워크스페이스 회귀에 안 실린다★ — 그 명령이 `--exclude engram-dashboard` 로 이 패키지를 통째로 뺀다.
//! 그래서 CI가 이 타깃만 따로 부르는 전용 스텝을 갖는다(`.github/workflows/ci.yml`).
//!
//! ## ★무엇을 지키나 — 겹친 번호로 연결이 죽지 않는다★
//! 이 맵의 키(`request_id`)는 더 이상 우리가 만드는 값이 아니다. `AgentCommand::Command { envelope }` 의
//! 번호는 **봉투를 지은 바깥 호출자**의 것이고, 웹뷰가 `forward_daemon_command` 로 그대로 실어 보낸다.
//! 겹친 번호가 옛 코드의 `debug_assert!(false)` 를 때리면 연결 태스크가 패닉해 **그 뒤 모든 명령이 끊기고
//! 재연결도 없다**(디버그 빌드 GUI 실측 2026-08-18 — 화면에서 같은 번호로 `tab.list` + `window.list`).
//! 그래서 여기서 재는 것은 셋이다: 패닉하지 않는다 · 옛 대기자가 **오류로 깨어난다**(영구 hang 금지) ·
//! 새 요청이 그 번호를 **승계한다**.
//!
//! ★패닉 단언을 따로 안 쓰는 이유★: `debug_assert` 가 되살아나면 이 타깃(디버그 빌드 = `debug_assertions`
//! on)의 그 테스트가 **그 자리에서 죽는다**. 즉 아래 단언들이 곧 패닉 회귀 감지기다.
//!
//! ## ★무엇이 안 덮이나(정직성)★
//! 연결 태스크의 `select!` arm 자체(소켓에서 온 명령을 이 함수까지 나르는 배선)는 실 소켓을 요구해
//! 여기서 안 돈다 — `tests/layout_commands.rs` 헤더가 적는 같은 잔여다. 여기서 재는 것은
//! 그 arm 이 부르는 **규칙 본체**다.

use engram_dashboard_lib::daemon_client::connection::{
    register_pending, CommandReply, PENDING_SUPERSEDED,
};
use engram_dashboard_lib::daemon_client::protocol_state::{take_pending, PendingMap};
use engram_dashboard_protocol::{AgentEvent, RequestId};
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;

// 연결 태스크가 들고 도는 그 맵과 같은 타입 — 값은 요청/응답 상관용 oneshot(`connection::CommandReply`).
type Pending = PendingMap<CommandReply>;

// 이 하네스의 세대값 — 로그 필드로만 쓰이므로 아무 값이나 되지만, 고정해 두면 실패 로그가 읽힌다.
const GEN: u64 = 7;

// 슬롯 하나를 걸고 그 요청의 대기자(수신단)를 돌려준다.
fn register(
    pending: &mut Pending,
    rid: RequestId,
) -> oneshot::Receiver<Result<AgentEvent, String>> {
    let (tx, rx) = oneshot::channel();
    register_pending(pending, rid, tx, GEN);
    rx
}

// 아직 답장 전 = 슬롯이 살아 있는 채 대기 중(`Closed` 와 구분한다 — 그쪽은 슬롯이 사라진 것이다).
fn assert_still_waiting(rx: &mut oneshot::Receiver<Result<AgentEvent, String>>, what: &str) {
    match rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        other => panic!("{what} 는 답장 전까지 살아서 대기해야 한다: {other:?}"),
    }
}

// ── 승계 ────────────────────────────────────────────────────────────────────

/// ★핵심 회귀★ 같은 번호로 두 요청이 들어와도 (1) 죽지 않고 (2) 옛 대기자가 오류로 깨어나며
/// (3) 새 요청이 그 번호를 승계한다.
///
/// 옛 코드는 이 자리에서 `debug_assert!(false)` 로 패닉했다 — 그 패닉이 연결 태스크를 통째로 접어
/// 셸↔데몬이 영구 단절됐다. 되살리면 이 테스트가 (1)에서 죽는다.
#[test]
fn duplicate_request_id_supersedes_instead_of_panicking() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    let mut old = register(&mut pending, rid);
    let mut new = register(&mut pending, rid); // 같은 번호 — 바깥 호출자가 정한 값이라 겹칠 수 있다.

    // (2) 옛 대기자는 **오류로 깨어난다** — 조용히 떨어뜨리면 그 호출자가 영구 hang 한다.
    match old.try_recv() {
        Ok(Err(msg)) => assert_eq!(
            msg, PENDING_SUPERSEDED,
            "옛 대기자가 받는 문구는 그 상수 하나여야 한다"
        ),
        other => panic!("옛 대기자는 Err 로 깨어나야 한다: {other:?}"),
    }
    assert_still_waiting(&mut new, "새 대기자");

    // (3) 맵에 남은 슬롯은 **하나**이고 그것이 새 요청의 것이다 — 그 번호의 답장이 새 대기자를 푼다.
    assert_eq!(pending.len(), 1, "같은 번호는 한 칸을 나눠 쓴다");
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯이 그 번호로 잡혀야 한다")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자가 살아 있어야 한다");
    match new.try_recv() {
        Ok(Ok(AgentEvent::Ack { request_id })) => {
            assert_eq!(request_id, rid, "승계한 쪽이 그 번호의 답장을 받는다")
        }
        other => panic!("새 대기자가 답장을 받아야 한다: {other:?}"),
    }
}

/// 승계 뒤 **같은 번호의 답장이 또 와도** 매달리는 쪽이 없다 — 두 번째는 짝 없는 답장으로 무시된다.
///
/// 데몬은 겹친 번호에 타입 있는 답을 짓는다(`command_delivery` 의 `Seat::Conflict` →
/// `REQUEST_ID_CONFLICT`, `Seat::Taken` → `REQUEST_ID_CONFLICT`/`OUTCOME_UNKNOWN`). 그 답이 이쪽 한 칸으로
/// 돌아오므로, 한 장이 슬롯을 푼 뒤 나머지는 갈 곳이 없다는 것이 이 경로의 정상 상태다.
#[test]
fn second_reply_for_same_id_finds_no_slot() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    let mut old = register(&mut pending, rid);
    let mut new = register(&mut pending, rid);

    assert!(
        matches!(old.try_recv(), Ok(Err(_))),
        "옛 대기자는 이미 깨어났다"
    );
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자 생존");
    assert!(
        matches!(new.try_recv(), Ok(Ok(_))),
        "첫 답장이 새 대기자를 푼다"
    );

    // 두 번째 답장 — 꺼낼 슬롯이 없다(무시). 여기서 옛 슬롯이 되살아나면 안 된다.
    assert!(
        take_pending(&mut pending, &rid).is_none(),
        "짝 없는 답장은 꺼낼 슬롯이 없다"
    );
    assert!(pending.is_empty(), "맵에 좀비 슬롯이 남지 않는다");
}

/// 다른 번호끼리는 서로를 깨우지 않는다 — 승계를 「항상 옛것을 버린다」로 넓히면 이 단언이 깨진다.
#[test]
fn distinct_request_ids_coexist() {
    let mut pending = Pending::new();
    let (a, b) = (RequestId::new(), RequestId::new());

    let mut wait_a = register(&mut pending, a);
    let mut wait_b = register(&mut pending, b);

    assert_eq!(pending.len(), 2, "번호가 다르면 칸도 다르다");
    assert_still_waiting(&mut wait_a, "a");
    assert_still_waiting(&mut wait_b, "b");

    // 각자 자기 번호의 답장으로만 풀린다.
    take_pending(&mut pending, &a)
        .expect("a 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: a }))
        .expect("a 대기자 생존");
    assert!(matches!(wait_a.try_recv(), Ok(Ok(_))), "a 만 풀린다");
    assert_still_waiting(&mut wait_b, "b");
}

/// 대기자가 이미 사라진(호출자 취소) 슬롯을 승계해도 조용히 진행한다 — 깨울 곳이 없다고 죽지 않는다.
#[test]
fn supersede_survives_dropped_waiter() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    drop(register(&mut pending, rid)); // 옛 대기자가 먼저 떠났다.
    let mut new = register(&mut pending, rid);

    assert_eq!(pending.len(), 1);
    assert_still_waiting(&mut new, "새 대기자");
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자 생존");
    assert!(matches!(new.try_recv(), Ok(Ok(_))));
}
