//! help 화면 원천 — `/control/help` 가 내는 화면을 본문 파일(`prompts/engram-help.md`)에서 짓는다.
//!
//! ★소유★: 화면 낱말 어휘(낱말 → 화면 · 옛 낱말 별칭) · 본문 파일 경로 규칙 · 구획 파싱 · 필수 구획 검사 ·
//!   렌더 · 요청 바디 해석. CLI 는 낱말 하나를 실어 보내고 받은 화면을 찍을 뿐이라 이 중 어느 것도 갖지
//!   않는다 — 두 벌이 되면 화면 낱말 목록과 파싱 규칙이 두 곳에서 갈린다.
//! ★요청마다 파일을 읽는다 — 캐시를 두지 말 것★: 데몬은 오래 살아서, 한 번 읽어 두면 본문을 고쳐도 데몬을
//!   다시 띄울 때까지 화면이 안 바뀐다(본문을 바이너리 밖에 둔 목적 — 고치면 다음 호출이 바뀐다 — 의
//!   퇴행). 비용 = 요청마다 약 20 KB 읽기 하나.
//! ★실패는 반려다 — 내장 사본도 반쪽 화면도 없다★: 파일을 못 읽거나 필수 구획이 빠지면 `INTERNAL` 로 답하고
//!   warn 을 남긴다. 반쪽 화면은 빠진 자리를 읽는 쪽이 알 길이 없고, 낡은 사본은 본문이 깨졌다는 사실을
//!   감춘다.
//! ★호출자를 보지 않는다★: 화면은 누가 묻든 같다(호출자별 필터는 ADR-0220 결정 4 가 걷었다). 그래서 이
//!   라우트는 우편이 아니고 우편 막힌 자격증명도 이 화면을 읽는다(`mcp_server::ControlRoute::is_mail`).
//! ★클라이언트가 보낸 글을 해석하지 않는다★: 읽는 것은 데몬 자신의 배포 파일(`prompts/engram-help.md`)이고
//!   하는 일은 그 안의 `## <id>` 구획을 골라 그대로 내는 것뿐이다 — 명부의 `help` 블롭(ADR-0156 — 데몬이
//!   파싱 · 검증 · 분기하면 위반)도, UI 명령 payload(ADR-0081 — 데몬은 불투명하게 중계만 한다)도 손대지
//!   않는다. 화면 낱말 `window` · `settings` 는 화면 이름이지 UI 명령 이름이 아니다 — 창 명령의 이름 ·
//!   인자는 그 화면 본문의 산문 속에만 있고, 데몬은 그 산문을 해석하지 않는다.
//!
//! tauri import 0(daemon crate).
// ADR-0081
// ADR-0092
// ADR-0156
// ADR-0212
// ADR-0220
// ADR-0285

use std::path::PathBuf;

use engram_dashboard_agent::types::{
    CLI_EXE_NAME, CLI_GROUP_AGENT, CLI_GROUP_MAIL, CLI_HELP_SCREEN_KEY, CLI_HELP_TOPIC_KEY,
};
use engram_dashboard_command::ErrorCode;

use super::agent::preview;
use super::ingress::ControlQueryResult;

/// help 템플릿의 실행파일 이름 자리. 렌더 때 `CLI_EXE_NAME` 으로 치환한다 — help 는 에이전트가 표면을
/// 배우는 유일한 자리라, 여기 적힌 이름이 실제 실행파일과 갈리면 배운 대로 쳐도 명령을 못 찾는다.
const HELP_TOOL_SLOT: &str = "{tool}";

/// 화면 본문의 고정 상대경로 — 프라이밍과 같은 `prompts/` 폴더에 산다. ADR-0100 이 그 폴더를 exe 옆으로
/// 통째 배송하고 manifest tripwire 가 부족·여분을 둘 다 잡으므로, 새 폴더를 만들면 그 두 장치를 한 벌 더
/// 지어야 한다.
const REL_HELP_FILE: &str = "prompts/engram-help.md";

/// 운영자·하네스가 본문을 갈아끼우는 자리(이 **데몬 프로세스**의 env) — 프라이밍의 `ENGRAM_PRIMING_FILE`
/// 과 같은 계약이다.
///
/// ★실패해도 고정 파일로 폴백하지 않는다(프라이밍과 같은 규율)★: 명시 지정을 조용히 다른 파일로
///   갈아치우면 무엇을 읽었는지 알 수 없다. 반려하고 사유에 그 경로를 싣는다.
const ENV_HELP_FILE: &str = "ENGRAM_HELP_FILE";

/// 구획 표시 줄의 앞. ★**줄 전체**가 표시여야 한다★ — substring 으로 보면 형식을 설명하는 산문 한 줄이
/// 구획을 끊는다(그 파일의 머리글이 실제로 그런 문장을 싣는다).
///
/// ★HTML 주석이 아니라 마크다운 제목인 것은 의도다★: 주석 꼴은 마크다운 뷰어가 **통째로 감춰서**, 파일을
///   고치는 사람에게 구획 경계가 하나도 안 보였다(실발생 — 7KB 가 구분선 없는 산문으로 읽혔다). 제목이면
///   뷰어 목차에 구획 id 가 그대로 뜬다. 렌더 결과는 같다 — 표시 줄은 어느 꼴이든 화면에 안 실린다.
///   머리글 제목은 레벨 1(`# `)이라 이 레벨 2 패턴에 안 걸린다.
const SECTION_OPEN: &str = "## ";

/// 최상위 화면의 구획 id. 나머지 넷은 계열 낱말 그대로라 따로 상수를 두지 않는다
/// ([`HelpTopic::section_id`]).
const SECTION_ROOT: &str = "root";

/// 계열 상수(`CLI_GROUP_*`)가 없는 두 화면의 낱말.
///
/// ★공유 상수가 아닌 것이 이유다★: 창·설정은 `window.list`·`settings.get` 처럼 **전체 이름**으로만
///   불리고 `<계열> <동사>` 입구가 없다. 그래서 이 둘은 help 화면 낱말로만 존재하고, 계열 상수에 올리면
///   CLI 파서가 받지도 않는 계열이 계열 목록에 생긴다.
const HELP_TOPIC_WINDOW: &str = "window";
const HELP_TOPIC_SETTINGS: &str = "settings";

