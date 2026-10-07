//! graceful 데몬 끄기 — 데몬에 WS 로 `StopDaemon` 을 일방 발사한다(트레이 「데몬 끄기」 · 앱 종료).
//!
//! ★임시 거처★ — 3-2(transport 셸 부착)가 정지 명령 클라이언트를 다시 쓴다(ADR-0271 결정 5).
//!
//! ★이 파일이 [`crate::discovery`] 에서 쓰는 내부는 이 파일을 위해 `pub(crate)` 로 연 것뿐이다★ — `check_acceptable` ·
//! `AcceptCheck` · `FileReader`(와 그 필드 `path`) · `RealLiveness`. 셸의 다른 모듈이 그것을 쓰기 시작하면 3-2 의
//! 다시 쓰기가 넓어진다.
//!
//! ★분담(daemon_stop 와의 차이)★: send_stop 은 그 위 계층의 **graceful** 경로 — 데몬에 WS 로
//! StopDaemon{force} 를 보내 데몬이 스스로 shutdown_all(자식 PTY 정리) + self-exit 하게
//! 한다(connection_core StopDaemon 핸들러가 처리).
//!
//! ★일방 발사(fire-and-forget) — 사용자 결정★: ack/응답을 읽지 않는다. 응답이 없거나 데몬이 정리
//! 중이면 데몬은 그대로 살아있고(probe 가 alive 로 보고), 사용자가 다시 누르면 재발사한다. close 전
//! flush 로 메시지가 소켓에 실제 나가는 것만 보장한다.
// ADR-0271
// ADR-0282

use std::path::Path;
use std::time::Duration;

use engram_dashboard_net::auth::AuthFrame;
use engram_dashboard_protocol::{AgentCommand, DaemonInfo, RequestId, PROTOCOL_VERSION};

use crate::discovery::{
    check_acceptable, AcceptCheck, DaemonReader, DataLayout, DiscoveryError, FileReader,
    PidLiveness, RealLiveness,
};

/// ★왜 enum 으로 끌어올리나(load-bearing)★: 끄기 직후 트레이가 `daemon_status`(PID probe)로 아이콘을
/// 정하면, 데몬이 죽기 직전 수 ms 동안 "아직 살아있음"으로 보여 **아이콘이 컬러로 고착**되는 race 가
/// 있었다(QA 실측). 해결 = PID 를 다시 묻지 않고, drain read 루프에서 관측한 **"데몬이 연결을 닫음"**
/// 을 "꺼짐 확정" 신호로 쓴다. send_stop 이 그 신호를 이 enum 으로 호출자(트레이)에게 올려, 트레이가
/// probe 우회로 아이콘을 회색 확정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// 데몬이 graceful 하게 **이 WS 연결을 닫았다**. 트레이는 probe 없이 회색 확정.
    /// ★의미 한정(과신 금지)★: 이것은 정확히는 "데몬이 StopDaemon 을 처리하고 **종료 경로에 진입**해
    /// 이 연결을 닫았다"는 신호다. 실제 프로세스 exit 는 그 직후(보통 ms)에 일어난다 — 연결 닫힘과
    /// 프로세스 소멸은 동일 순간이 아니다. 정상 경로에선 ms 차라 회색 확정이 맞지만, 데몬의 graceful
    /// 종료 자체가 hang/panic 하면 연결은 닫혔어도 프로세스가 잠깐 더 살 수 있다(그건 별도 버그 —
    /// 일방 발사 재발사 모델이 다음 클릭에서 회수). 이 신호를 "프로세스 죽음 확정"으로 더 신뢰해
    /// probe 폴백을 추가로 제거하지 말 것.
    DaemonClosed,
    /// STOP_WS_TIMEOUT(3s) 내 데몬이 닫지도 응답하지도 않음.
    /// = 불확실(데몬이 아직 정리 중일 수 있음) → 트레이는 기존 probe 폴백.
    Timeout,
    /// 끌 데몬이 없었음(daemon.json 없음/죽음/깨짐/버전 불일치 — send 자체를 안 함).
    /// = 트레이는 기존 probe 폴백(이미 회색일 것).
    NoTarget,
}

