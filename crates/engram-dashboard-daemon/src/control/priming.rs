//! PrimingProvider seam(ADR-0092 수신 계약) — 스폰 시 시스템 프롬프트에 주입할 프라이밍 파일의
//! **절대경로**를 산출한다.
//!
//! ★역할★: 데몬이 스폰(provision)마다 이 seam 에 프라이밍 파일 경로를 물어, 있으면 그 절대경로를
//!   `ControlEndpoint.priming_file` 로 실어 보낸다. backend/claude.rs 가 그걸 받아
//!   `--append-system-prompt-file <abs-path>` 로 주입하고, claude CLI 가 그 파일을 **직접 읽어**
//!   시스템 프롬프트에 덧붙인다. 즉 이 모듈은 **경로만** 다룬다 — 파일 내용을 읽지 않는다(하드코딩 금지,
//!   ADR-0092: 내용은 외부 MD `prompts/agent-priming.md` 에만 산다).
//!
//! ★seam 인 이유(ADR-0092 "길은 뚫어둔다")★: 현재 구현체(`FilePrimingProvider`)는 **전원 무조건 같은
//!   공용 파일**을 준다(임시판). 미래에 에이전트별 프롬프트 인젝션/스킬등록 시스템이 오면 이 trait 의
//!   구현만 갈아끼워(에이전트별·capability 별 프라이밍) 배선을 안 바꾸고 흡수한다. 그래서 provision 이
//!   `PrimingProvider` trait 에만 의존하게 둔다.
//!
//! ★graceful(스폰을 막지 않는다)★: 해석된 파일이 없으면 `None` 을 돌려주고 warn 로그만 남긴다 — 프라이밍
//!   부재는 스폰 실패 사유가 아니다(제어 채널 provision 의 fail-closed 와 **다른** 정책). 에이전트는
//!   프라이밍 없이 뜬다(수신 계약 미적용이나 기능적으로는 동작).
//!
//! tauri import 0(daemon crate).
// ADR-0092

use std::path::PathBuf;

/// 프라이밍 변형(ADR-0099) — 백엔드 MCP-capability 가 고르는 정적 파일 축. **정합 불변식**: 이 변형이
/// 가르치는 채널 집합은 provision 이 물리적으로 깐 채널 집합과 일치해야 한다(어기면 발신 freeze 재발 —
/// MCP 노출 + CLI-only 지시 = ~6/7 미발신 실측). 그래서 백엔드 capability 하나가 이 변형과 채널 배선을
/// 함께 움직인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimingVariant {
    /// MCP-capable 백엔드(claude). send_message 툴 주력 + engram-send CLI 폴백을 가르친다(both-teaching).
    ///   → `prompts/agent-priming.md`.
    McpPrimary,
    /// 비-MCP 백엔드(codex/gemini 등 미래). engram-send CLI 만 가르친다(send_message 단어 자체 부재).
    ///   → `prompts/agent-priming-cli.md`.
    CliOnly,
}

/// 프라이밍 파일 경로를 산출하는 seam. 구현은 스폰 시점에 `priming_file(variant)` 로 **절대경로 or None**
/// 을 돌려준다. Send+Sync+'static — DaemonControlChannel 이 Arc 로 들고 provision 마다 부른다.
pub trait PrimingProvider: Send + Sync + 'static {
    /// 이번 스폰에 주입할 프라이밍 MD 파일의 **절대경로**. 없거나(파일 부재) 미구성이면 `None`.
    /// ★절대경로 계약★: 에이전트의 cwd 는 데몬/repo 와 다르므로(각 워크스페이스) 반드시 절대경로여야
    ///   claude 가 파일을 찾는다 — 상대경로면 에이전트 cwd 기준으로 해석돼 어긋난다.
    /// `variant`(ADR-0099): 백엔드 MCP-capability 가 고른 프라이밍 축. 구현이 이 값으로 파일을 가른다.
    fn priming_file(&self, variant: PrimingVariant) -> Option<PathBuf>;
}

