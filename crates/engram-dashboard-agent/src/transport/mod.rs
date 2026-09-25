//! AgentTransport — 에이전트 1개의 데이터 채널 + 자원 제어 seam.
//!
//! transport는 바이트·이벤트를 만들어 `OutputCore::emit`/`finish`로 넘기기만 하면 된다.
//! 출력 fanout·종료 전이·구독 같은 공용 로직은 OutputCore가 담당하고, transport는
//! 자기 자원의 수명만 책임진다.
//!
//! tauri import 0.

use std::sync::Arc;
use std::time::Duration;

use crate::output_core::OutputCore;
use crate::types::{InputEvent, OutputEvent, PtyError, TransportCaps, TurnInput, Withdraw};

pub mod api;
pub mod input_queue;
pub mod pty;
pub mod stdio;

/// 출력 바이트 → OutputEvent 정제 seam (backend-agnostic — ADR-0004/0045).
///
/// ★왜 이 트레이트가 필요한가(ADR-0004 격리)★: transport(StdioTransport)는 **바보 파이프**라
///   자식 stdout 바이트가 무슨 스키마인지(claude stream-json / codex 프로토콜 / 평문) 몰라야 한다.
///   그런데 json 모드는 그 바이트를 구조화 OutputEvent 로 정제해야 한다 — 그 파싱 지식은 backend
///   소유다(claude 라면 `ClaudeStreamDecoder`, backend/claude/ 단독). 그래서 파싱 로직을 이
///   트레이트 뒤에 숨겨 **transport 는 "어떤 디코더인지 모른 채" 주입받아 적용만** 한다.
///
/// ★수명·상태(pump 스레드 단독 소유)★: decoder 는 라인 재조립을 위해 부분 라인 버퍼 등 **가변
///   상태**를 들고, pump 스레드(단일)가 `&mut` 로 배타 소유한다 — 그래서 `Send`(스레드로 move)만
///   요구하고 `Sync` 는 요구하지 않는다(공유 접근 없음). epoch 교체 = 새 transport = 새 decoder 라
///   리셋이 자동이다.
///
/// agent 도메인 타입(OutputEvent)만 생성한다 — Serialize 무관(ADR-0003: agent 는 wire 를 모른다).
pub trait OutputDecoder: Send {
    /// 임의 크기 바이트 청크를 밀어 넣고, 이번 청크로 **완성된** 이벤트만 돌려준다.
    /// 미완성 꼬리(개행 없는 부분 라인 등)는 구현체가 내부 버퍼에 남겨 다음 청크와 합친다.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent>;

    /// 스트림 종료 시 최대 1회 호출 — 개행으로 종단되지 않은 잔여 라인을 마저 처리한다.
    fn flush(&mut self) -> Vec<OutputEvent>;
}

// ★PTY 출력 바이트를 backend 클로저에 흘리는 seam 을 여기 되살리지 말 것(ADR-0217/ADR-0218)★ —
//   유일 소비자가 codex 의 TUI 상태줄 스캐너였고, 그 회수가 화면 읽기에서 **락 소유 판정**으로
//   옮겨 가며 함께 걷혔다(오늘 그 회수는 통로를 아예 지나지 않는다). 되살리면 pump(유일한 PTY
//   리더) 위에서 남의 코드가 도는 구조가 돌아온다 — 거기서 락을 잡거나 panic 하면 각각 자식
//   역압과 「멀쩡한 세션이 Failed 로 보고」가 된다.
// ADR-0217
// ADR-0218

pub trait AgentTransport: Send + Sync {
    /// 출력 pump/stream 기동 → core 연결. spawn 직후 1회 호출.
    fn start(&self, core: Arc<OutputCore>);

    /// ★수령 의미는 **「받았다」이지 「갔다」가 아니다**★: `Ok` = 전량을 순서까지 확정해 받았다,
    ///   `Err` = 받지 않았다(부분 수용을 `Ok` 로 축소 보고하지 않는다). 오늘 구현체 셋이 전부 유계 큐에
    ///   담고 전담 라이터가 빼서 쓰므로, **이 호출은 OS 쓰기를 기다리지 않는다** — 그것이 이 seam 의
    ///   요점이다(자식이 stdin 을 안 읽어도 부른 쪽이 함께 매달리지 않는다).
    /// ★그 대가 = 받아 둔 뒤에 실패한 쓰기는 이 호출자에게 돌아갈 길이 없다★. 그 실패가 나타나는 곳은
    ///   **다음 호출의 `Err`** 와 로그, 그리고 대개 곧 이어지는 종점 전이다. 계약·상한·처분의 정본은
    ///   [`input_queue`] 모듈 헤더(콘솔 계열)와 codex 통로의 `send_input` doc 이다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;