/// 화면의 옛 낱말 → 지금 화면. 요청 낱말로만 받고, 반려 문구에는 싣지 않는다.
///
/// ★걷지 말 것★: 개명(TRD S21-storage §10 F14) 전에 띄웠거나 이어받은 에이전트는 옛 프라이밍
///   (`engram help theme`)을 컨텍스트에 들고 있다 — 그 낱말이 반려되면 설정 화면을 못 찾는다.
/// ★본문 파일의 `theme` 구획을 내지 않는다★: 그 구획은 `theme` 을 필수로 요구하는 옛 바이너리가 새 파일을
///   받아들이게 남긴 짧은 포인터다. 별칭이 그것을 내면 읽는 쪽이 한 번 더 불러야 설정 화면에 닿는다.
const HELP_TOPIC_ALIASES: &[(&str, HelpTopic)] = &[("theme", HelpTopic::Settings)];

/// 어느 help 화면인가.
///
/// ★화면 하나 = 구획 하나 = 낱말 하나다(`Root` 만 낱말이 없다)★: 조각을 이어 붙이던 옛 모양은 한 화면이
///   여러 구획에 흩어져, 본문을 고치는 사람이 어느 조각이 어느 화면에 실리는지 파일만 보고는 몰랐다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpTopic {
    Root,
    Mail,
    Agent,
    Window,
    Settings,
}

impl HelpTopic {
    /// 화면 전량 — 요구 구획 목록도 낱말 표도 여기서 나온다. 화면을 더하면서 어느 한쪽을 빠뜨리는
    /// 드리프트가 성립하지 않는다.
    const ALL: &'static [HelpTopic] = &[
        HelpTopic::Root,
        HelpTopic::Mail,
        HelpTopic::Agent,
        HelpTopic::Window,
        HelpTopic::Settings,
    ];

    fn section_id(self) -> &'static str {
        match self {
            HelpTopic::Root => SECTION_ROOT,
            HelpTopic::Mail => CLI_GROUP_MAIL,
            HelpTopic::Agent => CLI_GROUP_AGENT,
            HelpTopic::Window => HELP_TOPIC_WINDOW,
            HelpTopic::Settings => HELP_TOPIC_SETTINGS,
        }
    }

    /// 요청이 싣는 낱말. `Root` 는 없다 — 낱말 자리의 null 이 그 화면이다.
    fn group_word(self) -> Option<&'static str> {
        match self {
            HelpTopic::Root => None,
            other => Some(other.section_id()),
        }
    }

    fn from_group_word(word: &str) -> Option<HelpTopic> {
        HelpTopic::ALL
            .iter()
            .copied()
            .find(|t| t.group_word() == Some(word))
            .or_else(|| {
                HELP_TOPIC_ALIASES
                    .iter()
                    .find(|(alias, _)| *alias == word)
                    .map(|&(_, topic)| topic)
            })
    }
}

/// 화면 본문 한 장(구획 id → 본문).
///
/// ★반쪽 로드가 없다★: 구획이 하나라도 빠지면 이 표는 통째로 버려지고 요청은 반려된다 — 반쪽 화면은
///   빠진 자리가 무엇이었는지 읽는 쪽이 알 길이 없다.
struct HelpText {
    sections: Vec<(String, String)>,
    /// 읽은 파일 — 빠진 구획 표시와 기동 로그가 싣는다.
    origin: String,
}

impl HelpText {
    /// ★본문 = 표시 줄 다음 바이트부터 다음 표시 줄 앞까지, **그대로**★: 들여쓰기·빈 줄·끝 줄바꿈이 곧
    ///   화면 서식이라 어느 것도 다듬지 않는다. 첫 표시 앞의 글은 버린다(파일 머리글 자리).
    /// ★CRLF 만은 접는다★: `core.autocrlf` 때문에 체크아웃된 파일의 줄끝이 기계마다 갈리는데, 이 화면은
    ///   바이트가 곧 계약이다(CLI 가 받은 그대로 찍는다). 여기서 접어 두면 어느 체크아웃에서도 같은 화면이
    ///   나간다.
    fn parse(src: &str, origin: &str) -> Self {
        let mut sections: Vec<(String, String)> = Vec::new();
        let mut current: Option<(String, String)> = None;
        for line in src.split_inclusive('\n') {
            match section_marker(line) {
                Some(id) => {
                    sections.extend(current.take());
                    current = Some((id.to_string(), String::new()));
                }
                None => {
                    if let Some((_, body)) = current.as_mut() {
                        body.push_str(&line.replace("\r\n", "\n"));
                    }
                }
            }
        }
        sections.extend(current.take());
        Self {
            sections,
            origin: origin.to_string(),
        }
    }

    /// ★빠진 구획도 **무언가는** 낸다★: 로더가 완전한 표만 고르므로 실제로는 안 걸리지만, 그 보장이
    ///   깨져도 화면이 조용히 비지 않게 자리와 id 를 남긴다 — 빈 출력은 표면 부재로 읽힌다.
    fn section(&self, id: &str) -> String {
        self.sections
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| format!("(help section `{id}` missing from {})\n", self.origin))
    }

    /// 화면 조립이 요구하는 구획 중 이 표가 못 채우는 것 전량.
    fn missing(&self) -> Vec<String> {
        required_section_ids()
            .into_iter()
            .filter(|id| !self.sections.iter().any(|(k, _)| k == id))
            .collect()
    }
}

/// 표시 줄이면 그 구획 id. 줄 전체가 표시여야 하고(양끝 공백 허용) id 에는 공백이 없다.
fn section_marker(line: &str) -> Option<&str> {
    let inner = line.trim().strip_prefix(SECTION_OPEN)?.trim();
    // 구획 id 는 점으로 이은 소문자 낱말뿐이다. 이 검사가 없으면 본문의 평범한 제목 한 줄이 구획을 끊는다.
    let is_id = !inner.is_empty()
        && !inner.contains(char::is_whitespace)
        && inner
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_');
    is_id.then_some(inner)
}

/// 화면이 요구하는 구획 전량 = 화면 전량. 화면 하나가 구획 하나라 파생이 없다.
fn required_section_ids() -> Vec<String> {
    HelpTopic::ALL
        .iter()
        .map(|t| t.section_id().to_string())
        .collect()
}

/// 화면 하나 = 구획 하나 + 실행파일 이름 치환.
fn render_help_from(text: &HelpText, topic: HelpTopic) -> String {
    text.section(topic.section_id())
        .replace(HELP_TOOL_SLOT, CLI_EXE_NAME)
}