/// 고정 파일 로더(ADR-0092 임시판 — 전원 무조건 같은 공용 MD 주입).
///
/// ★경로 해석(문서화된 선택)★:
///   1. env `ENGRAM_PRIMING_FILE` 이 **비어 있지 않게** 설정돼 있으면 그 경로를 절대화해 쓴다(override 우선).
///   2. 아니면 고정 상대경로 `prompts/agent-priming.md` 를 **base_dir**(생성 시 주입 — 운영은 exe 기준
///      설치/repo 루트)에 붙여 절대화한다.
/// 두 경우 모두 (a) **절대경로로 만들 수 없으면 None**(상대경로를 claude 에 절대 넘기지 않는다),
///   (b) **cmd.exe 부패 위험**(비-UTF8 또는 `% & ^ | < >` 포함)이면 None + warn(아래 path_is_cli_safe),
///   (c) 파일 존재를 확인하고 없으면 None(graceful).
///
/// ★왜 base 를 exe 기준 루트로 받나(ADR-0092, 두 리뷰어 PRIMARY)★: 예전엔 base 를 데몬 프로세스 cwd
///   (`from_cwd`)로 삼았다 — 그러나 운영 데몬은 WMI Win32_Process.Create 로 떠 **부모 cwd 를 상속하지
///   않아**(cwd=System32) 프라이밍이 **조용히 비활성**됐다. 해결 = `default_data_dir` 이 `.engram-data`
///   를 anchor 할 때 쓰는 것과 **동일한 exe-walk-up 패턴**(discovery::find_install_root)을 재사용해
///   신뢰 가능한 절대 루트를 base 로 삼는다(cwd 불신).
///
/// ★왜 base 주입(new)인가★: 루트 해석을 이 모듈이 직접 하지 않고 생성 시 base 를 받는다 — 테스트가
///   base 를 임시 dir 로 바꿔 cwd/exe 오염 없이 결정적으로 검증하게(seam 다움). 운영 배선은 from_install_root.
pub struct FilePrimingProvider {
    /// 상대경로 해석의 기준 디렉토리(운영 = exe 기준 설치/repo 루트, discovery::find_install_root).
    /// env override 시엔 쓰지 않는다. ★절대경로여야 안전★ — 상대 base 면 아래 absolutize 가 None 을 낸다.
    base_dir: PathBuf,
}

/// 프라이밍 정적 파일 2개(repo·버전관리, ADR-0099). base_dir 에 붙여 해석한다. 변형(PrimingVariant)이
///   MCP-capable→both-teaching, 비-MCP→CLI-only 를 가른다.
/// ★정합 불변식(ADR-0099)★: 두 파일이 가르치는 채널 집합은 provision 이 그 변형에 물리적으로 까는 채널
///   집합과 일치해야 한다 — MCP_PRIMARY 는 send_message + engram-send 를, CLI_ONLY 는 engram-send 만
///   (send_message 단어 부재) 가르친다.
const REL_MCP_PRIMARY: &str = "prompts/agent-priming.md";
const REL_CLI_ONLY: &str = "prompts/agent-priming-cli.md";
/// env override 키(설정 시 base_dir·변형 무시하고 이 경로를 절대화해 쓴다 — 아래 override 우선 참조).
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
// ADR-0092
const CMD_UNSAFE_CHARS: &[char] = &['%', '&', '^', '|', '<', '>'];

/// 경로가 CLI(cmd.exe 경유) 로 안전하게 실릴 수 있나 — (a) 유효 UTF-8 이고 (b) cmd 메타문자
///   (`% & ^ | < >`)를 포함하지 않아야 true. 하나라도 어기면 false(호출자가 None + warn).
///
/// ★왜 UTF-8 도 보나(Codex #5)★: 인자는 최종적으로 `to_string_lossy()` 로 문자열화돼 CLI 에 실린다
///   (claude.rs). 비-UTF8 경로는 그 lossy 변환에서 U+FFFD 로 **손상**돼 claude 가 존재하지 않는 경로를
///   받는다. 그런 경로는 애초에 주입하지 않는다(손상된 경로 < 프라이밍 없음).
fn path_is_cli_safe(p: &std::path::Path) -> bool {
    // (a) 유효 UTF-8 인가(비-UTF8 은 lossy 손상 → 거부). to_str() 이 None 이면 비-UTF8.
    let Some(s) = p.to_str() else {
        return false;
    };
    // (b) cmd.exe 메타문자 부재. 하나라도 있으면 부패 위험 → 거부.
    !s.chars().any(|c| CMD_UNSAFE_CHARS.contains(&c))
}

