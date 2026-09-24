//! AgentBackend — 백엔드별 명령 명세 산출 trait + 자유 함수 dispatch.
//!
//! transport(PtyTransport)는 claude/codex를 모른다. 누가 어떤 프로그램인지 아는 곳은
//! 오직 `backend/<이름>/` 폴더다.
//!
//! ★이 파일은 어느 백엔드의 항목도 이름으로 부르지 않는다★: 백엔드 이름이 적히는 자리는 **등록부**
//! (`pub mod`·`pub use`·정적 싱글턴)와 **두 dispatch 표**(`backend_for` · `backend_for_encoder`)뿐이고,
//! 백엔드별 지식은 전부 [`AgentBackend`] 메서드로만 나온다. 게이트는 `backend/claude/mod.rs` 헤더.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use shell::ShellBackend;

use std::path::PathBuf;
use std::sync::Arc;

use uuid::Uuid;

use crate::failure::AgentFailureKind;
use crate::profile::{AgentCommand, AgentProfile, SpawnMode};
use crate::session_tracker::SessionIdSource;
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::TurnSignal;
use crate::types::{
    AgentId, BackendCaps, CommandSpec, ControlEndpoint, OutputEvent, PtyError, CLI_EXE_ENV,
    CLI_EXE_NAME, TOKEN_ENV,
};

/// **왜 필요한가:** Windows에서 `claude`는 확장자 없는 npm shim이라, ConPTY가 쓰는 CreateProcessW가
/// 직접 못 띄운다(error 193 — PATHEXT/셸 해석을 안 함). `cmd.exe /c <prog> …`로 감싸면 cmd가
/// `<prog>.cmd` shim을 해석해 실제 프로세스를 띄운다. `cmd /c`는 대상이 종료되면 함께 종료되므로
/// "PTY 자식 = 에이전트" 수명이 유지된다(JobObject가 트리 통째 kill). 비Windows는 그대로 직접 실행.
///
/// shim이 아닌 일반 실행파일(Shell의 cmd.exe 등)에는 적용하지 않는다 — CLI 백엔드 전용.
pub(crate) fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let mut wrapped = Vec::with_capacity(args.len() + 2);
        wrapped.push("/c".to_string());
        wrapped.push(program.to_string());
        wrapped.extend(args);
        ("cmd.exe".to_string(), wrapped)
    }
    #[cfg(not(windows))]
    {
        (program.to_string(), args)
    }
}

// ── CLI 입구 주입(백엔드 공용) ────────────────────────────────────────────────

/// ADR-0086 스텝 2(CLI 입구): 스폰 env 에 CLI 크레덴셜 + 제어 평면 CLI(`CLI_EXE_NAME`) 형제 디렉토리
/// PATH 프리펜드.
///
/// ★여기 사는 이유 = 이 셋에 백엔드 지식이 하나도 없다(ADR-0004)★: 심는 것은 env 세 값과 PATH 뿐이고
///   그 어느 것도 특정 프로그램의 플래그·파일 규약이 아니다. 반대로 `--mcp-config` 같은 **번역**은 그
///   프로그램의 문법이라 그 백엔드 폴더에 남는다 — 그것까지 여기로 올리면 공용 자리가 한 백엔드의
///   어휘를 갖게 된다.
/// ★[`crate::types::ControlEndpoint::config_path`] 의 doc 이 이미 이 분담을 계약으로 적어 두었다★ —
///   그 Option 은 **MCP 입구의 유무**만 뜻하고 CLI 배선의 유무가 아니다. 이 함수가 공용인 것이 그 문장의
///   실물이다.
///
/// ★호출 조건 = control endpoint 가 있는 스폰 전부★: 제어 동사는 전원에게 열려 있고(ADR-0132 결정 5)
///   실행파일이 하나뿐이라 계열 단위로 갈라 깔 수 없다. 우편도 여기서 가리지 않는다 — 강제는 데몬의
///   자격증명 거절 한 곳뿐이다(ADR-0133 결정 3).
/// ★우편 가부를 env 로 실어 보내지 말 것★: 그 값은 에이전트 자신의 프로세스에 붙어 지울 수 있으므로
///   강제를 하나도 못 하면서, 외부가 CLI 화면을 고르는 입구만 만든다(사용자 결정 2026-09-23).
/// ★왜 env 인가★: 에이전트가 shell 로 그 명령을 부를 때 이 값을 읽어 데몬 제어 라우트에 Bearer
///   토큰으로 POST 한다. portable-pty CommandBuilder 가 부모 env 를 시드하므로 **모든 자식 프로세스
///   (Bash·그 손자)까지 상속**된다.
/// ★그래서 이 세 값은 에이전트 본인뿐 아니라 **그 프로그램이 대신 띄우는 보조 프로세스**의
///   자격증명이기도 하다★ — 그 프로그램이 자식 env 를 덧씌우지 않는 한. 어느 프로그램이 실제로 그런지는
///   그 백엔드 폴더가 실측으로 적는다(여기서 이름으로 세지 않는다 — ADR-0004).
/// ★보안★: 토큰이 env 로 노출된다 — 같은 OS 유저의 자식에만 상속되고 로그엔 안 찍지만 하드 격리는
///   원래 불가다(ADR-0086 §불변식). ★상속이 자식 전부에 걸린다는 것은 **에이전트가 띄운 하위
///   에이전트도 같은 토큰을 든다**는 뜻이다★ — 그 토큰으로 오는 보고를 신원 하나로 믿으면 자식의 값이
///   부모 자리에 앉는다. ★**그래서 이 토큰만으로 신원을 세우는 보고 입구를 새로 열지 말 것**★ — 한때
///   있던 그 입구(세션 id 훅 보고)는 ADR-0216 이 걷었고, 오늘 이 자격증명이 여는 것은 **에이전트 자신이
///   치는** 제어 동사와 우편뿐이다.
// ADR-0086 / ADR-0133 / ADR-0004 / ADR-0208 / ADR-0216
pub(crate) fn inject_cli_entrance(env: &mut Vec<(String, String)>, endpoint: &ControlEndpoint) {
    // ★ENGRAM_CONTROL_URL = base(스킴+호스트+포트)★: endpoint.url 은 MCP 라우트
    //   (`http://127.0.0.1:<port>/mcp`)라 CLI 가 붙을 base 로 쓰려면 라우트 suffix(`/mcp`)를 벗겨 base 만
    //   남긴다 — CLI 가 `<base>/control/send` 를 조립한다(라우트 경로 지식은 CLI 소유). suffix 가 없으면
    //   (형태 변주) url 을 그대로 base 로 쓴다(방어적).
    //   ★keep-in-sync(M5)★: 아래 strip_suffix 의 리터럴 "/mcp" 는 데몬측 MCP_PATH 상수와 **손으로 맞춰진**
    //   값이다 — 정본 = `crates/engram-dashboard-daemon/src/control/mcp_server.rs`(const MCP_PATH). 그쪽
    //   경로를 바꾸면 여기 리터럴도 함께 고쳐야 한다(빌드가 강제 못 함 → 어긋나면 base 파생이 틀어져 CLI 가
    //   조용히 404). 두 곳 상호 앵커.
    let base = endpoint
        .url
        .strip_suffix("/mcp")
        .unwrap_or(&endpoint.url)
        .to_string();
    env.push((TOKEN_ENV.to_string(), endpoint.token.clone()));
    env.push(("ENGRAM_CONTROL_URL".to_string(), base));
    // ★ENGRAM_CLI_EXE = CLI 바이너리 절대경로(F1)★: 프라이밍과 grant 는 bare 실행파일 이름
    //   (`CLI_EXE_NAME` — 아래 PATH 주입으로 해석)을 가르치지만, 이 절대경로 env 도 함께 싣는다 —
    //   진단·수동 조작용이다(ADR-0094 의 이름 정렬 자체는 PATH 로 이룬다).
    //   ★이 값을 가르치는 프라이밍은 없다 — 그러니 아래 loud skip 갈래(PATH 조합 실패·비-UTF8)의 복구
    //     수단으로 세지 말 것★: 그 갈래에서 에이전트는 bare 이름만 배운 채 PATH 로 해석하지 못하므로
    //     실질적으로 발신 불가이고, 신호는 그 warn 로그 하나뿐이다.
    //   None 갈래는 발신 입구가 하나도 안 남는 조합이라 데몬이 provision 에서 이미 fail-closed 로 끊는다
    //   — 여기 도달하지 않는 방어 경로다(도달해도 크레덴셜만 있고 부를 CLI 가 없는 무해한 상태).
    if let Some(send_exe) = &endpoint.send_exe {
        env.push((
            CLI_EXE_ENV.to_string(),
            send_exe.to_string_lossy().into_owned(),
        ));
        // ★PATH 주입(ADR-0094 bare 이름 해석)★: grant(`Bash(<CLI_EXE_NAME>:*)`)와 프라이밍이 모두 bare
        //   실행파일 이름을 가르치므로 스폰된 에이전트의 shell(및 그 자식 Bash 도구)이 그 이름을 실제로
        //   **찾을** 수 있어야 한다. send_exe 의 **부모 디렉토리**를 PATH **맨 앞**에 붙인다.
        //
        // ★base = env 벡터에 이미 있는 PATH(프로필 우선, FIX-1)★: 프로필 env 는 이 지점보다 **먼저**
        //   벡터에 들어와 있다. 데몬 프로세스 PATH(std::env::var_os) 로 리빌드하면 프로필이 실은 커스텀
        //   PATH 가 통째로 증발하므로, 벡터에 PATH 가 없을 때만 데몬 PATH 로 폴백한다.
        //   ★키 대소문자(Windows)★: 프로필이 "Path"·"PATH" 어느 표기로 넣어도 같은 변수다.
        //   ★last-match-wins + dedupe(load-bearing)★: transport(portable-pty)는 env 를 **순서대로**
        //   cmd.env(k,v) 하므로 같은 변수의 중복 항목이 있으면 자식엔 **마지막** 값이 산다(예: Windows
        //   에서 `[("PATH", 데몬), ("Path", 프로필)]`). 그래서 마지막 case-equivalent PATH 를 base 이자
        //   승리 항목으로 삼아 그 키 표기 그대로 제자리 교체하고, **나머지 PATH 항목은 전부 제거**한다 —
        //   중복을 남기면 앞쪽만 고친 뒤 뒤쪽 미수정 항목이 last-wins 로 이겨 주입이 **조용히 무력화**된다
        //   (adversarial 리뷰 must-fix). 구성: `send_exe_parent + separator + base` — 형제 디렉토리가
        //   **맨 앞**(shadowing 방어), 프로필/데몬 PATH 는 **tail 로 생존**.
        if let Some(parent) = send_exe.parent() {
            let is_path_key = |k: &str| {
                if cfg!(windows) {
                    k.eq_ignore_ascii_case("PATH")
                } else {
                    k == "PATH"
                }
            };
            let winner_idx = env.iter().rposition(|(k, _)| is_path_key(k));
            let base_os = winner_idx
                .map(|i| std::ffi::OsString::from(env[i].1.clone()))
                .or_else(|| std::env::var_os("PATH"));
            let mut dirs = vec![parent.to_path_buf()];
            if let Some(base) = &base_os {
                dirs.extend(std::env::split_paths(base));
            }
            match std::env::join_paths(dirs)
                .ok()
                .and_then(|j| j.into_string().ok())
            {
                Some(joined) => match winner_idx {
                    Some(i) => {
                        env[i].1 = joined;
                        let mut seen = 0usize;
                        env.retain(|(k, _)| {
                            if is_path_key(k) {
                                let keep = seen == i;
                                seen += 1;
                                keep
                            } else {
                                seen += 1;
                                true
                            }
                        });
                    }
                    None => env.push(("PATH".to_string(), joined)),
                },
                // ★loud skip(FIX-2/3)★: join 실패·비-UTF8 이면 주입을 **통째 건너뛴다** — lossy 변환한
                //   PATH 를 절대 push 하지 않는다(비-Unicode PATH 항목을 조용히 손상시키면 skip 보다
                //   나쁘다). skip 시 env 벡터는 **원래 그대로** 둬서 상속 PATH 가 안전 폴백이 된다.
                None => {
                    tracing::warn!(
                        "CLI PATH 주입 건너뜀(PATH 조합 실패 또는 비-UTF8) — grant/프라이밍은 bare `{}` 를 약속하나 이 설치에선 자식이 이름을 해석하지 못할 수 있음; 상속 PATH 유지",
                        CLI_EXE_NAME
                    );
                }
            }
        }
    }
}

// ── 세션 id 기록 포트 ─────────────────────────────────────────────────────────

