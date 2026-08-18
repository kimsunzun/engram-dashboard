//! `/control/agent` 입구(ADR-0132 결정 6) — CLI 동사를 **명령 표**로 배달하는 어댑터(ADR-0149).
//!
//! ★역할★: CLI(`engram agent …`)가 POST 한 `{verb, …}` 를 받아 `agent.<verb>` 를 표에서 찾아 부른다.
//!   동사 본문은 이 파일에 없다 — `agent.*` 의 선언과 본문은 **core 가 소유**하고(선언이 사는 곳이 곧
//!   주인), 여기가 더하는 것은 셋뿐이다: (a) `{verb}` → 명령 이름 (b) 입구 인자 검문(ADR-0151)
//!   (c) 표의 결말 → wire 봉투. 표가 아직 안 꽂혔을 때의 503 은 HTTP 어댑터 몫이다(`mcp_server`).
//!
//! ★이 라우트는 전원 개방이다(ADR-0132 결정 5)★: 스폰된 에이전트는 백엔드와 무관하게 `engram` 배선을
//!   받으므로 여기 닿는다. 그 배선이 **우편 CLI 도 함께** 깐다는 것은 사실이고(실행파일이 하나라 계열
//!   단위로 갈라 깔 수 없다), 그래서 우편은 **데몬이 자격증명으로 거절**한다(ADR-0133 — 강제 지점은
//!   `mcp_server.rs` 의 auth 미들웨어 하나뿐이다). 이 라우트에 우편 축의 검사를 다시 두지 말 것.
//!
//! ★`kill`·`rm` 이 없는 것도 미구현이 아니다★ — `CLI_AGENT_VERBS` 주석이 정본(ADR-0122 미해소).
//!
//! ★입력 규율은 층이 둘이다(ADR-0151)★
//!   - **모르는 칸·빠진 필수 칸은 이 입구가 거절한다.** 사람·LLM 이 방금 친 것이 오는 자리라 「모르는
//!     칸 = 오타」로 읽어도 안전하다. `parnet` 오타를 흘려보내면 `move … --parent lead` 가 **루트로 떼기**로
//!     조용히 바뀐다. 판정 목록은 손으로 두지 않고 **선언에서 파생**한다(`CommandTable::check_args`) —
//!     사본을 두면 동사를 하나 늘릴 때마다 두 곳이 갈리고, 갈린 쪽이 조용히 통과시킨다.
//!   - **공백 값·부재/`null` 의 구분은 표 쪽(core `agent::commands`)이 본다.** 어느 칸이 그런지는 동사마다
//!     다르고, 그 판정은 동사 본문과 한 집에 있어야 둘이 갈리지 않는다.
//!
//! ★지목 토큰을 다듬지 않는다(trim 없음)★ — 이 계열은 **정확 일치**를 약속하므로 `"worker "` 는
//!   `worker` 가 아니다. 규칙도 그 실물도 core 하나가 소유한다(`agent::commands::resolve_in`) — 이 입구는
//!   토큰을 손대지 않고 그대로 표에 넘긴다. 우편 입구와의 규칙 일치는 `tests/control_agent.rs` 의 교차
//!   대조가 그 함수를 태워 지킨다.
//!
//! tauri import 0(daemon crate).
// ADR-0149
// ADR-0151

use engram_dashboard_command::{CommandError, CommandFuture, CommandTable, ErrorCode};
use engram_dashboard_core::agent::types::{CLI_EXE_NAME, CLI_GROUP_AGENT};
use futures_util::FutureExt as _;

use super::ingress::ControlQueryResult;

