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
// ADR-0140
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
        //   아니게 만든다. 데몬은 그것을 불투명 문자열로 저장하는 것이 옳으므로(ADR-0141) 깨짐은 그
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

    /// 인자가 **선언과 맞는지** 부르기 전에 본다 — 모르는 칸·빠진 필수 칸을 이름 지어 반려한다.
    ///
    /// ★부르는 자리는 사람·LLM 이 방금 친 것이 들어오는 표면뿐이다(ADR-0142)★. 홉 간 배선에서 부르면
    /// 버전이 앞선 호출자가 실은 신규 칸이 옛 주인을 하드 실패시켜 additive 진화가 죽는다(TRD §4-③) —
    /// [`crate::route`] 가 이것을 부르지 않는 것이 그 결정의 실물이고, 거기 끼워 넣으면 무너진다.
    ///
    /// 반려 문구는 **틀린 칸과 선언된 칸 전량**을 함께 싣는다 — 호출자가 스스로 고칠 수 있어야 그물이
    /// 값을 한다. ★문구에 호출 표면의 어휘를 넣지 않는다★: 그 표면이 무엇인지 이 crate 는 모르고, 자기
    /// 안내는 어댑터가 [`CommandError::set_message`] 로 덧붙인다.
    ///
    /// 통과시키는 것 셋(검사할 재료가 없는 자리다):
    /// - **이 표에 없는 이름** — 그 계약은 남이 쥐고 있어 대조할 선언이 여기 없다. ★뒤에서 아무도 대신
    ///   봐 주지 않는다★: 명부가 그 이름을 알면 배달은 봉투를 **그대로 전달**하고, 받는 홉의
    ///   [`crate::route`]→[`CommandTable::call`] 도 검문하지 않는다(바로 위 문단의 그 결정이다). 그래서
    ///   사람·LLM 이 남의 이름을 오타 낀 칸과 함께 치면 그 칸은 어디서도 안 걸리고, 역직렬화가 조용히
    ///   버린 뒤 성공이 보고된다 — ADR-0142 이 막으려던 그 실패가 한 홉 건너에서 그대로 산다(그 결정의
    ///   층 가름은 **출처**이지 이름의 주인이 아니다).
    ///   ★이 구멍은 입구 어댑터 **쌍**의 의무로 남아 있다 — 배선 wave 가 어느 쪽인지 정한다★: 치는 쪽
    ///   어댑터는 남의 선언을 안 들고(명부의 모양은 데몬에게 불투명 문자열이다 — ADR-0141), 주인 쪽
    ///   어댑터는 그 봉투가 사람·LLM 이 방금 친 것인지를 봉투만 보고 모른다. 어느 쪽이 그 재료를 갖게
    ///   할지가 결정 사항이라 이 crate 에서 닫지 않는다.
    /// - **`properties` 를 안 실은 선언** — 허용 집합을 모르는 채로 반려하면 멀쩡한 인자를 막는다.
    /// - **꼭대기 아래** — 여기는 스키마 검증기가 아니라 선언 속성 집합과의 대조다. 중첩 객체 안의 오타는
    ///   통과하고, 그 자리는 역직렬화가 잡는다(선언된 칸이면 타입이 안 맞고, 아니면 무시된다).
    // ADR-0142
    pub fn check_args(&self, name: &str, args: &serde_json::Value) -> Result<(), CommandError> {
        let Some(entry) = self.entries.get(name) else {
            return Ok(());
        };
        let serde_json::Value::Object(given) = args else {
            return Err(CommandError::invalid_argument(format!(
                "arguments for '{name}' must be an object — declared arguments: {}",
                declared_names(&entry.args_schema)
            )));
        };
        let Some(declared) = entry
            .args_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(());
        };
        // 모르는 칸을 먼저 본다 — 오타는 「모르는 칸 하나 + 빠진 필수 칸 하나」로 함께 걸리는데, 호출자가
        //   고칠 것은 **자기가 친 그 칸**이다.
        // ★틀린 칸을 **전부** 센다★: 하나만 짚으면 호출자는 고치고 다시 보내고 또 반려당하기를 반복한다
        //   (칸 순서는 사전순이라 「먼저 친 것」도 아니다). 오타 셋을 한 번에 보여 주는 것이 그물의 값이다.
        let unknown: Vec<&String> = given
            .keys()
            .filter(|key| !declared.contains_key(key.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(CommandError::invalid_argument(format!(
                "not an argument of '{name}': {} — declared arguments: {}",
                quoted_list(&unknown),
                declared_names(&entry.args_schema)
            )));
        }
        if let Some(serde_json::Value::Array(required)) = entry.args_schema.get("required") {
            for wanted in required.iter().filter_map(serde_json::Value::as_str) {
                if !given.contains_key(wanted) {
                    return Err(CommandError::invalid_argument(format!(
                        "'{name}' requires '{wanted}' — declared arguments: {}",
                        declared_names(&entry.args_schema)
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 데몬에 등록할 명단 — 이름과 **모양**을 함께 나른다(ADR-0141).
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

/// 반려 문구가 싣는 허용 집합 — 자르지 않는다. 잘린 목록은 호출자가 고르지 못한 이름이 잘린 쪽에
/// 있는지 알 수 없어 스스로 고칠 수 없다(이 이름들은 **우리 선언**이라 크기가 우리 손에 있다 —
/// 호출자가 준 문자열은 [`quoted_input`] 이 자른다).
fn declared_names(args_schema: &serde_json::Value) -> String {
    let Some(properties) = args_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        // 허용 집합을 안 실은 선언 — 그 자리는 통과시키므로(`check_args`) 여기 오는 것은 객체가 아닌
        //   인자로 걸린 경우뿐이다.
        return "(not declared)".to_string();
    };
    if properties.is_empty() {
        return "(none)".to_string();
    }
    properties
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// 한 문구에 이름 지어 싣는 **틀린 칸 수**의 상한.
///
/// ★전부 세되 전부 싣지는 않는다★: 칸 수는 요청 바디 크기만큼 늘 수 있어(수천 개) 상한이 없으면 반려
/// 문구가 그만큼 커진다 — 그 비용은 친 쪽이 아니라 받는 쪽이 낸다([`quoted_input`] 과 같은 논거). 넘치면
/// **몇 개가 더 있는지 말한다**(말 안 하면 호출자는 목록을 전부로 읽고 남은 칸을 못 고친다).
/// ★이름 지어지는 여덟은 「호출자가 먼저 친 여덟」이 **아니다**★ — 인자 맵은 정렬 자료구조라 사전순 앞
/// 여덟이고, 그 순서는 호출자가 만든 것이 아니다. 「먼저 친 것부터」를 원하면 맵을 삽입 순서 보존형으로
/// 바꿔야 하고 그건 이 crate 밖 결정이다(아래 테스트가 이 선택을 못박는다).
const MAX_NAMED_UNKNOWN: usize = 8;

/// 이름 하나의 **리터럴**(`"…"` — 여닫이 따옴표와 말줄임표 포함) 크기 상한(바이트).
///
/// ★없으면 100 MiB 짜리 칸 이름 하나가 그만한 문구를 만든다★ — 그 문구는 오류 답장에 실려 선을 타고,
/// 그것을 만드는 비용은 인자를 친 쪽이 아니라 받는 쪽이 낸다. 값은 사람이 자기 오타를 알아볼 만큼이다.
/// ★이 수가 **재는 것과 안 재는 것**★: 재는 것은 리터럴이고, 문구에 실제로 들어가는 조각은 여기에 잘림
/// 표시(`(truncated, input was N bytes)` — 30 B 남짓)가 더 붙어 **~170 B**까지 간다. 조각 전체의 수가 필요하면
/// 그 합을 쓸 것 — 이 상수를 그 자리에 그대로 옮겨 적지 말 것.
/// ★줄이려고 두는 상한이 아니다★: 이스케이프 표기가 129~158 B 인 입력에서는 잘린 형태가 원본 표기보다 오히려
/// **길다**(꼬리표가 붙으니까). 이 상한이 지키는 것은 평균 크기가 아니라 **꼬리의 유한함**이다.
const MAX_QUOTED_INPUT_BYTES: usize = 128;

/// 틀린 칸 이름들을 문구에 싣는다 — 각 이름은 [`quoted_input`] 상한을, 개수는 [`MAX_NAMED_UNKNOWN`] 을 탄다.
fn quoted_list(keys: &[&String]) -> String {
    let named: Vec<String> = keys
        .iter()
        .take(MAX_NAMED_UNKNOWN)
        .map(|key| quoted_input(key))
        .collect();
    let listed = named.join(", ");
    match keys.len().checked_sub(MAX_NAMED_UNKNOWN) {
        Some(rest) if rest > 0 => format!("{listed} (+{rest} more)"),
        _ => listed,
    }
}

/// 호출자가 준 이름을 문구에 인용할 수 있게 다듬는다 — 자를 때는 **잘랐다고 말한다**(안 그러면 호출자는
/// 잘린 이름을 자기가 친 이름으로 읽고 멀쩡한 칸을 고치러 간다).
///
/// ★이름은 Rust 문자열 리터럴 표기로 감싼다★: 작은따옴표로 감싸면 `a', 'b` 라는 칸 **하나**가 `'a', 'b'`
/// 로 찍혀 **틀린 칸 둘**로 읽힌다 — 호출자는 있지도 않은 칸을 고치러 간다. `{:?}` 는 따옴표·역슬래시·제어
/// 문자를 이스케이프해 그 애매함을 없앤다.
/// ★잘린 이름도 **따옴표를 닫는다**(load-bearing)★: 안 닫으면 이름 여럿을 이어 붙인 문구에서 따옴표 짝이
/// 어긋나, 방금 없앤 그 애매함이 그대로 돌아온다(닫는 따옴표 없는 조각 + 다음 이름의 여는 따옴표가 한 쌍으로
/// 읽힌다).
/// ★상한은 **이스케이프한 뒤** 건다★: 먼저 자르면 제어문자 한 글자가 `\u{1b}` 여섯 바이트로 부풀어 문구가
/// 상한의 여섯 배까지 커진다 — 상한이 「입력 종류에 따라」 달라져 문서에 적은 숫자가 거짓이 된다.
/// ★이스케이프 시퀀스 한가운데서 자르지 않는다★: 글자 단위로 쌓다가 예산에서 멈추므로 `\` 하나만 남는
/// 조각이 생기지 않는다 — 그런 조각은 표기가 깨져 위의 「리터럴 표기」 약속을 어긴다.
/// ★잘림 표시는 **무엇을 센 수인지** 밝힌다★: 보이는 것은 이스케이프된 표기이고 수는 원본 바이트라, 라벨이
/// 없으면 제어문자 30개(표기로는 180자)가 「30 바이트를 잘랐다」로 읽혀 서로 어긋나 보인다.
fn quoted_input(text: &str) -> String {
    let quoted = format!("{text:?}");
    if quoted.len() <= MAX_QUOTED_INPUT_BYTES {
        return quoted;
    }
    // 여는 따옴표 · 닫는 따옴표 · 말줄임표 자리를 미리 뺀다.
    let budget = MAX_QUOTED_INPUT_BYTES.saturating_sub('"'.len_utf8() * 2 + '…'.len_utf8());
    let mut shown = String::with_capacity(budget);
    for ch in text.chars() {
        let piece = ch.escape_debug().to_string();
        if shown.len() + piece.len() > budget {
            break;
        }
        shown.push_str(&piece);
    }
    format!("\"{shown}…\"(truncated, input was {} bytes)", text.len())
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
    /// 필수 칸과 생략 가능한 칸을 함께 든 선언 — 입구 검문이 그 둘을 가르는지 여기서 본다.
    static ENTRANCE: CommandSpec = CommandSpec {
        name: "fixture.entrance",
        effect: Effect::Write,
        since: 1,
        summary: "입구 검문 픽스처",
        args_schema: concat!(
            r#"{"type":"object","properties":{"cwd":{"type":"string"},"#,
            r#""label":{"anyOf":[{"type":"string"},{"type":"null"}]}},"#,
            r#""required":["cwd"]}"#
        ),
        ok_schema: "{\"type\":\"object\",\"properties\":{}}",
        errors: &[],
        args_type: "EntranceArgs",
        ok_type: "EntranceOk",
    };
    static DECLARED: &[&CommandSpec] = &[&PING, &NUMBERS, &ENTRANCE];

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
    /// 문자열로 저장하는 것이 맞으니(ADR-0141) 그때는 이미 잡을 자리가 없다.
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
        // 파생 파일의 원소와 **같은 출처**여야 한다(ADR-0141).
        let parsed: serde_json::Value = serde_json::from_str(&decls[0].help).expect("help 는 JSON");
        assert_eq!(parsed["name"], "fixture.ping");
        assert_eq!(parsed["effect"], "Read");
        assert_eq!(parsed["summary"], "두 칸 들여쓴 요약");
        assert_eq!(parsed["ok"]["properties"]["pong"]["type"], "boolean");
        assert_eq!(parsed["errors"][0], "INTERNAL");
        // 표가 얹는 공통 오류가 광고에 들어 있다(선언엔 없다).
        assert_eq!(parsed["errors"][1], "INVALID_ARGUMENT");
    }

    /// ★디스크 파일과 등록 패킷이 **같은 한 출처**라는 주장을 바이트로 확인한다★(ADR-0141).
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

    // ── 입구 검문(ADR-0142) ──────────────────────────────────────────────────────────────────

    fn entrance_table() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.entrance", handler())
            .expect("선언된 이름");
        table
    }

    /// ★모르는 칸을 조용히 무시하면 **성공이 보고된다**★ — 호출자는 자기가 지시한 것과 다른 일이 일어난
    /// 줄 모른다(ADR-0142 이 이 그물을 세운 이유). 그래서 반려하되 **스스로 고칠 재료**를 함께 낸다.
    #[test]
    fn an_undeclared_argument_field_is_named_and_refused() {
        let err = entrance_table()
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha" }),
            )
            .expect_err("선언에 없는 칸");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("target"),
            "틀린 칸을 지목한다: {}",
            err.message()
        );
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "허용 집합을 함께 낸다: {}",
            err.message()
        );
    }

    /// ★틀린 칸이 여럿이면 여럿 다 나온다★ — 하나만 짚으면 호출자는 반려·수정·재시도를 오타 수만큼
    /// 반복한다. 칸 순서는 사전순이라 「처음 친 칸」이라는 뜻도 없다.
    #[test]
    fn every_undeclared_argument_field_is_named_not_just_the_first() {
        let err = entrance_table()
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha", "zzz": 1, "aaa": 2 }),
            )
            .expect_err("선언에 없는 칸 셋");

        for offender in ["target", "zzz", "aaa"] {
            assert!(
                err.message().contains(offender),
                "{offender} 이 빠졌다: {}",
                err.message()
            );
        }
    }

    /// 선언에 없는 칸 `count` 개(각 `key_bytes` 바이트)를 실은 반려 문구.
    ///
    /// ★넣는 순서를 **사전 역순**으로 둔다★: 오름차순으로 넣으면 삽입 순서와 사전순이 같아져, 맵이 삽입
    /// 순서 보존형으로 바뀌어도(`preserve_order`) 「사전순 앞 여덟」 단언이 그대로 통과한다 — 그 단언이
    /// 재려던 성질이 사라진 줄 아무도 모른다.
    fn flood_refusal(count: usize, key_bytes: usize) -> String {
        flood_refusal_of(count, key_bytes, "x")
    }

    /// `fill` = 이름을 채우는 문자 — 인쇄 가능 문자와 이스케이프되는 문자가 같은 상한을 타는지 재려고
    /// 갈라 둔다(이스케이프 확장은 상한을 종류별로 다르게 만들 수 있는 축이다).
    fn flood_refusal_of(count: usize, key_bytes: usize, fill: &str) -> String {
        let mut given = serde_json::Map::new();
        given.insert("cwd".to_string(), json!("C:/x"));
        for i in (0..count).rev() {
            given.insert(format!("stray{i:04}{}", fill.repeat(key_bytes)), json!(1));
        }
        entrance_table()
            .check_args("fixture.entrance", &serde_json::Value::Object(given))
            .expect_err("선언에 없는 칸 다수")
            .message()
            .to_string()
    }

    /// 개수 상한을 넘으면 **몇 개가 더 있는지** 말한다 — 목록이 전부인 줄 알면 남은 칸을 못 고친다.
    #[test]
    fn a_flood_of_unknown_fields_is_capped_and_says_how_many_are_left() {
        let message = flood_refusal(40, 0);

        assert!(
            message.contains("(+32 more)"),
            "남은 개수를 말한다: {message}"
        );
        assert!(
            message.contains("stray0000"),
            "이름 지어진 칸이 실제로 실린다: {message}"
        );
    }

    /// ★문구 길이는 요청 크기와 **무관**하다★ — 지켜야 할 성질은 「몇 바이트 이하」가 아니라 「입력이 커져도
    /// 안 자란다」다. 이름 하나의 상한은 [`quoted_input`], 이름 개수의 상한은 [`MAX_NAMED_UNKNOWN`] 이 잡고,
    /// 둘이 함께 서야 이 성질이 선다(한쪽만 있으면 다른 축으로 자란다).
    ///
    /// 실측 최악은 **약 1.4 KB**(이름 여덟 × 인용 상한 128B + 잘림 표시 + 선언 집합)이고, 이스케이프되는
    /// 이름도 같은 값이다(상한이 이스케이프 **뒤**에 걸리므로) — 그래도 숫자를 계약처럼 읽지 말 것. 계약은
    /// 아래 첫 단언(입력이 커져도 문구는 자릿수만큼만 는다)이다.
    #[test]
    fn the_refusal_message_does_not_grow_with_the_body() {
        let small = flood_refusal(40, 1 << 10);
        let huge = flood_refusal(4_000, 1 << 14);

        // 자라도 되는 폭 = **자릿수뿐**이다: 이름 여덟의 「N 바이트」 표기와 「+N more」의 숫자가 길어진다.
        //   여유 32 는 그 자릿수 증가분(입력이 1000배여도 십진 자릿수는 몇 자리만 는다)을 덮는 크기다.
        assert!(
            huge.len() <= small.len() + 32,
            "칸 수 100배·이름 크기 16배인데 문구가 자릿수 이상으로 자랐다: {} → {} 바이트",
            small.len(),
            huge.len()
        );
        // 이스케이프되는 이름도 같은 상한을 탄다 — 여기가 이전 두 판에서 문서 숫자를 거짓으로 만든 축이다.
        let escaped = flood_refusal_of(40, 1 << 10, "\u{1b}");
        for (label, measured) in [("printable", small.len()), ("escaped", escaped.len())] {
            assert!(
                measured < 2048,
                "{label}: 실측 최악(~1.4 KB) 근처를 벗어났다 — 상한 둘 중 하나가 풀렸다: {measured} 바이트"
            );
        }
    }

    /// ★이름 지어지는 여덟은 **사전순 앞 여덟**이고, 실리는 **차례도 사전순**이다★ — 호출자가 먼저 친
    /// 여덟이 아니다(인자 맵이 정렬 자료구조다). 픽스처가 사전 **역순**으로 넣으므로 맵이 삽입 순서
    /// 보존형으로 바뀌면 여기가 빨개진다.
    /// ★이름을 잘리게 두는 것이 요점이다★: 이름 상한과 개수 상한이 **함께** 걸리는 조합이라야 둘이 서로를
    /// 가리지 않는지 본다(짧은 이름만 쓰면 잘림 경로가 이 조합에서 한 번도 안 돈다).
    #[test]
    fn the_named_subset_is_the_alphabetically_first_ones_in_order() {
        let message = flood_refusal(40, 1 << 8);

        let at = |name: &str| message.find(name);
        // 양 끝만 보면 가운데 여섯이 섞여도 통과한다 — 여덟 자리를 전부 본다.
        let places: Vec<usize> = (0..MAX_NAMED_UNKNOWN)
            .map(|i| at(&format!("stray{i:04}")).expect("이름 지어진 여덟"))
            .collect();
        assert!(
            places.windows(2).all(|pair| pair[0] < pair[1]),
            "사전순 차례로 이어 붙인다: {message}"
        );
        assert!(
            at("stray0008").is_none(),
            "아홉째부터는 개수로만 센다: {message}"
        );
        assert!(
            at("stray0039").is_none(),
            "삽입 순서(역순 첫 칸)가 아니라 사전순으로 고른다: {message}"
        );
        assert!(
            message.contains("truncated"),
            "이 조합에서 이름 상한도 함께 걸린다: {message}"
        );
    }

    /// 잘린 조각을 `"본문…"` + 잘림 표시로 가른다 — 본문에 무엇이 들었나를 재려면 경계가 필요하다.
    fn literal_and_body(rendered: &str) -> (&str, &str) {
        let marker = rendered
            .find("(truncated")
            .expect("잘림 표시가 있어야 한다");
        let literal = &rendered[..marker];
        let body = literal
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_end_matches('…');
        (literal, body)
    }

    /// ★잘린 이름도 **닫힌 문자열 리터럴**이어야 한다★ — 이름 여럿을 이어 붙인 문구에서 따옴표 짝이
    /// 어긋나면 `{:?}` 로 없애려던 애매함(칸 하나가 둘로 읽힘)이 그대로 돌아온다.
    ///
    /// ★재는 것은 셋이고, 셋 다 있어야 한다★: ① 리터럴 **크기**가 상한을 지키나(예산에서 여닫이 따옴표
    /// 자리를 빼먹으면 여기서 걸린다) ② 본문에 **이스케이프 안 된 따옴표**가 없나(잘림 갈래에서 이스케이프를
    /// 빼먹으면 여기서 걸린다 — 그래서 픽스처 이름이 따옴표를 품는다) ③ 이스케이프 중간에서 안 잘렸나.
    /// 셋 중 하나라도 빠지면 이 함수의 옛 판본들이 실제로 낸 결함이 그대로 통과한다.
    #[test]
    fn a_truncated_name_is_still_a_closed_string_literal() {
        let key = format!("a\"b{}", "c".repeat(300));
        let rendered = quoted_input(&key);
        let (literal, body) = literal_and_body(&rendered);

        assert!(rendered.starts_with('"'), "여는 따옴표: {rendered}");
        assert!(
            rendered.contains("…\"(truncated"),
            "닫는 따옴표가 잘림 표시 **앞**에 온다: {rendered}"
        );
        assert!(
            literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "리터럴이 상한을 넘었다(따옴표·말줄임표 자리를 예산에서 안 뺐다): {} 바이트 — {rendered}",
            literal.len()
        );
        assert!(
            rendered.contains(&format!("input was {} bytes", key.len())),
            "무엇을 센 수인지 밝힌다: {rendered}"
        );

        // ★본문의 따옴표는 **전부 이스케이프돼 있어야** 한다★ — 하나라도 날것이면 그 자리가 칸 경계로
        //   읽힌다(개수만 세면 `\"` 도 한 개로 세어져 이 결함을 놓친다).
        let mut escaped = false;
        for ch in body.chars() {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => panic!("본문에 이스케이프 안 된 따옴표가 있다: {rendered}"),
                _ => {}
            }
        }

        // ★본문의 **마지막 이스케이프가 온전한지**를 본다★: `\u{1b}` 는 여섯 글자라 중간에서 잘릴 자리가
        //   다섯이고, 그중 「`\` 바로 뒤」 하나만 역슬래시 홀짝으로 잡힌다. 나머지 넷(`\u`·`\u{`·`\u{1`·
        //   `\u{1b`)은 홀짝 검사를 그대로 통과하므로, 마지막 `\` 부터 끝까지가 **닫힌 시퀀스**인지 본다.
        let control = quoted_input(&"\u{1b}".repeat(200));
        let (control_literal, control_body) = literal_and_body(&control);
        if let Some(last) = control_body.rfind('\\') {
            let tail = &control_body[last..];
            let complete = match tail.chars().nth(1) {
                // `\u{…}` 는 닫는 중괄호까지 있어야 한 글자를 뜻한다.
                Some('u') => tail.contains('}'),
                // 한 글자짜리 이스케이프(`\n`·`\\`·`\"` 등)는 그 한 글자가 붙어 있으면 온전하다.
                Some(_) => true,
                // 역슬래시로 끝났다 — 아무것도 안 이스케이프하는 조각이다.
                None => false,
            };
            assert!(
                complete,
                "본문이 미완성 이스케이프로 끝났다(시퀀스 한가운데서 잘렸다): {control}"
            );
        }
        assert!(
            control_literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "제어문자 이름의 리터럴이 상한을 넘었다: {} 바이트 — {control}",
            control_literal.len()
        );
        assert!(
            control.contains("input was 200 bytes"),
            "표기는 이스케이프된 것이고 수는 원본이라는 것을 라벨이 밝힌다: {control}"
        );
    }

    /// ★상한은 입력 **종류**와도 무관해야 한다★: 이스케이프 전에 자르면 제어문자 한 글자가 여섯 바이트로
    /// 부풀어, 같은 상한이 인쇄 가능 문자에는 128 B·제어문자에는 그 여섯 배로 달라진다 — 문서에 적은 숫자가
    /// 그 순간 거짓이 된다.
    #[test]
    fn an_escape_heavy_key_obeys_the_same_bound_as_a_printable_one() {
        let refusal_len = |key: String| {
            let mut given = serde_json::Map::new();
            given.insert(key, json!(1));
            entrance_table()
                .check_args("fixture.entrance", &serde_json::Value::Object(given))
                .expect_err("선언에 없는 칸")
                .message()
                .len()
        };

        let printable = refusal_len("k".repeat(4096));
        let escaped = refusal_len("\u{1b}".repeat(4096));

        assert!(
            escaped <= printable + 8,
            "제어문자 이름이 인쇄 가능 이름보다 긴 문구를 만들었다: {printable} → {escaped} 바이트"
        );
    }

    /// ★핸들러가 돌기 전에 걸린다★ — 검문은 순수한 사전 판정이라 본문이 없어도, 본문이 터지는 것이어도
    /// 같은 답을 낸다(터지는 핸들러가 그것을 단언한다).
    #[test]
    fn a_missing_required_field_is_refused_before_the_handler() {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert(
                "fixture.entrance",
                blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
                    panic!("검문이 통과시키면 안 되는 인자다")
                }),
            )
            .expect("선언된 이름");

        let err = table
            .check_args("fixture.entrance", &json!({ "label": "alpha" }))
            .expect_err("필수 칸이 빠졌다");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd"),
            "빠진 칸을 지목한다: {}",
            err.message()
        );
    }

    #[test]
    fn a_declared_optional_field_may_be_absent() {
        let table = entrance_table();

        table
            .check_args("fixture.entrance", &json!({ "cwd": "C:/x" }))
            .expect("생략 가능한 칸은 없어도 된다");
        table
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "label": "alpha" }),
            )
            .expect("실어도 된다");
    }

    /// ★반려는 **언제나** 허용 집합을 함께 낸다★ — 「객체가 아니다」만 받은 호출자는 무엇을 실어야 하는지
    /// 모르는 채로 다시 찍어 봐야 한다(스스로 고칠 재료를 내는 것이 이 검문의 계약이다).
    #[test]
    fn a_non_object_refusal_also_carries_the_declared_set() {
        let err = entrance_table()
            .check_args("fixture.entrance", &json!("C:/x"))
            .expect_err("객체가 아니다");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "허용 집합을 함께 낸다: {}",
            err.message()
        );
    }

    /// ★반려 문구는 호출자가 준 문자열을 되받아 인용한다★ — 상한이 없으면 거대한 칸 이름 하나가 그만한
    /// 문구를 만들고, 그것이 오류 답장에 실려 선을 탄다.
    #[test]
    fn a_huge_unknown_key_is_quoted_within_a_bound() {
        let huge = "k".repeat(1 << 20);
        let mut given = serde_json::Map::new();
        given.insert("cwd".to_string(), json!("C:/x"));
        given.insert(huge, json!(1));

        let err = entrance_table()
            .check_args("fixture.entrance", &serde_json::Value::Object(given))
            .expect_err("선언에 없는 칸");

        // 이름 하나짜리 문구의 실측은 ~230 바이트다(인용 상한 128 + 잘림 표시 + 선언 집합) — 아래 숫자는
        //   그 실측에 여유를 준 값이지 계약이 아니다. 계약은 「입력 크기와 무관」이고
        //   `the_refusal_message_does_not_grow_with_the_body` 가 그것을 잰다.
        assert!(
            err.message().len() < 512,
            "1 MiB 이름 하나가 그만한 문구를 만들었다: {} 바이트",
            err.message().len()
        );
        assert!(
            err.message().contains("truncated"),
            "잘랐다고 말한다: {}",
            err.message()
        );
    }

    /// ★문구가 호출 표면을 배우면 안 된다★ — 이 crate 는 자기 반려가 어느 표면에 뜨는지 모른다. 표면별
    /// 안내는 어댑터가 [`CommandError::set_message`] 로 덧붙이므로, 여기서 지어내면 두 안내가 어긋난다.
    #[test]
    fn check_args_does_not_know_any_cli_vocabulary() {
        let table = entrance_table();
        let refusals = [
            table.check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha" }),
            ),
            table.check_args("fixture.entrance", &json!({})),
            table.check_args("fixture.entrance", &json!("not an object")),
        ];

        for refusal in refusals {
            let message = refusal.expect_err("반려").message().to_lowercase();
            for vocabulary in [
                "engram",
                "--",
                "flag",
                "cli",
                "command line",
                "usage",
                "help",
            ] {
                assert!(
                    !message.contains(vocabulary),
                    "호출 표면의 어휘가 샜다({vocabulary}): {message}"
                );
            }
        }
    }

    /// ★남의 이름은 판정하지 않는다★ — 그 계약은 이 표에 없다. 여기서 반려하면 명부로 갈 봉투가 입구에서
    /// 죽어 2단 배달(TRD §3-8)이 성립하지 않는다.
    #[test]
    fn a_name_this_table_does_not_hold_is_left_to_the_next_step() {
        entrance_table()
            .check_args("tab.create", &json!({ "anything": 1 }))
            .expect("남의 이름은 통과한다");
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
