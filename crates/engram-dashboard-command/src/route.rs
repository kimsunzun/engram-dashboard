//! 배달 — **전 프로세스·전 홉이 같은 3단계**다(TRD §3-5).

use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::{
    CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable, ErrorCode, OwnerLookup,
    OwnerLookupSource,
};

/// 동기 본문의 패닉을 오류 답장으로 접는다.
///
/// ★없으면 요청이 답장 없이 끝난다★: 핸들러 패닉이 그대로 풀리면 `CommandReply` 가 만들어지지 않아
/// 호출자는 마감시각까지 매달린다 — 「한 `request_id` 에 답장은 정확히 하나」(TRD §4-⑤) 위반이다.
/// ★릴리즈 프로필의 한계(알려진 사실)★: 워크스페이스 릴리즈 프로필은 `panic = "abort"` 라 그 빌드에서는
/// 잡히지 않는다(프로세스가 죽는다). 이 그물은 개발·테스트 빌드에서만 실효가 있다.
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

/// 핸들러가 터졌다 — 이 홉에서 **확실히** 실패했다(봉투가 나간 적이 없다).
fn handler_panicked(detail: String) -> CommandError {
    CommandError::of(
        ErrorCode::Internal,
        format!("command handler panicked: {detail}"),
    )
}

/// 배달 중 패닉의 결말 — ★`INTERNAL` 이 아니다★.
///
/// 봉투는 이미 선을 탔을 수 있으므로 **이 홉의 확실성은 「불명」**이다(TRD §4-④). `INTERNAL`(= `retry:
/// never`)로 답하면 호출자는 「여기서 확실히 실패했다」로 읽고 새 id 로 다시 부르는데, 그러면 상대가
/// 이미 적용한 조작이 두 번 적용될 수 있다. `OUTCOME_UNKNOWN` 은 **같은 id 로만** 다시 묻게 한다.
fn forwarding_unknown(detail: String) -> CommandError {
    CommandError::of(
        ErrorCode::OutcomeUnknown,
        format!("the command link panicked while forwarding: {detail}"),
    )
}

/// future 의 패닉을 결말로 접는다 — 본문이 첫 poll 에서 도는 형태라 여기서 터진다.
///
/// 접는 값은 부르는 쪽이 준다(`on_panic`) — 로컬 실행과 전달은 **같은 그물을 쓰되 다른 확실성**을 답한다.
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
            // 패닉 후의 future 는 상태가 불명이라 다시 폴링하지 않는다 — 여기서 끝낸다.
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