/// help 화면의 원천 — 본문 파일을 어디서 찾나. 조립부가 지어 서버에 넘긴다
/// (`mcp_server::start_mcp_server_with_help`).
///
/// ★경로 모양은 프라이밍과 같다(ADR-0092)★: 고정 상대경로를 base 에 붙이고, `ENGRAM_HELP_FILE` 이 비어
///   있지 않으면 그것이 먼저이고 실패해도 고정 경로로 안 내려가며, 절대경로만 읽는다. ★cwd 는 쓰지
///   않는다★ — WMI 로 뜬 데몬의 cwd 는 System32 다. 다른 반쪽은 **내용을 이 프로세스가 읽는다**는 것이다
///   (프라이밍은 경로만 넘기고 claude 가 읽는다).
/// ★base 를 받는 이유★: 시험이 알려진 본문을 담은 폴더를 넘겨 시험 exe 의 자리 · `CARGO_TARGET_DIR` 와
///   무관하게 화면을 잰다. 서버 안에서 설치 위치로 짓지 않는 것도 그래서다.
// ADR-0092
// ADR-0285
pub struct HelpSource {
    anchor: Anchor,
}

enum Anchor {
    /// 고정 상대경로를 붙일 폴더.
    Base(PathBuf),
    /// 운영 생성자가 설치 위치를 못 얻었다 — 절대경로 override 만 읽힌다.
    NoInstallRoot,
    /// help 를 부르지 않는 서버 판(`start_mcp_server_without_help`) — env 와 무관하게 모든 요청을 반려한다.
    NotConfigured,
}

impl HelpSource {
    pub fn new(base: PathBuf) -> Self {
        Self {
            anchor: Anchor::Base(base),
        }
    }

    /// 운영 생성자 — 프라이밍과 같은 앵커(`data_dir::find_install_root`). 릴리스에서는 exe 폴더이고 거기
    /// `prompts/` 가 함께 배송된다(ADR-0100).
    pub fn from_install_root() -> Self {
        match crate::data_dir::find_install_root() {
            Some(root) => Self::new(root),
            None => Self {
                anchor: Anchor::NoInstallRoot,
            },
        }
    }

    pub(super) fn not_configured() -> Self {
        Self {
            anchor: Anchor::NotConfigured,
        }
    }

    /// 요청 하나에 답한다 — 성공 `{"screen": …}` 이든 반려 봉투든 늘 값 하나다. blocking(파일 읽기).
    ///
    /// ★낱말 판정이 파일 읽기보다 먼저다★: 어휘는 코드라, 본문이 깨져 있어도 모르는 낱말은
    ///   `INVALID_ARGUMENT` 다 — 호출자가 고칠 것(낱말)과 주인이 고칠 것(본문)을 한 코드로 섞지 않는다.
    pub fn answer(&self, body: &[u8]) -> ControlQueryResult {
        let topic = match parse_request(body) {
            Ok(topic) => topic,
            Err(rejected) => return rejected,
        };
        match self.load() {
            Ok(text) => {
                let mut screen = serde_json::Map::new();
                screen.insert(
                    CLI_HELP_SCREEN_KEY.to_string(),
                    serde_json::Value::String(render_help_from(&text, topic)),
                );
                ControlQueryResult::Ok(serde_json::Value::Object(screen))
            }
            Err(e) => {
                e.warn("help 화면을 못 냄 — INTERNAL 로 반려");
                ControlQueryResult::Error {
                    code: ErrorCode::Internal.as_str(),
                    hint: format!("help text unavailable — {e}; report this to the owner"),
                }
            }
        }
    }

    /// 기동 때 한 번 — 지금 해석되는 본문을 남긴다. 진단이지 판정이 아니다(요청은 그때마다 다시 읽는다).
    /// 정상은 info 라 기본 레벨(warn)에선 안 보이고, 못 쓰면 warn 이라 `RUST_LOG` 가 닿지 않는 릴리스
    /// 데몬에도 남는다.
    pub(super) fn log_startup(&self) {
        match self.load() {
            Ok(text) => tracing::info!(
                path = %text.origin,
                "help 본문 원천 확인 — /control/help 가 이 파일로 답한다"
            ),
            Err(e) => e.warn("help 본문 원천을 못 씀 — /control/help 가 INTERNAL 로 답한다"),
        }
    }

    fn path(&self) -> Result<PathBuf, Unavailable> {
        let base = match &self.anchor {
            Anchor::Base(base) => Some(base.as_path()),
            Anchor::NoInstallRoot => None,
            Anchor::NotConfigured => {
                return Err(Unavailable::bare(
                    "help source not configured on this server",
                ))
            }
        };
        let chosen = std::env::var_os(ENV_HELP_FILE)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(REL_HELP_FILE));
        if chosen.is_absolute() {
            return Ok(chosen);
        }
        let Some(base) = base else {
            return Err(Unavailable::bare(format!(
                "the install folder could not be found, so {} has no absolute path",
                chosen.display()
            )));
        };
        let joined = base.join(&chosen);
        if joined.is_absolute() {
            Ok(joined)
        } else {
            Err(Unavailable::at(joined, "not an absolute path"))
        }
    }

    fn load(&self) -> Result<HelpText, Unavailable> {
        let path = self.path()?;
        let src = std::fs::read_to_string(&path)
            .map_err(|e| Unavailable::at(path.clone(), e.to_string()))?;
        let text = HelpText::parse(&src, &path.display().to_string());
        let missing = text.missing();
        if missing.is_empty() {
            Ok(text)
        } else {
            Err(Unavailable::at(
                path,
                format!("missing sections: {}", missing.join(", ")),
            ))
        }
    }
}

/// 본문을 못 쓴 사유 — 반려 문구와 warn 이 같은 값을 싣는다.
struct Unavailable {
    /// 읽으려던 파일. 경로를 짓기 전에 멈췄으면 없다.
    path: Option<PathBuf>,
    reason: String,
}

impl Unavailable {
    fn bare(reason: impl Into<String>) -> Self {
        Self {
            path: None,
            reason: reason.into(),
        }
    }

    fn at(path: PathBuf, reason: impl Into<String>) -> Self {
        Self {
            path: Some(path),
            reason: reason.into(),
        }
    }