impl FilePrimingProvider {
    /// base_dir(상대경로 해석 기준)로 생성. 운영은 from_install_root 가 exe 기준 절대 루트를 넘긴다.
    ///   테스트는 임시 dir(절대)을 직접 넘겨 결정적으로 검증한다.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// 운영 배선 생성자(ADR-0092 PRIMARY 수정) — exe 기준 설치/repo 루트를 base 로 삼는다. 데몬 cwd 는
    ///   WMI-spawn 시 System32 라 신뢰 불가하므로 쓰지 않는다(discovery::find_install_root 재사용 —
    ///   default_data_dir 과 동일 exe-walk-up 패턴). 루트를 못 얻으면(current_exe 실패 등) base 를
    ///   `.`(상대)로 둔다 — 그 경우 absolutize 가 절대화에 실패해 None 을 산출한다(상대경로 절대 미주입).
    pub fn from_install_root() -> Self {
        let base =
            engram_dashboard_discovery::find_install_root().unwrap_or_else(|| PathBuf::from("."));
        Self::new(base)
    }

    /// 경로를 **절대경로로만** 만든다 — 이미 절대면 그대로, 상대면 base_dir(절대 전제)에 붙인다.
    ///   결과가 그래도 절대가 아니면(base 도 상대 등) `None` 을 돌린다.
    ///
    /// ★absolute-or-None 계약(ADR-0092, Codex #3)★: 예전엔 base 가 상대면 current_dir 로 한 번 더
    ///   절대화를 **시도**했고, 그마저 실패하면 상대 PathBuf 를 그대로 돌렸다 — 그 상대경로가 이후
    ///   priming_file 의 Some 으로 새면 claude 가 **에이전트 cwd 기준**으로 잘못 해석한다(어긋남).
    ///   이제 cwd 폴백을 제거하고, 절대화 못 하면 엄격히 None 을 낸다(상대경로는 절대 Some 이 되지 않는다).
    fn absolutize(&self, p: PathBuf) -> Option<PathBuf> {
        if p.is_absolute() {
            return Some(p);
        }
        let joined = self.base_dir.join(&p);
        // base_dir 이 절대(운영 = find_install_root 절대 루트, 테스트 = 임시 dir)면 joined 도 절대.
        //   base 가 상대(루트 미발견 폴백 `.`)면 joined 도 상대 → None(cwd 폴백 없음, 계약).
        joined.is_absolute().then_some(joined)
    }