    /// 턴 하나를 출처와 함께 넘긴다 — 세션은 `MidTurnPolicy::TransportOwned` 일 때만 부른다. 수령 의미는
    /// [`AgentTransport::send_input`] 과 같다.
    /// ★기본 구현 = 본문을 `send_input(Raw)` 로 그대로★ — 턴을 스스로 분류하지 않는 통로(PTY·stdio)는 이것을
    ///   구현하지 않는다. `InputEvent` 에 변형을 더하는 길은 쓰지 않는다: 그 통로들의 반박 불가 패턴
    ///   (`let InputEvent::Raw(b) = input`)이 깨진다.
    // ADR-0231
    fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
        self.send_input(InputEvent::Raw(turn.body))
    }

    /// 대기 입력 하나를 거둔다 — 세션의 취소가 `MidTurnPolicy::TransportOwned` 일 때 이리로 넘어온다.
    /// 목록 사건(거둠 · 취소 요청)은 **통로가** 낸다 — 그 항목을 쥐었는지 넘겼는지는 통로만 안다.
    /// ★기본 구현 = `NotHeld`★ — 아무것도 쥐지 않는 통로다.
    // ADR-0231
    fn withdraw(&self, _id: &str) -> Withdraw {
        Withdraw::NotHeld
    }

    /// 수락 모름으로 쥔 대기 입력의 id — 넘겼는데 벤더가 받았는지 모르는 항목(목록 조회가 `unconfirmed` 로
    /// 덧댄다). 그 전이는 목록 사건이 없어 명부에는 `Queued` 그대로다.
    /// ★조회 순간의 통로 상태다★ — 명부 스냅숏과 한 원자가 아니고, 호출자는 명부 락을 놓은 **뒤** 부른다
    ///   (두 락을 겹쳐 쥐지 않는다 — ADR-0006).
    /// ★기본 구현 = 빈 목록★ — 턴을 넘긴 뒤 수락을 따로 기다리지 않는 통로(claude · PTY)는 늘 비어 있다.
    // ADR-0231
    fn unconfirmed_inputs(&self) -> Vec<String> {
        Vec::new()
    }

    /// ★이 호출 시점까지 받아 둔 입력이 **실제로 나갈 때까지** 기다린다★ — `Ok` = 나갔다.
    ///
    /// ★왜 있나★: [`AgentTransport::send_input`] 의 `Ok` 는 「받았다」이지 「갔다」가 아니다. 그 둘의
    ///   차이를 **배달 영수증에 실으면 안 되는** 호출자가 하나 있다 — 우편 배달이다(ADR-0088 이 가르려는
    ///   "전송 실패" 와 "모델이 무시" 가 뒤집힌다). 그 호출자만 이 동사를 지나 `Ok` 의 뜻을 되돌린다.
    /// ★★키 입력 경로는 이것을 부르지 않는다 — 부르면 안 된다★★: 거기서 기다리면 큐가 없애려던 바로 그
    ///   head-of-line blocking 이 되돌아온다. 이 동사를 새 호출부에 붙이기 전에 그 호출부가 **이미
    ///   블로킹을 감수하는 경로인지** 먼저 답할 것.
    /// ★기본 구현이 `Unsupported` 인 것은 의도다 — `Ok` 로 위장하지 않는다★: 쓰기 완료를 확인할 수단이
    ///   없는 통로가 `Ok` 를 돌려주면 그것이 곧 위 뒤집힘이다. 확인 수단이 없다고 **말하는** 쪽이 정직하고,
    ///   호출자는 그 신호를 보고 「예전과 같은 수준의 앎」으로 다룰 수 있다(그 판단은 호출부가 적는다).
    fn flush_input(&self, _timeout: Duration) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "flush_input (이 통로는 쓰기 완료를 확인할 수단이 없다)".into(),
        ))
    }

    /// cols/rows의 보존(atomic 저장)은 AgentSession 책임.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;

    /// ≠kill. 진행 중 작업만 중단 — 프로세스는 살아 있다.
    fn interrupt(&self) -> Result<(), PtyError>;

    /// 자원 강제 종료(멱등). pump 종료 대기는 여기서 안 함(core.join_pump 몫).
    fn shutdown(&self);

    fn capabilities(&self) -> TransportCaps;
}

