//! Delivery — **every process and every hop uses the same 3 steps** (TRD §3-5).

use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::{
    CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable, ErrorCode, OwnerLookup,
    OwnerLookupSource,
};

/// Folds a panic in the synchronous body into an error reply.
///
/// ★Without it, the request ends with no reply★: if a handler panic unwinds as is, no `CommandReply` is made and
/// the caller hangs until the deadline — a violation of 「한 `request_id` 에 답장은 정확히 하나」 (exactly one reply per request id) (TRD §4-⑤).
/// ★It is not reached in production processes★: in the daemon and shell the hook ends any panic outside the 「가두는 중」 (containing) marker before it unwinds
/// (ADR-0292), and nowhere runs a command body inside the marker (ADR-0293). So this net only
/// has effect in test builds without the hook — the shell and daemon tests of that build rely on it, so it is not removed.
pub(crate) fn guard_panic<T>(
    body: impl FnOnce() -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(outcome) => outcome,
        Err(payload) => Err(handler_panicked(panic_detail(&payload))),
    }
}

fn panic_detail(payload: &Box<dyn Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown payload".to_string())
}

/// The handler blew up — it **definitely** failed at this hop (the envelope never went out).
// ADR-0159
fn handler_panicked(detail: String) -> CommandError {
    CommandError::of(
        ErrorCode::Internal,
        format!("command handler panicked: {detail}"),
    )
}

/// The outcome of a panic during delivery — ★it is not `INTERNAL`★.
///
/// The envelope may already have gone on the wire, so **this hop's certainty is 「불명」** (unknown) (TRD §4-④). If we answer with `INTERNAL` (= `retry:
/// never`), the caller reads it as 「여기서 확실히 실패했다」 (definitely failed here) and calls again with a new id, and then an operation the peer
/// had already applied can be applied twice. `OUTCOME_UNKNOWN` makes it ask again **with the same id only**.
// ADR-0159
fn forwarding_unknown(detail: String) -> CommandError {
    CommandError::of(
        ErrorCode::OutcomeUnknown,
        format!("the command link panicked while forwarding: {detail}"),
    )
}

/// Folds a panic in the future into an outcome — the body runs on the first poll, so it blows up here.
///
/// The folded value is supplied by the caller (`on_panic`) — local execution and delivery **use the same net but answer with different certainty**.
struct CatchUnwind<F, P> {
    inner: F,
    on_panic: Option<P>,
}

impl<F, P, T> Future for CatchUnwind<F, P>
where
    F: Future<Output = T> + Unpin,
    P: FnOnce(String) -> T + Unpin,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match catch_unwind(AssertUnwindSafe(|| Pin::new(&mut this.inner).poll(cx))) {
            Ok(polled) => polled,
            // A future after a panic has unknown state, so it is not polled again — it ends here.
            Err(payload) => {
                let fold = this
                    .on_panic
                    .take()
                    .expect("Ready 를 낸 future 는 다시 폴링되지 않는다");
                Poll::Ready(fold(panic_detail(&payload)))
            }
        }
    }
}

