//! Transport seam and inbound receiver (TRD §3-4 · §3-6).

use std::future::Future;
use std::pin::Pin;

use crate::{CommandEnvelope, CommandError, CommandReply, ErrorCode, RequestId};

/// Sends the envelope to the peer process and waits for the reply with **the same `request_id`**.
///
/// There is one implementation per process (daemon = WS connection · shell = daemon_client · screen = invoke).
/// ★This is the swap point, so the transport method is not tied into the code★ (CLAUDE.md 「설계 원칙」 (design principles)).
pub trait CommandLink: Send + Sync {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}

/// The entry that takes over received envelopes.
///
/// ★`on_command` must return immediately★ — applying runs **outside the connection task**. If it waits inline
/// where it received the envelope, a composite command (a handler calling a second command over the same link) cannot pull
/// its own reply and deadlocks (the self-deadlock of ADR-0081 decision 3). This rule applies to the shell, daemon, and webview **alike**.
pub trait InboundCommands: Send + Sync {
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink);
}

/// A one-shot exit that sends one reply back under its `request_id`.
///
/// ★It enforces 「정확히 한 번」 (exactly once) by shape★ — consuming (`fn send(self)`) prevents **two or more**, and [`Drop`]
/// prevents **zero**. If the owner disappears without producing an answer (a missed path · an early return · a panic unwind), an
/// error reply goes out right there. Without this, the caller hangs until the deadline, and that means 「한 `request_id` 에 답장은
/// 정확히 하나」 (exactly one reply per request id) (TRD §4-⑤) has weakened to **at most once**.
///
/// ★There is **only one** delivery call site: [`Drop`]★ — `send` only records the outcome. ★Do not revert to a form that
/// takes the callback out and calls it inside `send`★: with two call sites, a **zero-reply** path opens — 「앞 지점이 부르다 패닉 → 뒤 지점은
/// 이미 소비됐다고 보고 그냥 반환」 (the first site panics while calling → the second site sees it as already consumed and just returns). With one site, that path does not exist at all.
/// Since `send(self)` consumes self, delivery finishes before `send` returns.
/// ★Remaining limit (what shape cannot prevent)★: if the callback **itself** panics, we cannot know whether the reply went out — it is `FnOnce`, so
/// there is no means of calling it again. That panic is swallowed here and not let out of drop (a re-panic during unwind = abort).
/// How it is actually delivered (oneshot · queue · callback) is decided by the assembly code — this crate knows nothing about transport.
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

    /// Records the outcome — the actual delivery is done by the [`Drop`] just before this function returns (doc above).
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
        // drop runs even during unwind — if it panics again here, the process aborts.
        // In the daemon and shell the hook finishes any panic outside the 「가두는 중」 (containing) marker first (ADR-0292), so this net only has effect
        //   when dropped inside the marker and in test builds without the hook.
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

    /// ★Even if it blows up **after** producing the answer, that answer goes out as is★ — there is one delivery site, so there is no room
    /// for a panic to intrude between 「적었다 → 배달」 (recorded → delivered).
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

    /// ★A panic of the callback itself cannot get out of drop★ — a re-panic during unwind is abort, so it is swallowed here.
    /// Whether the reply actually went out is **unknowable** (it is `FnOnce`, so there is no means of calling it again) — a spot shape cannot prevent.
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