/// backend 가 상대에게서 **받아 온** 세션 id 를 조립점에 넘기는 **한 동사** 포트.
///
/// 호출은 그 id 를 실제로 받은 뒤 정확히 한 번이고, 받지 못하면 한 번도 불리지 않는다.
/// 인자는 상대가 준 문자열 그대로다 — ★uuid 인지 판정하지 않는다★. 그 해석은 넘겨받는 쪽 몫이고,
/// 그래서 uuid 로 못 읽히는 값이 와도 이 포트는 성립한다.
///
/// ★`ProfileRegistry` 를 `backend/` 로 들이지 않으려고 이 모양이다(ADR-0004)★ — 그 타입이 여기서 보이면
/// 통로가 프로필 스키마를 알게 된다. 조립점이 자기 기록 수단을 이 한 동사로 감싸 넘긴다.
/// ★이름에 백엔드가 들어가지 않는 것도 계약이다★ — "codex 용" 이라 부르는 순간 같은 격리가 샌다.
/// ★한 동사인 것도 계약이다★ — "적혔나" 를 되묻는 둘째 동사를 달지 않는다. 되물을 것이 없도록 호출
/// **순서**가 대신 서 있다([`AgentBackend::open_spawn`] 의 계약 참조).
///
/// 반환이 없으므로 **실패는 호출자에게 돌아가지 않는다** — 기록하는 쪽이 자기 안에서 로그로 삼킨다.
/// 그래서 이 포트가 돌아왔다는 사실 위에 「영속됐다」를 얹으면 없는 보장을 인용하게 된다.
// ADR-0004
// ADR-0185
pub type SessionIdSink = Arc<dyn Fn(&str) + Send + Sync>;

/// unit struct로 구현되어 &'static으로 사용된다 — 상태 없음.
pub trait AgentBackend: Send + Sync {
    /// **우리가** 세션 id 를 뽑아 spawn 때 이 프로그램에 건네주나.
    ///
    /// true 면 manager 가 Fresh 마다 uuid 를 뽑아 [`AgentBackend::build_spec`] 의 `session_id` 로 넘기고,
    /// 그 값은 **첫 제출 때** 프로필에 영속된다(ADR-0226 — 스폰 때는 안 쓴다). sid drift 관측기도 그 값을
    /// 기준값으로 삼으므로 이 축에 매달린다.
    ///
    /// ★false 를 「세션이 없다」로 읽지 말 것★: 그 프로그램이 자기 id 를 **스스로 발급**하는 쪽일 수
    ///   있다. 그때 우리 uuid 를 심으면 그 프로그램이 한 번도 쓰지 않을 값이 프로필에 남고, 이어받기
    ///   판정이 그 가짜 값을 보고 선다. 이어받기 가부는 별개 축
    ///   ([`AgentBackend::can_resume_stored_session`])이 답한다.
    // ADR-0185
    fn assigns_session_id(&self, command: &AgentCommand) -> bool;

    /// 프로필에 **저장된 backend sid 로 이 명령을 이어받을 수 있나**.
    ///
    /// 부팅 복원과 활성화 입구가 「Resume 으로 띄울까 Fresh 로 띄울까」를 이 값과 sid 존재 여부로 함께
    /// 판정한다. 그 sid 를 **누가 발급했는지는 묻지 않는다**(ADR-0185) — 저장된 값이 이어받기에 쓰이나만
    /// 묻는다.
    ///
    /// ★`command` 를 받는 이유★: 같은 프로그램이라도 **어떤 모양으로 띄우느냐에 따라 갈린다** — 한
    ///   모양에는 이어받을 식별자가 있고 다른 모양에는 없을 수 있다. 인자 없는 술어로 두면 그 프로그램의
    ///   두 모양 중 한쪽 값이 다른 쪽에 그대로 적용된다.
    // ADR-0185
    fn can_resume_stored_session(&self, command: &AgentCommand) -> bool;

    /// 이 백엔드가 데몬 제어 채널(MCP 입구)을 **소비**하는가(ADR-0086 F3).
    /// true 면 manager 가 spawn 전에 provision 을 부르고(토큰+mcp-config 발급), 그 endpoint 를
    /// build_spec 에 넘긴다(claude=`--mcp-config`). false 면 manager 가 provision 을 **아예 건드리지
    /// 않는다** — shell 처럼 제어 채널을 안 쓰는 backend 는 registry 에 손대지 않아, config-write 실패가
    /// MCP 가 필요 없던 스폰을 중단시키는 회귀(round-2 F3)가 생기지 않는다.
    ///
    /// ★fail-closed 는 provision 을 **부르는** backend 에만★: true 인 backend 는 provision 이 Err 면
    ///   스폰이 중단된다(제어 채널 없이 몰래 도는 에이전트 금지). false 인 backend 는 그 계약과 무관하다.
    fn supports_control_channel(&self) -> bool;

    /// 이 backend(프로그램)가 **MCP 로 데몬 제어 채널을 붙이는가**(ADR-0099 → ADR-0209). 오늘 claude =
    /// true(mcp-config 파일을 `--mcp-config` 로 붙임) · codex = true(명령줄 오버라이드로 붙임) ·
    /// shell·gemini stub = false. ★이름이 「config 를 받아들이나」인데 오늘 뜻은 「MCP 로 우편을 쓰나」에
    /// 가깝다 — 그 어긋남은 알고 남긴 것이고(아래 조합 표), 「파일을 먹나」를 묻는 칸은 따로 있다★.
    /// ★backend 지식(ADR-0004)★: "어느 프로그램이 MCP 를 어떻게 먹나"는 backend-kind 지식이라 여기서
    /// 선언한다 — manager 가 `matches!` 로 직접 분기하지 않는다.
    ///
    /// 이 플래그가 provision 의 grant·**프라이밍 적재 여부**·**우편 가부**를 구동한다(정합 불변식 =
    /// 프라이밍이 가르치는 우편 채널 **=** 그 스폰이 쓸 수 있는 우편 채널. 못 쓰는 채널을 가르치면
    /// 발신 freeze 가 재발하고, 쓸 수 있는데 안 가르치면 통제 없는 우회 표면이 남는다). true 면
    /// `DaemonControlChannel::provision` 이 MCP bits 를 endpoint 에 실으며 MCP-only 교육 프라이밍
    /// (`eg_send` 만 — ADR-0126 결정 1)과 CLI 우편 불가를, false 면 **프라이밍 미주입** + CLI 우편
    /// 가능을 고른다(ADR-0133). 제어 CLI 배선은 이 축과 무관하게 전원에게 간다.
    ///
    /// ★**mcp-config 파일 write 는 더 이상 이 칸이 가르지 않는다 — 이 문장을 되돌리지 말 것**★
    ///   (ADR-0209 · `control/mod.rs` 의 `writes_mcp_config_file` 게이트): 「우리가 쓴 파일을 읽나」가
    ///   아래 [`AgentBackend::writes_mcp_config_file`] 로 갈라져 나갔고, 그 write 와 fail-closed `?` 는
    ///   그쪽 칸에만 걸린다. 여기 「true 면 mcp-config 를 쓴다」가 적혀 있던 동안 codex 는 우편 축을
    ///   맞추려 이 칸을 켜는 것만으로 **아무도 안 여는 평문 Bearer 토큰 JSON** 을 스폰마다 받았다.
    ///
    /// ★그래서 **두 칸의 네 조합이 전부 정당하다** — 특히 `(true, false)` 가 그렇다★:
    ///   - `(true, true)` = claude. MCP 로 우편을 쓰고, 그 입구가 우리가 쓴 파일이다(`--mcp-config`).
    ///   - **`(true, false)` = codex. MCP 로 우편을 쓰지만 그 파일은 한 번도 열지 않는다** — 부착 수단이
    ///     명령줄 오버라이드(`-c mcp_servers.<서버>=…`)이고 토큰은 그 값이 **이름으로 가리키는 env** 로
    ///     간다. 디스크에 사본이 생길 자리가 아예 없고, 그것이 이 조합의 값어치다.
    ///   - `(false, false)` = shell·gemini stub. 우편 평면 밖이거나 미측정이다.
    ///   - `(false, true)` 는 오늘 쓰는 backend 가 없다(파일은 MCP 입구를 위한 것이라 짝이 없으면 낭비다).
    /// ★첫 칸만 켜고 둘째 칸을 **안 본** backend 는 조용히 MCP 를 못 받는다★ — 둘째 칸의 기본값이
    ///   `false` 라, 파일을 **읽어야** 하는 backend 가 그것을 선언하지 않으면 데몬은 파일을 안 쓰고
    ///   endpoint 의 `config_path` 가 `None` 으로 오며, 그 backend 의 `--mcp-config` 주입이 한 줄도 돌지
    ///   않는다. 증상은 오류가 아니라 「툴이 없다」는 침묵이다. 새 backend 를 들일 때 **두 칸을 함께
    ///   답할 것** — 첫 칸은 「MCP 로 우편을 쓰나」, 둘째 칸은 「그 입구가 우리가 쓴 **파일**인가」다.
    ///
    /// ★false 쪽이 고르는 것은 「다른 프라이밍」이 아니라 「프라이밍 없음」이다 — 변형 축을 되살리지 말 것★:
    ///   CLI 전용 사본(`prompts/agent-priming-cli.md`)은 커밋 `2ef6902` 에서 삭제됐고 판정식은
    ///   `wants_priming = uses_mail && accepts_mcp_config` 하나다(`control/mod.rs`). 그래서 비-MCP 스폰은
    ///   지시서를 한 글자도 못 받고, 위 정합 불변식은 **아무것도 안 가르쳐서** 성립한다(없는 툴을 설명하는
    ///   문서를 주는 것이 거짓말이고 침묵은 아니다 — ADR-0209 결정 3).
    ///
    /// ★`supports_control_channel` 과의 관계★: 후자는 "provision 을 **부르나**"(제어 채널 자체를 소비하나),
    ///   이것은 "provision 이 붙일 채널 중 **MCP 를 낄 수 있나**"다 — 직교 축이다. 오늘 claude 와 codex 가
    ///   둘 다 true 이고(codex 의 MCP 부착 수단만 파일이 아닐 뿐이다 — 위 조합 표), shell·gemini stub 은
    ///   둘 다 false 다. "제어 채널은 CLI 로만 쓰는 백엔드"는 전자 true·후자 false 가 된다.
    ///   ★「codex 는 둘 다 false」로 적힌 자리를 만나면 낡은 것이다★ — 그 서술은 ADR-0209 이전 것이다.
    // ADR-0126
    // ADR-0133
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool;

    /// 이 backend 가 **데몬이 쓴 mcp-config 파일을 경로로 읽나**(claude=true — `--mcp-config <path>`).
    ///
    /// ★위 칸에서 갈라져 나온 축이다 — 도로 접지 말 것★: 위 칸은 오늘 사실상 「MCP 로 우편을 쓰나」를
    ///   뜻하고(데몬이 `mail_allowed` 를 거기서 파생한다 — ADR-0133), 이 칸은 「우리가 디스크에 평문
    ///   Bearer 토큰 JSON 을 쓸까」 하나만 묻는다. 겸직하던 동안 codex 는 우편 축을 맞추려 위 칸을 켜는
    ///   것만으로 **아무도 안 읽는 평문 토큰 파일**을 스폰마다 받았고(기본 ACL · revoke 때 삭제), 그
    ///   write 는 fail-closed 라 **안 읽는 파일 때문에 스폰이 끊길 수 있었다**.
    /// ★기본값 = false(비밀을 안 쓰는 쪽)★: 아래 우편 두 축의 fail-open 과 방향이 반대인데, 재는 것이
    ///   다르기 때문이다 — 틀린 false 의 대가는 **파일 부재**(그 backend 의 MCP 가 안 붙고 계약 표가 그
    ///   행에서 깨진다 — 시끄럽다)이고, 틀린 true 의 대가는 **평문 토큰이 디스크에 남는 것 + 스폰 중단
    ///   위험**이다(조용하다). 모르는 backend 에 비밀을 쓰지 않는다.
    /// ★true 를 선언한 backend 의 fail-closed 는 그대로다★ — 그 backend 는 이 파일 없이는 MCP 입구가
    ///   물리적으로 사라지므로, write 실패에 스폰을 계속시키면 제어 채널 없이 도는 에이전트가 된다.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// cwd·env는 manager가 정규화한 값을 전달한다.
    ///
    /// `control`(ADR-0086): 데몬이 발급한 제어 채널 엔드포인트(추상 descriptor). 있으면 backend 가
    ///   자기 프로그램 방식으로 명령줄에 주입한다(claude=`--mcp-config <path>` — 그 지식은
    ///   `backend/claude/` 단독, ADR-0004). None 이거나 제어 채널을 안 쓰는 backend(shell)면 무시한다.
    ///
    /// `resume_session_id` = 이 spawn 이 **이어받을** 저장된 backend sid. `None` = 이어받지 않는다
    ///   (Fresh 로 띄우거나, 저장된 값이 없거나, 이 backend 의 이어받기 축이 꺼져 있다).
    /// ★바로 위 `session_id` 와 **다른 값이다 — 한 칸으로 접지 말 것**★: 그쪽은 우리가 발급해 건네주는
    ///   값([`AgentBackend::assigns_session_id`] 축)이고, 이 칸은 상대가 발급해 우리가 받아 적어 둔
    ///   값이다([`SessionIdSink`] 가 적은 그것). 접으면 `assigns_session_id() == false` 가 뜻하는
    ///   「우리는 id 를 발급하지 않는다」가 거짓이 된다 — 발급 축이 꺼진 backend 의 argv 에 우리가 넘긴
    ///   값이 실리게 되므로.
    /// ★[`AgentBackend::open_spawn`] 의 같은 이름 칸과 **같은 값이다**★ — 갈라지는 것은 그 값으로
    ///   **무엇을 하나**뿐이다: 명령줄로 이어받는 backend(codex 터미널)는 여기서 argv 를 조립하고,
    ///   통로가 이어받기를 요청하는 backend(codex app-server)는 그쪽에서 첫 요청을 고른다.
    /// ★모드를 함께 보는 것이 여기서는 정당하다 — `open_spawn` 과 다른 점★: 그쪽은 모드를 안 받아
    ///   「Fresh 인데 이어받을 값이 있다」가 표현 불가능하지만, 이 자리는 `mode` 를 이미 받고 있고
    ///   **Fresh 와 Resume 의 argv 가 애초에 갈린다**(claude 의 `--session-id` ↔ `--resume`). 조립점은
    ///   그쪽과 같은 규율로 Fresh 면 이 칸을 비워서 넘긴다.
    // ADR-0185
    // ADR-0208
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec;

