//! PrimingProvider seam(ADR-0092 수신 계약) — 스폰 시 시스템 프롬프트에 주입할 프라이밍 파일의
//! **절대경로**를 산출한다.
//!
//! ★역할★: 데몬이 스폰(provision)마다 이 seam 에 프라이밍 파일 경로를 물어, 있으면 그 절대경로를
//!   `ControlEndpoint.priming_file` 로 실어 보낸다. 이 모듈은 **경로만** 다룬다 — 파일 내용을 읽지
//!   않는다(하드코딩 금지, ADR-0092: 내용은 외부 MD `prompts/agent-priming.md` 에만 산다).
//!
//! ★seam 인 이유(ADR-0092 "길은 뚫어둔다")★: 현재 구현체(`FilePrimingProvider`)는 **에이전트를 가리지
//!   않고 변형별 공용 파일 하나씩**만 준다(임시판). 미래에 에이전트별 프롬프트 인젝션/스킬등록 시스템이
//!   오면 이 trait 의 구현만 갈아끼워(에이전트별·capability 별 프라이밍) 배선을 안 바꾸고 흡수한다.
//!   그래서 provision 이 `PrimingProvider` trait 에만 의존하게 둔다.
//!
//! ★graceful(스폰을 막지 않는다)★: 해석된 파일이 없으면 `None` 을 돌려주고 warn 로그만 남긴다 — 프라이밍
//!   부재는 스폰 실패 사유가 아니다(제어 채널 provision 의 fail-closed 와 **다른** 정책). 에이전트는
//!   프라이밍 없이 뜬다(수신 계약 미적용이나 기능적으로는 동작).
//!
//! tauri import 0(daemon crate).
// ADR-0092

use std::path::PathBuf;

use engram_dashboard_core::agent::types::{
    CLI_EXE_ENV, CLI_EXE_NAME, CLI_GROUP_MAIL, CLI_MAIL_FLAGS, CLI_MAIL_VERBS,
};

/// 토큰을 이어 가는 문자 — 이 문자가 매치 양옆에 붙어 있으면 **다른 낱말의 일부**다.
///
/// ★경계 없이 substring 만 보면 평범한 산문이 걸린다(실측 리뷰)★: `The Engram mailbox is managed by the
///   daemon.` 이 `engram mail` 을 품고 `--token` 이 `--to` 를 품는다. 그 오탐은 단순 소음이 아니라
///   **오진단**이다 — 금지 게이트가 죄 없는 문장을 걸고, 에러 메시지는 운영자에게 그 문장을 지우라고 한다.
///   `-`·`_` 까지 포함하는 이유는 플래그·식별자가 그 문자로 이어지기 때문이다(`--to-be`·`--token`).
fn continues_token(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '-' || ch == '_'
}

/// `needle` 이 **낱말 경계**로 끊겨 등장하는가. `engram mail send` ✓ · `engram mailbox` ✗ ·
/// `--to bob` ✓ · `--token` ✗.
fn contains_bounded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !continues_token(c));
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !continues_token(c));
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    false
}