/// 명부가 바뀌었음을 붙어 있는 클라이언트 전원에게 알리는 출구(포트).
///
/// ★왜 포트인가★: 실제 통지는 전-연결 팬아웃으로 `ProfileListUpdated` 를 미는 것인데, 그 조립은
///   `connection_core` 소유다. `control/` 이 그쪽을 직접 부르면 데몬 층 결정(ADR-0130)이 재론 대상이 되므로
///   (이 디렉토리의 나가는 간선 0 이 추적 중인 성질이다) 소비자인 여기가 좁은 trait 만 소유하고 실물
///   어댑터는 조립부가 준다 — 메시징 커널의 포트 규율(ADR-0110)과 같은 모양이다.
/// ★이게 없으면 나는 증상★: 에이전트가 이름을 바꾸거나 형제를 띄워도 대시보드 트리는 **무관한 이벤트가
///   올 때까지 옛 명부를 보여 준다**(조용한 stale).
/// ★부르는 쪽은 표다★ — 명부를 바꾼 동사가 통지까지 책임진다(core `agent::commands::RosterChanged`).
///   이 입구는 통지를 겹쳐 보내지 않는다(겹치면 깨우기 0회·변경 1회라는 분담이 무너진다).
// ADR-0132
// ADR-0130
pub trait RosterBroadcast: Send + Sync {
    /// 논블록이어야 한다 — 팬아웃은 연결별 큐에 try_send 만 한다.
    fn roster_changed(&self);
}

/// `/control/agent` 요청 바디 — `verb` 하나만 이 입구의 어휘이고 **나머지는 통째로 명령 인자**다.
///
/// ★인자를 타입으로 받지 않는 이유(load-bearing)★: 인자의 계약은 선언(core)이 쥐고 있다. 여기서 다시
///   struct 로 받으면 **두 번째 인자 어휘**가 생겨, 선언에 칸이 하나 늘 때 이 파일이 함께 안 늘면 그 칸이
///   조용히 사라진다. 날 것으로 실어 보내면 모르는 칸의 판정도(ADR-0151) 값의 해석도 선언 한 곳에서 난다.
/// ★그래도 역직렬화가 실패하는 부류는 남는다(정직한 범위)★: `verb` 의 타입이 어긋난 값(`{"verb": 5}`)·
///   중복 키·객체가 아닌 바디·깨진 JSON 은 여기까지 오지 못한다. 그 부류도 빈 400 이 되지 않도록
///   **어댑터가 직접 역직렬화해 serde 의 사유 문구를 봉투에 실어 보낸다**(`mcp_server::control_agent_handler`)
///   — 즉 이 라우트가 빈 body 를 내는 경우는 인증 실패와 요청 크기 초과뿐이다.
#[derive(Debug, Default, serde::Deserialize)]
pub struct AgentRequest {
    /// ★필수인데 `default` 인 이유★: 없으면 역직렬화가 실패해 **빈 400** 으로 끝나는데, 그 응답은 무엇을
    ///   고쳐야 하는지 말해 주지 않는다. 빈 문자열로 받아 아래 dispatch 가 "모르는 동사" 반려를 내면
    ///   호출자가 help 로 갈 수 있다.
    #[serde(default)]
    pub verb: String,
    #[serde(flatten)]
    pub args: CommandArgs,
}

/// 명령 인자 칸 전량 — **같은 키가 두 번 실리면 반려한다.**
///
/// ★마지막 값이 이기게 두면 오타 하나가 다른 동작이 된다★: `{"parent":"lead","parent":null}` 에서 뒤 값이
///   이기면 계층에 붙이려던 요청이 **루트로 떼기**로 조용히 바뀐다 — ADR-0151 이 막으려는 바로 그 형태다.
///   JSON 이 중복 키의 뜻을 정하지 않으므로 어느 값을 고르든 근거가 없고, 근거 없는 선택을 조용히 하지
///   않는 것이 이 입구의 규율이다.
/// ★`serde_json::Map` 을 그대로 쓰지 못하는 이유★: 그 타입의 역직렬화는 중복 키를 덮어쓰기로 접는다.
/// ★보는 깊이는 **꼭대기 한 겹뿐**이다★ — 인자 객체 안의 객체(`{"target":{"parent":1,"parent":2}}`)는
///   `serde_json::Value` 가 삼키므로 거기서는 마지막 값이 이긴다. 오늘 선언된 `agent.*` 인자는 전부
///   문자열이라 그 모양은 인자 타입 검사에서 죽지만, **객체 값을 선언하는 명령이 생기면 같은 사고가 한 겹
///   아래에서 되살아난다**. 그때는 이 자리를 깊이 우선으로 바꿔야 한다(입구 검문도 같은 깊이 한계를 갖는다 —
///   `CommandTable::check_args`).
#[derive(Debug, Default)]
pub struct CommandArgs(serde_json::Map<String, serde_json::Value>);