pub trait StopSender {
    /// 살아있는 데몬 `info` 에 graceful StopDaemon 을 보낸다(Auth → StopDaemon → flush → drain → close).
    /// 일방 발사라 ack **내용**은 해석하지 않지만, drain read 의 **종료 사유**로
    /// [`StopOutcome::DaemonClosed`](연결 닫힘=꺼짐 확정) / [`StopOutcome::Timeout`](3s 무응답)을 구분해
    /// 반환한다. 송신/연결 실패만 Err.
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError>;
}

/// 데몬에 graceful StopDaemon 을 WS 로 보낸다(real 진입점, S13 sub-step 2).
///
/// ★나중에 이어붙일 자리(load-bearing)★: 이 send_stop 경로엔 taskkill(daemon_stop) 강제 폴백이
/// 없다(사용자 결정: 응답 없으면 데몬 활성 유지, 다시 누르면 재발사). 나중에 graceful-with-fallback
/// 으로 키우려면 **이 함수(또는 TungsteniteStopSender::send_stop) 안에** "Timeout 시
/// daemon_stop(data_dir) 호출"을 추가하면 된다 — 호출부는 send_stop 시그니처만 보므로 폴백 자체는
/// 여기서 흡수.
pub fn send_stop(data_dir: &Path) -> Result<StopOutcome, DiscoveryError> {
    stop_with_sender(
        &FileReader {
            path: DataLayout::new(data_dir).daemon_file(),
        },
        &RealLiveness,
        &TungsteniteStopSender,
    )
}

/// ★대상 판정 = check_acceptable(Accept)★: 버전 불일치 데몬은 어차피 데몬의 Auth 가
/// protocol_version 검사로 거부하므로 일방 발사가 무의미하고, 그런 데몬 종료는 taskkill
/// 폴백(daemon_stop)의 몫이다(미래 연결).
fn stop_with_sender(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    sender: &dyn StopSender,
) -> Result<StopOutcome, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => sender.send_stop(&info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => Ok(StopOutcome::NoTarget),
        },
        _ => Ok(StopOutcome::NoTarget),
    }
}

/// force=true·kill_agents=true 고정(작업 중 에이전트가 있어도 데몬이 정리하고 끔 — 사용자 결정).
/// request_id 는 새 Uuid(데몬이 에코하지만 우리는 ack 를 안 읽으므로 매칭에 안 씀 — 프로토콜 필수
/// 필드라 채울 뿐).
fn build_stop_command() -> AgentCommand {
    AgentCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: RequestId::new(),
    }
}

/// 데몬은 연결 1초 내 첫 프레임으로 이걸 기대한다(네트워크 lib 의 AUTH_TIMEOUT).
///
/// ★타입 출처(ADR-0129 0-4)★: 이건 **명령이 아니라 네트워크 lib 소유 프레임**이다(`AuthFrame`).
/// 데몬의 인증 판정을 하는 그 crate 가 모양의 정본을 쥐므로, 발신자가 제 손으로 JSON 을 짜는 대신
/// 같은 타입을 쓴다 — 그래야 한쪽만 바뀌는 표류가 컴파일 에러가 된다.
fn build_auth_command(token: &str) -> AuthFrame {
    AuthFrame::Auth {
        token: token.to_string(),
        protocol_version: PROTOCOL_VERSION,
    }
}

/// ★데몬 핸드셰이크와 1:1(ws.rs)★: 데몬 read_task 는 Message::Text 만 AgentCommand 로 파싱하고
/// Binary 는 거부하므로 Text 로 보낸다.
struct TungsteniteStopSender;

/// send_stop 의 connect/handshake/read/write 상한(초). 이 값을 넘으면 깔끔히 에러로 빠진다.
///
/// ★왜 timeout 이 load-bearing 인가★: 기본 `tungstenite::connect(url)` 은 내부 `TcpStream::connect`
/// 를 **timeout 없이** 호출하고 handshake read 에도 상한이 없다. daemon.json 의 pid/port 가 stale
/// 인데 그 PID 가 재사용(M2)으로 liveness 판정을 우회한 드문 경우, 닫혔거나 방화벽이 막은 포트로의
/// connect 시도가 Windows 기본 ~21초까지 블록될 수 있다 — 트레이 stop 워커 스레드가 그동안 묶여
/// 아이콘/상태 갱신이 지연된다(워커 누수에 준함). connect_timeout + set_read/write_timeout 으로
/// 모든 블로킹 구간(TCP 연결 → WS handshake read → send/flush/close)에 상한을 박아 무한 블록을 막는다.
const STOP_WS_TIMEOUT: Duration = Duration::from_secs(3);

