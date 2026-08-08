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

/// 프라이밍 변형(ADR-0099) — 백엔드 MCP-capability 가 고르는 정적 파일 축. **정합 불변식(ADR-0128 로 등호
/// 복원)**: 이 변형이 **가르치는** 채널 집합 **=** provision 이 물리적으로 **깐** 채널 집합. 안 깐 채널을
/// 가르치면 발신 freeze 가 재발하고(MCP 노출 + CLI-only 지시 = ~6/7 미발신, ADR-0099), 반대로 깔고도 안
/// 가르치면 아무도 통제하지 못하는 우회 표면이 남는다. 그래서 백엔드 capability 하나가 이 변형과 채널 배선을
/// 함께 움직인다.
// ADR-0126
// ADR-0128
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimingVariant {
    /// MCP-capable 백엔드(claude). **send_message 툴만** 가르친다(ADR-0126 결정 1 — engram-send CLI 우회
    ///   교육 폐지). 그 스폰엔 engram-send 배선 자체가 없다(ADR-0128 결정 2) — 고장난 MCP 를 조용히
    ///   우회하면 고장이 관측되지 않으므로, 대신 principal 에게 보고하도록 가르친다(ADR-0126 결정 2).
    ///   → `prompts/agent-priming.md`.
    McpPrimary,
    /// 비-MCP 백엔드(codex/gemini 등 미래). engram-send CLI 만 가르친다(send_message 단어 자체 부재).
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
        // A 의 CLI 표면 **전면** 부재 — ADR-0126 영향/불변식의 검사 목록 그대로(바이너리 이름 + 딸린 플래그).
        //   이름만 지우고 플래그 표기가 남으면 우회 교육이 반쪽으로 살아남는다.
        for token in [
            "engram-send",
            "--to",
            "--body",
            "--request",
            "--reply-by",
            "--reply-to",
        ] {
            assert!(
                !a.contains(token),
                "A(McpPrimary)에 CLI 표면 {token:?} 이 다시 들어오면 ADR-0126 결정 1(우회 교육 폐지)의 회귀 — \
                 MCP 가능 스폰엔 engram-send 배선 자체가 없다(ADR-0128)"
            );
        }
        assert!(
            b.contains("engram-send"),
            "B(CliOnly)는 engram-send 를 가르쳐야"
        );
        assert!(
            !b.contains("send_message"),
            "B(CliOnly)는 send_message 단어가 부재여야(안 깐 MCP 입구 완전 삭제 — freeze 방지, ADR-0099)"
        );
    }

    /// ★채널 고장 시 에스컬레이션(ADR-0126 결정 2·5)★: "우회하지 마라" 와 "대신 principal 에게 보고하라"
    ///   는 한 몸이고(결정 2 는 분리 금지), 후자가 프라이밍에서 사라지면 결정이 반쪽이 된다.
    ///   ★ADR-0128 이후에도 이 교육은 필요하다★: 셸 우회 갈래는 배선 제거로 소멸했지만(engram-send 가
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