/// In my table? → in the registry? → error.
///
/// ★No different rule per hop★ — even when a new owner (a mobile client, etc.) attaches, the delivery code does not change and
/// only one more registration is added. A process with no registry passes an empty [`crate::Roster`].
/// ★The registry receives **a single lookup** only ([`OwnerLookupSource`])★ — if it were held by reference, the assembly code holding the shared registry
/// would have to hold the lock for the entire `link.send` round trip, and one slow peer would halt registration, connection cleanup and other deliveries
/// meanwhile. **Do not revert to an argument** — if a reference stands here, the caller cannot avoid that halt.
/// ★There is no argument screening here★ — rejecting fields absent from the declaration is the job of **the entry where a person or LLM types**
/// ([`CommandTable::check_args`] · ADR-0157), and inter-hop wiring must ignore unknown fields and run with the old meaning
/// for additive evolution to survive (TRD §4-③).
/// ★So arguments arriving under **another owner's name** are screened nowhere on this path (a known hole)★ — the typing side's entry
/// does not hold that declaration and so cannot screen, and putting it here kills additive evolution for the reason above. Who
/// plugs that hole remains a wiring decision of the entry adapter **pair** (typing side · owner side) —
/// that fork is written in the pass-through list of [`CommandTable::check_args`].
/// On the outgoing envelope the owner token the registry answered is **overwritten in** — the receiving hop must be able to check 「누구 앞으로 온 것인가」 (who was this addressed to)
/// against its own registry for the middle hop to sort it in two-stage delivery (TRD §3-8).
/// ★The deadline is not set here — it belongs to the assembly code★: the deadline is **set by the caller** (TRD §4-⑥), and
/// the routing-table entry of the first receiving daemon must hold the same `deadline` so the table does not leak even if the caller disappears. That table
/// arises in Step 2, so if a clock were put here before then, each hop would get a different deadline and the same round trip would be
/// cut in the middle (one reply per `request_id` across the whole path — §4-⑤).
/// ★Cancellation semantics (not introduced in v1 — TRD §4-⑤)★: dropping this future **discards the round trip as is** — no
/// notice goes to the peer hop, and the outcome of an envelope already sent remains unknown. Reclaiming is
/// the job of Step 2's deadline sweep.
/// ★A handler or link future must not panic in drop★ — the net below covers only `poll` and cannot cover drop, and
/// a re-panic during unwind is a process abort, so no reply can go out at all.
// ADR-0155
pub async fn route(
    table: &CommandTable,
    roster: &dyn OwnerLookupSource,
    link: &dyn CommandLink,
    mut env: CommandEnvelope,
) -> CommandReply {
    let request_id = env.request_id;

    // If it is not in the table, `call` returns `None` and the arguments remain as they are — the next step carries them.
    let called = catch_unwind(AssertUnwindSafe(|| table.call(&env.name, &mut env.args)));
    match called {
        Ok(Some(future)) => {
            let outcome = CatchUnwind {
                inner: future,
                on_panic: Some(|detail: String| Err(handler_panicked(detail))),
            }
            .await;
            return CommandReply {
                request_id,
                outcome,
            };
        }
        Ok(None) => {}
        Err(payload) => {
            return CommandReply::err(request_id, handler_panicked(panic_detail(&payload)))
        }
    }

    match roster.lookup(&env.name) {
        OwnerLookup::Available(owner) => {
            // ★The net is on the delivery side too★ — without it, when the link blows up the request ends with no reply (violates TRD §4-⑤).
            //   It covers both entry points: blowing up while receiving the envelope and making the future, and blowing up on the first poll.
            let sent = catch_unwind(AssertUnwindSafe(|| {
                link.send(CommandEnvelope { owner, ..env })
            }));
            let reply = match sent {
                Ok(future) => {
                    CatchUnwind {
                        inner: future,
                        on_panic: Some(move |detail: String| {
                            CommandReply::err(request_id, forwarding_unknown(detail))
                        }),
                    }
                    .await
                }
                Err(payload) => {
                    return CommandReply::err(
                        request_id,
                        forwarding_unknown(panic_detail(&payload)),
                    )
                }
            };
            // ★Do not count another's reply as my round trip★: if a reply with a different correlation key passed through as is, the caller would read
            //   another request's result as its own. We cannot know whether it was applied, so the certainty is **unknown**.
            if reply.request_id != request_id {
                return CommandReply::err(
                    request_id,
                    CommandError::of(
                        ErrorCode::OutcomeUnknown,
                        format!(
                            "link answered a different request_id ({} for {request_id})",
                            reply.request_id
                        ),
                    ),
                );
            }
            reply
        }
        // ★A disconnected owner's name vanishes from the registry, so there is no 「주인 부재」 (owner absent) branch here★ — that name gets the same answer as a name that
        //   was never registered (the loss of distinction ADR-0150 accepted).
        OwnerLookup::Unknown => CommandReply::err(
            request_id,
            CommandError::of(ErrorCode::UnknownCommand, env.name),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::testing::block_on;
    use crate::{
        blocking_handler, CommandDecl, CommandHandler, CommandSpec, CommandTable, Effect,
        OwnerToken, RequestId, Roster,
    };

    static AGENT_LIST: CommandSpec = CommandSpec {
        name: "agent.list",
        effect: Effect::Read,
        since: 1,
        summary: "",
        args_schema: "{}",
        ok_schema: "{}",
        errors: &[],
        args_type: "AgentListArgs",
        ok_type: "AgentListOk",
        agent_args: &[],
    };
    static DECLARED: &[&CommandSpec] = &[&AGENT_LIST];

    /// Records the envelope sent and returns a preset answer — asserts only the delivery rules, with zero sockets.
    #[derive(Default)]
    struct FakeLink {
        sent: Mutex<Vec<CommandEnvelope>>,
    }

    impl CommandLink for FakeLink {
        fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
            let request_id = env.request_id;
            self.sent.lock().expect("fake link poisoned").push(env);
            Box::pin(async move { CommandReply::ok(request_id, json!({ "forwarded": true })) })
        }
    }

    /// An envelope addressed to this hop — `owner` is **the destination, not the sender** (the previous hop looks at the registry and writes it in).
    fn envelope(name: &str) -> CommandEnvelope {
        CommandEnvelope {
            name: name.to_string(),
            request_id: RequestId::new(),
            owner: OwnerToken::new("this-hop"),
            proto_ver: 1,
            args: json!({}),
        }
    }

    #[derive(serde::Deserialize)]
    struct NoArgs {}

    fn table_with_agent_list() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert(
                "agent.list",
                blocking_handler(|_: NoArgs| Ok(json!({ "local": true }))),
            )
            .expect("선언된 이름");
        table
    }

    #[test]
    fn step1_my_table_runs_locally() {
        let table = table_with_agent_list();
        let link = FakeLink::default();
        let env = envelope("agent.list");
        let request_id = env.request_id;

        let reply = block_on(route(&table, &Roster::new(), &link, env));

        assert_eq!(reply.request_id, request_id);
        assert_eq!(reply.outcome, Ok(json!({ "local": true })));
        assert!(
            link.sent.lock().unwrap().is_empty(),
            "표에 있으면 안 보낸다"
        );
    }

    #[test]
    fn step2_roster_owner_gets_the_envelope_with_its_token() {
        let table = CommandTable::new(DECLARED);
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("한 이름은 상한 안이다");
        let link = FakeLink::default();
        let env = envelope("tab.create");
        let request_id = env.request_id;

        let reply = block_on(route(&table, &roster, &link, env));

        assert_eq!(reply.outcome, Ok(json!({ "forwarded": true })));
        let sent = link.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].owner, owner, "명부가 답한 주인이 겉봉에 실린다");
        assert_eq!(sent[0].request_id, request_id, "id 는 전 구간 동일");
    }

    /// ★A disconnected owner's name and a name never seen before get **the same answer**★
    ///
    /// The registry erases a disconnected owner's registration (ADR-0150 decision 3), so delivery has no basis to tell the two situations apart — a loss
    /// accepted knowingly. If that distinction is ever really needed, it is solved not by reviving the trace but by putting the connection list in the
    /// response.
    #[test]
    fn step3_answers_a_disconnected_owners_name_as_an_unknown_command() {
        let table = CommandTable::new(DECLARED);
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("한 이름은 상한 안이다");
        roster.disconnect(&owner);
        let link = FakeLink::default();

        let gone = block_on(route(&table, &roster, &link, envelope("tab.create")));
        let never = block_on(route(&table, &roster, &link, envelope("never.registered")));

        for (reply, what) in [(gone, "끊긴 주인의 이름"), (never, "모르는 이름")] {
            let err = reply.outcome.expect_err(what);
            assert_eq!(err.code(), ErrorCode::UnknownCommand, "{what}");
            assert_eq!(err.retry(), crate::RetryMode::Never, "{what}");
        }

        assert!(
            link.sent.lock().unwrap().is_empty(),
            "주인이 없으면 아무 데도 보내지 않는다"
        );
    }

    #[test]
    fn empty_roster_answers_unknown_not_unavailable() {
        let table = CommandTable::new(DECLARED);
        let link = FakeLink::default();

        let reply = block_on(route(&table, &Roster::new(), &link, envelope("tab.create")));

        assert_eq!(
            reply.outcome.expect_err("빈 명부").code(),
            ErrorCode::UnknownCommand
        );
    }

    #[test]
    fn handler_error_travels_as_the_reply() {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert(
                "agent.list",
                blocking_handler(|_: NoArgs| {
                    Err::<serde_json::Value, _>(CommandError::not_found("no agent"))
                }),
            )
            .expect("선언된 이름");
        let link = FakeLink::default();

        let reply = block_on(route(&table, &Roster::new(), &link, envelope("agent.list")));

        assert_eq!(
            reply.outcome.expect_err("핸들러 실패").code(),
            ErrorCode::NotFound
        );
    }

    /// ★This aims at `route`'s own net★ — if it used the handler's inner net (`blocking_handler`), this test would pass even if the outer
    /// net disappeared (the inner one folds first). So we run each of the two entry points through a **handler with no net**:
    /// the case where `call` blows up and the case where the future's first poll blows up.
    /// Without the net, this test dies with a panic, not a hang (both are faces of 「답장 없음」 (no reply)).
    #[test]
    fn a_panic_in_an_unguarded_handler_still_produces_exactly_one_reply() {
        struct PanicOnCall;
        impl CommandHandler for PanicOnCall {
            fn call(&self, _args: serde_json::Value) -> crate::CommandFuture {
                panic!("handler blew up while being called")
            }
        }

        struct PanicOnPoll;
        impl CommandHandler for PanicOnPoll {
            fn call(&self, _args: serde_json::Value) -> crate::CommandFuture {
                Box::pin(async { panic!("handler blew up while running") })
            }
        }

        for (handler, reason) in [
            (
                Arc::new(PanicOnCall) as Arc<dyn CommandHandler>,
                "being called",
            ),
            (Arc::new(PanicOnPoll) as Arc<dyn CommandHandler>, "running"),
        ] {
            let mut table = CommandTable::new(DECLARED);
            table.insert("agent.list", handler).expect("선언된 이름");
            let link = FakeLink::default();
            let env = envelope("agent.list");
            let request_id = env.request_id;

            let reply = crate::testing::with_quiet_panic_hook(|| {
                block_on(route(&table, &Roster::new(), &link, env))
            });

            assert_eq!(reply.request_id, request_id);
            let err = reply.outcome.expect_err("패닉은 오류 답장이 된다");
            assert_eq!(err.code(), ErrorCode::Internal);
            assert!(
                err.message().contains(&format!("blew up while {reason}")),
                "사유가 실린다: {}",
                err.message()
            );
        }
    }

    /// ★There must be no asymmetry in the net★ — if only local execution is covered and delivery is left empty, at the moment the link blows up the request ends
    /// with no reply (the caller hangs until the deadline — TRD §4-⑤). Here too we run each of the two entry points.
    /// ★The answer is `OUTCOME_UNKNOWN`, not `INTERNAL`★ — the envelope may already have gone on the wire so this hop's
    /// certainty is 「불명」 (unknown), and only then does retry happen **with the same id only** (TRD §4-④).
    #[test]
    fn a_panic_while_forwarding_still_produces_exactly_one_reply() {
        struct PanicOnSend;
        impl CommandLink for PanicOnSend {
            fn send(
                &self,
                _env: CommandEnvelope,
            ) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
                panic!("link blew up while being called")
            }
        }

        struct PanicOnPoll;
        impl CommandLink for PanicOnPoll {
            fn send(
                &self,
                _env: CommandEnvelope,
            ) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
                Box::pin(async { panic!("link blew up while running") })
            }
        }

        let table = CommandTable::new(DECLARED);
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("한 이름은 상한 안이다");

        for (link, reason) in [
            (&PanicOnSend as &dyn CommandLink, "being called"),
            (&PanicOnPoll as &dyn CommandLink, "running"),
        ] {
            let env = envelope("tab.create");
            let request_id = env.request_id;

            let reply = crate::testing::with_quiet_panic_hook(|| {
                block_on(route(&table, &roster, link, env))
            });

            assert_eq!(reply.request_id, request_id);
            let err = reply.outcome.expect_err("패닉은 오류 답장이 된다");
            assert_eq!(err.code(), ErrorCode::OutcomeUnknown);
            assert_eq!(
                err.retry(),
                crate::RetryMode::SameRequestId,
                "새 id 로 다시 부르면 두 번 적용될 수 있다"
            );
            assert!(
                err.message().contains(&format!("blew up while {reason}")),
                "사유가 실린다: {}",
                err.message()
            );
        }
    }

    /// ★Do not count another's reply as my round trip★
    #[test]
    fn a_link_answering_a_different_request_id_is_outcome_unknown() {
        struct WrongIdLink;
        impl CommandLink for WrongIdLink {
            fn send(
                &self,
                _env: CommandEnvelope,
            ) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
                Box::pin(async move { CommandReply::ok(RequestId::new(), json!({ "oops": true })) })
            }
        }

        let table = CommandTable::new(DECLARED);
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("한 이름은 상한 안이다");
        let env = envelope("tab.create");
        let request_id = env.request_id;

        let reply = block_on(route(&table, &roster, &WrongIdLink, env));

        assert_eq!(reply.request_id, request_id);
        assert_eq!(
            reply.outcome.expect_err("상관 깨짐").code(),
            ErrorCode::OutcomeUnknown
        );
    }

    /// ★Delivery looks at the registry only for **a single lookup**★ — the reason [`OwnerLookupSource`] exists is that the assembly code holding the shared registry need not
    /// hold the lock for the whole round trip. We assert it with an implementation that takes the lock only inside the lookup: if delivery
    /// were holding the registry, `try_lock` would fail while the link runs.
    #[test]
    fn the_roster_is_not_held_while_the_link_is_in_flight() {
        struct SharedRoster(Arc<Mutex<Roster>>);
        impl OwnerLookupSource for SharedRoster {
            fn lookup(&self, name: &str) -> OwnerLookup {
                self.0.lock().expect("shared roster poisoned").lookup(name)
            }
        }

        struct LockProbingLink {
            roster: Arc<Mutex<Roster>>,
            free_while_sending: Mutex<Option<bool>>,
        }
        impl CommandLink for LockProbingLink {
            fn send(
                &self,
                env: CommandEnvelope,
            ) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
                *self.free_while_sending.lock().expect("probe poisoned") =
                    Some(self.roster.try_lock().is_ok());
                let request_id = env.request_id;
                Box::pin(async move { CommandReply::ok(request_id, json!({ "forwarded": true })) })
            }
        }

        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                }],
            )
            .expect("한 이름은 상한 안이다");
        let roster = Arc::new(Mutex::new(roster));
        let link = LockProbingLink {
            roster: Arc::clone(&roster),
            free_while_sending: Mutex::new(None),
        };

        let reply = block_on(route(
            &CommandTable::new(DECLARED),
            &SharedRoster(Arc::clone(&roster)),
            &link,
            envelope("tab.create"),
        ));

        assert_eq!(reply.outcome, Ok(json!({ "forwarded": true })));
        assert_eq!(
            *link.free_while_sending.lock().expect("probe poisoned"),
            Some(true),
            "왕복이 도는 동안에도 명부는 잠겨 있지 않다"
        );
    }

    #[test]
    fn arc_table_is_shareable_across_threads() {
        // The table is held in an Arc by the assembly code and shared by several connections — checks that Send + Sync is kept by shape.
        let table = Arc::new(table_with_agent_list());
        let clone = Arc::clone(&table);
        std::thread::spawn(move || {
            assert!(clone.contains("agent.list"));
        })
        .join()
        .expect("스레드");
    }
}
