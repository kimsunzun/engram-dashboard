//! 전송 seam 과 인바운드 수신기(TRD §3-4 · §3-6).

use std::future::Future;
use std::pin::Pin;

use crate::{CommandEnvelope, CommandError, CommandReply, ErrorCode, RequestId};

/// 봉투를 상대 프로세스로 보내고 **같은 `request_id`** 의 답을 기다린다.
///
/// 구현체는 프로세스마다 하나다(데몬 = WS 연결 · 셸 = daemon_client · 화면 = invoke).
/// ★이것이 교체점이라 전송 방식이 코드에 안 묶인다★(CLAUDE.md 「아키텍처 원칙」).
pub trait CommandLink: Send + Sync {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}

/// 받은 봉투를 넘겨받는 입구.
///
/// ★`on_command` 는 즉시 반환해야 한다★ — 적용은 **연결 태스크 밖**에서 돈다. 받은 자리에서 인라인으로
/// 기다리면 합성 명령(핸들러가 같은 링크로 두 번째 명령을 부르는 경우)이 자기 답을 자기가 못 꺼내
/// 교착한다(ADR-0081 결정 3 의 self-deadlock). 셸·데몬·웹뷰 **모두**에 적용되는 규칙이다.
pub trait InboundCommands: Send + Sync {
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink);
}

/// 답장 한 장을 그 `request_id` 로 되돌려 보내는 일회용 출구.
///
/// ★「정확히 한 번」을 형태로 강제한다★ — 소비(`fn send(self)`)가 **두 번 이상**을 막고, [`Drop`] 이
/// **0번**을 막는다. 소유자가 답을 못 내고 사라지면(경로 누락 · 조기 return · 패닉 unwind) 그 자리에서
/// 오류 답장이 나간다. 이게 없으면 호출자는 마감시각까지 매달리고, 그건 「한 `request_id` 에 답장은
/// 정확히 하나」(TRD §4-⑤)가 **최대 한 번**으로 약해진 것이다.
///
/// ★배달 호출 지점은 [`Drop`] **하나뿐**이다★ — `send` 는 결말을 적어 두기만 한다. ★`send` 안에서
/// 콜백을 꺼내 부르는 형태로 되돌리지 말 것★: 부르는 지점이 둘이면 「앞 지점이 부르다 패닉 → 뒤 지점은
/// 이미 소비됐다고 보고 그냥 반환」이라는 **답장 0번** 경로가 열린다. 지점이 하나면 그 경로 자체가 없다.
/// `send(self)` 가 self 를 소비하므로 배달은 `send` 반환 전에 끝난다.
/// ★남는 한계(형태로 못 막는 것)★: 콜백 **자체**가 패닉하면 답장이 나갔는지 알 수 없다 — `FnOnce` 라
/// 다시 부를 수단이 없다. 그 패닉은 여기서 삼켜 drop 밖으로 내보내지 않는다(unwind 중 재패닉 = abort).
/// 실물 전달 방식(oneshot·큐·콜백)은 조립부가 정한다 — 이 crate 는 전송을 모른다.
pub struct ReplySink {
    request_id: RequestId,
    outcome: Option<Result<serde_json::Value, CommandError>>,
    deliver: Option<Box<dyn FnOnce(CommandReply) + Send>>,
}

impl ReplySink {
    pub fn new(request_id: RequestId, deliver: impl FnOnce(CommandReply) + Send + 'static) -> Self {
        Self {
            request_id,
            outcome: None,
            deliver: Some(Box::new(deliver)),
        }
    }

    pub fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// 결말을 적는다 — 실제 배달은 이 함수가 반환하기 직전의 [`Drop`] 이 한다(위 doc).
    pub fn send(mut self, outcome: Result<serde_json::Value, CommandError>) {
        self.outcome = Some(outcome);
    }
}

impl Drop for ReplySink {
    fn drop(&mut self) {
        let Some(deliver) = self.deliver.take() else {
            return;
        };
        let outcome = self.outcome.take().unwrap_or_else(|| {
            Err(CommandError::of(
                ErrorCode::Internal,
                "the command owner dropped the reply sink without answering",
            ))
        });
        let reply = CommandReply {
            request_id: self.request_id,
            outcome,
        };
        // drop 은 unwind 중에도 돈다 — 여기서 다시 패닉하면 프로세스가 abort 한다.
        // 릴리즈 프로필은 `panic = "abort"` 라 이 그물은 개발·테스트 빌드에서만 실효가 있다.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deliver(reply)));
    }
}

impl std::fmt::Debug for ReplySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplySink")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn sink(seen: &Arc<Mutex<Vec<CommandReply>>>, id: RequestId) -> ReplySink {
        let seen = Arc::clone(seen);
        ReplySink::new(id, move |reply| seen.lock().expect("poisoned").push(reply))
    }

    #[test]
    fn dropping_without_answering_still_delivers_exactly_one_reply() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let id = RequestId::new();
        drop(sink(&seen, id));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "0번이 되지 않는다");
        assert_eq!(seen[0].request_id, id);
        assert_eq!(
            seen[0].outcome.as_ref().expect_err("오류 답장").code(),
            ErrorCode::Internal
        );
    }

    #[test]
    fn answering_delivers_once_and_drop_adds_nothing() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let id = RequestId::new();
        sink(&seen, id).send(Ok(serde_json::json!({ "ok": true })));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].outcome.is_ok());
    }

    #[test]
    fn a_panicking_owner_still_answers() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let id = RequestId::new();
        let handed = sink(&seen, id);
        let result = crate::testing::with_quiet_panic_hook(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _held = handed;
                panic!("owner blew up mid-command");
            }))
        });

        assert!(result.is_err());
        assert_eq!(seen.lock().unwrap().len(), 1, "unwind 중에도 답장은 나간다");
    }

    /// ★답을 낸 **뒤** 터져도 그 답이 그대로 나간다★ — 배달 지점이 하나라 「적었다 → 배달」 사이에
    /// 패닉이 끼어들 자리가 없다.
    #[test]
    fn an_owner_that_panics_after_answering_still_sends_its_own_outcome() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let id = RequestId::new();
        let handed = sink(&seen, id);
        let result = crate::testing::with_quiet_panic_hook(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                handed.send(Ok(serde_json::json!({ "ok": true })));
                panic!("owner blew up after answering");
            }))
        });

        assert!(result.is_err());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].outcome.is_ok(), "오류 답장으로 덮이지 않는다");
    }

    /// ★콜백 자체의 패닉은 drop 밖으로 못 나간다★ — unwind 중 재패닉은 abort 라 여기서 삼킨다.
    /// 답장이 실제로 나갔는지는 **알 수 없다**(`FnOnce` 라 다시 부를 수단이 없다) — 형태로 못 막는 자리다.
    #[test]
    fn a_panicking_delivery_callback_does_not_escape() {
        let attempts = Arc::new(Mutex::new(0usize));
        let counted = Arc::clone(&attempts);
        let id = RequestId::new();
        let sink = ReplySink::new(id, move |_reply| {
            *counted.lock().expect("poisoned") += 1;
            panic!("the transport blew up while delivering");
        });

        crate::testing::with_quiet_panic_hook(|| sink.send(Ok(serde_json::json!({}))));

        assert_eq!(*attempts.lock().unwrap(), 1, "배달은 정확히 한 번 시도된다");
    }
}