/// 내 표에 있나 → 명부에 있나 → 오류.
///
/// ★홉마다 다른 규칙을 두지 않는다★ — 새 주인(모바일 클라이언트 등)이 붙어도 배달 코드는 안 바뀌고
/// 등록이 하나 늘 뿐이다. 명부가 없는 프로세스는 빈 [`crate::Roster`] 를 넘긴다.
/// ★명부는 **조회 한 번**만 받는다([`OwnerLookupSource`])★ — 참조로 붙들면 공유 명부를 쥔 조립부가
/// `link.send` 왕복 내내 락을 들어야 하고, 느린 상대 하나가 그동안 등록·연결 정리·다른 배달을 전부
/// 세운다. **인자로 되돌리지 말 것** — 여기 참조가 서면 그 정지는 호출자가 피할 수 없다.
/// ★인자 검문은 여기 없다★ — 선언에 없는 칸의 거절은 **사람·LLM 이 치는 입구**의 일이고
/// ([`CommandTable::check_args`] · ADR-0142), 홉 간 배선은 모르는 칸을 무시하고 옛 의미로 실행해야
/// additive 진화가 산다(TRD §4-③).
/// ★그래서 **남의 이름**으로 온 인자는 이 경로 어디서도 검문받지 않는다(알려진 구멍)★ — 치는 쪽 입구는
/// 그 선언을 안 들어 검문하지 못하고, 여기서 넣으면 위 이유로 additive 진화가 죽는다. 그 구멍을 누가
/// 메우나는 입구 어댑터 **쌍**(치는 쪽 · 주인 쪽)의 배선 결정으로 남아 있다 —
/// [`CommandTable::check_args`] 의 통과 목록에 그 갈림이 적혀 있다.
/// 나가는 봉투에는 명부가 답한 주인 토큰을 **덮어 싣는다** — 받는 홉이 「누구 앞으로 온 것인가」를
/// 자기 명부와 대조할 수 있어야 2단 배달에서 중간 홉이 갈라 줄 수 있다(TRD §3-8).
/// ★마감시각은 여기서 걸지 않는다 — 조립부의 몫이다★: 마감은 **호출자가 정하고**(TRD §4-⑥) 최초 수신
/// 데몬의 라우팅 표 엔트리가 같은 `deadline` 을 들고 있어야 호출자가 사라져도 표가 안 샌다. 그 표가
/// Step 2 에서 생기므로, 그 전에 여기 시계를 넣으면 홉마다 다른 마감이 생겨 같은 왕복이 중간에서
/// 잘린다(전 구간 하나의 `request_id` 에 답장 하나 — §4-⑤).
/// ★취소 의미(v1 미도입 — TRD §4-⑤)★: 이 future 를 drop 하면 왕복이 **그대로 버려진다** — 상대 홉에는
/// 아무 통보도 가지 않고, 이미 나간 봉투의 결말은 불명으로 남는다. 회수는 Step 2 의 마감시각 sweep 이
/// 맡는다.
/// ★핸들러·링크 future 는 drop 에서 패닉하면 안 된다★ — 아래 그물은 `poll` 만 덮고 drop 은 못 덮으며,
/// unwind 중 재패닉은 프로세스 abort 라 답장이 아예 나가지 못한다.
// ADR-0140
pub async fn route(
    table: &CommandTable,
    roster: &dyn OwnerLookupSource,
    link: &dyn CommandLink,
    mut env: CommandEnvelope,
) -> CommandReply {
    let request_id = env.request_id;

    // 표에 없으면 `call` 이 `None` 을 내고 인자는 그대로 남는다 — 다음 단계가 그것을 실어 보낸다.
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
            // ★그물은 전달 쪽에도 있다★ — 없으면 링크가 터질 때 답장 없이 요청이 끝난다(TRD §4-⑤ 위반).
            //   두 진입점을 함께 덮는다: 봉투를 받아 future 를 만들다가 터지는 경우와 첫 poll 에서 터지는 경우.
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
            // ★남의 답을 내 왕복으로 세지 않는다★: 상관 키가 다른 답장을 그대로 통과시키면 호출자는 다른
            //   요청의 결과를 자기 것으로 읽는다. 적용 여부는 알 수 없으므로 확실성은 **불명**이다.
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
        OwnerLookup::Unavailable => CommandReply::err(
            request_id,
            CommandError::of(ErrorCode::OwnerUnavailable, env.name),
        ),
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
    };
    static DECLARED: &[&CommandSpec] = &[&AGENT_LIST];

    /// 보낸 봉투를 기록하고 미리 정한 답을 돌려준다 — 소켓 0으로 배달 규칙만 단언한다.
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

    /// 이 홉 앞으로 온 봉투 — `owner` 는 **보낸 이가 아니라 목적지**다(앞 홉이 명부를 보고 적어 넣는다).
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

    #[test]
    fn step3_splits_unknown_command_from_owner_unavailable() {
        let table = CommandTable::new(DECLARED);
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(
                &owner,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{}".to_string(),
                }],
            )
            .expect("한 이름은 상한 안이다");
        roster.disconnect(&owner);
        let link = FakeLink::default();

        let gone = block_on(route(&table, &roster, &link, envelope("tab.create")));
        let never = block_on(route(&table, &roster, &link, envelope("theme.set")));

        let gone = gone.outcome.expect_err("주인 부재");
        assert_eq!(gone.code(), ErrorCode::OwnerUnavailable);
        assert_eq!(gone.retry(), crate::RetryMode::AfterCondition);

        let never = never.outcome.expect_err("모르는 이름");
        assert_eq!(never.code(), ErrorCode::UnknownCommand);
        assert_eq!(never.retry(), crate::RetryMode::Never);

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

    /// ★`route` 자신의 그물을 겨눈다★ — 핸들러 안쪽 그물(`blocking_handler`)을 쓰면 이 테스트는 바깥
    /// 그물이 없어져도 통과한다(안쪽이 먼저 접으므로). 그래서 **그물 없는 핸들러**로 두 진입점을 각각
    /// 태운다: `call` 이 터지는 경우와 future 첫 poll 이 터지는 경우.
    /// 그물이 없으면 이 테스트는 hang 이 아니라 패닉으로 죽는다(둘 다 「답장 없음」의 얼굴이다).
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

    /// ★그물의 비대칭이 없어야 한다★ — 로컬 실행만 덮고 전달을 비워 두면, 링크가 터지는 순간 답장 없이
    /// 요청이 끝난다(호출자는 마감시각까지 매달린다 — TRD §4-⑤). 여기서도 두 진입점을 각각 태운다.
    /// ★답은 `INTERNAL` 이 아니라 `OUTCOME_UNKNOWN` 이다★ — 봉투가 이미 선을 탔을 수 있어 이 홉의
    /// 확실성이 「불명」이고, 그래야 재시도가 **같은 id 로만** 일어난다(TRD §4-④).
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

    /// ★남의 답을 내 왕복으로 세지 않는다★
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

    /// ★배달은 명부를 **조회 한 번** 동안만 본다★ — 공유 명부를 쥔 조립부가 왕복 내내 락을 들지 않아도
    /// 되는 것이 [`OwnerLookupSource`] 의 존재 이유다. 락을 조회 안에서만 잡는 구현으로 단언한다: 배달이
    /// 명부를 붙들고 있으면 링크가 도는 동안 `try_lock` 이 실패한다.
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
        // 표는 조립부가 Arc 로 들고 여러 연결이 나눠 쓴다 — Send + Sync 가 형태로 유지되는지 본다.
        let table = Arc::new(table_with_agent_list());
        let clone = Arc::clone(&table);
        std::thread::spawn(move || {
            assert!(clone.contains("agent.list"));
        })
        .join()
        .expect("스레드");
    }
}