    /// 데몬이 발급한 제어 endpoint 로 이 spawn 을 **세울 수 있나**. `Err(사유)` = fail-closed(스폰 중단).
    ///
    /// ★왜 [`AgentBackend::build_spec`] 안에서 못 하나★: 그 메서드는 `CommandSpec` 을 돌려줄 뿐이라
    ///   「인자를 못 만들었다」를 호출자에게 말할 칸이 없다 — 조용히 빠진 인자로 스폰이 그대로 뜬다.
    ///   그래서 조립점이 조립 **전에** 이 술어를 묻고, `Err` 면 스폰을 끊는다(발급된 토큰은 조립점의
    ///   provision 가드가 회수한다).
    /// ★기본값 = `Ok(())`★ — 대부분의 backend 는 제어 endpoint 로 못 세울 상태가 없다. claude 처럼
    ///   데몬 쪽 `?`(mcp-config write 실패)가 이미 끊어 주는 backend 도 여기 손댈 것이 없다. 이 축이
    ///   필요한 것은 **데몬이 못 보는 실패**를 가진 backend 뿐이다 — codex 의 MCP 부착 값이 그것이다
    ///   (그 backend 는 파일을 안 쓰므로 데몬에 끊을 재료가 없다).
    /// ★「인가되지 않음」을 여기서 끊지 말 것★ — 데몬이 우편 입구를 안 준 스폰은 **정상**이고(운영자가
    ///   껐거나 그 backend 가 우편 평면 밖이다), 끊으면 그 조합에서 스폰이 통째로 죽는다. 이 술어가
    ///   재는 것은 「붙여야 하는데 못 붙인다」 하나다.
    // ADR-0004
    // ADR-0209
    fn precheck_control_endpoint(
        &self,
        _command: &AgentCommand,
        _control: Option<&ControlEndpoint>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// 이 backend(프로그램)가 결정하는 caps — session(resume)·model.
    /// transport(물리 채널)가 만드는 input/output/control 과 별개로, 최종 Capabilities 는
    /// `Capabilities::compose(transport_caps, backend_caps)` 로 합성된다.
    ///
    /// `command` 를 받는 이유(FIX 5): 같은 프로그램(claude)이라도 **모드에 따라 caps 가 다르다** —
    /// json(stream-json) 모드는 resume 미지원(ADR-0044 후속)이라 resume=false 를 신고해야 한다.
    /// backend 가 session caps 의 출처(ADR-0030)이고 mode 는 command 에 있으므로, 여기서 command 를
    /// 보고 정직하게 산출한다(type split 유지 — output/control 은 여전히 transport 소관).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps;

    /// 이 backend 가 `command` 에 대해 요구하는 **물리 통로 모양**.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 프로그램을 어떤 통로로 띄워야 하나" 는 프로그램별 지식이다.
    /// ★공용 층에 판정 술어를 되살리지 말 것★: 조립점이 `AgentCommand` payload 를 직접 되묻는 순간 한
    ///   backend 의 출력 형식 축이 전원의 통로 선택을 굴리게 되고, 그 backend 를 안 쓰는 항목까지 그
    ///   축을 통과한다.
    /// ★렌더 모드 축과 같은 것이 아니다★: 그 프로그램이 무엇을 그리나(터미널 TUI ↔ 구조화 스트림)는
    ///   그 backend 의 명령 payload 가 갖는 축이고, 이것은 그 선택이 **우리 쪽 통로**에 무엇을 요구하나다.
    ///   한 backend 안에서 둘이 1:1 로 붙어 있어도 어휘를 합치지 말 것 — 합치면 새 backend 가 자기 렌더
    ///   모드를 우리 통로 이름으로 신고해야 한다.
    /// ★기본값 = `Pty`★: 선언하지 않은 backend 는 터미널로 뜬다.
    // ADR-0004
    // ADR-0044
    fn transport_shape(&self, _command: &AgentCommand) -> TransportShape {
        TransportShape::Pty
    }

    /// 이 backend 가 `command` 를 띄울 **물리 통로를 직접 만들어**, 세션 조립에 필요한 나머지 값과 함께
    /// 한 번에 내준다.
    ///
    /// ★왜 backend 인가(ADR-0191)★: "이 프로그램을 어떤 통로 구현체로 띄우나" 는 프로그램별 지식이다.
    ///   조립점이 [`TransportShape`] 를 다시 match 해 생성자를 고르면 **가르는 자리가 두 곳**이 되고
    ///   (`backend_for` + 그 match), 그 자리가 백엔드 전용 통로 타입을 이름으로 알게 된다(ADR-0004).
    /// ★조립점은 돌려받은 통로의 실제 타입을 모른다★ — `Box<dyn AgentTransport>` 로만 받는다.
    /// ★아스펙트를 한 dispatch 에 모아 싣는다★: caps·인코더·턴 분류자·우편 자격은 각자 자기 메서드가
    ///   그대로 소유하고, 여기서는 그것들을 **모으기만** 한다. 조립점이 같은 switch 를 아스펙트마다 다시
    ///   타지 않게 하는 것이 이 묶음의 존재 이유다.
    /// ★`output.structured` 를 통로 구현체가 하드코딩하는 것은 여전히 금지(ADR-0044/0030)★ — 구조화
    ///   파이프를 고른 backend 가 그 자리에서 주입한다. 규칙은 그대로이고 주입하는 **자리**만 여기다.
    /// `cols`/`rows` 는 터미널 통로에만 쓰인다 — 파이프에는 크기 개념이 없어 무시된다.
    ///
    /// `sid_sink` = 상대가 세션 id 를 **발급해 주는** backend 가 그 값을 조립점으로 돌려보낼 곳
    /// ([`SessionIdSink`]). `None` = 조립점이 기록 수단을 주지 않았다 → 받아도 남길 곳이 없다.
    /// ★그 id 를 스스로 받지 않는 backend 는 이 칸을 그냥 무시한다★ — claude 처럼 자기 파일을 감시해
    ///   관측하는 쪽은 여기가 아니라 그 관측기 경로로 기록한다.
    /// ★순서 계약★: 이 포트는 **그 세션으로 무엇을 보내기 전에** 불린다. 그래서 기록됐는지 되묻는
    ///   둘째 동사가 필요 없다(포트가 한 동사인 이유 — [`SessionIdSink`]).
    ///
    /// `resume_session_id` = 이 spawn 이 **이어받을** 저장된 backend sid. `None` = 이어받지 않는다
    ///   (Fresh 로 띄우거나, 저장된 값이 없거나, 이 backend 의 이어받기 축이 꺼져 있다).
    /// ★[`AgentBackend::build_spec`] 의 `session_id` 와 **다른 값이다 — 같은 것으로 접지 말 것**★:
    ///   그쪽은 우리가 발급해 건네주는 값([`AgentBackend::assigns_session_id`] 축)이고, 이 칸은 상대가
    ///   발급해 우리가 받아 적어 둔 값이다([`SessionIdSink`] 가 적은 그것).
    /// ★모드를 함께 받지 않는 것은 의도다★ — 조립점이 Fresh 면 이 칸을 비워서 넘긴다. 모드와 값을 둘
    ///   다 받으면 「Fresh 인데 이어받을 값이 있다」는 조합이 표현 가능해지고, 그 조합의 해석이 backend
    ///   마다 갈린다.
    /// ★명령줄로 이어받는 backend(claude)는 이 칸을 쓰지 않는다★ — 그쪽은 `build_spec` 이
    ///   `--resume <sid>` 를 조립한다. 이 칸이 필요한 것은 **통로가 이어받기를 요청하는** backend 다.
    /// ★기본값 = PTY + 각 아스펙트가 신고한 값★: 통로를 따로 만들지 않는 backend 는 터미널로 뜨고
    ///   ([`AgentBackend::transport_shape`] 기본값과 같은 자리), 나머지 칸은 자기 메서드의 산출을 그대로
    ///   싣는다.
    /// ★단 이 기본값은 `transport_shape` 를 **읽지 않는다**★: 파이프를 요구한다고 신고해 놓고 이 메서드를
    ///   구현하지 않으면 조용히 터미널로 뜬다. 선언 표 트립와이어(`tests::expected_codec_axis`)는
    ///   `transport_shape` 의 신고값만 재므로 그 어긋남을 못 본다.
    ///
    /// `control` = [`AgentBackend::build_spec`] 이 받은 것과 **같은 endpoint**. 명령줄이 아니라 **통로
    ///   핸드셰이크로** 제어 평면 데이터를 실어야 하는 backend 를 위해 온다(codex app-server 가
    ///   `thread/start` 의 `developerInstructions` 에 프라이밍 내용을 싣는다).
    /// ★그쪽에서 이미 썼다고 여기서 다시 쓰지 말 것★ — 한 spawn 이 같은 값을 두 수단으로 보내면 상대가
    ///   그 둘을 어떻게 합치는지에 우리 동작이 매달린다. 조립하는 자리는 모드마다 하나여야 한다
    ///   (`resume_session_id` 가 argv 와 핸드셰이크로 갈리는 것과 같은 규율).
    /// ★대부분의 backend 는 이 칸을 무시한다★ — 명령줄로 번역해 끝나는 backend 는 `build_spec` 에서
    ///   이미 다 했다.
    // ADR-0004
    // ADR-0191
    // ADR-0215
    /// 이 backend 의 통로가 연결을 세워야 하나(공개 래퍼 [`declares_link`] 의 doc 이 정본).
    /// ★기본값 = `false`★ — 선언하지 않은 backend 는 배달 포트를 받지 않고 옛 판정 경로를 그대로 탄다.
    // ADR-0004
    fn declares_link(&self, _command: &AgentCommand) -> bool {
        false
    }

    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        resume_session_id: Option<Uuid>,
        // 연결의 결말을 배달할 곳 — `declares_link()` 가 true 인 backend 에만 온다.
        _link_sink: Option<LinkSink>,
        control: Option<&ControlEndpoint>,
    ) -> Result<SpawnParts, PtyError> {
        // 이 기본값은 세션 id 를 받아 오지도, 통로로 이어받지도, 제어 평면 데이터를 핸드셰이크에 싣지도
        //   않는다 — 밑줄 이름 대신 여기서 명시적으로 버린다(이름은 위 doc 이 부르는 것과 같아야 한다:
        //   rustdoc 이 시그니처를 그대로 렌더한다).
        let _ = (sid_sink, resume_session_id, control);
        let (transport, child_pid) = PtyTransport::open(spec, cols, rows)?;
        Ok(SpawnParts {
            transport: Box::new(transport),
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
        })
    }

    /// 이 backend 의 **턴 신호 분류자**(ADR-0113 사실 계층의 백엔드 지식 몫).
    ///
    /// ★왜 backend 인가(ADR-0004 · ADR-0110 결정 4 의 취지 승계)★: "어떤 출력 이벤트가 턴 진행이고
    ///   어떤 게 턴 종료인가" 는 프로그램별 지식이다. 특히 `OutputEvent::Structured` 는 **백엔드별
    ///   이벤트 탈출구**라 그 `kind` 의 의미가 백엔드마다 다르다 — 공용 층에서 해석하면 한 백엔드의
    ///   관례가 전원에게 강제된다(구조화 메타 라인을 내는 백엔드가 종료 신호 없이 영구 "턴 중" 이 된다).
    /// ★신호 어휘(`TurnSignal`)는 공용 하나뿐★: 백엔드마다 다른 건 **매핑**이지 어휘가 아니다.
    ///   백엔드별 신호 enum 을 만들면 소비자가 백엔드를 알아야 한다.
    /// ★기본값 = 신호 없음(fail-open)★: 매핑을 선언하지 않은 backend 는 관측 대상이 아니다. 미관측은
    ///   소비자 쪽에서 "즉시 배달" 로 흡수되지만(positive-knowledge-only), 근거 없는 진행 신호는
    ///   깨울 수 없는 "턴 중" 을 만든다 — 그래서 기본은 침묵이다.
    // ADR-0004
    // ADR-0113
    fn turn_classifier(&self) -> TurnClassifier {
        no_turn_signals
    }

