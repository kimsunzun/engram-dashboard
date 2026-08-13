//! 프로세스별 핸들러 표 — 「내 표」(TRD §3-3).

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use crate::{CommandDecl, CommandError, CommandSpec, ErrorCode};

pub type CommandFuture =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, CommandError>> + Send>>;

pub trait CommandHandler: Send + Sync {
    fn call(&self, args: serde_json::Value) -> CommandFuture;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableError {
    /// 같은 이름이 두 번 들어왔다 — 조용한 덮어쓰기를 하지 않는다.
    Duplicate(&'static str),
    /// 이 표를 만든 crate 의 선언 집합에 없는 이름 — 두 번째 어휘가 생기는 것을 여기서 막는다.
    NotDeclared(&'static str),
    /// 선언의 스키마 텍스트(`args` 또는 `ok`)가 JSON 이 아니다 — 광고할 수도, 그것으로 인자를 맞출
    /// 수도 없다. ★두 칸을 함께 본다★ — 어느 쪽이 깨져도 등록 패킷의 `help` 가 통째로 깨진다.
    /// (`lint_spec` 이 선언 crate 의 테스트에서 먼저 잡지만, 조립도 이것을 싣고 달리지 않는다.)
    InvalidSchema(&'static str),
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(name) => write!(f, "command already in this table: {name}"),
            Self::NotDeclared(name) => {
                write!(f, "command not declared by this crate: {name}")
            }
            Self::InvalidSchema(name) => {
                write!(f, "declared args/ok schema is not JSON: {name}")
            }
        }
    }
}

impl std::error::Error for TableError {}

struct Entry {
    spec: &'static CommandSpec,
    /// 선언된 인자 스키마 — **조립 때 한 번** 판다. 배달 경로에 파서를 앉히지 않으려는 것이다.
    args_schema: serde_json::Value,
    handler: Arc<dyn CommandHandler>,
}

/// 한 프로세스(정확히는 한 조립 단위)가 **직접 실행할 수 있는** 명령의 표.
///
/// ★전역 static 이 아니다★ — `make_table(deps)` 가 조립 때 만들고 핸들러는 그때 주입된다(규칙 T-1).
/// 그래서 하네스가 가짜 의존을 꽂아 프로세스 없이 핸들러를 단언할 수 있다(ADR-0012).
// ADR-0134
pub struct CommandTable {
    declared: &'static [&'static CommandSpec],
    entries: BTreeMap<&'static str, Entry>,
}

impl CommandTable {
    /// `declared` = **이 표를 만드는 crate 가 선언한 집합**(선언 매크로가 만드는 `COMMAND_SPECS`).
    /// 바이너리 전량([`crate::command_specs`])이 아니다 — 남의 crate 이름을 내 표에 넣는 것도 막아야 한다.
    ///
    /// ★같은 이름이 두 번 선언돼 있으면 여기서 패닉한다★ — 조립보다 먼저 터뜨린다. 중복은 빌드가
    /// 정하는 값이라 런타임에 달라지지 않고, 그대로 두면 표와 광고 스키마가 서로 다른 선언을 가리킨다.
    pub fn new(declared: &'static [&'static CommandSpec]) -> Self {
        for (i, spec) in declared.iter().enumerate() {
            if declared[i + 1..]
                .iter()
                .any(|other| other.name == spec.name)
            {
                panic!(
                    "command declared more than once in this crate: {} — 두 선언이 같은 이름을 쥐면 어느 계약이 이기는지 알 수 없다",
                    spec.name
                );
            }
        }
        Self {
            declared,
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: &'static str,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), TableError> {
        let Some(spec) = crate::spec::find_unique(self.declared.iter().copied(), name) else {
            return Err(TableError::NotDeclared(name));
        };
        if self.entries.contains_key(name) {
            return Err(TableError::Duplicate(name));
        }
        let Ok(args_schema) = serde_json::from_str::<serde_json::Value>(spec.args_schema) else {
            return Err(TableError::InvalidSchema(name));
        };
        // ★`ok_schema` 는 표가 쓰지 않지만 여기서 함께 본다★: `spec_item_json` 이 그것을 `help` 안에
        //   **그대로 이어 붙이므로**(`spec.rs`) 깨진 `ok` 하나가 등록 패킷의 `help` 를 통째로 JSON 이
        //   아니게 만든다. 데몬은 그것을 불투명 문자열로 저장하는 것이 옳으므로(ADR-0135) 깨짐은 그
        //   문자열을 읽는 LLM·CLI 앞에서야 드러난다 — 조립이 마지막 검문소다. 값은 안 쓰니 버린다.
        if serde_json::from_str::<serde::de::IgnoredAny>(spec.ok_schema).is_err() {
            return Err(TableError::InvalidSchema(name));
        }
        self.entries.insert(
            name,
            Entry {
                spec,
                args_schema,
                handler,
            },
        );
        Ok(())
    }

    /// 이름 하나를 **광고한 계약대로** 실행한다 — 표에 없으면 `None` 이고 `args` 는 손대지 않은 채 남는다
    /// (다음 단계인 명부가 그것을 그대로 실어 보내야 한다).
    ///
    /// ★핸들러를 꺼내 주는 문을 두지 않는 것이 요점이다★: 인자와 선언 스키마를 맞추는 자리가 여기
    /// 하나라야 「광고한 것을 실제로 받는다」가 어댑터별 성질이 아니라 **표 전체의 성질**이 된다
    /// ([`crate::coerce`]). 그래서 [`CommandHandler`] 를 직접 구현한 핸들러도 같은 조정을 받는다.
    /// 본문 패닉을 접는 그물은 여기 없다 — 부르는 쪽([`crate::route`])이 두 진입점(`call` · 첫 poll)을
    /// 함께 덮는다.
    pub fn call(&self, name: &str, args: &mut serde_json::Value) -> Option<CommandFuture> {
        let entry = self.entries.get(name)?;
        crate::coerce::integral_numbers_to_integers(&entry.args_schema, args);
        Some(entry.handler.call(args.take()))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 데몬에 등록할 명단 — 이름과 **모양**을 함께 나른다(ADR-0135).
    pub fn decls(&self) -> Vec<CommandDecl> {
        self.entries
            .values()
            .map(|e| CommandDecl {
                name: e.spec.name.to_string(),
                help: crate::spec_item_json(e.spec),
            })
            .collect()
    }

    pub fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.entries.values().map(|e| e.spec)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

struct Blocking<F, A, O> {
    run: Arc<F>,
    _types: PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Blocking<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        // ★본문은 future 안에서 돈다(`call` 안이 아니라)★: `call` 이 일을 끝내고 결과만 담은 future 를
        //   돌려주면 `timeout(d, handler.call(args))` 가 **절대 발화하지 않는다** — 마감시각(TRD §4-⑥)이
        //   이 형태 위엔 못 선다. 폴링 전이면 취소가 실제로 취소가 된다.
        let run = Arc::clone(&self.run);
        Box::pin(async move { crate::route::guard_panic(|| Self::apply(&run, args)) })
    }
}

impl<F, A, O> Blocking<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn apply(run: &F, args: serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let parsed: A = serde_json::from_value(args)
            .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
        let ok = run(parsed)?;
        serde_json::to_value(ok).map_err(|e| CommandError::of(ErrorCode::Internal, e.to_string()))
    }
}

/// 동기(블로킹) 본문을 핸들러로 감싼다 — 인자 역직렬화와 반환 직렬화를 여기서 한 번만 한다.
///
/// ★blocking 계약(호출자가 지킬 것)★: 본문은 **첫 poll 에서 끝까지 돈다**. 조립부가 async 런타임
/// 스레드에서 폴링하면 그 스레드를 막는다 — 에이전트 제어 동사들은 프로필 락을 쥔 채 디스크를 쓰고
/// resume 조기 종료를 폴링하므로(daemon `control/agent.rs` 의 같은 계약) `spawn_blocking` 뒤에서 불러야 한다.
/// 인자 역직렬화 실패 = `INVALID_ARGUMENT` · 본문 패닉 = `INTERNAL`(답장 없이 끝나지 않는다).
/// ★스키마와 serde 의 조정은 여기 없다★ — [`CommandTable::call`] 이 선언을 보고 하므로 이 어댑터에
/// 다시 넣으면 두 자리가 서로 다른 규칙을 갖는다.
pub fn blocking_handler<A, O, F>(run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Blocking {
        run: Arc::new(run),
        _types: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::block_on;
    use crate::Effect;

    static PING: CommandSpec = CommandSpec {
        name: "fixture.ping",
        effect: Effect::Read,
        since: 1,
        summary: "  두 칸 들여쓴 요약  ",
        args_schema: "{\"type\":\"object\",\"properties\":{}}",
        ok_schema: "{\"type\":\"object\",\"properties\":{\"pong\":{\"type\":\"boolean\"}}}",
        errors: &[ErrorCode::Internal],
        args_type: "PingArgs",
        ok_type: "PingOk",
    };

    /// 정수·실수 칸을 함께 든 선언 — 표가 스키마를 보고 인자를 맞추는지 여기서 본다.
    static NUMBERS: CommandSpec = CommandSpec {
        name: "fixture.numbers",
        effect: Effect::Write,
        since: 1,
        summary: "수치 칸",
        args_schema: concat!(
            r#"{"type":"object","properties":{"count":{"type":"integer","minimum":0},"#,
            r#""depth":{"type":"integer"},"ratio":{"type":"number"},"#,
            r#""rows":{"type":"array","items":{"type":"object","properties":{"#,
            r#""size":{"type":"integer"}},"required":["size"]}}},"#,
            r#""required":["count","depth","ratio","rows"]}"#
        ),
        ok_schema: "{\"type\":\"object\",\"properties\":{}}",
        errors: &[],
        args_type: "NumbersArgs",
        ok_type: "NumbersOk",
    };
    static DECLARED: &[&CommandSpec] = &[&PING, &NUMBERS];

    #[derive(serde::Deserialize)]
    struct NoArgs {}

    fn handler() -> Arc<dyn CommandHandler> {
        blocking_handler(|_: NoArgs| Ok(json!({ "pong": true })))
    }

    #[test]
    fn duplicate_name_is_refused_not_overwritten() {
        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("첫 삽입");
        assert_eq!(
            table.insert("fixture.ping", handler()),
            Err(TableError::Duplicate("fixture.ping"))
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn name_outside_this_crates_declarations_is_refused() {
        let mut table = CommandTable::new(DECLARED);
        assert_eq!(
            table.insert("tab.create", handler()),
            Err(TableError::NotDeclared("tab.create"))
        );
        assert!(table.is_empty());
    }

    /// ★조용히 하나를 고르지 않는다★ — 같은 이름의 두 선언은 계약이 둘이라는 뜻이다.
    #[test]
    fn a_name_declared_twice_fails_the_table_instead_of_picking_one() {
        static OTHER_PING: CommandSpec = CommandSpec {
            name: "fixture.ping",
            effect: Effect::Write,
            since: 2,
            summary: "같은 이름, 다른 계약",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "OtherPingArgs",
            ok_type: "OtherPingOk",
        };
        static COLLIDING: &[&CommandSpec] = &[&PING, &OTHER_PING];

        let outcome = crate::testing::with_quiet_panic_hook(|| {
            std::panic::catch_unwind(|| CommandTable::new(COLLIDING))
        });

        assert!(outcome.is_err(), "중복 선언은 표 생성에서 터진다");
    }

    #[test]
    fn unknown_lookup_is_none() {
        let mut table = CommandTable::new(DECLARED);
        assert!(!table.contains("fixture.ping"), "아직 안 꽂혔다");
        assert!(table.call("fixture.ping", &mut json!({})).is_none());

        table.insert("fixture.ping", handler()).expect("삽입");
        assert!(!table.contains("nope"));
        let mut args = json!({ "kept": 1 });
        assert!(table.call("nope", &mut args).is_none());
        assert_eq!(args, json!({ "kept": 1 }), "못 찾으면 인자를 안 가져간다");
    }

    /// ★조립이 광고할 수 없는 선언을 싣고 달리지 않는다★ — 스키마 텍스트가 JSON 이 아니면 등록 패킷의
    /// `help` 도 깨지고 인자를 맞출 재료도 없다.
    #[test]
    fn a_declaration_whose_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN: CommandSpec = CommandSpec {
            name: "fixture.broken",
            effect: Effect::Read,
            since: 1,
            summary: "깨진 스키마",
            args_schema: "{not json",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "BrokenArgs",
            ok_type: "BrokenOk",
        };
        static WITH_BROKEN: &[&CommandSpec] = &[&BROKEN];

        let mut table = CommandTable::new(WITH_BROKEN);
        assert_eq!(
            table.insert("fixture.broken", handler()),
            Err(TableError::InvalidSchema("fixture.broken"))
        );
        assert!(table.is_empty());
    }

    /// ★`ok` 가 깨져도 같다★ — 표는 `ok_schema` 를 안 쓰지만 `spec_item_json` 이 그것을 `help` 에 그대로
    /// 이어 붙이므로 인자 스키마만 보면 **깨진 `help` 를 실은 등록 패킷이 통과한다**. 데몬은 불투명
    /// 문자열로 저장하는 것이 맞으니(ADR-0135) 그때는 이미 잡을 자리가 없다.
    #[test]
    fn a_declaration_whose_ok_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN_OK: CommandSpec = CommandSpec {
            name: "fixture.broken-ok",
            effect: Effect::Read,
            since: 1,
            summary: "인자는 멀쩡하고 반환이 깨졌다",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{not json",
            errors: &[],
            args_type: "BrokenOkArgs",
            ok_type: "BrokenOkOk",
        };
        static WITH_BROKEN_OK: &[&CommandSpec] = &[&BROKEN_OK];

        let mut table = CommandTable::new(WITH_BROKEN_OK);
        assert_eq!(
            table.insert("fixture.broken-ok", handler()),
            Err(TableError::InvalidSchema("fixture.broken-ok"))
        );
        assert!(table.is_empty());
        // 이 선언이 통과했다면 나갔을 `help` — JSON 이 아니다.
        assert!(
            serde_json::from_str::<serde_json::Value>(&crate::spec_item_json(&BROKEN_OK)).is_err()
        );
    }

    #[test]
    fn decls_carry_the_schema_item_as_help() {
        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("삽입");

        let decls = table.decls();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "fixture.ping");
        assert_eq!(decls[0].help, crate::spec_item_json(&PING));
        // 파생 파일의 원소와 **같은 출처**여야 한다(ADR-0135).
        let parsed: serde_json::Value = serde_json::from_str(&decls[0].help).expect("help 는 JSON");
        assert_eq!(parsed["name"], "fixture.ping");
        assert_eq!(parsed["effect"], "Read");
        assert_eq!(parsed["summary"], "두 칸 들여쓴 요약");
        assert_eq!(parsed["ok"]["properties"]["pong"]["type"], "boolean");
        assert_eq!(parsed["errors"][0], "INTERNAL");
        // 표가 얹는 공통 오류가 광고에 들어 있다(선언엔 없다).
        assert_eq!(parsed["errors"][1], "INVALID_ARGUMENT");
    }

    /// ★디스크 파일과 등록 패킷이 **같은 한 출처**라는 주장을 바이트로 확인한다★(ADR-0135).
    ///
    /// 두 경로를 서로 견주면 같은 함수를 두 번 부른 항등식이라 **둘이 함께 틀어져도 통과한다** — 그래서
    /// 고정한 기대 바이트를 가운데 둔다.
    #[test]
    fn the_schema_file_element_is_byte_identical_to_help() {
        const EXPECTED: &str = concat!(
            r#"{"name":"fixture.ping","effect":"Read","since":1,"summary":"두 칸 들여쓴 요약","#,
            r#""args":{"type":"object","properties":{}},"#,
            r#""ok":{"type":"object","properties":{"pong":{"type":"boolean"}}},"#,
            r#""errors":["INTERNAL","INVALID_ARGUMENT"]}"#
        );

        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("삽입");
        let help = table.decls().remove(0).help;

        assert_eq!(help, EXPECTED, "등록 패킷의 help");
        assert!(
            crate::catalog_json(1, DECLARED).contains(EXPECTED),
            "파생 파일의 원소"
        );
    }

    #[test]
    fn bad_arguments_become_invalid_argument() {
        let table_handler = handler();
        let outcome = block_on(table_handler.call(json!({ "unexpected": [1, 2] })));
        // 모르는 필드는 무시되고(deny_unknown_fields 를 달지 않는다 — additive 규칙) 성공한다.
        assert!(outcome.is_ok());

        let outcome = block_on(table_handler.call(json!("not an object")));
        assert_eq!(
            outcome.expect_err("객체가 아니면 반려").code(),
            ErrorCode::InvalidArgument
        );
    }

    #[derive(serde::Deserialize)]
    struct Numbers {
        count: u32,
        depth: i64,
        ratio: f64,
        rows: Vec<Inner>,
    }

    #[derive(serde::Deserialize)]
    struct Inner {
        size: u32,
    }

    /// ★어댑터를 안 거치는 핸들러★ — 조정이 `blocking_handler` 의 성질이면 이 핸들러는 같은 스키마를
    /// 광고하고 다르게 받는다. 그래서 조정 테스트를 이 형태 위에 세운다(Step 3 의 `await` 하는 명령들이
    /// 이 형태다).
    struct RawNumbers;
    impl CommandHandler for RawNumbers {
        fn call(&self, args: serde_json::Value) -> CommandFuture {
            Box::pin(async move {
                let parsed: Numbers = serde_json::from_value(args)
                    .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
                Ok(json!({
                    "count": parsed.count,
                    "depth": parsed.depth,
                    "ratio": parsed.ratio,
                    "size": parsed.rows.first().map(|r| r.size),
                }))
            })
        }
    }

    fn numbers_table() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.numbers", Arc::new(RawNumbers))
            .expect("선언된 이름");
        table
    }

    fn call_numbers(mut args: serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let table = numbers_table();
        let future = table
            .call("fixture.numbers", &mut args)
            .expect("표에 있는 이름");
        block_on(future)
    }

    /// ★광고한 스키마가 받는 것을 실제로 받는다★ — `{"type":"integer"}` 는 `1.0` 을 포함하는데(JSON 수치
    /// 모델에 `1` 과 `1.0` 의 구분이 없다) serde 는 정수 칸의 f64 를 반려한다. 그 갈림을 표가 메운다.
    #[test]
    fn an_integral_float_is_accepted_the_way_the_schema_says() {
        let outcome = call_numbers(json!({
            "count": 2.0,
            "depth": -3.0,
            "ratio": 1.0,
            "rows": [{ "size": 4.0 }],
        }))
        .expect("스키마가 광고한 대로 받는다");

        assert_eq!(outcome["count"], 2);
        assert_eq!(outcome["depth"], -3);
        assert_eq!(outcome["ratio"], 1.0, "실수 칸은 실수로 남는다");
        assert_eq!(outcome["size"], 4, "중첩·배열 안까지 같다");
    }

    #[test]
    fn a_fractional_value_is_still_refused_for_an_integer_field() {
        let outcome = call_numbers(json!({
            "count": 2.5, "depth": 0, "ratio": 1.0, "rows": [],
        }));

        assert_eq!(
            outcome.expect_err("소수부가 있으면 정수가 아니다").code(),
            ErrorCode::InvalidArgument
        );
    }

    /// ★2^53 밖에서는 f64 가 정수를 정확히 담지 못한다★ — 옮기는 것 자체가 값을 바꾸므로 그대로 둔다.
    #[test]
    fn a_float_beyond_exact_integer_range_is_left_alone() {
        let outcome = call_numbers(json!({
            "count": 0, "depth": 1e300, "ratio": 1.0, "rows": [],
        }));

        assert_eq!(
            outcome.expect_err("정수 칸에 담을 수 없다").code(),
            ErrorCode::InvalidArgument
        );
    }

    /// ★그물은 표에도 있다★ — 라우팅 밖에서 핸들러를 직접 부르는 경로(하네스 · 조립부)에서도 패닉이
    /// 답장을 삼키면 안 된다. 이 단언이 없으면 `route` 의 바깥 그물이 이 자리의 결함을 가린다.
    #[test]
    fn a_panicking_body_is_folded_into_an_error_without_route() {
        let panicking = blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
            panic!("handler blew up")
        });

        let outcome = crate::testing::with_quiet_panic_hook(|| block_on(panicking.call(json!({}))));

        let err = outcome.expect_err("패닉은 오류가 된다");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.message().contains("handler blew up"), "사유가 실린다");
    }
}