    fn warn(&self, what: &str) {
        match &self.path {
            Some(path) => tracing::warn!(path = %path.display(), reason = %self.reason, "{what}"),
            None => tracing::warn!(reason = %self.reason, "{what}"),
        }
    }
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{}: {}", path.display(), self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

/// 요청 바디 → 화면. 반려는 그대로 돌려줄 봉투다.
///
/// ★`topic` 은 필수이고 다른 키는 반려다★: 빈 바디를 목차로 읽으면 CLI 가 키 이름을 틀려도 목차가 나와
///   어긋남이 안 보이고, 모르는 키를 넘기면 철자 틀린 키가 조용히 무시된다. 모르는 필드를 관용하는
///   ADR-0157 은 빌드 경계를 건너는 배선의 규칙인데, CLI 와 데몬은 한 빌드로 배송되므로(ADR-0100) 이
///   라우트에는 그 경계가 없다. 판정은 [`HelpRequest`] 의 역직렬화가 한다.
// ADR-0157
// ADR-0285
fn parse_request(body: &[u8]) -> Result<HelpTopic, ControlQueryResult> {
    let HelpRequest { topic } = serde_json::from_slice(body).map_err(|e| {
        malformed_request(&format!("is malformed ({})", preview(&e.to_string())), body)
    })?;
    let Some(word) = topic else {
        return Ok(HelpTopic::Root);
    };
    HelpTopic::from_group_word(&word).ok_or_else(|| ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "unknown help topic: {} — run `{CLI_EXE_NAME} help` to list groups, or `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`",
            preview(&word)
        ),
    })
}

/// 바디가 계약을 못 지킨 반려 — 고칠 재료(기대하는 모양)와 받은 바디의 머리를 싣는다.
fn malformed_request(what: &str, body: &[u8]) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the help request body {what} — it must be a JSON object like {{\"{CLI_HELP_TOPIC_KEY}\":null}} for the list of groups or {{\"{CLI_HELP_TOPIC_KEY}\":\"{CLI_GROUP_MAIL}\"}} for one group; got: {}",
            preview(&String::from_utf8_lossy(body))
        ),
    }
}

/// help 요청 바디 — 키 [`CLI_HELP_TOPIC_KEY`] 하나뿐인 JSON 객체. `topic: None` = 목차.
struct HelpRequest {
    topic: Option<String>,
}