    /// 이어받기가 실패했을 때 **그 프로그램의 출력에서 원인을 알아볼 수 있나**(ADR-0172 분류 입구).
    ///
    /// `evidence` = 그 화신이 남긴 텍스트(best-effort — 비어 있을 수 있다). 호출자가 **두 스트림을
    /// 합쳐** 넘긴다: 콘솔 꼬리(PTY 세션) + 진단 stderr 꼬리(파이프 세션). 둘은 transport 에 따라
    /// 배타적으로 차므로 구현체는 어느 쪽에서 왔는지 알 필요가 없다 — 문구만 본다.
    /// `None` = 이 텍스트만으로는 종류를 단정할 수 없다 → 호출자가 맥락 기본값으로 떨어뜨린다.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 문구가 무슨 뜻인가" 는 프로그램별 지식이다. manager 가 문자열을
    ///   직접 보면 그 지식이 공용 층으로 샌다.
    /// ★기본값 = 모름(fail-open)★: 선언하지 않은 backend 는 아무 것도 단정하지 않는다.
    /// ★살아 있는 세션에도 불린다★: 호출자는 종료를 기다리지 않고 진단 스트림만으로 확정할 수 있다
    ///   (`EarlyVerdict::Diagnosed`). 그러니 **"그 프로그램이 실패했을 때만 낼 수 있는 문구"** 만
    ///   선언할 것 — 대화 본문에 섞여 나올 수 있는 문구를 선언하면 성공한 활성화가 실패로 도장 찍힌다.
    // ADR-0004
    // ADR-0172
    fn resume_failure_kind(&self, _evidence: &str) -> Option<AgentFailureKind> {
        None
    }

    /// 이 backend 가 **편지를 읽는 주체**인가 = 에이전트 간 우편의 수신자 명단 자격.
    ///
    /// ★"턴 관측 가능성" 축이 아니다(ADR-0116 결정 1·7 을 되돌리는 게 아니다)★: 구조화 출력이 없는
    ///   터미널 claude 는 **그대로 받는다**. 여기서 가르는 건 관측 가능성이 아니라 **입력이 무엇으로
    ///   해석되는가** 다 — shell 은 입력을 명령으로 **실행**하는 채널이라 봉투가 닿으면 읽히는 게 아니라
    ///   실행된다(본문은 LLM 자유 텍스트라 `&`·`|`·`;` 가 섞이면 그 뒤가 별도 명령으로 파싱된다).
    /// ★기본값 = true(fail-open)★: 새 CLI 백엔드는 편지를 읽는 쪽이 정상이다. 모른다고 배달을 끊으면
    ///   편지가 조용히 사라지므로, 실행 채널인 backend 만 스스로 false 를 선언한다.
    // ADR-0004
    fn reads_messages(&self) -> bool {
        true
    }

    /// 이 backend 에 **우편 채널을 배정하나** = [`AgentBackend::reads_messages`] 의 보내기 짝.
    ///
    /// ★이 칸이 없던 동안 무엇이 벌어졌나(되살리지 말 것)★: 우편 가부를 데몬이
    ///   `!accepts_mcp_config` **하나로** 파생했다. 그래서 「제어 채널은 쓰지만 우편은 안 쓰는」 backend 가
    ///   제어 채널을 켜는 순간 **보내기 인가까지 함께 열렸다** — 받기는 위 칸이 false 인 채로. 보내기만
    ///   열린 그 비대칭은 아무 선언에도 안 적혀 있었고, 데몬은 「CLI 우편을 가르쳤다」고 기록하는데
    ///   실제 스폰은 그 교육을 한 글자도 안 받는 상태로 갈렸다.
    /// ★그래서 이 축은 **backend 가 말한다**(ADR-0004)★: 어느 프로그램이 우편 평면에 드나는 프로그램별
    ///   사실이다. 데몬은 이 값과 `accepts_mcp_config` 둘로 **한 값**(`mail_allowed`)을 파생하므로
    ///   ADR-0133 결정 2 의 단일 파생점은 그대로다 — 갈린 것은 재료가 하나에서 둘로 는 것뿐이다.
    /// ★기본값 = true(fail-open)★: 위 받기 칸과 같은 이유다. 모른다고 우편을 끊으면 편지가 조용히
    ///   사라지므로, 우편 평면 밖에 있는 backend 만 스스로 false 를 선언한다.
    /// ★`false` 는 "우편을 못 쓴다" 이지 "제어를 못 쓴다" 가 아니다★ — 제어 동사는 전원 개방이다
    ///   (ADR-0132 결정 5). 그 둘을 가르는 것이 이 칸을 세운 이유다.
    // ADR-0004
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// 이 backend 가 `command` 에 대해 쓰는 **입력 인코딩 태그**. 세션이 spawn 때 한 번 받아 보관한다.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 프로그램이 stdin 을 무엇으로 읽나" 는 프로그램별 지식이다.
    ///   dispatch 층이 명령 모양을 직접 보고 태그를 고르면 그 지식이 공용 층으로 샌다.
    /// ★기본값 = `Raw`★: 모르는 프로그램의 stdin 에 지어낸 봉투를 씌우면 그 프로그램은 입력을 통째로
    ///   못 읽는다 — 그래서 선언하지 않은 backend 는 바이트를 그대로 통과시킨다.
    // ADR-0004
    // ADR-0044
    fn input_encoder(&self, _command: &AgentCommand) -> InputEncoder {
        InputEncoder::Raw
    }

    /// 텍스트 1턴을 이 backend 의 구조화 입력 라인으로 감싼다 — [`InputEncoder::encode`] 의 실물.
    ///
    /// `msg_uuid` 는 [`AgentBackend::input_echo_event`] 에 넘어가는 값과 **반드시 같다**(호출자가 한 번
    /// 생성해 양쪽에 넘긴다 — dedup 키).
    /// ★기본값 = 감싸지 않음★: `input_encoder` 가 `Raw` 인 backend 는 [`backend_for_encoder`] 가 `None` 을
    ///   줘 이 경로를 아예 타지 않는다. 계약을 총(total)으로 두려는 자리채움이고, 불려도 `Raw` 와 같은
    ///   바이트를 낸다.
    fn wrap_input_turn(&self, text: &str, _msg_uuid: Uuid) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// 입력 성공 직후 세션 층이 emit 할 입력-시점 유저 에코 이벤트. `None` = 만들지 않는다.
    ///
    /// ★이벤트 shape 는 그 backend 의 decoder 가 replay 에 대해 만드는 것과 같아야 한다★ — 프론트가
    ///   uuid 로 둘을 합치므로 어긋나면 화면에 두 개로 남는다. 그 shape 는 구현체 소유다.
    /// ★기본값 = 없음★: PTY 가 입력을 즉시 로컬 에코하는 채널은 합성 에코를 만들면 중복이 된다.
    // ADR-0044
    // ADR-0045
    fn input_echo_event(&self, _text: &str, _msg_uuid: Uuid) -> Option<OutputEvent> {
        None
    }

    /// pump→core 앞에 꽂히는 출력 정제 decoder. `None` = 바이트 직통.
    ///
    /// [`AgentBackend::input_encoder`] 의 출력 방향 짝 — 감싼 쪽이 푸는 쪽도 소유한다.
    /// ★기본값 = 없음★: 선언하지 않은 backend 의 출력은 정제 없이 그대로 흐른다(터미널·평문 불변).
    // ADR-0004
    // ADR-0044
    fn output_decoder(&self, _command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        None
    }

    /// resume 스폰 시 이 명령의 과거 대화를 복원한 이벤트 목록. 빈 Vec = seed 안 함.
    ///
    /// transcript 의 파일 배치·포맷 지식은 전부 구현체 몫이다 — manager 는 이 호출만 하고 경로 규칙을
    /// 모른다.
    /// ★기본값 = 빈 Vec★: 선언하지 않은 backend 는 과거를 복원하지 않는다(fresh 버퍼 동작 불변).
    // ADR-0004
    // ADR-0079
    fn resume_transcript_events(
        &self,
        _command: &AgentCommand,
        _cwd: &std::path::Path,
        _session_id: Uuid,
    ) -> Vec<OutputEvent> {
        Vec::new()
    }

    /// 이 화신의 **세션 id 가 바뀐 것을 밖에서 알아볼 수 있나** — 알아본다면 그 관측기.
    ///
    /// 우리가 spawn 때 지정한 sid 는 그 프로그램 안에서 바뀔 수 있다(claude 는 `/clear`·`/resume` 로
    /// 프로세스는 그대로인 채 갈아탄다 — spike 실측). 바뀐 값을 못 따라잡으면 다음 복원이 옛 세션을
    /// 살린다. 그것을 어디서 어떻게 읽나(파일 경로·JSON 스키마·PID 우회)는 전부 구현체 몫이고,
    /// 러너([`crate::session_tracker`])는 주기·명부·콜백만 갖는다.
    ///
    /// `agent_id` 는 구현체 진단 로그의 상관 키로만 쓴다 — 관측 대상을 고르는 데 쓰지 않는다.
    /// ★기본값 = `None`(관측 없음)★: 선언하지 않은 backend 는 추적 대상이 아니다. 근거 없는 관측기를
    ///   기본으로 두면 남의 파일을 읽고 엉뚱한 sid 를 프로필에 적는다.
    /// ★`None` 은 실패가 아니다★: 관측이 없어도 복원은 최초 지정 sid 로 정상 동작한다(best-effort).
    // ADR-0004
    fn session_id_source(
        &self,
        _agent_id: AgentId,
        _child_pid: u32,
        _expected_sid: Uuid,
    ) -> Option<Box<dyn SessionIdSource>> {
        None
    }
}

/// spawn 한 번을 조립하는 데 필요한, backend 가 **한 dispatch 로** 내주는 묶음([`AgentBackend::open_spawn`]).
///
/// ★caps 소유권 분할은 그대로다(ADR-0030)★: `backend_caps` 는 backend 몫뿐이고, 통로가 신고하는
///   input/output/control caps 와는 세션 층에서 `Capabilities::compose` 로 합성된다. 여기서 미리 합치지
///   않는다 — 합치면 두 출처가 한 값으로 뭉개져 어느 쪽이 무엇을 신고했는지 추적이 끊긴다.
// ADR-0191
pub struct SpawnParts {
    pub transport: Box<dyn AgentTransport>,
    pub child_pid: Option<u32>,
    pub backend_caps: BackendCaps,
    pub encoder: InputEncoder,
    pub turn_classifier: TurnClassifier,
    pub reads_messages: bool,
}

/// 출력 이벤트 → 턴 신호 매핑 함수(ADR-0113). 백엔드가 자기 함수를 내주고 `OutputCore` 가 그 포인터를
/// 세션 수명 동안 들고 이벤트마다 부른다.
///
/// ★왜 함수 포인터인가★: emit 은 에이전트 출력마다 도는 hot path 다 — 매 이벤트에 `AgentCommand` 를
///   들고 dispatch 하면 세션이 자기 명령 사본을 들거나 매니저를 되짚어야 하고, `Box<dyn Fn>` 은 할당을
///   낳는다. backend 는 상태 없는 unit struct 라 매핑이 순수 함수로 떨어지므로, spawn 때 한 번 뽑아
///   포인터로 들고 있으면 할당·락·조회가 전부 0 이다.
pub type TurnClassifier = fn(&OutputEvent) -> Option<TurnSignal>;

/// 관측이 필요 없는 조립(테스트 하네스)도 이걸 꽂아 "신호 없음" 을 명시한다.
pub fn no_turn_signals(_event: &OutputEvent) -> Option<TurnSignal> {
    None
}

// ── 정적 싱글턴 ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;
static CODEX_BACKEND: CodexBackend = CodexBackend;

// 새 variant 연결 시: tests::expected_channel_matrix(tripwire)가 의식적 capability 선언을 강제한다 — ADR-0099
fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
        AgentCommand::Codex { .. } => &CODEX_BACKEND,
    }
}