impl CommandArgs {
    fn into_value(self) -> serde_json::Value {
        serde_json::Value::Object(self.0)
    }
}

impl std::ops::Deref for CommandArgs {
    type Target = serde_json::Map<String, serde_json::Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> serde::Deserialize<'de> for CommandArgs {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CommandArgs;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object of command arguments")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<CommandArgs, M::Error> {
                let mut args = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if args.insert(key.clone(), value).is_some() {
                        // 문구는 serde 의 중복 필드 반려와 같은 모양으로 맞춘다 — `verb` 중복과 인자 중복이
                        //   호출자에게 다른 사고로 보이면 안 된다.
                        // ★키를 짧게 싣는 이유★: 이 문구는 봉투의 `hint` 앞자리에 들어가고 회복 안내는
                        //   그 뒤에 붙는다. 키를 통째로 실으면 안내가 문구 저 끝으로 밀려 사람도 LLM 도
                        //   다음에 무엇을 할지 못 읽는다.
                        return Err(serde::de::Error::custom(format!(
                            "duplicate field `{}`",
                            preview(&key)
                        )));
                    }
                }
                Ok(CommandArgs(args))
            }
        }

        de.deserialize_map(Visitor)
    }
}

/// 제어 동사 공통 핸들러 — HTTP 어댑터가 유일하게 부르는 진입점.
///
/// ★blocking 함수다(호출자 계약)★: 표의 `agent.*` 핸들러는 전부 blocking 이다 — resume 모드의 조기 종료를
///   **약 3초 폴링**하고, 이름 변경·계층 이동은 프로필 락을 쥔 채 디스크에 저장한다(core `make_table` doc).
///   그래서 async 런타임 스레드가 아니라 blocking 풀에서 불러야 한다 — 어댑터
///   (`mcp_server::control_agent_handler`)가 `spawn_blocking` 으로 감싼다.
/// ★명부 통지는 여기서 안 한다★ — 표가 조립될 때 꽂힌 통지 포트가 동사별로 부른다(위 [`RosterBroadcast`]).
pub fn handle_agent(table: &CommandTable, req: AgentRequest) -> ControlQueryResult {
    // CLI 표면과 카탈로그 이름은 점↔공백 하나 차이다(TRD §2-1) — 그래서 동사별 표를 손으로 두지 않는다.
    let name = format!("{CLI_GROUP_AGENT}.{}", req.verb);
    let mut args = req.args.into_value();

    // 표에 없는 이름은 **인자 검문보다 먼저** 갈라낸다: `check_args` 는 모르는 이름을 통과시키므로(대조할
    //   선언이 이 표에 없다) 순서를 뒤집으면 오타 동사가 "인자 이상 없음" 을 지나 여기까지 온다.
    if !table.contains(&name) {
        return unknown_verb(&req.verb);
    }
    // ADR-0151: 사람·LLM 이 치는 입구라 선언에 없는 칸·빠진 필수 칸을 거절한다. 홉 간 배선은 관용이므로
    //   (`route` 는 이것을 안 부른다) 이 호출을 배달 경로로 옮기지 말 것 — additive 진화가 죽는다.
    if let Err(rejection) = table.check_args(&name, &args) {
        return refused(rejection);
    }
    let Some(future) = table.call(&name, &mut args) else {
        // 바로 위 `contains` 를 통과했다 — 표는 이 호출 동안 불변이라 여기 오지 않는다.
        return unknown_verb(&req.verb);
    };
    match run_here(future, &name) {
        Ok(payload) => ControlQueryResult::Ok(payload),
        Err(e) => refused(e),
    }
}