impl StopSender for TungsteniteStopSender {
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
        use std::net::{SocketAddr, TcpStream};
        use tungstenite::{Error as WsError, Message};

        // ws://host:port — 데몬은 로컬 평문 WS(TLS 없음, ws:// 고정). host 는 항상 127.0.0.1 loopback.
        let url = format!("ws://{}:{}", info.host, info.port);

        let addr: SocketAddr = format!("{}:{}", info.host, info.port)
            .parse()
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 주소 파싱 실패({url}): {e}")))?;
        let stream = TcpStream::connect_timeout(&addr, STOP_WS_TIMEOUT)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS 접속 실패({url}): {e}")))?;
        stream
            .set_read_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon read timeout 설정 실패: {e}")))?;
        stream
            .set_write_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon write timeout 설정 실패: {e}")))?;
        // 요청은 URL 뿐이라 HandshakeError Display 에도 token 은 들어가지 않는다.
        let (mut ws, _resp) = tungstenite::client(&url, stream)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS handshake 실패({url}): {e}")))?;

        let auth = serde_json::to_string(&build_auth_command(&info.token))
            .map_err(|e| DiscoveryError::Io(format!("Auth 직렬화 실패: {e}")))?;
        ws.send(Message::Text(auth.into()))
            .map_err(|e| DiscoveryError::Io(format!("Auth 전송 실패: {e}")))?;

        let stop = serde_json::to_string(&build_stop_command())
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 직렬화 실패: {e}")))?;
        ws.send(Message::Text(stop.into()))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 전송 실패: {e}")))?;

        // ★flush 로 두 프레임을 소켓에 실제 밀어낸다★ — tungstenite send 는 내부 버퍼링이라
        // flush 없이 곧장 close 하면 미전송 가능. 일방 발사의 "도달 보장"이 이 flush 다.
        ws.flush()
            .map_err(|e| DiscoveryError::Io(format!("WS flush 실패: {e}")))?;