/// 인코딩 태그를 **소유한 backend**. `backend_for`(명령 축)의 인코더 축 짝이다 — 세션 층은 태그만 들고
/// 명령은 갖고 있지 않아(소유권 분할) 여기서 되짚는다.
///
/// `None` = 그 태그에는 backend 지식이 없다(바이트 통과).
/// ★백엔드 이름은 이 표와 바로 위 `backend_for`, 그리고 싱글턴 선언에만 적는다★ — 그 바깥에서 백엔드
///   이름이 나오면 `backend/<이름>/` 폴더 격리가 샌 것이다(ADR-0004).
fn backend_for_encoder(e: InputEncoder) -> Option<&'static dyn AgentBackend> {
    match e {
        InputEncoder::Raw => None,
        InputEncoder::ClaudeStreamJson => Some(&CLAUDE_BACKEND),
        // 봉투를 통로가 만드는 태그라 backend 가 감쌀 것이 없다 — `encode` 는 통과, 에코도 없다.
        InputEncoder::TransportFramed => None,
    }
}

// ── 자유 함수 dispatch ─────────────────────────────────────────────────────────

pub fn assigns_session_id(c: &AgentCommand) -> bool {
    backend_for(c).assigns_session_id(c)
}

pub fn can_resume_stored_session(c: &AgentCommand) -> bool {
    backend_for(c).can_resume_stored_session(c)
}

/// 이 프로필을 **저장된 sid 로 이어받을 수 있나** = 이어받기 축 ∧ 저장된 sid 존재.
///
/// ★활성화 입구들이 이 규칙을 각자 적지 않게 하는 자리다★: 부팅 복원·명령 버스·WS 가 전부 여기를 부른다.
/// 규칙을 베끼면 다음 입구가 두 항 중 하나만 옮겨 적고, 그 입구만 조용히 다르게 판정한다.
/// ★명시 resume 요청은 이 함수가 모른다★ — 그것을 얹을지는 부르는 입구가 정한다.
// ADR-0185
pub fn can_resume_profile(p: &AgentProfile) -> bool {
    can_resume_stored_session(&p.command) && p.backend_session_id.is_some()
}

pub fn supports_control_channel(c: &AgentCommand) -> bool {
    backend_for(c).supports_control_channel()
}

pub fn accepts_mcp_config(c: &AgentCommand) -> bool {
    backend_for(c).accepts_mcp_config()
}

pub fn writes_mcp_config_file(c: &AgentCommand) -> bool {
    backend_for(c).writes_mcp_config_file()
}

pub fn uses_mail(c: &AgentCommand) -> bool {
    backend_for(c).uses_mail()
}

pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    resume_session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    control: Option<ControlEndpoint>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, resume_session_id, cwd, env, control)
}

/// 조립 **직전**의 fail-closed 게이트(정본 doc = [`AgentBackend::precheck_control_endpoint`]).
///
/// ★조립점이 이것을 부르는 자리는 provision 가드가 무장된 **뒤**여야 한다★ — 그래야 여기서 끊길 때
///   이미 발급된 토큰·파일이 회수된다.
pub fn precheck_control_endpoint(
    c: &AgentCommand,
    control: Option<&ControlEndpoint>,
) -> Result<(), String> {
    backend_for(c).precheck_control_endpoint(c, control)
}

pub fn transport_shape(c: &AgentCommand) -> TransportShape {
    backend_for(c).transport_shape(c)
}

/// 통로 실물도 그 위에 실리는 아스펙트 값도 전부 [`AgentBackend::open_spawn`] 이 소유하고 이 함수는
/// dispatch 뿐이다 — 새 backend 는 자기 폴더에서 그 메서드를 구현하면 되고 이 함수는 손대지 않는다
/// (교체성). ★이 호출이 자식 프로세스를 띄운다★ — 위아래 dispatch 들과 달리 부작용이 있고, 실패하면
/// 그 spawn 이 성립하지 않는다.
// ADR-0191
pub fn open_spawn(
    c: &AgentCommand,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
    sid_sink: Option<SessionIdSink>,
    resume_session_id: Option<Uuid>,
    link_sink: Option<LinkSink>,
    control: Option<&ControlEndpoint>,
) -> Result<SpawnParts, PtyError> {
    backend_for(c).open_spawn(
        c,
        spec,
        cols,
        rows,
        sid_sink,
        resume_session_id,
        link_sink,
        control,
    )
}

/// 이 backend 의 통로가 **연결을 세워야** 쓸 수 있나 = 결말을 배달할 축이 있나.
///
/// ★조립점이 이것으로 배달 포트를 **깔지 말지**를 가른다★ — 축이 없는 backend 에 포트를 주면 아무도
///   부르지 않는 채널을 감독자가 기다리게 된다. `false` 인 backend 의 활성화 판정은 옛 경로 그대로다
///   (claude·shell·stdio·codex 터미널 — 바이트 단위로 같다).
/// ★왜 backend 인가(ADR-0004)★: 「이 프로그램이 뜬 직후부터 쓸 수 있나, 아니면 왕복을 해야 하나」는
///   프로그램별 지식이다. manager 가 command 를 직접 matches! 하면 그 지식이 공용 층으로 샌다.
pub fn declares_link(c: &AgentCommand) -> bool {
    backend_for(c).declares_link(c)
}

pub fn turn_classifier(c: &AgentCommand) -> TurnClassifier {
    backend_for(c).turn_classifier()
}

pub fn resume_failure_kind(c: &AgentCommand, evidence: &str) -> Option<AgentFailureKind> {
    backend_for(c).resume_failure_kind(evidence)
}

pub fn reads_messages(c: &AgentCommand) -> bool {
    backend_for(c).reads_messages()
}

pub fn session_id_source(
    c: &AgentCommand,
    agent_id: AgentId,
    child_pid: u32,
    expected_sid: Uuid,
) -> Option<Box<dyn SessionIdSource>> {
    backend_for(c).session_id_source(agent_id, child_pid, expected_sid)
}

// ── 입력 인코딩(ADR-0044/0004) ────────────────────────────────────────────────

/// 세션 입력(write_input)을 transport 로 보내기 **직전** 인코딩 방식. AgentSession 이 spawn 시
/// 받아 보관하고 write_input 마다 적용한다.
///
/// ★설계 의도★: transport 는 항상 raw 바이트만 쓴다(바보 파이프 — ADR-0044). "텍스트 턴을
/// claude JSON 라인으로 감싸는" 지식은 backend 소유다. session 은 이 enum(태그)만 들고, 실제
/// 스키마는 [`AgentBackend::wrap_input_turn`] 구현체 안에만 산다(ADR-0004 격리 — 이 모듈도 session 도
/// transport 도 형태를 모른다).
/// backend 가 요구하는 물리 통로 모양 — **신고값**이다. 통로 실물을 만드는 것은
/// [`AgentBackend::open_spawn`] 이고, 그 안에서도 이 값을 다시 match 해 생성자를 고르지 않는다
/// (ADR-0191 — 가르는 switch 는 `backend_for` 하나뿐). 오늘 이 값을 읽는 곳은 선언 표 트립와이어
/// (`tests::expected_codec_axis`)뿐이다.
///
/// ★출력 구조화 여부가 여기 함의돼 있다★: `StdioNdjson` 은 그 파이프가 나르는 바이트가 줄단위 JSON
/// 이라는 뜻이고, 통로 자신은 그것을 모른다(바보 파이프 — ADR-0044). 그래서 그 `structured` output caps
/// 는 하드코딩되지 않고 주입되는데, 주입하는 쪽이 파이프를 고른 backend 자신이다(ADR-0030 분담 유지).
// ADR-0004
// ADR-0044
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportShape {
    /// 터미널(ConPTY/pty) — 바이트 그대로, resize 있음.
    Pty,
    /// stdio 파이프 + 줄단위 JSON 출력 — resize 개념 없음.
    StdioNdjson,
    /// stdio 파이프 + 줄단위 JSON 이 **양방향**으로 흐른다 — resize 개념 없음.
    ///
    /// ★[`TransportShape::StdioNdjson`] 과의 구분★: 그쪽은 우리가 쓴 바이트를 상대가 그대로 읽고
    ///   우리는 상대의 줄을 읽기만 하는 단방향 파이프라, 봉투를 만드는 것은 backend 인코더다
    ///   ([`InputEncoder`]). 이쪽은 상대가 **우리에게도 묻고 답을 기다리는** 프로토콜이라, 나가는 줄의
    ///   id·메서드·순서를 통로가 직접 쥔다 — 그래서 [`AgentTransport::send_input`] 이 받는 바이트는
    ///   와이어 프레임이 아니라 **메시지 본문**이고, 인코더는 그것을 건드리지 않는다
    ///   ([`InputEncoder::TransportFramed`]).
    StdioBidiJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEncoder {
    /// 바이트 그대로 통과(PTY/터미널·shell). 기존 동작과 **바이트 동일**.
    Raw,
    /// claude stream-json: 텍스트 1턴을 user JSON 라인(`\n` 종단)으로 감싼다(스키마 = `backend/claude/`).
    ClaudeStreamJson,
    /// 봉투를 **통로가 만든다** — 인코더는 바이트를 그대로 통과시킨다([`backend_for_encoder`] 가 `None`).
    ///
    /// ★`Raw` 를 재사용하지 않는 이유★: `Raw` 는 "PTY 로 사람 키보드를 흉내내는 채널" 이라
    ///   [`InputEncoder::submit_sequence`] 가 CR 을 내고, 세션이 그 CR 을 **본문과 별개의
    ///   `send_input` 호출**로 한 번 더 낸다([`crate::session::AgentSession::submit_input_observed`]).
    ///   봉투를 스스로 만드는 통로에 그 호출이 닿으면 빈 본문짜리 턴이 하나 더 나간다.
    TransportFramed,
}

impl InputEncoder {
    /// `msg_uuid`: 이 유저 턴의 메시지 uuid(replay dedup 키). ClaudeStreamJson 은 stdin user 라인에
    ///   심어 claude 가 replay 시 그대로 되울리게 한다(uuid dedup 계약 = `backend/claude/`).
    ///   같은 write_input 이 이 uuid 를 input_echo_event 에도 넘겨 합성 에코와 replay 를 uuid 로 합친다.
    ///   Raw(터미널·shell)는 uuid 를 쓰지 않는다(무시) — 바이트 동일 보장 유지.
    pub fn encode(&self, bytes: &[u8], msg_uuid: Uuid) -> Vec<u8> {
        match backend_for_encoder(*self) {
            None => bytes.to_vec(),
            // ※from_utf8_lossy(FIX 6b): 비-UTF8 입력은 U+FFFD 로 치환돼 손상될 수 있으나, 구조화 입력은
            //   텍스트 챗 메시지라 UTF-8 이 전제다(MVP=텍스트 챗, ADR-0044) → 허용.
            Some(b) => b.wrap_input_turn(&String::from_utf8_lossy(bytes), msg_uuid),
        }
    }

    /// 입력 성공 직후 세션 층이 core.emit 할 **입력-시점 유저 에코 이벤트**를 만든다(ADR-0044/0045).
    ///
    /// ★왜 여기(backend) 인가★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만, json 모드는 claude
    ///   가 되울릴 때까지 화면에 안 뜬다. 그 왕복 지연을 없애려 write_input 직후 합성 유저 이벤트를
    ///   emit 한다. 어떤 encoder 가 이 에코가 필요한지·이벤트의 json 스키마가 뭔지는 backend 지식이라
    ///   session 이 아니라 여기서 판정한다(ADR-0004 — session 은 encoder 태그만 들고 형태를 모른다).
    ///   Raw(터미널)는 None 을 돌려줘 세션이 아무 것도 emit 하지 않는다(PTY 가 이미 에코 — 중복 방지).
    ///
    /// ★decoder uuid dedup 과 짝(blunt-suppress → uuid dedup 교체)★: 이 이벤트는 decoder 가 replay 된
    ///   user-role 블록에 대해 만드는 것과 동일 shape(`Structured{kind:"user", json:{"type":"text",
    ///   "text":<raw>,"uuid":"X"}}`)이다. `msg_uuid` 가 stdin(encode)에 심은 값과 같아, 이후 claude 가
    ///   되울린 replay(같은 uuid)를 프론트 accumulator 가 uuid 로 dedup 해 한 개로 합친다. 예전엔 decoder 가
    ///   user text 블록을 blunt 억제해 이 합성 에코가 "자리 대체"했으나, resume 시 과거 대화가 사라지는
    ///   버그라 uuid dedup 으로 바꿨다(과거/비매칭 uuid user text 는 전부 보존).
    pub fn input_echo_event(
        &self,
        bytes: &[u8],
        msg_uuid: Uuid,
    ) -> Option<crate::types::OutputEvent> {
        backend_for_encoder(*self)?.input_echo_event(&String::from_utf8_lossy(bytes), msg_uuid)
    }

    /// 인코딩된 본문 뒤에 **별도 write** 로 한 번 더 내보내야 하는 제출(턴 시작) 바이트.
    /// `None` = 본문 write 하나가 이미 제출이다.
    ///
    /// ★왜 `encode` 안에서 붙일 수 없나(실측 2026-08-17 — 되살리지 마라)★: 살아 있는 claude TUI 세션에
    ///   `본문 + CR` 을 **한 번의 write** 로 넣으면 텍스트가 입력창에 남고 턴이 시작되지 않는다. 제출을
    ///   만드는 건 바이트열이 아니라 **수신자가 그것을 언제 읽느냐**라, 바이트열 하나를 돌려주는 `encode`
    ///   로는 표현할 수 없다 — 그래서 별도 질문이다.
    /// ★write 를 나누는 것만으로는 부족하다★: 나눠 써도 **간격이 없으면** 두 write 가 PTY 에서 한 덩이로
    ///   묶여 수신자가 한 번의 read 로 받고, 결국 합쳐 쓴 것과 같아진다. 그래서 짝이 되는 대기가 필요하고
    ///   그 값이 [`SUBMIT_PACING`] 이다(둘은 항상 같이 간다 — 한쪽만 보고 고치지 말 것).
    /// ★`Raw` = CR★: 터미널·shell 은 PTY 로 사람 키보드를 흉내내는 채널이라 Enter = CR(0x0D) 이다.
    /// ★`ClaudeStreamJson` = None★: `encode` 가 붙이는 종단 `\n` 이 그 프로토콜의 제출이다. 여기에 CR 을
    ///   더하면 라인 경계 뒤에 잉여 바이트가 붙어 페이로드가 오염된다.
    /// ★`TransportFramed` = None★: 본문 바이트 하나가 그대로 한 턴이고, 그것을 봉투에 넣어 제출하는 것은
    ///   통로다. 여기에 값을 두면 세션이 **본문과 별개의 `send_input` 호출**로 그 바이트를 한 번 더 내고,
    ///   통로는 그것을 또 하나의 본문으로 읽어 빈 턴을 연다.
    /// ★키 입력 경로는 이 값을 보지 않는다★: 소비자는 "완성된 메시지 하나 = 턴 하나" 인
    ///   `AgentSession::submit_input_observed` 뿐이다. 사람이 Enter 를 직접 치는 터미널 스트리밍 입력
    ///   (`write_input`)에 제출을 끼워 넣으면 키 한 번마다 턴이 제출된다.
    // ADR-0004
    pub fn submit_sequence(&self) -> Option<&'static [u8]> {
        match self {
            InputEncoder::Raw => Some(b"\r"),
            InputEncoder::ClaudeStreamJson => None,
            InputEncoder::TransportFramed => None,
        }
    }

    /// 이 입력 한 조각이 **사용자 턴을 제출하나** — 세션 id 첫 제출 래치가 세는 기준이다.
    ///
    /// - `ClaudeStreamJson`·`TransportFramed` → 언제나 `true`: 호출 1회 = 완결된 유저 턴 1개다
    ///   (`AgentSession::write_input` 의 FIX 6a 계약 · codex 통로는 쓰기마다 턴 하나).
    /// - `Raw` → 제출 바이트(CR)가 들어 있으면 `true`.
    ///
    /// ★`Raw` 의 CR 판정은 프론트의 키 인코딩에 매인다★: 터미널 Enter = CR 은 xterm 이 **기본** 키
    ///   인코딩으로 보내는 바이트다. kitty 키보드·win32-input-mode 처럼 Enter 를 다른 시퀀스로 보내는
    ///   모드를 켜면 이 판정이 **조용히** 거짓이 되어 제출이 안 세어지고, 그 모드의 이어받기가 전부
    ///   사라진다. 같은 가정에 선 [`InputEncoder::submit_sequence`] 와 짝이다 — 한쪽만 고치지 말 것.
    // ADR-0226
    pub fn submits_turn(&self, bytes: &[u8]) -> bool {
        match self {
            InputEncoder::Raw => bytes.contains(&b'\r'),
            InputEncoder::ClaudeStreamJson | InputEncoder::TransportFramed => true,
        }
    }
}