/// 프라이밍 본문 토큰 매칭용 정규화. **순서가 load-bearing 이다**: ① 문자 단위 접기(소문자 · 구분 기호 →
/// 공백 · 무의미 문자 삭제) → ② 구조 제거(HTML 주석) → ③ 공백 축약. **모든 판정 arm 이 이걸 통과한
/// 문자열만 본다**(원문 비교 경로 없음) — 그래야 "어느 arm 은 접히고 어느 arm 은 안 접힌다" 는 예외가 없다.
///
/// ★①이 ②보다 먼저인 이유(뒤집으면 서로를 무력화한다 — 실측 리뷰)★: 제로폭 문자나 이스케이프가 주석 닫는
///   표시 안에 끼면(`<!--x--` + 제로폭 + `>`) ②는 그 주석을 못 닫힌 것으로 보고 남긴다. 그러면 명령이 주석
///   텍스트에 끊긴 채 판정을 통과한다. 먼저 접어 두면 그 조합이 성립하지 않는다.
/// ★왜 정규화가 필요한가(여러 낱말 토큰의 취약점)★: 판정 토큰이 여러 낱말이라, 원문 그대로 비교하면
///   줄바꿈으로 끊긴 문장·연속 공백·`**engram** mail` 같은 강조가 전부 "안 가르침" 으로 읽힌다. 그 방향의
///   오판은 MCP 가능 스폰에 CLI-교육 프라이밍을 통과시켜 ADR-0099 의 발신 freeze 를 정상 negative 로
///   오귀속하게 만든다 — 판정기가 막으려던 바로 그 부류다.
/// ★두 문자 부류를 가른다★: **구분 기호**(`*`·백틱·`~`·중괄호·따옴표)는 낱말 **사이**에 서므로 **공백으로**
///   바꾼다 — 지워 버리면 `` `--to`-recipient `` 가 `--to-recipient` 로 **붙어** 판정이 뒤집히고,
///   `"${ENGRAM_CLI_EXE}" mail send` 의 인접이 깨진다. **무의미 문자**(`_`·백슬래시·제로폭류)는 낱말 **안**에
///   끼므로 지운다 — 이스케이프된 env 이름이 한 토큰으로 접혀야 실제 이름과 맞는다.
/// ★닫히지 않은 `<!--` 는 남긴다★: 프라이밍은 렌더링되지 않고 **원문 그대로** 시스템 프롬프트에 주입되므로,
///   렌더러 흉내로 뒷부분을 통째로 버리면 에이전트에겐 보이는 텍스트가 판정기에게만 사라진다.
/// ★찾는 토큰도 같은 정규화를 거친 값으로 만든다★: 리터럴을 손으로 적지 말고 `normalized_token()` 을 쓸 것.
fn normalize_for_token_match(content: &str) -> String {
    // ① 문자 단위 접기.
    let mut folded = String::with_capacity(content.len());
    for ch in content.chars() {
        match ch {
            // 낱말 사이에 서는 구분 기호 — 공백으로(삭제하면 양옆 토큰이 붙는다).
            '*' | '`' | '~' | '{' | '}' | '"' | '\'' => folded.push(' '),
            // 낱말 안에 끼는 무의미 문자 — 삭제(마크다운 강조/이스케이프 + 보이지 않는 서식 문자).
            '_' | '\\' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{200E}' | '\u{200F}'
            | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' => {}
            other => folded.extend(other.to_lowercase()),
        }
    }

    // ② 구조 제거 — 닫힌 HTML 주석만.
    let mut stripped = String::with_capacity(folded.len());
    let mut rest = folded.as_str();
    while let Some(open) = rest.find("<!--") {
        match rest[open..].find("-->") {
            Some(close_rel) => {
                stripped.push_str(&rest[..open]);
                stripped.push(' ');
                rest = &rest[open + close_rel + 3..];
            }
            None => break,
        }
    }
    stripped.push_str(rest);

    // ③ 공백 축약(①·②가 만든 연속 공백까지 여기서 한 번에).
    let mut out = String::with_capacity(stripped.len());
    let mut last_was_space = false;
    for ch in stripped.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        last_was_space = false;
        out.push(ch);
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// 찾는 토큰을 본문과 **같은 규칙**으로 접는다(리터럴 손타이핑 금지 — 정규화가 바뀌면 함께 움직인다).
fn normalized_token(raw: &str) -> String {
    normalize_for_token_match(raw)
}

/// 본문이 **우편 CLI 발신을 가르치는가**(강한 신호) — 판정 대상은 **실행 가능한 호출 형태**뿐이다:
/// 실행 주체(실행파일 이름 또는 env 변수) + 계열 토큰 + **동사**의 인접(`engram mail send` 등).
///
/// ★이름만 등장하는 것은 교육이 아니다★: `ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send
///   mail.` 은 **쓰지 말라는 문장**인데 예전 판정은 이걸 교육으로 셌다. 그래서 변수는 뒤에 계열 토큰이
///   붙을 때만 교육이고, 맨 언급은 `mentions_mail_cli_surface`(표면 흔적)로만 센다.
/// ★변수 형태를 교육에서 빼지 않는 이유(실재하는 교육 형태다)★: 변수로 부르는 호출은 명령 이름을 **한 번도
///   적지 않는다** — `run \`$ENGRAM_CLI_EXE mail send --to alice\`` 가 그 예이고, ADR-0098 이 기록한 옛
///   프라이밍이 정확히 그 형태였다(그래서 grant 가 미매칭됐다). 이걸 교육에서 빼면 그런 파일이 "발신을 안
///   가르침" 으로 읽혀 `--cli-only` 요구 게이트가 멀쩡한 실험을 거부한다.
/// ★basename 이 아니라 본문으로 판정한다 — 되돌리지 마라★: 이전 판본은 하드코딩된 basename 리스트만 봤고,
///   새 CLI-지시 프라이밍이 그 리스트에서 누락되자 가드가 조용히 우회돼 CLI 바이너리 부재(인프라 부재)가
///   SETUP-SKIP 대신 정상 negative(B_SENT=false)로 오귀속됐다. 대소문자·공백·강조를 접는 이유도 같다.
/// ★bare 실행파일 이름으로 보면 안 된다★: MCP 프라이밍이 같은 단어를 MCP **서버 이름**으로 정당하게
///   쓰므로(`on the engram server`), bare 판정은 MCP 전용 파일을 CLI-지시로 잡아 모든 MCP 실행을 스폰 전에
///   거부한다. 그래서 판정은 **계열 토큰과의 인접**이고, 그 매치는 낱말 경계로 끊는다(`engram mailbox` 제외).
/// ★동사까지 요구한다★: `engram mail` 만으로는 **실행되지 않는 조각**이라 "가르쳤다" 가 거짓이 된다.
///   그 조각을 교육으로 세면 `--cli-only` 요구 게이트가 만족돼, 동사를 모르는 B 의 미발신이 정상 negative 로
///   채점된다 — 이 판정기가 막으려는 오귀속 그 자체다. 동사 목록은 core `CLI_MAIL_VERBS`(파서와 공유).
/// ★이 게이트가 **못 잡는** 것(알려진 한계 — 늘리려 하기 전에 읽을 것)★:
///   - **부정문**: `Do NOT use engram mail send; use MCP instead.` 는 여전히 "가르침" 으로 잡힌다. 문장의
///     부정 여부를 텍스트로 판정하는 것은 휴리스틱 늪이라 **의도적으로 시도하지 않는다**(사용자 결정
///     2026-08-04, 이번 라운드 재확인). 대가는 그런 파일을 `--cli-only` 없이 넘길 때의 헛된 SETUP-FAIL
///     이고, 운영 프라이밍 2종엔 그런 문장이 없어 실경로 영향은 0 이다. **요란한 거부가 조용한 오귀속보다
///     낫다** 는 이 게이트의 기조와 같은 방향이다.
///   - **셸 문법 일반**: 이건 토큰 인접 매처지 셸 파서가 아니다. 변수 재바인딩(`X=$ENGRAM_CLI_EXE; $X mail
///     send`)·별칭·여러 줄 분할 호출은 못 본다. 여기를 파서로 키우는 대신, 그 부류가 실제로 나오면 그때
///     프라이밍 쪽을 고친다.
///   - **괄호류 구분 기호**: `{}`·따옴표는 낱말 사이 기호로 접지만 `()`·`[]` 는 접지 않는다 — `engram
///     (mail) 계열` 같은 산문이 명령으로 잡히는 오탐(그쪽이 더 나쁘다)을 피하기 위해서다.
/// ★계열을 **우편으로 한정**하는 것도 의도다(임의 계열로 넓히지 말 것)★: 이 판정이 지키는 불변식은
///   ADR-0128/0099 의 **우편** 교육↔배선 등호이고, 오귀속되는 관측도 우편 발신(`B_SENT`)이다. 제어 계열
///   (`engram help`·후속 `engram agent …`)은 **모든** 스폰이 쓰는 표면이라(ADR-0132 결정 4·5) 그것까지
///   세면 ① MCP 프라이밍이 제어 한 줄만으로 거부되고 ② "발신 방법을 하나도 안 가르친 `--cli-only`
///   프라이밍" 이 역방향 게이트를 통과한다 — 지금 막고 있는 오귀속이 양쪽에서 되살아난다.
// ADR-0128
// ADR-0132
pub fn teaches_mail_cli(content: &str) -> bool {
    let normalized = normalize_for_token_match(content);
    let group = normalized_token(CLI_GROUP_MAIL);
    CLI_MAIL_VERBS.iter().any(|verb| {
        let verb = normalized_token(verb);
        [CLI_EXE_NAME, CLI_EXE_ENV].iter().any(|invoker| {
            contains_bounded(
                &normalized,
                &format!("{} {group} {verb}", normalized_token(invoker)),
            )
        })
    })
}

/// 본문에 **우편 CLI 표면의 흔적이 있는가**(넓은 신호) — 강한 신호 ∪ CLI 절대경로 env 변수 이름 ∪ CLI 전용
/// 플래그 표기.
///
/// ★왜 강한 신호와 나눠 두는가(둘을 합치면 한쪽이 반드시 틀린다)★: 두 게이트의 요구가 반대다.
///   - **금지 방향**(MCP 가능 스폰이 CLI 교육을 지니면 안 된다)은 의심스러우면 막는 게 안전하다 →
///     이 넓은 신호를 쓴다. 명령 이름을 안 적고 플래그·변수만 적은 문서도 CLI 표면이 새어 든 것이다.
///   - **요구 방향**(`--cli-only` 는 CLI 발신 교육을 **요구**한다)에 이 넓은 신호를 쓰면, 플래그만 스치고
///     호출 형태는 안 가르친 문서가 통과해 B 가 발신 방법을 모른 채 도는 실험이 valid 로 채점된다.
///     그쪽은 `teaches_mail_cli`(강한 신호)여야 한다.
pub fn mentions_mail_cli_surface(content: &str) -> bool {
    if teaches_mail_cli(content) {
        return true;
    }
    let normalized = normalize_for_token_match(content);
    // 동사 없는 `engram mail` 조각도 **표면**으로는 센다 — 실행은 못 해도 CLI 표면이 새어 든 문서다.
    let group_form = format!(
        "{} {}",
        normalized_token(CLI_EXE_NAME),
        normalized_token(CLI_GROUP_MAIL)
    );
    if contains_bounded(&normalized, &group_form)
        || contains_bounded(&normalized, &normalized_token(CLI_EXE_ENV))
    {
        return true;
    }
    CLI_MAIL_FLAGS
        .iter()
        .any(|f| contains_bounded(&normalized, &normalized_token(f)))
}

/// 프라이밍 변형(ADR-0099) — 백엔드 MCP-capability 가 고르는 정적 파일 축. **정합 불변식(ADR-0128 로 등호
/// 복원)**: 이 변형이 **가르치는** 채널 집합 **=** provision 이 물리적으로 **깐** 채널 집합. 안 깐 채널을
/// 가르치면 발신 freeze 가 재발하고(MCP 노출 + CLI-only 지시 = ~6/7 미발신, ADR-0099), 반대로 깔고도 안
/// 가르치면 아무도 통제하지 못하는 우회 표면이 남는다. 그래서 백엔드 capability 하나가 이 변형과 채널 배선을
/// 함께 움직인다.
// ADR-0126
// ADR-0128
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimingVariant {
    /// MCP-capable 백엔드(claude). **send_message 툴만** 가르친다(ADR-0126 결정 1 — 우편 CLI 우회
    ///   교육 폐지). 그 스폰엔 CLI 배선 자체가 없다(ADR-0128 결정 2) — 고장난 MCP 를 조용히
    ///   우회하면 고장이 관측되지 않으므로, 대신 principal 에게 보고하도록 가르친다(ADR-0126 결정 2).
    ///   → `prompts/agent-priming.md`.
    McpPrimary,
    /// 비-MCP 백엔드(codex/gemini 등 미래). `engram mail` CLI 만 가르친다(send_message 단어 자체 부재).
    ///   → `prompts/agent-priming-cli.md`.
    CliOnly,
}

/// Send+Sync+'static — DaemonControlChannel 이 Arc 로 들고 provision 마다 부른다.
pub trait PrimingProvider: Send + Sync + 'static {
    /// 이번 스폰에 주입할 프라이밍 MD 파일의 **절대경로**. 없거나(파일 부재) 미구성이면 `None`.
    /// ★절대경로 계약★: 에이전트의 cwd 는 데몬/repo 와 다르므로(각 워크스페이스) 반드시 절대경로여야
    ///   claude 가 파일을 찾는다 — 상대경로면 에이전트 cwd 기준으로 해석돼 어긋난다.
    fn priming_file(&self, variant: PrimingVariant) -> Option<PathBuf>;
}