/// 표가 준 future 를 **이 스레드에서** 끝낸다.
///
/// ★실행기를 두지 않는 근거★: 이 표의 핸들러는 전부 `blocking_handler` 라 본문이 **첫 poll 에서 끝까지
///   돈다**(도구 crate 가 그것을 계약으로 적었다). 이미 blocking 풀 위에 있으므로 여기서 async 런타임을
///   다시 부르면 blocking 경계가 두 겹이 된다.
/// ★계약이 깨지면 조용히 성공하지 않는다★: 진짜 async 핸들러가 표에 들어오면 첫 poll 이 `Pending` 이고,
///   그때 이 어댑터는 답을 지어내는 대신 `OUTCOME_UNKNOWN` 으로 드러낸다.
/// ★`INTERNAL` 이 아니다★: 그 코드는 `retry: never` = **이 홉에서 확실히 실패했다**는 뜻인데, 첫 poll 이
///   이미 일의 일부를 적용했을 수 있다(그리고 폐기되는 future 는 나머지를 안 돌린다). 확실성은 「불명」이라
///   같은 request_id 로만 다시 묻게 해야 한다(TRD §4-④ · 도구 crate 의 전달 패닉이 같은 코드를 쓴다).
/// ★타입이 강제하지 않는 계약이라 계측한다★: 이 갈래는 표에 async 핸들러가 들어오는 순간에만 나고, 그때
///   증상은 오류 답장 하나뿐이라 로그가 없으면 원인을 못 찾는다.
fn run_here(future: CommandFuture, name: &str) -> Result<serde_json::Value, CommandError> {
    future.now_or_never().unwrap_or_else(|| {
        tracing::error!(
            entrance = "cli",
            command = name,
            "명령이 첫 poll 에서 끝나지 않았다 — 이 입구는 blocking 핸들러만 몬다(표에 async 핸들러가 들어왔다)"
        );
        Err(CommandError::of(
            ErrorCode::OutcomeUnknown,
            format!(
                "'{name}' did not finish on its first poll — this entrance only drives blocking handlers, so part of it may already have been applied"
            ),
        ))
    })
}

/// 표의 실패 → wire 봉투.
///
/// ★코드는 타입드 어휘 그대로 나간다★(TRD §4-⑦) — 입구마다 도메인 문자열을 지어내면 같은 실패가 표면마다
///   다른 이름으로 나가고, 기계 분기가 그 이름에 묶인다.
/// ★자기교정 경로는 **어댑터가** 붙인다★: 표와 도구 crate 는 자기가 어느 표면에서 불렸는지 모른다(그래서
///   문구에 CLI 어휘를 넣지 않는다). 이 한 줄이 없으면 호출자(LLM)는 반려를 받고도 어디서 규격을 확인할지
///   모른 채 같은 인자로 재시도한다.
fn refused(e: CommandError) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: e.code().as_str(),
        hint: format!("{} — {}.", e.message(), recovery_for(e.code())),
    }
}

/// 코드마다 **다음에 할 일**이 다르다.
///
/// ★한 문구를 전부에 붙이면 거짓말이 된다★: 인자를 고칠 수 없는 실패(대상 부재 · 내부 실패)에 "인자 규격을
///   보라" 를 달면 호출자는 없는 오타를 찾는다. 반대로 아무 경로도 안 달면 자기교정이 멈춘다.
/// ★붙이는 것은 언제나 **칠 수 있는 명령 하나**다★ — 문장으로만 안내하면 LLM 이 그 문장을 명령으로 친다.
fn recovery_for(code: ErrorCode) -> String {
    match code {
        // 무엇을 실을 수 있는지가 답이다.
        ErrorCode::InvalidArgument | ErrorCode::UnknownCommand => {
            format!("run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}` for this group's verbs and fields")
        }
        // 인자 문법 문제가 아니다 — 지금 명부에 무엇이 있는지를 봐야 대상을 고른다(부재 · 동명 · 상한).
        ErrorCode::NotFound | ErrorCode::Conflict => {
            format!("run `{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}` to see the roster")
        }
        // 고칠 인자가 없다 — 무엇이 실제로 남았는지 확인하는 것이 다음 걸음이다(예: 만들어졌지만 못 뜬 것).
        _ => format!(
            "run `{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}` to see what actually exists now"
        ),
    }
}