/// 본문 write 와 제출 write([`InputEncoder::submit_sequence`]) **사이에 두는 대기**.
///
/// ★왜 필요한가(실측 2026-08-17 — 지우지 마라)★: write 를 둘로 나누는 것만으로는 제출되지 않는다. 간격이
///   없으면 두 write 가 PTY 에서 한 덩이로 묶여 claude TUI 가 **한 번의 read** 로 받고, CR 을 붙여넣은
///   텍스트의 일부로 취급해 입력창에 그대로 담아 둔다. 경계를 만드는 건 write 횟수가 아니라 **수신자의
///   read 경계**이고, 그걸 벌리는 수단이 이 대기다.
/// ★"지연 0 으로도 제출된다" 는 옛 관측은 측정 오류였다(되살리지 마라)★: 그 측정은 제어 평면 명령을 **두
///   번 따로** 보낸 것이라 웹소켓 왕복·태스크 전환으로 이미 수 밀리초가 벌어져 있었는데 0ms 로 읽혔다.
///   같은 함수에서 연달아 쓰는 배달 경로에는 그 간격이 없어 실제로 제출되지 않았다(살아 있는 에이전트로
///   재확인).
/// ★값의 출처 = 임의 상수가 아니다(ADR-0038)★: 같은 문제를 푸는 tmux 기반 멀티에이전트 오케스트레이터들이
///   claude Code 를 상대로 쓰는 정착값이 0.5초이고, 사유도 같다("텍스트와 Enter 를 합치거나 너무 붙여
///   보내면 입력 버퍼와 경쟁이 난다"). 근거 없이 줄이거나 늘리지 말 것 — 줄이면 이 결함이 그대로 재발한다.
pub const SUBMIT_PACING: std::time::Duration = std::time::Duration::from_millis(500);

pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    backend_for(c).input_encoder(c)
}

// ── 출력 정제(ADR-0044/0004/0045) — 입력 인코더의 대칭 짝 ──────────────────────────

/// pump→core 앞에 꽂히는 출력 정제 decoder. None = 바이트 직통(터미널·평문 불변).
///
/// ★대칭★: `input_encoder`(입력 방향)의 출력 방향 짝이다. 판정도 decoder 실물도
/// [`AgentBackend::output_decoder`] 가 소유하고 이 함수는 dispatch 뿐이다 — session 은 encoder 태그만,
/// transport 는 `dyn OutputDecoder` 만 안다(ADR-0004). 새 backend 는 자기 폴더에서 그 메서드를 구현하면
/// 되고 이 함수는 손대지 않는다(교체성).
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    backend_for(c).output_decoder(c)
}

// ── ADR-0079: resume 시 `.jsonl` transcript → 과거 이벤트 seed (backend dispatch) ──────