    /// 절대화 + CLI-안전 검사 + 존재 검사를 한데 묶은 최종 관문. 세 관문을 모두 통과해야 Some.
    ///   `label` 은 warn 로그 식별용(override/fixed).
    fn resolve_checked(&self, raw: PathBuf, label: &str) -> Option<PathBuf> {
        // 관문 1: 절대화(absolute-or-None 계약, Codex #3).
        let Some(abs) = self.absolutize(raw) else {
            tracing::warn!(
                label,
                "프라이밍 경로를 절대경로로 해석 못 함(base 가 절대 아님) — 프라이밍 없이 스폰 진행(ADR-0092 graceful)"
            );
            return None;
        };
        // 관문 2: CLI 안전(비-UTF8/cmd 메타문자 → 부패 위험, Codex #1+#5).
        if !path_is_cli_safe(&abs) {
            tracing::warn!(
                label,
                path = %abs.display(),
                "프라이밍 경로가 비-UTF8 이거나 cmd.exe 메타문자(% & ^ | < >)를 포함 — 부패 위험으로 미주입(ADR-0092)"
            );
            return None;
        }
        // 관문 3: 존재 검사.
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
        // 1) env override(비어 있지 않을 때만) — 절대화 + CLI-안전 + 존재 검사.
        //    ★override 실패는 fixed 로 폴백하지 않는다★: 명시 override 를 조용히 다른 파일로 갈아치우면
        //      혼란스럽다. 어느 관문에서 걸리든 None(프라이밍 없이 진행 — resolve_checked 가 warn).
        //    ★env override 는 두 변형을 아우르는 **단일 전역 승자**(ADR-0099 test-seam)★: 설정되면 variant
        //      와 무관하게 이 경로가 이긴다 — 하네스/운영자가 어떤 백엔드에도 특정 프라이밍을 강제할 수 있는
        //      test-seam 이다(roundtrip_smoke `--priming` 이 이 env 로 넘긴다). 운영은 미설정이라 아래 변형별
        //      정적 파일이 산다.
        //    ★이 override 를 ENGRAM_FORCE_CLI_ONLY_SEND 와 손으로 조합 금지★: override→MCP-teaching 파일 +
        //      force→CLI-only 물리 = 정합 불변식 위반(tooling 이 막던 pairing 위반 부활 — roundtrip_smoke 가 둘을 함께 거부).
        if let Some(v) = std::env::var_os(ENV_OVERRIDE) {
            if !v.is_empty() {
                return self.resolve_checked(PathBuf::from(v), "override");
            }
        }

        // 2) 변형별 고정 상대경로 → base_dir(exe 기준 루트) 기준 절대화 + CLI-안전 + 존재 검사(ADR-0099).
        let rel = match variant {
            PrimingVariant::McpPrimary => REL_MCP_PRIMARY,
            PrimingVariant::CliOnly => REL_CLI_ONLY,
        };
        self.resolve_checked(PathBuf::from(rel), "fixed")
    }
}