/// 회복 안내가 제안하는 조회 동사. 어느 동사를 제안할지는 **선택**이지만, 그 동사가 실재하는지는 선택이
/// 아니다 — 아래 테스트가 계열 동사 명단과 대조한다(안 하면 안내가 없는 명령을 치게 한다).
const AGENT_LIST_VERB: &str = "list";

fn unknown_verb(verb: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::UnknownCommand.as_str(),
        hint: format!(
            "unknown verb '{verb}' — {}.",
            recovery_for(ErrorCode::UnknownCommand)
        ),
    }
}

/// 문구 **한 조각**이 차지해도 되는 몫(문자 수).
const PREVIEW_CHARS: usize = 80;

/// 호출자가 준 문자열을 문구의 한 조각으로 줄인다 — 자를 때는 **잘랐다고 말한다**.
///
/// ★이 함수가 지키는 것은 **자기 조각**뿐이다★: 부르는 자리에서 호출자 문자열이 문구를 다 먹지 않게 해,
/// 뒤에 붙는 회복 안내(칠 수 있는 명령)가 문구 안에 남게 한다. 잘랐다고 말하지 않으면 호출자는 잘린 값을
/// 자기가 친 값으로 읽고 멀쩡한 자리를 고치러 간다.
/// ★반려 문구 전체에 대한 상한은 **없다**(알려진 열린 조건)★ — 이 함수를 안 거치고 호출자 문자열을 싣는
/// 생산 지점이 이 라우트에도 이웃 라우트에도 있고, 그것들은 길이 그대로 나간다. 그 축은 이 조각의 범위가
/// 아니다. **여기에 「모든 문구가 어딘가에서 잘린다」는 문장을 쓰지 말 것** — 네 판 연속 거짓이었다.
fn preview(text: &str) -> String {
    let mut head: String = text.chars().take(PREVIEW_CHARS).collect();
    if head.len() < text.len() {
        head.push_str(&format!("…(truncated, {} bytes)", text.len()));
    }
    head
}