/// ADR-0079: resume 스폰 시 이 명령의 과거 대화를 복원한 `OutputEvent` 목록. 빈 Vec = seed 안 함
/// (기존 fresh 버퍼 동작 불변).
///
/// ★backend 지식 격리(ADR-0004)★: transcript 의 파일 배치·포맷·seed 가부 판정은 전부
///   [`AgentBackend::resume_transcript_events`] 구현체 몫이고 이 함수는 dispatch 뿐이다. manager 도 이
///   dispatch 만 부르고 경로 규칙을 모른다. `output_decoder`(라이브 정제)의 resume 방향 짝.
pub fn resume_transcript_events(
    c: &AgentCommand,
    cwd: &std::path::Path,
    session_id: Uuid,
) -> Vec<crate::types::OutputEvent> {
    backend_for(c).resume_transcript_events(c, cwd, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AgentOutputFormat;

    // ── ADR-0099 트립와이어: 새 AgentCommand variant 배선 시 capability 의식적 선언 강제 ──────
    //
    // ★ 와일드카드를 절대 추가하지 말 것 ★ — 새 AgentCommand variant가 생기면 이 match가
    // 컴파일 에러를 내서 아래 체크리스트를 강제로 방문하게 하는 장치다(목록 rot 방지 —
    // 갱신이 컴파일로 강제되는 목록은 rot하지 않는다).
    //
    // 새 variant 배선 시 체크리스트:
    //   ① 새 백엔드의 두 capability는 stub 복붙 금지 — CLI spike로 실측한 값으로 채울 것.
    //   ② supports_control_channel=false로 연결하면 메시징 없는 단독 에이전트가 된다(의도인지 확인).
    //   ③ 비-MCP(accepts_mcp_config=false)면 provision이 [Cli] grant를 자동 선택하고 **프라이밍은 아예
    //      안 싣는다**(CLI판 사본은 커밋 `2ef6902`에서 삭제됐고 `roundtrip-smoke --cli-only` 노브도 함께
    //      사라졌다 — 되살리려 하지 말 것). 그 갈래는 오늘 실측 수단이 없다 — ADR-0209.
    //   ④ MCP-capable이면 기본 roundtrip으로 실측.
    //   ⑤ 셋째 칸(writes_mcp_config_file)은 **둘째 칸에서 파생하지 말 것** — 「MCP 로 우편을 쓰나」와
    //      「우리가 쓴 mcp-config 파일을 읽나」는 별개다. 그 프로그램에 그 파일을 가리킬 플래그가 실제로
    //      있을 때만 true(claude 의 `--mcp-config`). true 면 스폰마다 평문 토큰 JSON 이 디스크에 쓰이고
    //      그 write 실패가 스폰을 끊는다(fail-closed) — 안 읽는 backend 에 켜면 순수 손실이다.
    //   참조: ADR-0099.
    fn expected_channel_matrix(c: &AgentCommand) -> (bool, bool, bool) {
        // (supports_control_channel, accepts_mcp_config, writes_mcp_config_file) — CLI spike 실측값
        match c {
            AgentCommand::Claude { .. } => (true, true, true),
            AgentCommand::Shell { .. } => (false, false, false),
            // ★이 행의 둘째 칸은 **이름대로 읽으면 틀린 값이다 — 알고 켰다**★: codex 는 우리가 만든
            //   mcp-config 파일을 여전히 못 먹는다(그 파일을 가리킬 플래그가 없다 — 실측 0.155.0).
            //   그런데 데몬이 이 한 칸으로 우편 채널 판정까지 파생해서(`mail_allowed`), false 로 두면
            //   MCP 로 우편을 쓰는 이 백엔드에 **CLI 미러가 열린 채로** 남는다. 사유의 정본은
            //   `backend/codex/` 의 그 메서드 주석이고 여기 되풀어 적지 않는다.
            // ★그래서 셋째 칸이 false 다 — 둘째를 복사하지 말 것★: 안 읽는 파일을 쓰던 대가(스폰마다
            //   평문 토큰 JSON · 그 write 실패로 스폰 중단)는 이 칸이 갈라지면서 사라졌다. 여기를 true 로
            //   되돌리면 그 대가가 그대로 돌아온다.
            // ADR-0209
            AgentCommand::Codex { .. } => (true, true, false),
        }
    }

    #[test]
    fn backend_channel_matrix_is_consciously_declared() {
        let variants: Vec<AgentCommand> = vec![
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];

        for c in &variants {
            let (expected_control, expected_mcp, expected_writes) = expected_channel_matrix(c);
            let actual_control = supports_control_channel(c);
            let actual_mcp = accepts_mcp_config(c);
            let actual_writes = writes_mcp_config_file(c);
            assert_eq!(
                actual_control,
                expected_control,
                "variant {:?}: supports_control_channel 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
            assert_eq!(
                actual_mcp,
                expected_mcp,
                "variant {:?}: accepts_mcp_config 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
            assert_eq!(
                actual_writes,
                expected_writes,
                "variant {:?}: writes_mcp_config_file 불일치 — 이 칸이 true 면 스폰마다 평문 토큰 JSON 이 디스크에 쓰이고 그 write 실패가 스폰을 끊는다. 위 체크리스트 ⑤ 를 따라 의식적으로 선언할 것(ADR-0209)",
                c
            );
        }
    }

    // ── ADR-0113/0004: 턴 신호 분류자 dispatch(매핑은 백엔드 소유, 어휘는 공용) ──────────────

    #[test]
    fn turn_classifier_dispatch_maps_claude_events_and_silences_shell() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let delta = OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let done = OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        };
        let claude_classify = turn_classifier(&json);
        assert_eq!(claude_classify(&delta), Some(TurnSignal::Progress));
        assert_eq!(claude_classify(&done), Some(TurnSignal::Ended));
        // ★`Structured` 해석이 백엔드별인 이유의 회귀★: claude 는 입력 시점 유저 에코를 여기 싣기에
        //   진행으로 세지만, 매핑을 선언하지 않은 backend 는 같은 이벤트에 침묵해야 한다 — 안 그러면
        //   턴과 무관한 구조화 메타 라인이 종료 신호 없는 영구 "턴 중" 을 만든다.
        let meta = OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        };
        assert_eq!(claude_classify(&meta), Some(TurnSignal::Progress));
        let shell_classify = turn_classifier(&shell);
        for ev in [&delta, &done, &meta] {
            assert_eq!(
                shell_classify(ev),
                None,
                "매핑 미선언 backend 는 어떤 이벤트에도 신호를 내지 않는다(기본 = 침묵)"
            );
        }
    }

    // ── ADR-0172/0004: 이어받기 실패 분류 dispatch(문구 지식은 백엔드 소유) ──────────────────

    #[test]
    fn resume_failure_dispatch_reads_claudes_marker_and_stays_silent_elsewhere() {
        let claude = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let tail = "\u{1b}[?25hNo conversation found with session ID: 8b1c…\r\n";
        assert_eq!(
            resume_failure_kind(&claude, tail),
            Some(AgentFailureKind::NoConversationToResume)
        );
        assert_eq!(
            resume_failure_kind(&claude, "NO CONVERSATION FOUND with session ID: x"),
            Some(AgentFailureKind::NoConversationToResume),
            "대소문자는 외부 프로그램이 정하므로 우리 판정이 거기 매달리면 안 된다"
        );
        assert_eq!(
            resume_failure_kind(&claude, ""),
            None,
            "꼬리가 비면 단정하지 않는다 — 구조화 세션은 진단을 stderr 로 흘려 여기 안 온다"
        );
        assert_eq!(
            resume_failure_kind(&claude, "Error: EPERM"),
            None,
            "모르는 문구는 단정하지 않는다(호출자가 맥락 기본값으로 떨어뜨린다)"
        );
        assert_eq!(
            resume_failure_kind(&shell, tail),
            None,
            "선언하지 않은 backend 는 남의 문구를 읽지 않는다"
        );
    }

    #[test]
    fn input_encoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert_eq!(input_encoder(&term), InputEncoder::Raw);
        assert_eq!(input_encoder(&shell), InputEncoder::Raw);
        assert_eq!(input_encoder(&json), InputEncoder::ClaudeStreamJson);
    }

    #[test]
    fn raw_encoder_is_byte_identical() {
        let input = b"echo hi\r\n\x1b[A\x03";
        assert_eq!(
            InputEncoder::Raw.encode(input, Uuid::new_v4()),
            input.to_vec()
        );
    }

    // ── 우편 자격 트립와이어: 새 variant 는 분류를 **의식적으로** 선언해야 한다 ──────────────────
    //
    // ★와일드카드를 추가하지 말 것★ — 기본값이 true(fail-open)라 아무 선언 없이도 수신자가 되는데,
    // 이 축이 막는 건 **LLM 자유 텍스트가 명령으로 실행되는 것**이라 조용한 상속은 안전 문제가 된다.
    //
    // ★목록 누락도 잡힌다(그래서 슬롯을 쓴다)★: 손으로 채우는 Vec 하나였을 땐 새 variant 를 거기 넣는 걸
    // 잊으면 `reads_messages` 가 한 번도 안 불려 테스트가 초록인 채 fail-open 이 통과했다. 새 variant 를
    // 추가하면 ① `variant_slot` 의 match 가 컴파일 에러 → ② 슬롯 번호를 늘리면 `BACKEND_VARIANTS` 와
    // 샘플 배열 길이가 안 맞아 다시 컴파일 에러 → ③ 그래도 샘플을 안 채우면 아래 "슬롯 전부 채움" 단언이
    // 깨진다. 세 관문 중 하나는 반드시 걸린다.
    //
    // 분류 기준: 입력을 **읽고 해석하는** 에이전트(CLI 코딩 에이전트 등)면 true, 입력을 **실행**하는
    //   채널(셸·REPL 류)이면 false. 판단이 서지 않으면 false 로 두고 사용자에게 올린다.
    const BACKEND_VARIANTS: usize = 3;

    fn variant_slot(c: &AgentCommand) -> usize {
        match c {
            AgentCommand::Claude { .. } => 0,
            AgentCommand::Shell { .. } => 1,
            AgentCommand::Codex { .. } => 2,
        }
    }

    fn expected_reads_messages(c: &AgentCommand) -> bool {
        match c {
            AgentCommand::Claude { .. } => true,
            AgentCommand::Shell { .. } => false,
            // ★한때 false 였고 그 사유는 「턴을 관측할 수 없어 바쁜 때를 못 가린다」였다 — 그 전제를
            //   ADR-0116 결정 7 이 기각했다★(「관측할 수 없으니 배달할 수 없다」 = 그 ADR 의 거부한
            //   대안). 터미널 claude 가 같은 자리에서 이미 받으므로 이 백엔드만 뺄 근거가 없고, 위
            //   분류 기준으로 재면 codex 는 입력을 **읽고 해석하는** 쪽이라 true 다.
            //   사유의 정본은 `backend/codex/` 의 그 메서드 doc.
            AgentCommand::Codex { .. } => true,
        }
    }

    /// 아래 트립와이어들이 전부 도는 표본 목록.
    ///
    /// ★이 목록에서 한 줄을 지우면 그 **모드가 그 트립와이어들에서 통째로 빠진 채 전부 초록이 된다**★
    ///   (실측 — codex app-server 줄을 지우고 돌려 확인했다). variant 슬롯 커버리지는 모드를 안 세므로
    ///   그 침묵을 못 잡는다. 그래서 목록 자체를 재는 관문을 따로 뒀다:
    ///   [`the_sample_list_covers_every_mode_of_every_backend`].
    fn mail_eligibility_samples() -> Vec<AgentCommand> {
        let per_variant: [AgentCommand; BACKEND_VARIANTS] = [
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];
        let mut all = per_variant.to_vec();
        all.push(AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        all.push(AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        all
    }

    /// 표본 목록을 지키는 관문. ★그 트립와이어들이 재는 것은 전부 이 목록이 닿은 것뿐이라, 목록에 구멍이
    /// 나면 그 구멍은 **아무 데서도 안 보인다**★ — 전부 초록인 채로 그 모드를 한 번도 안 묻는다.
    ///
    /// ★슬롯이 variant 가 아니라 **(variant × 출력 모드)** 인 것이 요점이다★: variant 슬롯은 모드를 안
    /// 세므로 둘째 모드가 빠져도 채워진 것으로 보인다. 새 variant 든 새 [`AgentOutputFormat`] 값이든
    /// 늘면 아래 match 가 컴파일 에러를 낸다.
    const BACKEND_MODES: usize = 5;

    fn mode_slot(c: &AgentCommand) -> usize {
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => 0,
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => 1,
            AgentCommand::Shell { .. } => 2,
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => 3,
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => 4,
        }
    }

    #[test]
    fn the_sample_list_covers_every_mode_of_every_backend() {
        let mut covered = [false; BACKEND_MODES];
        for c in &mail_eligibility_samples() {
            covered[mode_slot(c)] = true;
        }
        assert!(
            covered.iter().all(|c| *c),
            "표본 목록이 안 닿은 모드가 있다 — 그 모드는 아래 트립와이어 어디에도 안 걸린 채 전부 초록이 된다: {covered:?}"
        );
    }

    #[test]
    fn mail_eligibility_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            assert_eq!(
                reads_messages(c),
                expected_reads_messages(c),
                "variant {c:?}: 우편 자격 불일치 — 위 expected_reads_messages 의 분류 기준을 따라 의식적으로 선언할 것"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "샘플이 안 닿은 variant 가 있다(그 variant 는 reads_messages 가 한 번도 안 불려 fail-open 이 그대로 통과한다): {covered:?}"
        );
    }

    // ── 세션 두 축 트립와이어: 발급 축과 이어받기 축을 모드마다 의식적으로 선언한다 ────────────────────
    //
    // ★와일드카드를 추가하지 말 것★ — 두 축 다 trait 에 기본값이 없어 **새 backend** 는 값을 적을 수밖에
    // 없지만, **새 출력 모드**는 컴파일러가 못 본다(한 impl 이 그 백엔드의 모든 모드를 받는다). 그 자리를
    // 세우는 것이 아래 모드별 match 다.
    //
    // 두 축이 서로 다른 것을 묻는다 — 한 칸으로 접지 말 것:
    //   ① assigns_session_id — **우리가** id 를 뽑아 spawn 때 건네주나. true 면 manager 가
    //      발급해 프로필에 영속한다. 자기 id 를 스스로 발급하는 프로그램에 켜면 그 프로그램이 한 번도
    //      쓰지 않을 uuid 가 심기고, 그 뒤 이어받기 판정이 그 가짜 값을 보고 선다.
    //   ② can_resume_stored_session — 저장된 그 id 로 이어받을 수 있나. **발급 주체는 안 묻는다**.
    // ★두 칸을 한 칸으로 접지 말 것★: 접는 순간 한쪽을 고치면 다른 쪽이 딸려 가고, 그 둘이 서로 다른
    // 소비자(발급 = spawn 시점 · 이어받기 = 활성화 입구 셋)를 굴린다. 값이 실제로 갈리는 행이 이미 있다 —
    // codex 두 모드가 다 (false, true) 다: 발급은 codex 가 하고(우리가 심을 값이 없다), 우리는 받아 적은 그
    // 값으로 이어받는다. ★터미널 모드가 (false, false) 로 적혀 있는 자리를 만나면 낡은 것이다★ —
    // 그 모드는 `codex resume <id>` 로 이어받고(ADR-0208), 손잡이는 훅이 적어 준다.
    // ADR-0185
    fn expected_session_axes(c: &AgentCommand) -> (bool, bool) {
        // (assigns_session_id, can_resume_stored_session)
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (true, true),
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (true, true),
            AgentCommand::Shell { .. } => (false, false),
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (false, true),
            // ★이 행이 두 칸의 값이 갈리는 첫 행이다★ — 발급은 여전히 codex 가 하고(①), 우리는 그
            //   받아 적은 thread id 로 `thread/resume` 을 낸다(②). 사유의 정본은 `backend/codex/` 의 그
            //   두 메서드 주석.
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (false, true),
        }
    }

    /// 표본 목록의 구멍은 [`the_sample_list_covers_every_mode_of_every_backend`] 가 잰다 — 여기서 슬롯
    /// 커버리지를 다시 세지 않는다(같은 목록·같은 슬롯이라 같은 답만 두 번 나온다).
    #[test]
    fn session_axes_are_consciously_declared_for_every_mode() {
        for c in &mail_eligibility_samples() {
            let (expected_assign, expected_resume) = expected_session_axes(c);
            assert_eq!(
                assigns_session_id(c),
                expected_assign,
                "{c:?}: 발급 축 불일치 — 위 expected_session_axes 의 ①을 따라 의식적으로 선언할 것"
            );
            assert_eq!(
                can_resume_stored_session(c),
                expected_resume,
                "{c:?}: 이어받기 축 불일치 — 위 expected_session_axes 의 ②를 따라 의식적으로 선언할 것"
            );
        }
    }

    /// 활성화 입구 셋이 함께 부르는 판정의 회귀망 — ★저장된 sid 가 있다는 것만으로 이어받을 수 있는 게
    /// 아니다★. 두 항을 다 재므로, 어느 입구가 이 함수 대신 `sid.is_some()` 만 보도록 되돌아가면 그 입구의
    /// 테스트가 이것과 함께 깨진다.
    #[test]
    fn a_stored_sid_alone_does_not_make_a_profile_resumable() {
        let sid = Some(Uuid::new_v4());
        let profile = |c: AgentCommand, s: Option<Uuid>| {
            let mut p =
                AgentProfile::new("t".into(), c, std::path::PathBuf::from("."), vec![], false);
            p.backend_session_id = s;
            p
        };

        // ★축이 꺼진 표본은 이제 셸뿐이다★ — codex 터미널 모드가 이 자리를 떠났다(ADR-0208 로 그 모드도
        //   `codex resume <id>` 로 이어받게 됐다). 셸에는 재개 개념 자체가 없어 이 자리가 비지 않는다.
        let not_resumable = profile(
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            sid,
        );
        assert!(
            !can_resume_profile(&not_resumable),
            "이어받기 축이 false 인 명령은 sid 가 있어도 이어받지 않는다 — 켜면 새 대화가 「이어받음」으로 보고된다"
        );

        // ★codex 는 **두 통로 다** 이어받는다 — 수단만 갈린다★: app-server 는 `thread/resume`,
        //   터미널은 `codex resume <id>`. 둘을 다 재는 것이 요점이다 — 한쪽만 재면 다른 쪽이 조용히
        //   꺼졌을 때 「이어받는 스폰이 새 대화로 보고된다」가 여기서 안 보인다.
        for output_format in [AgentOutputFormat::StreamJson, AgentOutputFormat::Terminal] {
            let resumable_codex = profile(
                AgentCommand::Codex {
                    extra_args: vec![],
                    output_format: output_format.clone(),
                },
                sid,
            );
            assert!(
                can_resume_profile(&resumable_codex),
                "{output_format:?}: codex 는 받아 적은 id 로 이어받는다 — 이 칸이 꺼지면 이어받는 스폰이 「새 대화」로 보고된다"
            );
        }

        let resumable = profile(
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            sid,
        );
        assert!(can_resume_profile(&resumable));

        let no_sid = profile(
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            None,
        );
        assert!(
            !can_resume_profile(&no_sid),
            "축이 true 여도 이어받을 값이 없으면 Fresh 다"
        );
    }

    // ── 코덱 축 트립와이어: 새 variant·새 출력 모드는 입출력 코덱을 의식적으로 선언해야 한다 ──────────
    //
    // ★와일드카드를 추가하지 말 것★ — 이 축의 기본값(`Raw` · decoder 없음)은 둘 다 fail-open 이라
    // 아무 선언 없이도 컴파일되고 조용히 초록이 된다. 그런데 구조화 stdin 을 요구하는 프로그램에 `Raw`
    // 가 물리면 그 에이전트는 **입력을 통째로 못 읽고**, 런타임엔 아무 신호도 안 난다. `AgentCommand`
    // variant 든 `AgentOutputFormat` 값이든 늘어나면 아래 match 가 컴파일 에러를 낸다.
    //
    // 채우는 법 — CLI spike 실측값으로:
    //   ① input_encoder — 그 프로그램이 stdin 을 무엇으로 읽나(감쌀 게 없으면 `Raw`)
    //   ② output_decoder 유무 — 출력이 구조화라 정제가 필요한가
    //   ③ transport_shape — 그 출력을 받으려면 터미널이어야 하나 파이프여야 하나
    //
    // ★③ 은 ①②와 붙어 다니지만 같은 축이 아니다★: 통로 모양은 잘못 골라도 컴파일되고 **런타임에도
    // 그럴싸하게 뜬다** — TUI 를 파이프로 받으면 화면이 깨지고, 줄단위 JSON 을 PTY 로 받으면 이스케이프가
    // 섞인다. 기본값(`Pty`)이 fail-open 이라 여기 열이 없으면 새 backend 가 조용히 그것을 물려받는다.
    //
    // ★이 표가 덮지 않는 것 = resume transcript seed★: `resume_transcript_events` 도 같은 fail-open
    //   기본값(빈 Vec)을 갖지만, 판정하려면 실제 transcript 파일이 있어야 해서 "seed 안 함" 과 "파일이
    //   없어서 빈 Vec" 이 여기서 구별되지 않는다. 그쪽 회귀망은 `backend/claude/` 의 transcript 단위
    //   테스트가 진다 — 여기 세 번째 열을 만들면 아무 것도 재지 않는 열이 하나 생길 뿐이다.
    fn expected_codec_axis(c: &AgentCommand) -> (InputEncoder, bool, TransportShape) {
        // (input_encoder, output_decoder 유무, transport_shape)
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (InputEncoder::Raw, false, TransportShape::Pty),
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (
                InputEncoder::ClaudeStreamJson,
                true,
                TransportShape::StdioNdjson,
            ),
            AgentCommand::Shell { .. } => (InputEncoder::Raw, false, TransportShape::Pty),
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (InputEncoder::Raw, false, TransportShape::Pty),
            // ★app-server 모드의 인코더가 `Raw` 가 아닌 것이 이 행의 요점이다★ — `Raw` 였다면 세션이
            //   본문 뒤에 CR 을 **별도 호출**로 한 번 더 내고, 봉투를 스스로 만드는 통로는 그것을 또 하나의
            //   본문으로 읽어 빈 턴을 연다.
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (
                InputEncoder::TransportFramed,
                true,
                TransportShape::StdioBidiJson,
            ),
        }
    }

    #[test]
    fn codec_axis_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            let (expected_encoder, expects_decoder, expected_shape) = expected_codec_axis(c);
            assert_eq!(
                input_encoder(c),
                expected_encoder,
                "variant {c:?}: input_encoder 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
            assert_eq!(
                output_decoder(c).is_some(),
                expects_decoder,
                "variant {c:?}: output_decoder 유무 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
            assert_eq!(
                transport_shape(c),
                expected_shape,
                "variant {c:?}: transport_shape 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "샘플이 안 닿은 variant 가 있다(그 variant 는 코덱 축이 한 번도 안 불려 fail-open 이 그대로 통과한다): {covered:?}"
        );
    }

    // ── 선언한 통로 모양과 실제로 넘어오는 통로 ────────────────────────────────────────────
    //
    // ★왜 위 코덱 축 표와 따로 재나(ADR-0191 이후)★: 조립점이 `transport_shape` 를 match 해 생성자를
    //   고르던 시절엔 그 표의 셋째 열이 **실제로 뜨는 통로**까지 전이적으로 단언했다. 통로 생성이
    //   backend 의 `open_spawn` 으로 들어가면서 그 신고값을 읽는 생산 코드가 0 이 됐고, 그때부터
    //   파이프를 선언해 놓고 기본 `open_spawn`(=PTY)을 타도 그 열은 초록이다. 런타임 증상은 깨진
    //   화면뿐이라 지금 그 어긋남을 잡는 것은 이 테스트뿐이다.
    // ★`structured` 로 가르지 않는 이유★: 그 칸은 통로가 아니라 backend 가 주입하는 값이라
    //   (ADR-0030/0044) 평문 파이프도 false 를 신고한다 — 두 구현체를 실제로 가르는 것은 양쪽이
    //   하드코딩하는 `terminal_bytes` 와 `resize` 다.
    // ★슬롯 커버리지 단언을 두지 않았다★: 같은 샘플 목록을 도는 위 트립와이어들이 이미 잰다.
    // ★spec 은 그 백엔드의 실 CLI 가 아니다★: `open_spawn` 은 무엇을 띄울지를 spec 에서, 어떤 통로로
    //   띄울지를 command 에서 따로 받으므로, 즉시 끝나는 프로브를 띄워 통로 선택만 본다(ADR-0012).
    // ADR-0191
    #[cfg(windows)]
    #[test]
    fn declared_transport_shape_matches_the_transport_handed_over() {
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo shape-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        for c in &mail_eligibility_samples() {
            let shape = transport_shape(c);
            let expected = match shape {
                TransportShape::Pty => (true, true),
                TransportShape::StdioNdjson => (false, false),
                // 파이프라 터미널 바이트도 크기도 없다 — 단방향 파이프와 같은 짝이다. 둘을 가르는
                //   `interrupt` 는 이 쌍에 안 들어 있는데, 그 칸은 PTY 도 true 라 통로를 못 가른다.
                TransportShape::StdioBidiJson => (false, false),
            };
            let parts = open_spawn(c, &probe, 80, 24, None, None, None, None).expect("open_spawn");
            let caps = parts.transport.capabilities();
            let actual = (caps.output.terminal_bytes, caps.control.resize);
            parts.transport.shutdown();
            assert_eq!(
                actual, expected,
                "variant {c:?}: {shape:?} 를 신고했는데 open_spawn 이 넘긴 통로의 (terminal_bytes, resize) 가 다르다 — 신고와 통로 생성 중 한쪽만 고친 것이다"
            );
        }
    }

    // ── 제출 바이트(submit_sequence) — 백엔드별 "본문 write 만으론 턴이 안 시작되나" ──────────────
    #[test]
    fn submit_sequence_is_cr_for_raw_and_absent_for_stream_json() {
        assert_eq!(
            InputEncoder::Raw.submit_sequence(),
            Some(b"\r".as_slice()),
            "터미널·shell 은 PTY 키보드 흉내 — Enter = CR"
        );
        assert_eq!(
            InputEncoder::ClaudeStreamJson.submit_sequence(),
            None,
            "encode 의 종단 \\n 이 이미 제출 — CR 을 더하면 라인 뒤에 잉여 바이트가 붙는다"
        );
    }

    #[test]
    fn submit_sequence_is_not_baked_into_encode() {
        // ★합치면 claude TUI 가 제출하지 않는다(실측)★ — encode 산출물에 제출 바이트가 섞여 들어가면
        //   session 이 두 write 로 나눠도 첫 write 가 이미 오염된 상태다.
        let out = InputEncoder::Raw.encode(b"<message from=\"bob\">hi</message>", Uuid::new_v4());
        assert!(
            !out.contains(&b'\r'),
            "Raw encode 는 제출 바이트를 붙이지 않는다(제출은 write 경계 — session 소관): {out:?}"
        );
    }

    // ── 턴 제출 판정(submits_turn) — 세션 id 첫 제출 래치의 기준(ADR-0226) ──────────────────
    /// ★이름에 가정을 박는다★: 이 판정은 xterm 기본 키 인코딩(Enter = CR)에 선다. Enter 를 다른 시퀀스로
    ///   보내는 키 인코딩 모드가 들어오면 이 항목이 아니라 **판정 자체**를 다시 봐야 한다.
    #[test]
    fn raw_submission_is_cr_under_default_xterm_encoding() {
        assert!(
            !InputEncoder::Raw.submits_turn(b"hel"),
            "CR 없는 키 입력 조각은 턴이 아니다"
        );
        assert!(
            !InputEncoder::Raw.submits_turn(b""),
            "빈 조각은 턴이 아니다"
        );
        assert!(
            !InputEncoder::Raw.submits_turn(b"\n"),
            "LF 는 xterm 기본 키 인코딩의 Enter 가 아니다"
        );
        assert!(InputEncoder::Raw.submits_turn(b"\r"), "Enter 단독");
        assert!(
            InputEncoder::Raw.submits_turn(b"hello\r"),
            "본문 뒤 Enter 가 같은 조각에 실렸다"
        );
        assert!(
            InputEncoder::Raw.submits_turn(b"a\rb"),
            "붙여넣기처럼 CR 이 가운데 든 조각도 제출로 센다"
        );
    }

    #[test]
    fn structured_encoders_always_submit_a_turn() {
        for encoder in [
            InputEncoder::ClaudeStreamJson,
            InputEncoder::TransportFramed,
        ] {
            assert!(
                encoder.submits_turn(b"hello"),
                "{encoder:?}: 호출 1회 = 유저 턴 1개 — CR 이 없어도 턴이다"
            );
            assert!(
                encoder.submits_turn(b""),
                "{encoder:?}: 내용과 무관하게 호출 자체가 턴이다"
            );
        }
    }

    /// ★제출 바이트를 내는 인코더는 그 바이트를 턴으로 센다★ — 우편 배달은 제출 바이트 앞에서
    ///   `submits_turn(제출 바이트)` 로 세므로, 한쪽(예: `Raw` 의 제출 바이트)만 바뀌면 우편이 **조용히**
    ///   안 세어지고 그 에이전트의 이어받기가 사라진다.
    // ADR-0226
    #[test]
    fn every_submit_sequence_counts_as_a_turn_submission() {
        // 변형이 늘면 이 match 가 컴파일을 깬다 — 아래 목록에 더하라는 신호다.
        let _exhaustive = |e: InputEncoder| match e {
            InputEncoder::Raw | InputEncoder::ClaudeStreamJson | InputEncoder::TransportFramed => {}
        };
        for encoder in [
            InputEncoder::Raw,
            InputEncoder::ClaudeStreamJson,
            InputEncoder::TransportFramed,
        ] {
            if let Some(submit) = encoder.submit_sequence() {
                assert!(
                    encoder.submits_turn(submit),
                    "{encoder:?}: 제출 바이트 {submit:?} 가 턴 제출로 안 세어진다 — 우편이 영속을 못 부른다"
                );
            }
        }
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 이벤트 dispatch(input_echo_event) — uuid dedup ──────
    #[test]
    fn input_echo_event_json_mode_emits_structured_user_with_uuid() {
        use crate::types::OutputEvent;
        let id = Uuid::new_v4();
        let ev = InputEncoder::ClaudeStreamJson
            .input_echo_event(b"hi there", id)
            .expect("json 모드 → 합성 유저 에코 이벤트");
        match ev {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "hi there");
                assert_eq!(
                    v["uuid"],
                    id.to_string(),
                    "합성 에코에 msg_uuid 부착(dedup 키)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn input_echo_event_raw_is_none() {
        assert!(
            InputEncoder::Raw
                .input_echo_event(b"echo hi\r\n", Uuid::new_v4())
                .is_none(),
            "Raw 는 합성 유저 에코를 만들지 않아야 함(PTY 에코 중복 방지)"
        );
    }

    #[test]
    fn claude_stream_json_encoder_wraps_and_terminates() {
        let id = Uuid::new_v4();
        let out = InputEncoder::ClaudeStreamJson.encode(b"hi", id);
        assert_eq!(*out.last().unwrap(), b'\n');
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"text\":\"hi\""));
        assert!(
            s.contains(&id.to_string()),
            "stdin user 라인에 msg_uuid 포함"
        );
    }

    // ── S15 B3: output_decoder dispatch(입력 encoder 의 대칭) ──────────────────────
    #[test]
    fn output_decoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert!(
            output_decoder(&term).is_none(),
            "터미널 모드 → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&shell).is_none(),
            "shell → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&json).is_some(),
            "json 모드 → ClaudeStreamDecoder 주입"
        );
    }

    #[test]
    fn output_decoder_produces_structured_events_through_trait_object() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let mut dec = output_decoder(&json).expect("json → decoder");
        let mut ev = dec.decode(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        ev.extend(dec.flush());
        assert!(
            ev.iter()
                .any(|e| matches!(e, OutputEvent::MessageDone { .. })),
            "trait object decode 가 result 라인을 MessageDone 으로 정제해야 함: {ev:?}"
        );
    }
}