impl<'de> serde::Deserialize<'de> for HelpRequest {
    /// ★손으로 쓴 visitor 다 — `#[derive(Deserialize)]` 나 `serde_json::Value` 로 되돌리지 마라★:
    ///   - `Value` 는 같은 키가 두 번 오면 뒤의 값으로 접어, `{"topic":"mail","topic":"agent"}` 가 반려
    ///     없이 화면 하나로 답한다. 여기서는 중복이 `duplicate field` 반려다.
    ///   - 파생 구현은 **JSON 배열**도 필드 순서열로 읽어 `["mail"]` 이 화면이 된다(`catalog::CallRequest`
    ///     가 밟은 함정과 같다). `deserialize_map` 만 받으므로 객체 아닌 바디는 반려다.
    ///   - 파생 구현에서 `Option` 칸은 키가 없어도 `None` 으로 채워져, 키 철자가 틀린 바디가 목차로 답한다.
    // ADR-0285
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        const FIELDS: &[&str] = &[CLI_HELP_TOPIC_KEY];
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = HelpRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "a JSON object with exactly one key `{CLI_HELP_TOPIC_KEY}`"
                )
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<HelpRequest, M::Error> {
                let mut topic: Option<Option<String>> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key != CLI_HELP_TOPIC_KEY {
                        return Err(serde::de::Error::unknown_field(&key, FIELDS));
                    }
                    if topic.is_some() {
                        return Err(serde::de::Error::duplicate_field(CLI_HELP_TOPIC_KEY));
                    }
                    topic = Some(map.next_value()?);
                }
                match topic {
                    Some(topic) => Ok(HelpRequest { topic }),
                    None => Err(serde::de::Error::missing_field(CLI_HELP_TOPIC_KEY)),
                }
            }
        }

        de.deserialize_map(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::{CLI_AGENT_FLAGS, CLI_AGENT_VERBS};
    use std::sync::{Mutex, MutexGuard};

    /// ★`ENGRAM_HELP_FILE` 은 프로세스 전역이다★: 원천을 거쳐 읽는 시험은 전부 이 잠금을 쥐고 그 변수를
    ///   지운 채 돈다 — 다른 시험이 그 변수를 세운 사이에 읽으면 엉뚱한 파일을 잰다.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(ENV_HELP_FILE);
        guard
    }

    /// 시험은 언제나 컴파일타임 소스 트리 안에서 돌므로 MANIFEST_DIR 이 신뢰 가능하다.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo 루트")
            .to_path_buf()
    }

    /// 배포되는 본문 파일 원문 — 릴리스가 exe 옆 `prompts/` 로 싣는 바로 그 파일이다.
    fn shipped_file() -> String {
        std::fs::read_to_string(repo_root().join(REL_HELP_FILE))
            .expect("본문 파일이 REL_HELP_FILE 자리에 있어야")
    }

    fn shipped_text() -> HelpText {
        HelpText::parse(&shipped_file(), "shipped")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-help-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("임시 폴더");
        dir
    }

    /// `<base>\prompts\engram-help.md` 하나만 둔 폴더 — 릴리스 조립이 exe 옆에 놓는 모양이다.
    fn release_shaped(tag: &str, body: &str) -> PathBuf {
        let base = temp_dir(tag);
        let file = base.join(REL_HELP_FILE);
        std::fs::create_dir_all(file.parent().expect("prompts 폴더")).expect("prompts 폴더");
        std::fs::write(&file, body).expect("본문 쓰기");
        base
    }

    fn request(word: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ CLI_HELP_TOPIC_KEY: word })).expect("요청 바디")
    }

    fn screen_of(source: &HelpSource, word: Option<&str>) -> String {
        match source.answer(&request(word)) {
            ControlQueryResult::Ok(v) => v[CLI_HELP_SCREEN_KEY]
                .as_str()
                .unwrap_or_else(|| panic!("성공 봉투에 화면 문자열이 있어야({word:?}): {v}"))
                .to_string(),
            ControlQueryResult::Error { code, hint } => {
                panic!("화면이어야({word:?}): {code} {hint}")
            }
        }
    }

    fn rejection_of(source: &HelpSource, body: &[u8]) -> (&'static str, String) {
        match source.answer(body) {
            ControlQueryResult::Error { code, hint } => (code, hint),
            ControlQueryResult::Ok(v) => panic!("반려여야: {v}"),
        }
    }

    // ── 배포되는 본문 파일 ─────────────────────────────────────────────────────────────

    /// ★배포되는 파일이 완전하지 않으면 help 가 통째로 반려된다★ — 사본이 없으므로 이 파일이 모든 화면의
    ///   유일한 집이다. 구획 id 를 주제에서 파생하므로 화면을 늘리고 본문 파일에 구획을 안 더하면 여기서
    ///   빨개진다.
    #[test]
    fn the_shipped_help_file_carries_every_section_the_screens_need() {
        let text = shipped_text();
        assert!(
            text.missing().is_empty(),
            "배포 본문에 빠진 구획: {:?}",
            text.missing()
        );
    }

    /// ★배포되는 파일 하나로 모든 화면이 선다★ — 낱말마다 실제 요청 경로로 그려 본다.
    #[test]
    fn every_screen_renders_from_the_shipped_help_file() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let words = std::iter::once(None)
            .chain(
                HelpTopic::ALL
                    .iter()
                    .filter_map(|t| t.group_word().map(Some)),
            )
            .chain(HELP_TOPIC_ALIASES.iter().map(|(alias, _)| Some(*alias)));
        for word in words {
            let screen = screen_of(&source, word);
            assert!(!screen.trim().is_empty(), "빈 화면({word:?})");
            assert!(
                !screen.contains(HELP_TOOL_SLOT),
                "치환 안 된 자리({word:?}): {screen}"
            );
            assert!(
                screen.contains(CLI_EXE_NAME),
                "실행파일 이름이 없다({word:?}): {screen}"
            );
        }
    }

    /// ★개명 전 바이너리가 요구하던 구획 전량이 본문 파일에 남아 있다★ — 하나라도 빠지면 그 바이너리는 새
    ///   파일을 통째로 거부하고, 제 내장 사본(파일의 `theme` 키를 고치라는 옛 안내)을 낸다. 이 원천은 그
    ///   여분 구획을 모르는 구획으로 무시하고, `theme` 낱말은 별칭으로 settings 화면을 낸다.
    #[test]
    fn the_help_file_still_satisfies_the_pre_rename_binary_and_this_one_ignores_the_extra() {
        let text = shipped_text();
        for id in ["root", "mail", "agent", "window", "theme"] {
            assert!(
                text.sections.iter().any(|(k, _)| k == id),
                "개명 전 바이너리가 요구하는 구획 `{id}` 가 없다"
            );
        }
        assert!(text.missing().is_empty(), "여분 구획이 로드를 깨면 안 된다");
        let compat = text.section("theme");
        let settings = render_help_from(&text, HelpTopic::Settings);
        assert!(
            settings.starts_with(&format!("{CLI_EXE_NAME} {HELP_TOPIC_SETTINGS} ")),
            "별칭 대상은 settings 구획이어야: {settings}"
        );
        assert_ne!(
            settings,
            compat.replace(HELP_TOOL_SLOT, CLI_EXE_NAME),
            "`help theme` 이 옛 바이너리용 포인터 구획을 내면 안 된다"
        );
    }

    // ── 회신 계약 · 프라이밍 다리 ─────────────────────────────────────────────────────

    /// ★옮겨 온 pin(`control/priming.rs::production_priming_files_teach_the_reply_contract`)★: 예전엔
    ///   프라이밍 파일이 봉투 문법을 싣는지 봤다. 그 문법이 이 화면으로 내려오면서(프라이밍은 포인터로
    ///   줄었다) pin 도 따라왔다 — 표면을 소유한 파일이 그 표면을 지킨다.
    ///
    /// ★무엇이 걸려 있나★: 데몬은 `type="request"` 봉투를 내보내고 기한 초과 시 **발신자에게**
    ///   `<notice>` 를 쏘는데, **회신 자체는 LLM 준수(soft)** 다(ADR-0103 결정 2/3). 이 화면이 회신 규칙을
    ///   안 가르치면 엄격 매칭(받은 id 필수)이 구조적으로 회신을 못 받아 계약이 반쪽이 된다.
    ///
    /// ★철자는 이 입구의 것으로 본다(ADR-0126 결정 1)★: 회신 대상을 지목하는 낱말은 도착한 봉투의 것
    ///   (`in-reply-to`)이고, 같은 계약의 툴 인자 표기(snake_case `reply_to`·`reply_by`)는
    ///   `control/mcp_server.rs::the_send_message_entry_teaches_its_own_call` 이 진다. 한쪽 철자를 다른
    ///   화면에 끌어오면 폐지한 우회 교육이 되살아난다.
    // ADR-0103
    // ADR-0126
    #[test]
    fn the_mail_screen_teaches_the_reply_contract() {
        let screen = render_help_from(&shipped_text(), HelpTopic::Mail);
        assert!(
            screen.contains("type=\"request\""),
            "request 봉투를 알아보게 가르쳐야: {screen}"
        );
        assert!(
            screen.contains("<notice>"),
            "notice 는 회신 대상이 아님을 가르쳐야(데몬 전용 태그, from 없음): {screen}"
        );
        assert!(
            screen.contains("in-reply-to"),
            "받은 id 로 온 답을 알아보게 가르쳐야: {screen}"
        );
        assert!(
            screen.contains("reply-by"),
            "기한이 무엇인지 가르쳐야(발신자의 기한이지 수신자의 것이 아니다): {screen}"
        );
        // ★`mcp_server.rs` 의 `the_send_message_entry_teaches_its_own_call` 에서 옮겨 온 고정 —
        //   지우지 말 것★: 「`pending` 을 원인 하나로 읽지 말고 조회하라」(ADR-0211 결정 2)는 도구
        //   설명문이 지던 것인데, 그 설명문에서 인자 밖 계약을 걷어내며 이 화면이 유일한 집이 됐다.
        //   두 자리 다 비면 그 금지를 가르치는 표면이 하나도 안 남는다.
        assert!(
            screen.contains("pending") && screen.contains("eg_messages"),
            "pending 으로 상대 상태를 단정하지 말고 eg_messages 로 조회하라고 가르쳐야: {screen}"
        );
    }

    /// ★프라이밍의 포인터와 이 화면이 같은 이름을 가리키는가★: 프라이밍은 계약을 싣지 않고 **`engram
    ///   help <계열>` 을 쳐라**는 줄로 줄었다 — 그 줄이 곧 에이전트가 표면을 만나는 유일한 다리다. 화면
    ///   낱말을 갈거나 프라이밍에서 그 줄을 빼면 에이전트는 **아무 오류도 없이** 표면을 영영 못 찾는다(양쪽
    ///   다 자기 파일 안에서는 멀쩡하다). 그 침묵을 잡는 곳이 여기다. 그 줄이 CLI 에서 help 요청으로
    ///   파싱되는지는 CLI 쪽 시험이 진다.
    ///
    /// ★낱말이 받아들여지는 것만으로는 부족하다★: 화면이 비거나 치환이 안 된 자리(`{tool}`)가 남으면
    ///   가리킨 곳에 아무것도 없는 것과 같다 — 그래서 그 낱말로 실제 요청을 보내 화면이 서는지까지 본다.
    // ADR-0092
    #[test]
    fn the_priming_pointer_names_help_entries_that_actually_render() {
        let _env = env_guard();
        let priming = std::fs::read_to_string(repo_root().join("prompts/agent-priming.md"))
            .expect("프라이밍 파일 존재");
        let pointer = format!("{CLI_EXE_NAME} help");
        assert!(
            priming.contains(&pointer),
            "프라이밍이 `{pointer}` 를 가리켜야(이 줄이 빠지면 에이전트는 표면을 못 찾는다)"
        );

        let source = HelpSource::new(repo_root());
        for topic in HelpTopic::ALL {
            let word = topic.group_word();
            if let Some(word) = word {
                assert!(
                    priming.contains(&format!("{pointer} {word}")),
                    "프라이밍이 `{pointer} {word}` 를 가리켜야"
                );
            }
            let screen = screen_of(&source, word);
            assert!(!screen.trim().is_empty(), "화면이 비어 있다({topic:?})");
            assert!(
                !screen.contains(HELP_TOOL_SLOT),
                "치환 안 된 자리가 남으면 안 된다({topic:?}): {screen}"
            );
        }

        // ★반대 방향 — 프라이밍이 가리키는 낱말은 전부 실제 화면이다★: 위는 화면마다 그 줄이 있는지만 봐서,
        //   프라이밍에 오타 낱말(`help mailx`) 줄이 하나 더 있어도 초록이다. 그 줄을 따라 친 에이전트는 반려를
        //   받는다.
        let named: Vec<&str> = priming
            .match_indices(&format!("{pointer} "))
            .filter_map(|(at, p)| priming[at + p.len()..].split_whitespace().next())
            .collect();
        assert!(!named.is_empty(), "프라이밍에 `{pointer} <낱말>` 줄이 없다");
        for word in named {
            assert!(
                HelpTopic::from_group_word(word).is_some(),
                "프라이밍이 가리키는 `{pointer} {word}` 는 화면 낱말이 아니다"
            );
            let screen = screen_of(&source, Some(word));
            assert!(
                !screen.trim().is_empty() && !screen.contains(HELP_TOOL_SLOT),
                "프라이밍이 가리키는 `{pointer} {word}` 화면이 서지 않는다: {screen}"
            );
        }
    }

    // ── 낱말 → 화면 ─────────────────────────────────────────────────────────────────

    /// 계열 목록 화면과 agent 화면이 표면을 그대로 가르친다 — 없는 동사를 가르치면 LLM 이 없는 명령을
    /// 시도한다(ADR-0122 미해소분).
    #[test]
    fn the_agent_screen_documents_every_verb_and_flag() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let root = screen_of(&source, None);
        let mail = screen_of(&source, Some(CLI_GROUP_MAIL));
        let agent = screen_of(&source, Some(CLI_GROUP_AGENT));
        assert!(root.contains(CLI_GROUP_AGENT), "계열 목록에 agent: {root}");
        assert!(root.contains(CLI_GROUP_MAIL), "계열 목록에 mail: {root}");
        for verb in CLI_AGENT_VERBS {
            assert!(agent.contains(verb), "{verb} 동사가 help 에: {agent}");
        }
        for flag in CLI_AGENT_FLAGS {
            assert!(agent.contains(flag), "{flag} 가 help 에: {agent}");
        }
        for absent in ["kill", " rm ", "delete"] {
            assert!(
                !agent.contains(absent),
                "표면에 없는 동사가 help 에 있다({absent}): {agent}"
            );
        }
        for text in [&root, &mail, &agent] {
            assert!(
                !text.contains(HELP_TOOL_SLOT),
                "치환 안 된 자리가 남으면 안 된다: {text}"
            );
            assert!(text.contains(CLI_EXE_NAME), "실행파일 이름이 상수에서 와야");
        }
    }

    /// ★계열 낱말 넷이 각자 자기 화면에 닿는다★ — 한 낱말이 다른 화면으로 배선되면 성공 봉투만으론 안
    ///   보인다(둘 다 화면이다). 그래서 **서로 다른지**와 머리가 자기 계열인지까지 본다.
    #[test]
    fn each_group_word_renders_its_own_screen() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let mut seen: Vec<(&str, String)> = Vec::new();
        for topic in HelpTopic::ALL {
            let Some(word) = topic.group_word() else {
                continue;
            };
            let screen = screen_of(&source, Some(word));
            assert!(
                screen.starts_with(&format!("{CLI_EXE_NAME} {word} ")),
                "화면 머리가 자기 계열이어야({word}): {screen}"
            );
            seen.push((word, screen));
        }
        assert_eq!(seen.len(), 4, "계열 낱말은 넷이다: {seen:?}");
        let root = screen_of(&source, None);
        for (word, screen) in &seen {
            assert_ne!(*screen, root, "{word} 가 최상위 화면과 같다");
            assert_eq!(
                seen.iter().filter(|(_, s)| s == screen).count(),
                1,
                "{word} 와 같은 본문을 내는 화면이 또 있다"
            );
        }
    }

    #[test]
    fn the_old_theme_word_reaches_the_settings_screen() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        assert_eq!(
            screen_of(&source, Some("theme")),
            screen_of(&source, Some(HELP_TOPIC_SETTINGS)),
            "옛 낱말 `theme` 은 settings 화면을 내야"
        );
        // 별칭이 계열 낱말과 같으면 계열 쪽이 먼저 잡혀 별칭은 죽은 줄이 된다.
        for (alias, _) in HELP_TOPIC_ALIASES {
            assert!(
                HelpTopic::ALL.iter().all(|t| t.group_word() != Some(alias)),
                "별칭 `{alias}` 가 계열 낱말과 겹친다"
            );
        }
    }

    // ── 경로 규칙 · 실패 = 반려 ──────────────────────────────────────────────────────

    /// 고정 경로 해석은 base 에 붙인 절대경로다 — cwd 도 아니고 base 밖도 아니다.
    #[test]
    fn the_fixed_help_path_resolves_absolute_to_a_real_file() {
        let _env = env_guard();
        let p = HelpSource::new(repo_root())
            .path()
            .unwrap_or_else(|e| panic!("절대 base 면 경로는 산출된다: {e}"));
        assert!(p.is_absolute(), "절대경로여야: {p:?}");
        assert!(
            p.ends_with("prompts/engram-help.md") || p.ends_with("prompts\\engram-help.md"),
            "고정 상대경로로 끝나야: {p:?}"
        );
        assert!(p.is_file(), "소스 트리에서 실제 파일을 가리켜야: {p:?}");
    }

    /// ★반쪽 로드 금지★: 구획 하나가 빠진 파일은 통째로 버려지고 요청은 `INTERNAL` 로 반려된다. 반쪽
    ///   화면은 낡은 화면보다 나쁘다 — 읽는 쪽에는 빠진 자리가 그냥 없는 표면으로 보인다.
    #[test]
    fn a_help_file_missing_one_section_is_rejected_whole() {
        let _env = env_guard();
        let full = shipped_file().replace("\r\n", "\n");
        let marker = format!("{SECTION_OPEN}{HELP_TOPIC_SETTINGS}");
        assert!(full.contains(&marker), "표시 줄 표기가 바뀌었다: {marker}");
        let crippled = full.replace(&marker, "<!-- 그냥 주석 -->");
        assert_eq!(
            HelpText::parse(&crippled, "crippled").missing(),
            vec![HELP_TOPIC_SETTINGS.to_string()],
            "빠진 구획을 정확히 지목해야"
        );

        let base = release_shaped("crippled", &crippled);
        // 빠진 것은 settings 하나인데 다른 화면도 반려된다 — 표째 버린다.
        let (code, hint) = rejection_of(&HelpSource::new(base.clone()), &request(Some("mail")));
        assert_eq!(code, ErrorCode::Internal.as_str());
        assert!(
            hint.starts_with("help text unavailable — ")
                && hint.ends_with("; report this to the owner")
                && hint.contains(HELP_TOPIC_SETTINGS),
            "빠진 구획을 싣고 주인에게 알리라고 해야: {hint}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★최후의 바닥 — 표가 통째로 비어도 화면은 비지 않는다★: 빈 출력은 읽는 쪽에게 「그런 표면이
    ///   없다」로 읽히므로, 그 경우에도 무엇이 빠졌는지 이름을 남긴다. 로더가 완전한 표만 고르므로
    ///   운영에서는 안 걸리지만, 그 보장이 깨지는 날 조용히 사라지지 않게 박아 둔다.
    #[test]
    fn a_screen_never_renders_empty_even_with_no_sections_at_all() {
        let empty = HelpText::parse("", "아무것도 없음");
        assert_eq!(empty.missing().len(), required_section_ids().len());
        for topic in HelpTopic::ALL {
            let screen = render_help_from(&empty, *topic);
            assert!(!screen.trim().is_empty(), "빈 화면({topic:?})");
            assert!(
                screen.contains("missing from") && screen.contains("아무것도 없음"),
                "무엇이 어디서 빠졌는지 남겨야({topic:?}): {screen}"
            );
        }
    }

    /// ★표시는 **줄 전체**여야 한다★: 그렇지 않으면 형식을 설명하는 산문 한 줄이 구획을 끊는다 —
    ///   본문 파일의 머리글이 실제로 그런 문장을 싣고 있고, 그 글은 첫 표시 앞이라 버려져야 한다.
    #[test]
    fn a_section_marker_must_be_a_whole_line() {
        let src = format!(
            "머리글: 표시는 `{SECTION_OPEN}<id>` 꼴이다.\n\
             {SECTION_OPEN}one\n\
             body one\n\
             여기서도 {SECTION_OPEN}two 를 설명만 한다\n"
        );
        let text = HelpText::parse(&src, "fixture");
        assert_eq!(
            text.sections.len(),
            1,
            "산문 속 표기가 구획을 끊었다: {:?}",
            text.sections
        );
        assert_eq!(
            text.section("one"),
            format!("body one\n여기서도 {SECTION_OPEN}two 를 설명만 한다\n")
        );
    }

    /// ★본문은 공백까지 그대로다(들여쓰기·빈 줄·끝 줄바꿈이 곧 화면 서식)★ — 단 CRLF 만은 접는다:
    ///   `core.autocrlf` 때문에 체크아웃된 줄끝이 기계마다 갈리는데 이 화면은 바이트가 곧 계약이다.
    #[test]
    fn section_bodies_keep_their_whitespace_but_fold_crlf() {
        let text = HelpText::parse("## s\r\n\r\n  indented\r\ntail\r\n", "f");
        assert_eq!(text.section("s"), "\n  indented\ntail\n");
        let no_trailing_newline = HelpText::parse("## s\nlast", "f");
        assert_eq!(no_trailing_newline.section("s"), "last");
    }

    /// ★env override 는 고정 파일을 이기고, 실패하면 **반려다** — 고정 파일로 되돌아가지 않는다★(프라이밍과
    ///   같은 규율 — 명시 지정을 조용히 다른 파일로 갈아치우면 무엇을 읽었는지 알 수 없다). base 에는 멀쩡한
    ///   고정 파일을 둬서, 되돌아가면 화면이 나와 이 단언이 빨개지게 한다.
    /// ★요청마다 읽는다★: 같은 원천에서 파일만 바꾼 뒤의 다음 요청이 바뀐 글을 내야 한다 — 캐시를 두면
    ///   데몬을 다시 띄울 때까지 옛 글이 나간다.
    // ADR-0092
    #[test]
    fn the_env_override_wins_and_its_failure_is_a_rejection_not_a_fallback() {
        let _env = env_guard();
        let source = HelpSource::new(repo_root());
        let dir = temp_dir("override");
        let custom = dir.join("custom-help.md");
        let landmark = "이 팀에서 할 수 있는 것";
        let shipped = shipped_file();
        assert!(shipped.contains(landmark), "최상위 화면의 표지가 바뀌었다");

        std::fs::write(
            &custom,
            shipped.replace(landmark, &format!("{landmark}(custom)")),
        )
        .unwrap();
        std::env::set_var(ENV_HELP_FILE, &custom);
        assert!(
            screen_of(&source, None).contains("(custom)"),
            "override 파일이 이겨야"
        );

        std::fs::write(
            &custom,
            shipped.replace(landmark, &format!("{landmark}(edited)")),
        )
        .unwrap();
        let edited = screen_of(&source, None);
        assert!(
            edited.contains("(edited)") && !edited.contains("(custom)"),
            "파일을 고친 뒤의 다음 요청이 고친 글을 내야(요청마다 읽는다): {edited}"
        );

        let ghost = dir.join("does-not-exist.md");
        std::env::set_var(ENV_HELP_FILE, &ghost);
        let (code, hint) = rejection_of(&source, &request(None));
        assert_eq!(
            code,
            ErrorCode::Internal.as_str(),
            "override 실패는 고정 파일이 아니라 반려로 간다"
        );
        assert!(
            hint.contains("does-not-exist.md"),
            "반려가 읽으려던 override 경로를 실어야: {hint}"
        );

        std::env::remove_var(ENV_HELP_FILE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 요청 바디 · 릴리스 모양 ──────────────────────────────────────────────────────

    /// ★바디 계약 전량★: `topic` 은 필수(null = 목차)이고 다른 키 · 다른 타입 · 객체 아닌 바디는 반려다.
    ///   모르는 낱말은 본문을 읽기 **전에** 반려되므로, 본문이 없는 원천에서도 `INVALID_ARGUMENT` 다 —
    ///   같은 원천에서 아는 낱말은 `INTERNAL` 로 갈린다.
    #[test]
    fn the_request_body_must_name_one_topic_and_nothing_else() {
        assert_eq!(parse_request(&request(None)).ok(), Some(HelpTopic::Root));
        assert_eq!(
            parse_request(&request(Some(CLI_GROUP_MAIL))).ok(),
            Some(HelpTopic::Mail)
        );
        assert_eq!(
            parse_request(&request(Some("theme"))).ok(),
            Some(HelpTopic::Settings)
        );
        let invalid = ErrorCode::InvalidArgument.as_str();
        for body in [
            "{}".to_string(),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":\"{CLI_GROUP_MAIL}\",\"extra\":1}}"),
            "{\"topc\":\"mail\"}".to_string(),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":3}}"),
            format!("{{\"{CLI_HELP_TOPIC_KEY}\":[\"{CLI_GROUP_MAIL}\"]}}"),
            "[]".to_string(),
            format!("[\"{CLI_GROUP_MAIL}\"]"),
            "\"mail\"".to_string(),
            "not json".to_string(),
            String::new(),
        ] {
            match parse_request(body.as_bytes()) {
                Err(ControlQueryResult::Error { code, hint }) => {
                    assert_eq!(code, invalid, "{body:?}: {hint}");
                    assert!(
                        hint.contains(CLI_HELP_TOPIC_KEY),
                        "고칠 모양을 실어야({body:?}): {hint}"
                    );
                }
                other => panic!("반려여야({body:?}): {other:?}"),
            }
        }

        let unconfigured = HelpSource::not_configured();
        let (code, hint) = rejection_of(&unconfigured, &request(Some("wat")));
        assert_eq!(code, invalid, "모르는 낱말은 본문을 읽기 전에 반려");
        assert_eq!(
            hint,
            format!(
                "unknown help topic: wat — run `{CLI_EXE_NAME} help` to list groups, or `{CLI_EXE_NAME} help {CLI_GROUP_MAIL}`"
            )
        );
        let (code, hint) = rejection_of(&unconfigured, &request(Some(CLI_GROUP_MAIL)));
        assert_eq!(code, ErrorCode::Internal.as_str());
        assert!(
            hint.contains("help source not configured on this server"),
            "{hint}"
        );
    }

    /// ★같은 키가 두 번 오면 반려다 — 어느 한쪽으로 접지 않는다★: 접으면 바디가 무엇을 물었는지 이 데몬이
    ///   혼자 정하게 된다. 값이 같아도(null 둘) 반려다 — 반려 여부가 값에 따라 갈리면 안 된다.
    #[test]
    fn a_repeated_topic_key_is_rejected_not_collapsed() {
        for body in [
            r#"{"topic":"mail","topic":"agent"}"#,
            r#"{"topic":null,"topic":null}"#,
            r#"{"topic":"mail","topic":"mail"}"#,
        ] {
            match parse_request(body.as_bytes()) {
                Err(ControlQueryResult::Error { code, hint }) => {
                    assert_eq!(code, ErrorCode::InvalidArgument.as_str(), "{body}: {hint}");
                    assert!(
                        hint.contains("duplicate field") && hint.contains(CLI_HELP_TOPIC_KEY),
                        "중복 키라고 말해야({body}): {hint}"
                    );
                }
                other => panic!("반려여야({body}): {other:?}"),
            }
        }
    }

    /// ★릴리스 모양 폴더에서 화면이 선다★: 릴리스 조립은 exe 와 `prompts\` 를 한 폴더에 둔다 — 그 폴더를
    ///   base 로 준 원천이 체크아웃 없이 화면을 낸다. `prompts\` 가 없는 폴더(예: 체크아웃 밖
    ///   `CARGO_TARGET_DIR` 의 exe 폴더)면 `INTERNAL` 이다.
    #[test]
    fn a_release_shaped_folder_renders_and_one_without_prompts_is_internal() {
        let _env = env_guard();
        let release = release_shaped("release", &shipped_file());
        let screen = screen_of(&HelpSource::new(release.clone()), Some(CLI_GROUP_MAIL));
        assert_eq!(
            screen,
            render_help_from(&shipped_text(), HelpTopic::Mail),
            "릴리스 모양 폴더의 화면이 배포 본문의 화면과 같아야"
        );

        let bare = temp_dir("bare");
        let (code, hint) = rejection_of(&HelpSource::new(bare.clone()), &request(None));
        assert_eq!(code, ErrorCode::Internal.as_str(), "{hint}");
        assert!(
            hint.contains("engram-help.md"),
            "읽으려던 경로를 실어야: {hint}"
        );

        let _ = std::fs::remove_dir_all(&release);
        let _ = std::fs::remove_dir_all(&bare);
    }
}