        // ★drain read — 즉시 close 금지(QA 실측 회귀 수정)★:
        //    flush 직후 곧장 ws.close() 하면 데몬 write_task 가 닫힌 소켓에 outbound(Hello 등)를 write
        //    하다 os error 10053 으로 실패 → write_task 종료 → 데몬의 "한쪽 끝나면 상대 abort"(ws.rs)로
        //    read_task 가 StopDaemon 을 read 하기 전에 abort → StopDaemon dispatch 안 됨 → 데몬이
        //    graceful self-exit 못 함(생존). 즉시 close 가 데몬에게서 "StopDaemon 을 read 하고 처리할
        //    시간"을 뺏는 게 결함이었다. 그래서 데몬이 self-exit 로 연결을 닫을 때까지(또는 read_timeout
        //    3s) 소켓에서 read 를 돌려 처리 시간을 준다.
        //    "받기"가 아니라 "데몬에 시간 주기"로 read 를 도는 것이다. 3s 상한이라 데몬이 안 죽어도
        //    send_stop 은 최대 3s 후 반환(connect timeout 과 같은 워커 블록 bound).
        let outcome = loop {
            match ws.read() {
                Ok(Message::Close(_)) => break StopOutcome::DaemonClosed,
                Ok(_) => {}
                Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                    break StopOutcome::DaemonClosed
                }
                Err(WsError::Io(e)) => {
                    use std::io::ErrorKind;
                    match e.kind() {
                        ErrorKind::WouldBlock | ErrorKind::TimedOut => break StopOutcome::Timeout,
                        // EOF/연결 끊김 — 데몬 프로세스가 사라져 소켓이 끊김 → 꺼짐 확정.
                        _ => break StopOutcome::DaemonClosed,
                    }
                }
                // 그 외 WS 에러(프로토콜/Utf8 등) — 더 받을 게 없으니 종료하되, 데몬이 닫았다고 단정할 수
                // 없어 Timeout(불확실)으로 본다(probe 폴백으로 안전하게 회수).
                Err(_) => break StopOutcome::Timeout,
            }
        };

        // 데몬이 이미 닫았으면 무해하고, 안 닫았어도 drop 으로 닫힌다 — 명시적 close 는 best-effort.
        let _ = ws.close(None);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::discovery::tests::{info, FakeLiveness, FakeReader};

    // ── send_stop (graceful StopDaemon WS 일방 발사) — 순수 판정/조립 ──────────────────
    //
    // 실 WS 왕복은 QA(실 데몬) 영역. 여기선 (1) 대상 판정(어떤 데몬에 보내고 안 보내는지), (2) 보낼
    // 메시지 조립(Auth/StopDaemon 직렬화 형태)을 StopSender fake 로 검증한다.

    struct CountingStopSender {
        sent: RefCell<Vec<u32>>,
    }
    impl CountingStopSender {
        fn new() -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl StopSender for CountingStopSender {
        fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
            self.sent.borrow_mut().push(info.pid);
            Ok(StopOutcome::DaemonClosed)
        }
    }

    #[test]
    fn send_stop_live_daemon_sends() {
        let reader = FakeReader::new(vec![Ok(Some(info(1001, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(sender.sent.borrow().as_slice(), &[1001], "live 면 1회 발사");
        assert_eq!(
            outcome,
            StopOutcome::DaemonClosed,
            "sender 결과를 그대로 전파"
        );
    }

    #[test]
    fn send_stop_dead_daemon_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1002, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![1002] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "죽은 데몬 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "죽은 데몬엔 graceful stop 안 보냄"
        );
    }

    #[test]
    fn send_stop_missing_file_is_noop() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_corrupt_file_is_noop() {
        let reader = FakeReader::new(vec![Err(DiscoveryError::Parse("bad".into()))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).expect("깨진 파일은 no-op Ok");
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_version_mismatch_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1003, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "버전 불일치 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "버전 불일치는 graceful 대상 아님"
        );
    }

    #[test]
    fn send_stop_propagates_sender_outcome_timeout() {
        struct TimeoutSender;
        impl StopSender for TimeoutSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Ok(StopOutcome::Timeout)
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1005, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let outcome = stop_with_sender(&reader, &liveness, &TimeoutSender).unwrap();
        assert_eq!(
            outcome,
            StopOutcome::Timeout,
            "live 데몬 + sender Timeout → Timeout 전파"
        );
    }

    #[test]
    fn send_stop_propagates_sender_error() {
        struct FailingSender;
        impl StopSender for FailingSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Err(DiscoveryError::Io("send boom".into()))
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1004, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let err = stop_with_sender(&reader, &liveness, &FailingSender).unwrap_err();
        assert!(matches!(err, DiscoveryError::Io(_)), "{err:?}");
    }

    #[test]
    fn build_stop_command_is_force_kill_stopdaemon() {
        // 데몬 read_task 의 serde_json::from_str 이 파싱할 형태를 못 박는다.
        match build_stop_command() {
            AgentCommand::StopDaemon {
                force, kill_agents, ..
            } => {
                assert!(force, "force=true(작업 중 에이전트 있어도 끔)");
                assert!(kill_agents, "kill_agents=true");
            }
            other => panic!("StopDaemon 이 아님: {other:?}"),
        }
        let json = serde_json::to_string(&build_stop_command()).unwrap();
        assert!(
            json.contains("StopDaemon"),
            "externally-tagged 태그: {json}"
        );
        assert!(json.contains("\"force\":true"));
        assert!(json.contains("\"kill_agents\":true"));
    }

    #[test]
    fn build_auth_command_carries_token_and_version() {
        // ★단일 variant 라 반증 불가 패턴이다(ADR-0129 0-4)★ — 옛 `other => panic!` 갈래는 이제 존재할
        //   수 없는 상태라 지웠다(단언이 약해진 게 아니라 컴파일러가 대신 보증한다).
        let token = "f".repeat(64);
        let AuthFrame::Auth {
            token: t,
            protocol_version,
        } = build_auth_command(&token);
        assert_eq!(t, token);
        assert_eq!(protocol_version, PROTOCOL_VERSION);

        // wire 형태 — 태그 존재만이 아니라 **바이트 전체**를 못 박는다. 이 프레임을 받는 쪽(네트워크 lib)이
        // 같은 문자열을 golden 으로 들고 있고, 데몬은 이 crate 의 타입을 쓰지 않으므로 둘을 잇는 것은
        // 이 형태뿐이다(트레이 stop 이 조용히 인증에 실패하면 데몬이 안 꺼진다).
        let json = serde_json::to_string(&build_auth_command(&token)).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"Auth":{{"token":"{token}","protocol_version":{PROTOCOL_VERSION}}}}}"#),
            "핸드셰이크 wire 형태(externally-tagged)"
        );
    }
}