/// ★왜 base 를 exe 기준 루트로 받나(ADR-0092, 두 리뷰어 PRIMARY)★: 예전엔 base 를 데몬 프로세스 cwd
///   (`from_cwd`)로 삼았다 — 그러나 운영 데몬은 WMI Win32_Process.Create 로 떠 **부모 cwd 를 상속하지
///   않아**(cwd=System32) 프라이밍이 **조용히 비활성**됐다. 해결 = `default_data_dir` 이 `.engram-data`
///   를 anchor 할 때 쓰는 것과 **동일한 exe-walk-up 패턴**(discovery::find_install_root)을 재사용해
///   신뢰 가능한 절대 루트를 base 로 삼는다(cwd 불신).
///
/// ★왜 base 주입(new)인가★: 루트 해석을 이 모듈이 직접 하지 않고 생성 시 base 를 받는다 — 테스트가
///   base 를 임시 dir 로 바꿔 cwd/exe 오염 없이 결정적으로 검증하게(seam 다움). 운영 배선은 from_install_root.
pub struct FilePrimingProvider {
    base_dir: PathBuf,
}

const REL_MCP_PRIMARY: &str = "prompts/agent-priming.md";
const REL_CLI_ONLY: &str = "prompts/agent-priming-cli.md";
const ENV_OVERRIDE: &str = "ENGRAM_PRIMING_FILE";