/// 바디 자체가 JSON 계약을 못 지킨 경우의 반려 — 어댑터가 부른다(`control_agent_handler`).
///
/// ★빈 body 400 을 쓰지 않는 이유★: 이 라우트의 호출자는 자기교정하는 LLM 이라 **사유가 실려야** 한다.
///   serde 의 문구(어느 필드가 어떤 타입이어야 하는지·중복 키·JSON 구문 위치)를 그대로 싣는다.
/// ★바디 원문은 [`preview`] 로 줄인다★ — 요청 바디를 통째로 되돌려 주는 것은 입구가 할 일이 아니고,
///   앞자리를 다 먹으면 뒤에 붙는 회복 안내가 문구 저 끝으로 밀린다.
/// ★`reason` 은 여기서 자르지 않는다(알려진 열린 조건)★: serde 문구는 대개 짧지만 호출자 문자열이 섞여
///   길어질 수 있고, 그때는 길이 그대로 나간다. 이미 짧은 문구를 한 번 더 자르면 우리 문구가 경계에서
///   잘려 무엇이 문제인지조차 안 보이므로 여기서는 안 자른다.
pub fn malformed_body(reason: &str, raw: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the request body is not a valid {CLI_GROUP_AGENT} command object: {reason} — it must be a JSON object like {{\"verb\":\"list\"}}; got: {} — {}.",
            preview(raw),
            recovery_for(ErrorCode::InvalidArgument)
        ),
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_core::agent::types::CLI_AGENT_VERBS;

    use super::*;

    fn parse(body: serde_json::Value) -> AgentRequest {
        serde_json::from_value(body).expect("요청 역직렬화")
    }

    fn error_of(result: ControlQueryResult) -> (&'static str, String) {
        match result {
            ControlQueryResult::Error { code, hint } => (code, hint),
            other => panic!("반려여야: {other:?}"),
        }
    }

    // ── 요청 → 명령 ──────────────────────────────────────────────────────────────

    /// ★`verb` 만 이 입구의 어휘다★ — 나머지는 아는 칸이든 모르는 칸이든 **전부** 인자로 실려 표가 본다.
    ///   여기서 아는 칸만 골라 담으면 모르는 칸이 이 파일에서 증발해 ADR-0151 의 거절이 성립하지 않는다.
    #[test]
    fn every_field_but_the_verb_is_carried_through_as_command_arguments() {
        let req = parse(serde_json::json!({
            "verb": "move", "target": "a", "parent": null, "parnet": "lead"
        }));
        assert_eq!(req.verb, "move");
        assert_eq!(
            *req.args,
            *serde_json::json!({ "target": "a", "parent": null, "parnet": "lead" })
                .as_object()
                .expect("객체")
        );
    }

    /// ★같은 칸이 두 번 실리면 어느 값도 고르지 않는다★: `{"parent":"lead","parent":null}` 에서 뒤 값을
    ///   택하면 붙이려던 요청이 **루트로 떼기**로 조용히 바뀐다. `verb` 든 인자 칸이든 같은 규율이다 —
    ///   한쪽만 막으면 막히지 않은 쪽으로 같은 사고가 들어온다.
    #[test]
    fn a_duplicate_key_is_refused_for_the_verb_and_for_arguments_alike() {
        for body in [
            r#"{"verb":"list","verb":"move"}"#,
            r#"{"verb":"move","target":"a","target":"b"}"#,
            r#"{"verb":"move","target":"a","parent":"lead","parent":null}"#,
        ] {
            let refusal = serde_json::from_str::<AgentRequest>(body).map(|_| ());
            assert!(refusal.is_err(), "중복 키는 반려여야: {body}");
        }
        // 다른 이름의 칸 여럿은 정상이다(중복 판정이 「칸이 여럿이면 반려」로 번지지 않는다).
        let ok = parse(serde_json::json!({ "verb": "move", "target": "a", "parent": null }));
        assert_eq!(ok.args.len(), 2);
    }

    /// ★반려 문구는 요청만큼 커지지 않는다★ — 키를 그대로 실으면 500 KB 짜리 키 하나가 그만한 200 응답을
    /// 만들고, 그 비용은 친 쪽이 아니라 받는 쪽이 낸다. 잘랐다는 사실도 함께 나가야 호출자가 오해하지 않는다.
    #[test]
    fn a_huge_duplicate_key_is_reported_within_a_bound() {
        let huge = "k".repeat(500_000);
        let body = format!(r#"{{"verb":"move","{huge}":1,"{huge}":2}}"#);

        let reason = serde_json::from_str::<AgentRequest>(&body)
            .expect_err("중복 키")
            .to_string();
        let (_, hint) = error_of(malformed_body(&reason, &body));

        assert!(hint.contains("duplicate field"), "{hint}");
        assert!(hint.contains("truncated"), "잘랐다고 말한다: {hint}");
        // 실측은 ~434 B다 — 아래 수는 **여유이지 계약이 아니다**(그래서 `PREVIEW_CHARS` 를 250 까지 올려도
        //   이 단언은 통과한다). 계약은 「요청 크기와 무관」이고, 그것을 세우는 것은 조각마다의 [`preview`] 다.
        assert!(
            hint.len() < 1024,
            "문구가 요청만큼 커졌다: {} 바이트",
            hint.len()
        );
    }

    /// ★중복 판정이 닿는 깊이는 꼭대기 한 겹뿐이다(알려진 경계)★ — 중첩 객체 안의 중복은 `serde_json::Value`
    /// 가 삼켜 마지막 값이 이긴다. 오늘은 선언된 인자가 전부 문자열이라 그 모양이 인자 타입 검사에서 죽지만,
    /// 객체 값을 선언하는 명령이 생기면 같은 사고가 한 겹 아래에서 되살아난다.
    #[test]
    fn duplicate_detection_is_top_level_only() {
        let nested = parse(serde_json::json!({ "verb": "move", "target": "a" }));
        assert_eq!(nested.args.len(), 1, "전제: 꼭대기 칸은 그대로 실린다");

        let req: AgentRequest =
            serde_json::from_str(r#"{"verb":"move","target":{"parent":1,"parent":2}}"#)
                .expect("중첩 중복은 여기서 안 걸린다(알려진 경계)");
        assert_eq!(req.args["target"]["parent"], 2);
    }

    // ── 반려 봉투 ────────────────────────────────────────────────────────────────

    /// ★반려는 언제나 **칠 수 있는 명령 하나**를 달고 나간다★ — 표와 도구 crate 는 이 표면을 모르므로
    ///   그 안내는 여기서만 붙는다. 문장만 있으면 LLM 이 그 문장을 명령으로 친다.
    #[test]
    fn every_rejection_carries_the_typed_code_and_a_runnable_command() {
        let cases = [
            CommandError::invalid_argument("bad field"),
            CommandError::not_found("no agent called 'ghost'"),
            CommandError::of(ErrorCode::Conflict, "more than one agent"),
            CommandError::internal("was created but did not start"),
        ];
        for error in cases {
            let (code, hint) = error_of(refused(error.clone()));
            assert_eq!(code, error.code().as_str());
            assert!(hint.contains(error.message()), "{hint}");
            assert!(
                hint.contains(&format!("`{CLI_EXE_NAME} ")),
                "칠 수 있는 명령이 없다: {hint}"
            );
        }
    }

    /// 코드마다 **다음에 할 일**이 다르다 — 고칠 인자가 없는 실패에 "인자 규격을 보라" 를 달면 호출자는
    /// 없는 오타를 찾고, 대상을 못 고른 실패에 규격을 달면 명부를 볼 생각을 못 한다.
    #[test]
    fn the_suggested_recovery_matches_what_the_caller_can_actually_fix() {
        let help = format!("`{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`");
        let roster = format!("`{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}`");

        for argument_fault in [ErrorCode::InvalidArgument, ErrorCode::UnknownCommand] {
            assert!(
                recovery_for(argument_fault).contains(&help),
                "{argument_fault:?}"
            );
        }
        for targeting_fault in [ErrorCode::NotFound, ErrorCode::Conflict] {
            assert!(
                recovery_for(targeting_fault).contains(&roster),
                "{targeting_fault:?}"
            );
        }
        // 고칠 인자가 없는 실패(내부·불명)도 다음 걸음은 있다 — 무엇이 실제로 남았는지 본다.
        for opaque_fault in [ErrorCode::Internal, ErrorCode::OutcomeUnknown] {
            assert!(
                recovery_for(opaque_fault).contains(&roster),
                "{opaque_fault:?}"
            );
        }
    }

    /// 제안하는 동사가 **실재해야** 한다 — 없는 동사를 안내하면 자기교정이 반려로 이어진다.
    #[test]
    fn the_suggested_roster_verb_is_one_this_group_actually_has() {
        assert!(
            CLI_AGENT_VERBS.contains(&AGENT_LIST_VERB),
            "{AGENT_LIST_VERB} 이 계열 동사 명단에 없다: {CLI_AGENT_VERBS:?}"
        );
    }

    #[test]
    fn an_unknown_verb_names_the_verb_and_points_at_the_group_help() {
        let (code, hint) = error_of(unknown_verb("explode"));
        assert_eq!(code, "UNKNOWN_COMMAND");
        assert!(hint.contains("explode"), "{hint}");
        assert!(hint.contains("help"), "{hint}");
    }
}