/// 이 통로가 연결의 결말을 **배달할** 곳. `None` = 세울 연결이 없다(PTY·stdio 는 프로세스가 뜬
/// 순간부터 쓸 수 있어 이 축이 아예 없다).
///
/// ★★셀에 담아 두고 **읽히기를 기다리지 않는다** — 그것이 이 포트의 존재 이유다★★:
///   옛 모양은 통로가 연결 상태를 셀에 두고 감독자가 루프에서 그것을 **읽었다**. 그 사이가 곧 결함
///   넷이었다 — 읽은 뒤 행동하기까지 통로가 움직이면, 감독자는 이미 사라진 사실 위에서 판정한다.
///   (① 수거된 화신의 정리가 산 후임을 죽이고 ② `Establishing` 을 읽은 뒤 `Ready` 가 서도 시한이
///   실패로 확정하며 ③ 사용자 kill 이 만든 `Down` 이 이어받기 실패로 기록되고 ④ stdout EOF 뒤에도
///   `Up` 이 남아 성공으로 읽힌다.) 한 번 **배달된** 값은 철회되지 않으므로 그 틈이 아예 없다.
/// ★화신 표식은 **조립점이 찍는다**★ — 이 포트는 `Fn(LinkResolution)` 이고 통로는 화신을 모른다
///   ([`SessionIdSink`] 와 같은 모양·같은 사유). 소비자는 그 표식을 **일치/불일치로만** 본다
///   (ADR-0163/0164 — 대소로 「더 새 것」을 유도하면 ①이 되돌아온다).
/// ★★계약은 **많아야 한 번**(zero-or-once)이다 — 「정확히 한 번」이 아니다★★. 두 결말이 있다:
///   - **한 번** — 연결이 결말을 내는 그 순간. 그 뒤로는 부르지 않는다.
///   - **영(零)** — 사용자가 이 화신을 껐다. kill 때문에 풀린 핸드셰이크는 「상대가 답했다」가 아니라서
///     배달하지 않는다(그 갈래를 배달하면 사용자의 취소가 이어받기 실패로 기록된다 — 결함 ③).
/// ★★그래서 **받는 쪽이 「영」을 처리해야 한다 — 이것이 이 문단의 요점이다**★★: 한때 이 자리가
///   「정확히 한 번」과 「껐으면 안 부른다」를 세 줄 간격으로 함께 적고 있었고, 앞엣것을 믿고 쓴 소비자가
///   [`crate::manager::AgentManager::link_activation_verdict`] 의 채널 소멸 갈래였다 — 배달이 억제되면
///   보내는 끝이 떨어지는데, 그것을 「통로가 결말을 못 냈다」로 읽어 **사용자의 취소를 활성화 실패로
///   기록했다.** 지금 그 갈래는 종료 의도 래치를 **먼저** 본다.
/// ★억제되는 것은 **메시지뿐이고 채널 소멸은 아니다**★ — 이 포트를 쥔 것은 라이터 스레드 하나라, 그
///   스레드가 끝나면 보내는 끝이 떨어진다. 그 신호는 억제할 수 없으므로 「영」의 관측 가능한 형태가
///   곧 **채널 소멸**이다. 받는 쪽은 그것을 결말로 읽어서는 안 된다.
/// ★배달은 종료를 **몰지 않는다**★ — 통로는 여전히 아무것도 죽이지 않고 `finish` 도 부르지 않는다
///   (ADR-0199 의 선). 결말을 말하는 것과 결말을 집행하는 것은 다른 일이다.
pub type LinkSink = Arc<dyn Fn(LinkResolution) + Send + Sync>;

/// 연결이 어떻게 끝났나 — ★두 값뿐이고 둘 다 **확정**이다★(「아직」은 배달되지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// 상대가 **입력을 받을 준비가 됐다고 신호했다** = 활성화 성립(사용자 결정).
    Ready,
    /// 연결이 서지 못했다. `reason` = 사람이 읽을 사유이자 backend 분류의 입력 — 이 통로의 실패
    /// 문구는 stdout 의 JSON-RPC 오류라 콘솔 꼬리에도 stderr 진단 꼬리에도 없다.
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ControlCaps, InputCaps, InputOrigin, OutputCaps};
    use std::sync::Mutex;

    /// 기본 구현만 쓰는 통로 — `send_input` 만 기록한다.
    struct RawOnly(Mutex<Vec<Vec<u8>>>);
    impl AgentTransport for RawOnly {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.0.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    structured: false,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    #[test]
    fn the_default_send_turn_hands_the_body_to_send_input_byte_for_byte() {
        let t = RawOnly(Mutex::new(Vec::new()));
        for origin in [InputOrigin::User, InputOrigin::Mail] {
            t.send_turn(TurnInput {
                id: "id-1".into(),
                body: b"echo hi\r\n\x03".to_vec(),
                origin,
            })
            .unwrap();
        }
        assert_eq!(
            *t.0.lock().unwrap(),
            vec![b"echo hi\r\n\x03".to_vec(), b"echo hi\r\n\x03".to_vec()],
            "기본 구현은 출처와 무관하게 본문만 그대로 넘긴다 — id·출처는 바이트에 안 섞인다"
        );
    }

    #[test]
    fn the_default_withdraw_holds_nothing() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert_eq!(t.withdraw("id-1"), Withdraw::NotHeld);
        assert!(
            t.0.lock().unwrap().is_empty(),
            "거두기는 아무것도 쓰지 않는다"
        );
    }

    #[test]
    fn the_default_transport_holds_no_unconfirmed_input() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert!(t.unconfirmed_inputs().is_empty());
    }
}