/// ★cmd.exe 부패 위험 문자(ADR-0092, Codex #1+#5)★: Windows 에서 claude 인자는 `console_command`
///   (core/backend/mod.rs)가 `cmd.exe /c claude …` 로 감싸 실행한다 — 이 경로가 `%VAR%` 를 **따옴표
///   안에서도** 확장하고, `& ^ | < >` 를 셸 메타로 해석해 인자를 부패시킨다. 프라이밍 경로에 이 문자가
///   있으면 claude 가 엉뚱한/잘린 경로를 받으므로 아예 주입하지 않는다(None).
///   ★PRE-EXISTING·별도 follow-up(scope, ADR-0092)★: `console_command` 자체의 cmd.exe 이스케이프 결함은
///   이 슬라이스가 도입한 게 아니라 **기존** 문제이고, 같은 경로로 실리는 `--mcp-config`(config_path)도
///   동일하게 노출된다(그건 데몬이 만드는 경로라 사실상 안전하나 원리는 같다). 여기선 슬라이스 수준의
///   싼 방어(프라이밍 경로 필터)만 하고, console_command 를 고치지 않는다 — 그건 **모든 backend** 인자에
///   영향을 주는 별도 과업으로 추적한다(scope creep 회피).
const CMD_UNSAFE_CHARS: &[char] = &['%', '&', '^', '|', '<', '>'];

/// ★왜 UTF-8 도 보나(Codex #5)★: 인자는 최종적으로 `to_string_lossy()` 로 문자열화돼 CLI 에 실린다
///   (claude.rs). 비-UTF8 경로는 그 lossy 변환에서 U+FFFD 로 **손상**돼 claude 가 존재하지 않는 경로를
///   받는다. 그런 경로는 애초에 주입하지 않는다(손상된 경로 < 프라이밍 없음).
fn path_is_cli_safe(p: &std::path::Path) -> bool {
    let Some(s) = p.to_str() else {
        return false;
    };
    !s.chars().any(|c| CMD_UNSAFE_CHARS.contains(&c))
}

impl FilePrimingProvider {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// 루트를 못 얻으면(current_exe 실패 등) base 를 `.`(상대)로 둔다 — 그 경우 absolutize 가 절대화에
    ///   실패해 None 을 산출한다(상대경로 절대 미주입).
    pub fn from_install_root() -> Self {
        let base =
            engram_dashboard_discovery::find_install_root().unwrap_or_else(|| PathBuf::from("."));
        Self::new(base)
    }

    /// ★absolute-or-None 계약(ADR-0092, Codex #3)★: 예전엔 base 가 상대면 current_dir 로 한 번 더
    ///   절대화를 **시도**했고, 그마저 실패하면 상대 PathBuf 를 그대로 돌렸다.
    ///   이제 cwd 폴백을 제거하고, 절대화 못 하면 엄격히 None 을 낸다(상대경로는 절대 Some 이 되지 않는다).
    fn absolutize(&self, p: PathBuf) -> Option<PathBuf> {
        if p.is_absolute() {
            return Some(p);
        }
        let joined = self.base_dir.join(&p);
        joined.is_absolute().then_some(joined)
    }

    fn resolve_checked(&self, raw: PathBuf, label: &str) -> Option<PathBuf> {
        let Some(abs) = self.absolutize(raw) else {
            tracing::warn!(
                label,
                "프라이밍 경로를 절대경로로 해석 못 함(base 가 절대 아님) — 프라이밍 없이 스폰 진행(ADR-0092 graceful)"
            );
            return None;
        };
        if !path_is_cli_safe(&abs) {
            tracing::warn!(
                label,
                path = %abs.display(),
                "프라이밍 경로가 비-UTF8 이거나 cmd.exe 메타문자(% & ^ | < >)를 포함 — 부패 위험으로 미주입(ADR-0092)"
            );
            return None;
        }
        // ★TOCTOU 잔여(수용, ADR-0092 Codex #4)★: 여기 is_file 통과 시점과 claude 가 실제로 파일을
        //   여는 시점(스폰 뒤) 사이에 파일이 사라지면 claude 는 프라이밍 없이 뜨거나 에러를 낸다. 프라이밍
        //   파일은 racing 대상이 아닌 안정 인프라(버전관리 MD)라 저위험 — best-effort 존재 검사로 수용하고
        //   락을 걸지 않는다(비용 대비 무가치). 잔여 리스크는 graceful 부재와 동급(스폰은 계속 뜬다).
        if abs.is_file() {
            return Some(abs);
        }
        tracing::warn!(
            label,
            path = %abs.display(),
            "프라이밍 파일을 못 찾음 — 프라이밍 없이 스폰 진행(ADR-0092 graceful)"
        );
        None
    }
}