/// 프라이밍을 아예 안 주입하는 provider(항상 None). 프라이밍 무관 테스트·경로에서 seam 을 채우되
/// `--append-system-prompt-file` 이 안 붙게 한다(오늘 동작과 byte-identical). 실물 파일 의존을 없애
/// 테스트를 결정적으로 둔다.
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
        // 테스트 전용 — 변형과 무관하게 주어진 경로를 그대로 돌려준다(배선 도달만 검증).
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::Mutex;

    /// ★env 는 프로세스 전역★: `ENGRAM_PRIMING_FILE` 을 만지는 테스트끼리 병렬 실행 시 set/remove 가
    ///   서로를 지운다(플레이키). 이 mutex 로 그 테스트들을 직렬화한다(cargo 는 기본 병렬이라 필수).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 임시 dir 에 두 변형 프라이밍 MD 실물을 만들고 dir 을 돌려준다(ADR-0099).
    /// `prompts/agent-priming.md`(McpPrimary) + `prompts/agent-priming-cli.md`(CliOnly)를 만들어 base_dir
    ///   해석·변형 매핑을 검증한다.
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
        // env override 가 없어야 fixed 경로를 본다(테스트 격리 — 명시 remove).
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

    // ── ADR-0099: 변형 매핑 — McpPrimary → agent-priming.md / CliOnly → agent-priming-cli.md ──────
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
        // 파일 없는 빈 임시 dir → None(graceful, panic 없음).
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
        // fixture dir 에 fixed 파일이 있어도, env override 가 가리키는 다른 파일이 이긴다.
        let dir = make_fixture_dir();
        let override_file = dir.join("custom-priming.md");
        {
            let mut f = std::fs::File::create(&override_file).unwrap();
            writeln!(f, "# override 프라이밍").unwrap();
        }
        // ★env 는 프로세스 전역이라 이 테스트끼리 경합할 수 있다★: 다른 프라이밍 테스트가 remove_var 를
        //   하므로, 이 테스트는 set → 검증 → remove 를 한 스레드 안에서 순차로 하고, 검증 직전에 set 한다.
        std::env::set_var(ENV_OVERRIDE, &override_file);
        let provider = FilePrimingProvider::new(dir.clone());
        // ★env override 는 두 변형을 아우르는 단일 전역 승자(ADR-0099)★ — 어떤 variant 로 물어도 override 가
        //   이긴다. 여기선 McpPrimary 로 물어도 override 파일이 나오는지 확인.
        let got = provider
            .priming_file(PrimingVariant::McpPrimary)
            .expect("override 파일 존재 → Some");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("custom-priming.md"),
            "override 경로가 이겨야: {got:?}"
        );
        // CliOnly 로 물어도 동일 override(단일 전역 승자).
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
        // override 를 줬는데 그 파일이 없으면, fixed 로 폴백하지 않고 None(명시 override 실패 = 조용한
        //   갈아치우기 금지). fixture 에 fixed 파일이 **있어도** override 실패면 None 이어야 한다.
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
        // base_dir 이 **상대**면(운영 폴백 `.` 처럼) fixed 상대경로를 절대화할 수 없다 → None.
        //   상대경로를 Some 으로 흘리면 claude 가 에이전트 cwd 기준으로 잘못 해석하므로 절대 금지(계약).
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
        // fixed 파일이 실재하더라도, base 경로에 cmd 메타문자(`%`,`&`)가 있으면 부패 위험으로 None.
        //   임시 dir 밑에 메타문자 포함 하위 dir 을 만들고 그 안에 prompts/agent-priming.md 를 둔다.
        let root =
            std::env::temp_dir().join(format!("engram-priming-meta-{}", uuid::Uuid::new_v4()));
        // 메타문자 % 와 & 를 모두 포함하는 base(존재하는 실 디렉토리).
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
        // 각 메타문자를 포함하는 절대경로가 개별적으로 거부되는지 확인.
        for c in CMD_UNSAFE_CHARS {
            let p = std::path::PathBuf::from(format!("C:/base/na{c}me/agent.md"));
            assert!(
                !path_is_cli_safe(&p),
                "메타문자 {c:?} 포함 경로는 거부되어야"
            );
        }
        // 메타문자 없는 평범한 경로는 통과.
        assert!(
            path_is_cli_safe(std::path::Path::new("C:/base/prompts/agent-priming.md")),
            "메타문자 없는 경로는 안전"
        );
    }

    // ── ADR-0099: 정합 불변식 pin(content-based) — 운영 프라이밍 파일이 실제로 가르치는 채널 ──────────
    /// repo 루트(이 크레이트 매니페스트 두 단계 위). roundtrip_smoke 의 repo_root_from_manifest 와 동형 —
    ///   테스트는 항상 컴파일타임 소스 트리 안에서 도므로 MANIFEST_DIR 이 신뢰 가능하다.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")) // .../crates/engram-dashboard-daemon
            .parent()
            .and_then(|p| p.parent())
            .expect("repo 루트")
            .to_path_buf()
    }

    #[test]
    fn production_priming_files_pin_taught_channels() {
        // ★정합 불변식(ADR-0099)★: 물리적으로 깐 채널 집합 == 프라이밍이 가르치는 채널 집합. 여기선 파일
        //   수준에서 그 불변식을 못박는다 —
        //   - A(McpPrimary, agent-priming.md): send_message **와** engram-send 를 모두 가르쳐야(both-teaching).
        //   - B(CliOnly, agent-priming-cli.md): engram-send 는 가르치되 send_message 단어는 **부재**여야
        //     (MCP 입구를 프롬프트에서 완전히 삭제 — 지시-도구 불일치 freeze 방지).
        //   문구가 드리프트하면 이 테스트가 깨져 프라이밍-배선 정합을 강제한다.
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        let b = std::fs::read_to_string(root.join(REL_CLI_ONLY)).expect("B 프라이밍 파일 존재");
        assert!(
            a.contains("send_message"),
            "A(McpPrimary)는 send_message 를 가르쳐야(both-teaching)"
        );
        assert!(
            a.contains("engram-send"),
            "A(McpPrimary)는 engram-send 폴백도 가르쳐야(both-teaching)"
        );
        assert!(
            b.contains("engram-send"),
            "B(CliOnly)는 engram-send 를 가르쳐야"
        );
        assert!(
            !b.contains("send_message"),
            "B(CliOnly)는 send_message 단어가 부재여야(MCP 입구 완전 삭제 — freeze 방지)"
        );
    }
}