impl PrimingProvider for FilePrimingProvider {
    fn priming_file(&self, variant: PrimingVariant) -> Option<PathBuf> {
        // ★override 실패는 fixed 로 폴백하지 않는다★: 명시 override 를 조용히 다른 파일로 갈아치우면
        //   혼란스럽다. 어느 관문에서 걸리든 None(프라이밍 없이 진행 — resolve_checked 가 warn).
        // ★env override 는 두 변형을 아우르는 **단일 전역 승자**(ADR-0099 test-seam)★: 설정되면 variant
        //   와 무관하게 이 경로가 이긴다 — 하네스/운영자가 어떤 백엔드에도 특정 프라이밍을 강제할 수 있는
        //   test-seam 이다(roundtrip_smoke `--priming` 이 이 env 로 넘긴다). 운영은 미설정이라 아래 변형별
        //   정적 파일이 산다.
        if let Some(v) = std::env::var_os(ENV_OVERRIDE) {
            if !v.is_empty() {
                return self.resolve_checked(PathBuf::from(v), "override");
            }
        }

        let rel = match variant {
            PrimingVariant::McpPrimary => REL_MCP_PRIMARY,
            PrimingVariant::CliOnly => REL_CLI_ONLY,
        };
        self.resolve_checked(PathBuf::from(rel), "fixed")
    }
}

/// 프라이밍 무관 테스트·경로에서 seam 을 채우되 `--append-system-prompt-file` 이 안 붙게 한다.
/// 실물 파일 의존을 없애 테스트를 결정적으로 둔다.
pub struct NoopPrimingProvider;

impl PrimingProvider for NoopPrimingProvider {
    fn priming_file(&self, _variant: PrimingVariant) -> Option<PathBuf> {
        None
    }
}

/// 고정 경로 provider(테스트 전용) — 주어진 절대경로를 그대로 돌려준다(존재 검사 없음). 스폰 배선에
/// 프라이밍이 실려 내려가는지 확인하는 통합 테스트에서 쓴다.
pub struct FixedPrimingProvider(pub PathBuf);

impl PrimingProvider for FixedPrimingProvider {
    fn priming_file(&self, _variant: PrimingVariant) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::{CLI_EXE_NAME, CLI_GROUP_MAIL};
    use std::io::Write as _;
    use std::sync::Mutex;

    /// ★env 는 프로세스 전역★: `ENGRAM_PRIMING_FILE` 을 만지는 테스트끼리 병렬 실행 시 set/remove 가
    ///   서로를 지운다(플레이키). 이 mutex 로 그 테스트들을 직렬화한다(cargo 는 기본 병렬이라 필수).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_fixture_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("engram-priming-test-{}", uuid::Uuid::new_v4()));
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let mut f = std::fs::File::create(prompts.join("agent-priming.md")).unwrap();
        writeln!(f, "# 테스트 프라이밍 (mcp-primary)").unwrap();
        let mut f2 = std::fs::File::create(prompts.join("agent-priming-cli.md")).unwrap();
        writeln!(f2, "# 테스트 프라이밍 (cli-only)").unwrap();
        dir
    }

    #[test]
    fn resolves_fixed_relative_to_absolute_under_base() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let provider = FilePrimingProvider::new(dir.clone());
        std::env::remove_var(ENV_OVERRIDE);
        let got = provider
            .priming_file(PrimingVariant::McpPrimary)
            .expect("고정 파일이 있으면 Some");
        assert!(got.is_absolute(), "해석 결과는 절대경로여야: {got:?}");
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md")
        );
        assert!(got.is_file(), "실제 파일을 가리켜야");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ADR-0099: 변형 매핑 ──────────────────────────────────────────────────────────
    #[test]
    fn variant_maps_to_distinct_files() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(dir.clone());
        let mcp = provider
            .priming_file(PrimingVariant::McpPrimary)
            .expect("McpPrimary 파일");
        let cli = provider
            .priming_file(PrimingVariant::CliOnly)
            .expect("CliOnly 파일");
        assert!(
            mcp.ends_with("prompts/agent-priming.md") || mcp.ends_with("prompts\\agent-priming.md"),
            "McpPrimary → agent-priming.md: {mcp:?}"
        );
        assert!(
            cli.ends_with("prompts/agent-priming-cli.md")
                || cli.ends_with("prompts\\agent-priming-cli.md"),
            "CliOnly → agent-priming-cli.md: {cli:?}"
        );
        assert_ne!(mcp, cli, "두 변형은 서로 다른 파일을 가리켜야");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_fixed_file_yields_none_no_panic() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("engram-priming-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(dir.clone());
        assert!(
            provider.priming_file(PrimingVariant::McpPrimary).is_none(),
            "부재 파일 → None"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_wins_over_fixed() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let override_file = dir.join("custom-priming.md");
        {
            let mut f = std::fs::File::create(&override_file).unwrap();
            writeln!(f, "# override 프라이밍").unwrap();
        }
        std::env::set_var(ENV_OVERRIDE, &override_file);
        let provider = FilePrimingProvider::new(dir.clone());
        let got = provider
            .priming_file(PrimingVariant::McpPrimary)
            .expect("override 파일 존재 → Some");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("custom-priming.md"),
            "override 경로가 이겨야: {got:?}"
        );
        let got_cli = provider
            .priming_file(PrimingVariant::CliOnly)
            .expect("override 파일 존재 → Some");
        assert!(
            got_cli.ends_with("custom-priming.md"),
            "CliOnly 로 물어도 override 가 이겨야(단일 전역 승자): {got_cli:?}"
        );
        std::env::remove_var(ENV_OVERRIDE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_missing_file_yields_none_not_fallback() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let ghost = dir.join("does-not-exist.md");
        std::env::set_var(ENV_OVERRIDE, &ghost);
        let provider = FilePrimingProvider::new(dir.clone());
        assert!(
            provider.priming_file(PrimingVariant::McpPrimary).is_none(),
            "override 파일 부재 → None(fixed 폴백 안 함)"
        );
        std::env::remove_var(ENV_OVERRIDE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ADR-0092 하드닝: absolute-or-None 계약(Codex #3) ──────────────────────────────
    #[test]
    fn relative_base_yields_none_never_relative_path() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(PathBuf::from("relative-base"));
        assert!(
            provider.priming_file(PrimingVariant::McpPrimary).is_none(),
            "상대 base → 절대화 불가 → None(상대경로 유출 금지)"
        );
    }

    // ── ADR-0092 하드닝: cmd.exe 메타문자 경로 → None(Codex #1+#5) ────────────────────
    #[test]
    fn cmd_metachar_in_path_yields_none() {
        let _env = ENV_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("engram-priming-meta-{}", uuid::Uuid::new_v4()));
        let meta_base = root.join("we&ird%dir");
        let prompts = meta_base.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let mut f = std::fs::File::create(prompts.join("agent-priming.md")).unwrap();
        writeln!(f, "# meta").unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(meta_base.clone());
        assert!(
            provider.priming_file(PrimingVariant::McpPrimary).is_none(),
            "cmd 메타문자(% &) 포함 경로 → None(부패 위험 미주입)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── ADR-0092 하드닝: path_is_cli_safe 단위(메타문자별) ────────────────────────────
    #[test]
    fn path_is_cli_safe_rejects_each_metachar() {
        for c in CMD_UNSAFE_CHARS {
            let p = std::path::PathBuf::from(format!("C:/base/na{c}me/agent.md"));
            assert!(
                !path_is_cli_safe(&p),
                "메타문자 {c:?} 포함 경로는 거부되어야"
            );
        }
        assert!(
            path_is_cli_safe(std::path::Path::new("C:/base/prompts/agent-priming.md")),
            "메타문자 없는 경로는 안전"
        );
    }

    // ── ADR-0099/0126: 정합 불변식 pin(content-based) — 운영 프라이밍 파일이 실제로 가르치는 채널 ──────
    /// 테스트는 항상 컴파일타임 소스 트리 안에서 도므로 MANIFEST_DIR 이 신뢰 가능하다.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")) // .../crates/engram-dashboard-daemon
            .parent()
            .and_then(|p| p.parent())
            .expect("repo 루트")
            .to_path_buf()
    }

    // ── 판정기 정규화(두 단어 토큰의 취약점) ────────────────────────────────────────────────
    /// 되돌리면 MCP 가능 스폰이 CLI-교육 프라이밍을 달고 통과한다 — 각 케이스가 그 우회 경로 하나씩이다.
    #[test]
    fn teaches_mail_cli_survives_whitespace_case_and_markdown_between_the_two_words() {
        for text in [
            "run engram mail send --to bob",
            "RUN ENGRAM MAIL SEND",
            "Use Engram Mail Send to deliver.",
            "run `engram mail send`",
            "run **engram** mail send",
            "run *engram* *mail* send",
            "run engram  mail send",    // 공백 2칸
            "run engram\nmail send",    // 줄바꿈으로 끊긴 문장
            "run engram\n   mail send", // 줄바꿈 + 들여쓰기
            "run engram\tmail send",
        ] {
            assert!(teaches_mail_cli(text), "CLI 교육으로 잡혀야: {text:?}");
        }
    }

    /// bare 이름 판정으로 되돌아가면 MCP 전용 프라이밍이 전부 CLI-교육으로 잡혀 MCP 실행이 스폰 전에
    /// 거부된다 — 그 회귀를 막는 음성 케이스.
    #[test]
    fn mcp_only_prose_is_not_mistaken_for_cli_teaching() {
        for text in [
            "call the MCP tool `send_message` with the recipient and body.",
            "on the `engram` server",
            "**engram** server tools are listed at startup",
            "its body opens with an [engram] marker",
            "the Engram broker daemon attaches the envelope",
            "",
        ] {
            assert!(!teaches_mail_cli(text), "CLI 교육이 아니어야: {text:?}");
            assert!(
                !mentions_mail_cli_surface(text),
                "CLI 표면 흔적도 없어야: {text:?}"
            );
        }
    }

    /// ★경계 없는 substring 으로 되돌리면 평범한 산문이 걸린다★ — 그 오탐은 소음이 아니라 오진단이다:
    ///   금지 게이트가 죄 없는 문장을 걸고, 메시지는 운영자에게 그 문장을 지우라고 지시한다.
    #[test]
    fn ordinary_prose_that_merely_contains_the_tokens_is_not_a_match() {
        for text in [
            "The Engram mailbox is managed by the daemon.",
            "engram mailbox",
            "engram mailing list",
            "pass the --token to the server",
            "see the --tool and --topic switches",
            "the --bodyguard flag is unrelated",
        ] {
            assert!(!teaches_mail_cli(text), "교육이 아니어야: {text:?}");
            assert!(
                !mentions_mail_cli_surface(text),
                "표면 흔적도 아니어야: {text:?}"
            );
        }
    }

    /// 렌더링엔 안 보이지만 정규화를 그냥 통과하던 구성 — 금지 게이트를 우회시킨다.
    #[test]
    fn markdown_constructs_between_the_two_words_do_not_hide_the_command() {
        for text in [
            "Run engram<!--split--> mail pending",
            "Run engram <!-- comment --> mail pending",
            "Run engram\u{200B} mail pending",
            "Run engram\u{00AD} mail pending",
            "Run engram\\ mail pending",
            // 문자 접기가 구조 제거보다 **먼저** 돌지 않으면 이 조합이 주석을 살린 채 통과한다.
            "Run engram<!--split--\u{200B}> mail pending",
            "Run engram<!--split--\\> mail pending",
            // 구분 기호는 지우지 않고 **공백으로** 바꿔야 인접이 산다.
            "run \"${ENGRAM_CLI_EXE}\" mail send --to alice --body hi",
            "run `engram` `mail` `send`",
        ] {
            assert!(teaches_mail_cli(text), "CLI 교육으로 잡혀야: {text:?}");
        }
        // 닫히지 않은 주석은 뒤를 버리지 않는다 — 프라이밍은 렌더링 없이 원문 그대로 주입되므로,
        //   렌더러 흉내로 잘라내면 에이전트에겐 보이는 텍스트가 판정기에게만 사라진다.
        assert!(
            teaches_mail_cli("intro <!-- unterminated\nRun engram mail send --to bob"),
            "닫히지 않은 주석 뒤의 교육도 보여야"
        );
    }

    /// ★변수 이름을 **부르는** 것과 **언급하는** 것은 다르다★: 금지 문장이 교육으로 세지면 안 되고,
    ///   변수로 부르는 실제 호출(옛 프라이밍의 형태 — ADR-0098)은 교육으로 세야 한다.
    #[test]
    fn env_var_counts_as_teaching_only_in_an_invocation_shape() {
        let mention = "ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send mail.";
        assert!(
            !teaches_mail_cli(mention),
            "맨 언급 + 금지문은 교육이 아니다: {mention:?}"
        );
        assert!(
            mentions_mail_cli_surface(mention),
            "그래도 CLI 표면 흔적이므로 금지 방향에선 걸려야: {mention:?}"
        );

        for text in [
            "run `$ENGRAM_CLI_EXE mail send --to alice --body hi`",
            "invoke $engram_cli_exe mail pending",
            "invoke $ENGRAM\\_CLI\\_EXE mail pending",
        ] {
            assert!(teaches_mail_cli(text), "변수 호출은 교육이다: {text:?}");
        }
    }

    /// 넓은 신호만 잡는 자리 — 실행 가능한 호출을 안 적은 문서(금지 방향에서만 쓰인다).
    #[test]
    fn surface_evidence_without_a_runnable_invocation_is_not_teaching() {
        for text in [
            "pass --to <name> and --body <text> to deliver",
            // 동사 없는 조각 — 실행되지 않으므로 "가르쳤다" 가 아니다.
            "Run engram mail",
            "the engram mail surface exists",
            // 구분 기호를 공백으로 접기 때문에 이 표기도 플래그로 보인다.
            "the `--to`-recipient argument",
        ] {
            assert!(mentions_mail_cli_surface(text), "표면 흔적: {text:?}");
            assert!(
                !teaches_mail_cli(text),
                "명령을 가르친 것은 아니다(요구 방향에서 이게 통과하면 B 는 발신법을 모른 채 돈다): {text:?}"
            );
        }
    }

    /// ★알려진 한계를 **테스트로 박제**한다★: 부정문은 여전히 "가르침" 으로 잡힌다 — 문장 부정 판정은
    ///   휴리스틱 늪이라 의도적으로 시도하지 않는다(요란한 거부 > 조용한 오귀속). 이 테스트가 빨개지면
    ///   누군가 negation 판정을 넣은 것이고, 그건 결정 사항이다.
    #[test]
    fn a_prohibition_sentence_still_reads_as_teaching_known_limit() {
        assert!(teaches_mail_cli(
            "Do NOT use engram mail send; use MCP instead."
        ));
    }

    // ADR-0126
    // ADR-0128
    #[test]
    fn production_priming_files_pin_taught_channels() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        let b = std::fs::read_to_string(root.join(REL_CLI_ONLY)).expect("B 프라이밍 파일 존재");
        assert!(
            a.contains("send_message"),
            "A(McpPrimary)는 send_message 를 가르쳐야(A 의 유일한 교육 표면)"
        );
        // A 의 CLI 표면 **전면** 부재 — ADR-0126 영향/불변식의 검사 목록 그대로(명령 표기 + 딸린 플래그).
        //   이름만 지우고 플래그 표기가 남으면 우회 교육이 반쪽으로 살아남는다.
        // ★맨 `contains` 로 되돌리지 말 것★: 대소문자·줄바꿈·마크다운 강조로 표기만 흐트러뜨려도
        //   `!a.contains(...)` 는 통과한다 — 판정기와 **같은 정규화·같은 낱말 경계**를 써야 이 pin 이
        //   실제로 막고, `--token` 같은 평범한 낱말에 헛걸리지도 않는다.
        //   (`mentions_mail_cli_surface` = 호출 형태 ∪ `engram mail` 조각 ∪ env 변수 이름 ∪ CLI 전용 플래그.)
        assert!(
            !mentions_mail_cli_surface(&a),
            "A(McpPrimary)에 CLI 표면(호출 형태 · `engram mail` 조각 · env 변수 이름 · CLI 전용 플래그 중 \
             하나)이 다시 들어오면 ADR-0126 결정 1(우회 교육 폐지)의 회귀 — MCP 가능 스폰엔 CLI 배선 \
             자체가 없다(ADR-0128)"
        );
        let cli_command = format!("{CLI_EXE_NAME} {CLI_GROUP_MAIL}");
        assert!(
            teaches_mail_cli(&b),
            "B(CliOnly)는 우편 CLI 명령({cli_command})을 가르쳐야"
        );
        assert!(
            !b.contains("send_message"),
            "B(CliOnly)는 send_message 단어가 부재여야(안 깐 MCP 입구 완전 삭제 — freeze 방지, ADR-0099)"
        );
    }

    /// ★채널 고장 시 에스컬레이션(ADR-0126 결정 2·5)★: "우회하지 마라" 와 "대신 principal 에게 보고하라"
    ///   는 한 몸이고(결정 2 는 분리 금지), 후자가 프라이밍에서 사라지면 결정이 반쪽이 된다.
    ///   ★ADR-0128 이후에도 이 교육은 필요하다★: 셸 우회 갈래는 배선 제거로 소멸했지만(우편 CLI 가
    ///   MCP 스폰에 없다) **조용한 포기** 갈래는 그대로 남고, auto mode 에선 grant 가 NO-OP 이라(ADR-0097)
    ///   이 지시를 붙드는 장치는 프라이밍 문장 하나뿐 — 그래서 파일 수준에서 못박는다.
    ///
    /// ★왜 "broken channel" 한 토큰만 pin 하나★: 문장 전체를 pin 하면 평범한 문구 손질에도 깨진다. "우회
    ///   하지 마라" 쪽 반쪽은 여기서 안 봐도 된다 — 위 pin_taught_channels 가 A 의 CLI 표면 부재와 B 의
    ///   send_message 부재를 이미 강제하므로 "대신 다른 입구를 써라" 식 회귀는 그쪽에서 잡힌다. 여기선
    ///   **고장을 고장이라 부르는 문장이 존재하는지**만 본다.
    ///
    /// ★두 파일 모두(결정 5)★: 유일한 입구가 고장났을 때 편지를 조용히 버리는 실패 모드는 변형과 무관하게
    ///   같다 — 표면 차이는 툴 이름뿐이라 에스컬레이션 교육은 양쪽에 같이 산다.
    // ADR-0126
    #[test]
    fn production_priming_files_teach_channel_failure_escalation() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        let b = std::fs::read_to_string(root.join(REL_CLI_ONLY)).expect("B 프라이밍 파일 존재");
        for (label, text) in [("A(McpPrimary)", &a), ("B(CliOnly)", &b)] {
            assert!(
                text.contains("broken channel"),
                "{label}: 발신 입구가 고장나면 우회하지 말고 principal 에게 보고하도록 가르쳐야(ADR-0126 결정 2·5)"
            );
        }
    }

    /// ★C3 회신 계약 프라이밍 정합(ADR-0103 결정 2/3 · spec §3)★: 데몬은 `type="request"` 봉투를 내보내고
    ///   기한 초과 시 발신자에게 `<notice>` 를 쏜다 — 그런데 **회신 자체는 LLM 준수(soft)** 라, 프라이밍이
    ///   회신 규칙을 안 가르치면 엄격 매칭(`reply_to` 필수)이 구조적으로 회신을 못 받는다(계약 반쪽).
    ///   그래서 두 변형 모두 "request 를 받으면 그 id 로 회신" 을 가르치는지 파일 수준에서 못박는다.
    ///
    /// ★변형별 표기(ADR-0126 결정 1 로 개정)★: **각 변형은 자기 입구의 표기만** 가르친다 — A(McpPrimary)는
    ///   툴 인자(snake_case `reply_to`)만, B(CliOnly)는 CLI 플래그(`--reply-to`)만. 봉투 인식(`type="request"`)
    ///   과 `<notice>` 는 입구와 무관한 수신측 계약이라 양쪽 공통이다. A 에 CLI 플래그가 남으면 폐지한 우회
    ///   교육이 되살아나고(ADR-0126), B 에 툴 인자 표기가 있으면 없는 입구를 가리킨다(지시-도구 불일치,
    ///   ADR-0099).
    // ADR-0126
    #[test]
    fn production_priming_files_teach_the_reply_contract() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        let b = std::fs::read_to_string(root.join(REL_CLI_ONLY)).expect("B 프라이밍 파일 존재");
        for (label, text) in [("A(McpPrimary)", &a), ("B(CliOnly)", &b)] {
            assert!(
                text.contains("type=\"request\""),
                "{label}: request 봉투를 알아보게 가르쳐야"
            );
            assert!(
                text.contains("<notice>"),
                "{label}: notice 는 회신 대상이 아님을 가르쳐야(데몬 전용 태그)"
            );
        }
        assert!(
            a.contains("reply_to") && a.contains("reply_by"),
            "A(McpPrimary)는 회신·기한을 툴 인자 표기(snake_case)로 가르쳐야(A 의 유일한 입구)"
        );
        assert!(
            b.contains("--reply-to"),
            "B(CliOnly)는 CLI 회신 플래그를 가르쳐야"
        );
        assert!(
            b.contains("--request") && b.contains("--reply-by"),
            "B(CliOnly)는 CLI request/기한 플래그를 가르쳐야"
        );
        assert!(
            !b.contains("reply_to") && !b.contains("reply_by"),
            "B(CliOnly)는 툴 인자 표기가 부재여야(없는 입구를 가리키지 않게)"
        );
    }
}
