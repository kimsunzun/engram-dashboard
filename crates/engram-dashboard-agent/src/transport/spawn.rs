//! 통로가 자식을 띄우는 자리 — 세 통로(stdio · pty · codex)가 함께 쓰는 띄우기 실패 가드 [`ChildGuard`] 와 무리
//! 넣기 [`new_group_with`], 파이프 통로 둘(stdio · codex)의 띄우기 [`spawn_piped`] 와 stderr 비우기 [`drain_stderr`].
//!
//! ★띄운 뒤 통로가 서기 전의 실패 경로는 무리와 가드가 정리한다★ — 띄운 바로 다음에 가드를 세우고, 띄운 뒤 첫 실패
//!   가능 걸음이 무리 넣기다. 넣은 뒤의 `?` 하나가 돌아가면 무리가 넣은 자식과 그 뒤에 뜬 후손을, 가드가 직속 자식을
//!   끝낸다(순서 = [`new_group_with`] doc). 둘 다 이 띄우기 **실패** 경로만 다룬다: 통로가 선 뒤의 끄기는 통로
//!   `shutdown()` 의 kill 인과(ADR-0001) 그대로이고, 성공 경로에서 가드는 아무것도 끄지 않고 풀린다. 그 배치는 소스
//!   시험 `the_guard_is_armed_and_the_child_adopted_first_after_spawn` 이 잰다 — 실패를 주입할 이음매가 없는 자리다.
//! ★넣기 전에 뜬 후손은 어느 경로에서도 안 끝난다 — 이 모듈은 닫지 않는다★: 셋 다 깨운 채 띄운 뒤 무리에 넣으므로,
//!   넣기 전에 직속 자식(`cmd.exe`)이 이미 띄운 손자는 무리 밖이다(platform `group` 의 `adopt` doc). 성공 경로에서는
//!   통로를 닫아도(`KILL_ON_JOB_CLOSE`) 안 끝나고, 실패 경로에서는 가드가 직속 자식만 끈다. 무리 넣기 자체가 실패해도
//!   같다. 닫는 법 = 멈춘 채 띄우기(stdio · codex 만 — portable-pty 엔 그 길이 없다)이고 범위 밖이다(TRD
//!   `docs/process/S21-crate-boundaries/trd-A-data-file-unification.md` §3-7 · `docs/tracking.md` T-53).
// ADR-0291

use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;
use engram_dashboard_platform::group::GroupOwner;
use engram_dashboard_platform::spawn::hide_console_window;

use crate::output_core::OutputCore;
use crate::types::{CommandSpec, PtyError};

/// [`ChildGuard`] 가 쥘 수 있는 자식 — std `Child` 와 portable-pty 자식을 같은 규칙으로 끄고 거두게 한다.
///
/// ★구현은 거둘 때까지 자식의 프로세스 핸들을 쥐어야 한다★ — [`new_group_with`] 가 가드의 번호로 자식을 여는데,
///   핸들을 쥐는 동안에는 그 번호가 다른 프로세스로 넘어가지 않는다. 둘 다 버려질 때 핸들을 닫는다.
pub(crate) trait Reap {
    /// `None` = 번호를 모른다 — 무리에 넣지 못한다.
    fn pid(&self) -> Option<u32>;
    /// 직속 자식에게 끝내기를 건다 — 기다리지 않는다. 이미 끝난 자식이면 `Ok` 다. ★Windows 에서는 `Ok` 가 끝내기가
    /// 먹었다는 뜻이 아니다★(두 구현의 doc).
    fn kill_direct(&mut self) -> io::Result<()>;
    /// 끝났으면 거두고 `true` · 아직 돌면 `false` — 기다리지 않는다. `Err` = 끝났는지 물을 수 없다.
    fn try_reap(&mut self) -> io::Result<bool>;
}

impl Reap for Child {
    fn pid(&self) -> Option<u32> {
        Some(self.id())
    }

    /// std 의 Windows `kill` 은 `TerminateProcess` 가 접근 거부로 실패해도 그 자식이 아직 돌면(`try_wait` 가 `Err` 가
    /// 아니면) `Ok` 다.
    fn kill_direct(&mut self) -> io::Result<()> {
        self.kill()
    }

    fn try_reap(&mut self) -> io::Result<bool> {
        Ok(self.try_wait()?.is_some())
    }
}

impl Reap for Box<dyn portable_pty::Child + Send + Sync> {
    fn pid(&self) -> Option<u32> {
        self.process_id()
    }

    /// portable-pty 0.8.1 의 Windows `kill` 은 `TerminateProcess` 의 결과를 버리고 늘 `Ok` 다.
    fn kill_direct(&mut self) -> io::Result<()> {
        self.kill()
    }

    fn try_reap(&mut self) -> io::Result<bool> {
        Ok(self.try_wait()?.is_some())
    }
}

/// 가드가 끈 직속 자식을 거두려고 기다리는 상한 — 넘기면 warn 하고 핸들을 놓는다.
///
/// ★상한이 있어야 한다★ — Windows 에서는 [`Reap::kill_direct`] 의 `Ok` 가 끝내기가 먹었다는 뜻이 아니어서, 시한 없이
///   기다리면 끄기가 실제로 실패한 자식이 스스로 끝날 때까지 띄우기를 부른 쪽이 선다. 2 초는 재지 않은 여유값이다.
const REAP_BUDGET: Duration = Duration::from_secs(2);
const REAP_POLL: Duration = Duration::from_millis(10);

/// 띄운 자식을 통로가 넘겨받기 전까지 쥐는 가드 — 풀지 않고 버려지면 직속 자식을 끄고 [`REAP_BUDGET`] 까지 기다려
/// 거둔다.
///
/// ★`Child` 는 drop 으로 자식을 끄지 않는다★ — 띄운 바로 다음 줄에서 이것으로 감싸고, 통로를 세울 마지막 실패 가능
///   걸음 뒤에 [`Self::into_inner`] 로 푼다. 그 뒤의 끄기는 통로 몫이다.
/// ★끄는 것은 직속 자식뿐이다 — 후손은 무리가 끝낸다★: Windows 의 claude · codex 는 backend 가 platform
///   `console_command` 로 감싼 `cmd.exe /c <CLI>` 로 떠 실제 CLI 가 손자다. 가드를 세운 바로 다음 걸음이
///   [`new_group_with`] 라, 넣은 뒤의 실패에서는 무리가 넣은 뒤에 뜬 후손까지 끝내고 이 가드는 직속 자식을 다시 끈다
///   (이미 끝났으면 무해). 넣기 전에 뜬 후손 · 넣기 자체가 실패한 경우 · 무리가 없는 OS(Windows 밖)는 직속 자식만
///   끝난다(모듈 doc).
/// ★번호로 후손을 찾아 끄지 말 것★ — `taskkill /T`(platform `process::kill_tree`)는 부모 번호만으로 나무를 엮고 시작
///   시각을 안 봐, 죽은 부모의 번호를 단 프로세스가 흔한 이 PC 에서(실측 2026-10-10 · U7 리뷰) 그 번호를 다시 받은
///   `cmd.exe` 아래로 남의 나무까지 끈다. 신원 목록을 찍고(platform `process::subtree`) 하나씩 시작시각을 다시 읽어
///   끄는 길도 찍은 뒤 읽기 전에 번호가 넘어가면 남을 끈다(ADR-0291 「거부한 대안」).
// ADR-0291
pub(crate) struct ChildGuard<C: Reap> {
    child: Option<C>,
}

impl<C: Reap> ChildGuard<C> {
    pub(crate) fn new(child: C) -> Self {
        Self { child: Some(child) }
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Reap::pid)
    }

    pub(crate) fn child_mut(&mut self) -> &mut C {
        self.child
            .as_mut()
            .expect("가드는 풀리기 전까지 자식을 쥔다")
    }

    /// 가드를 풀고 자식을 넘긴다 — 이 뒤로는 가드가 아무것도 끄지 않는다.
    pub(crate) fn into_inner(mut self) -> C {
        self.child.take().expect("가드는 한 번만 풀린다")
    }
}

impl<C: Reap> Drop for ChildGuard<C> {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Err(e) = child.kill_direct() {
            tracing::warn!(
                pid = ?child.pid(),
                "띄우기 실패 정리: 직속 자식을 못 껐다 — 거두지 않고 놓는다: {e}"
            );
            return;
        }
        let deadline = Instant::now() + REAP_BUDGET;
        loop {
            match child.try_reap() {
                Ok(true) => return,
                Ok(false) if Instant::now() < deadline => std::thread::sleep(REAP_POLL),
                Ok(false) => {
                    tracing::warn!(
                        pid = ?child.pid(),
                        budget_ms = REAP_BUDGET.as_millis() as u64,
                        "띄우기 실패 정리: 끈 직속 자식이 시한 안에 안 끝났다 — 거두지 않고 놓는다"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        pid = ?child.pid(),
                        "띄우기 실패 정리: 직속 자식이 끝났는지 물을 수 없다 — 거두지 않고 놓는다: {e}"
                    );
                    return;
                }
            }
        }
    }
}

/// 새 무리를 만들고 `pid` 를 넣는다. `None` = 넣을 번호가 없다(빈 무리만 만든다).
///
/// ★띄운 자식을 쥔 [`ChildGuard`] 를 세운 바로 다음, 띄운 뒤의 **첫** 실패 가능 걸음으로 부른다★ — 넣기가 늦을수록
///   직속 자식이 무리 밖에서 띄우는 후손이 는다(모듈 doc). `pid` = 가드의 번호(가드가 핸들을 쥐어 번호가 안 넘어간다
///   — [`Reap`] doc).
/// 실패 경로의 정리 — 부르는 쪽 지역 변수는 선언 역순으로 버려지고, 가드를 먼저 선언하므로:
///   - 이것 뒤의 걸음이 실패하면 돌려받은 무리 주인이 먼저 버려져 Job 이 닫히며 OS 가 넣은 자식과 넣은 뒤에 뜬 후손을
///     끝내고(`KILL_ON_JOB_CLOSE`), 이어 가드가 직속 자식을 다시 끄고(이미 끝났으면 무해) 거둔다.
///   - 이것이 실패하면 무리가 없다(넣기 실패면 이 안에서 빈 무리가 먼저 버려진다) — 가드가 직속 자식만 끈다.
// ADR-0291
pub(crate) fn new_group_with(pid: Option<u32>) -> Result<GroupOwner, PtyError> {
    let group = GroupOwner::new()?;
    if let Some(pid) = pid {
        group.adopt(pid)?;
    }
    Ok(group)
}

/// [`spawn_piped`] 이 넘기는 것 — 무리에 든 자식과 그 파이프 셋.
///
/// ★여기서부터는 가드가 없다★ — 받는 통로는 실패할 걸음 없이 자기 칸에 옮긴다. 이것을 버리면 Windows 는 무리
///   주인이 함께 버려지며 멤버를 끝내지만, 무리가 없는 OS(Windows 밖)에서는 자식이 남는다.
pub(crate) struct PipedChild {
    pub(crate) child: Child,
    pub(crate) stdin: Option<ChildStdin>,
    pub(crate) stdout: Option<ChildStdout>,
    pub(crate) stderr: Option<ChildStderr>,
    pub(crate) group: GroupOwner,
}

/// `spec` 을 파이프 셋 · 창 없이 띄워 새 무리에 넣는다. 넣기가 실패하면 [`ChildGuard`] 가 직속 자식을 끄고 돌아간다.
/// `what` = 띄우기 실패 문구의 머리(`"{what} spawn: …"`).
///
/// `spec` 을 그대로 띄운다 — Windows shim 감싸기(`cmd.exe /c …`)는 backend 가 platform `console_command` 로 이미 했다.
// ADR-0291
pub(crate) fn spawn_piped(spec: &CommandSpec, what: &str) -> Result<PipedChild, PtyError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    cmd.current_dir(&spec.cwd);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    // 세 파이프 모두 확보 — stdout/stderr를 우리가 읽어야 자식이 파이프 버퍼 full로 블록되지 않는다.
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // 헤드리스 백그라운드 프로세스다 — 데몬은 창 없는 프로세스일 수 있어 cmd.exe shim 이 콘솔을 새로 띄운다.
    hide_console_window(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| PtyError::SpawnFailed(format!("{what} spawn: {e}")))?;
    let mut guard = ChildGuard::new(child);
    let group = new_group_with(guard.pid())?;

    let child = guard.child_mut();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    Ok(PipedChild {
        child: guard.into_inner(),
        stdin,
        stdout,
        stderr,
        group,
    })
}

/// 자식 stderr 를 줄 단위로 끝까지 비우는 스레드를 띄운다 — 줄마다 마스킹해 `core` 의 진단 버퍼에 쌓고 debug 로
/// 흘린다. 스레드를 못 띄우면 삼키지 않고 warn 을 남긴 뒤 돌아간다(FIX 4). `thread_name` = 스레드 이름 · `what` = 그
/// warn 의 머리.
///
/// ★비우지 않으면 자식이 stderr 파이프 버퍼 full 로 블록해 진행이 멈춘다★.
/// ★출력 스트림에 섞지 않는다(ADR-0044)★: 구조화 모드의 stdout 은 줄 단위 JSON 이라 비-JSON 진단 줄이 끼면 파서가
///   깨진다.
/// ★출력 링이 아니라 진단 버퍼에 쌓는다(ADR-0172)★: 구조화 세션에서는 이 텍스트가 활성화 실패의 **유일한 증거**다
///   (claude 의 "No conversation found with session ID: …" 가 여기로만 온다). 계약은 `OutputCore::push_diagnostic`.
///   ★이 스레드가 stderr 를 읽는 유일한 자리다★ — 파이프는 두 번 읽을 수 없으니 두 번째 캡처를 만들지 않는다.
/// ★마스킹은 버퍼에도 그대로 적용한다(FIX 4)★: 외부 프로세스 출력이라 자격증명이 섞일 수 있고, 이 텍스트는 실패
///   사유로 다시 로그에 실릴 수 있다 — 원문을 붙들면 마스킹을 우회하는 경로가 생긴다.
/// ★레벨 = debug(FIX 4)★: claude 는 진행 · 진단 텍스트를 stderr 로 흘리는 것이 정상이다 — warn 이면 로그를 범람시킨다.
// ADR-0044
// ADR-0172
pub(crate) fn drain_stderr(
    stderr: ChildStderr,
    core: &Arc<OutputCore>,
    thread_name: &str,
    what: &str,
) {
    let agent_id = core.id();
    let diag_core = core.clone();
    let spawn_result = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.is_empty() => {
                        let masked = mask_secrets(&l);
                        // ★쌓기가 로그보다 **먼저**다★: 기본 로그 필터는 warn 이라 아래 debug 줄은 평소 버려진다 —
                        //   분류를 그 줄에 매달면 로그 레벨이 기능을 켜고 끈다.
                        diag_core.push_diagnostic(&masked);
                        tracing::debug!(target: "agent_stderr", agent = %agent_id, "{}", masked)
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
    if let Err(e) = spawn_result {
        tracing::warn!(agent = %agent_id, "{what} stderr drain 스레드 기동 실패: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 운영 구획만 — 첫 시험 모듈 앞에서 자르고 줄마다 `//` 뒤를 걷는다. 이 저장소의 주석은 자기가 지키는 이름을 그대로
    /// 인용하므로, 걷지 않으면 주석 한 줄이 실물 호출 행세를 한다.
    fn production(src: &str) -> String {
        src.split("mod tests {")
            .next()
            .expect("운영 구획")
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `code` 에서 `signature` 로 시작하는 함수의 본문 — 첫 `{` 부터 짝 맞는 `}` 까지.
    fn body<'a>(code: &'a str, signature: &str) -> &'a str {
        let start = code
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` 가 없다 — 이 항목의 전제가 낡았다"));
        let open = start + code[start..].find('{').expect("본문 여는 괄호");
        let mut depth = 0usize;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &code[open..=open + i];
                    }
                }
                _ => {}
            }
        }
        panic!("`{signature}` 본문이 닫히지 않는다");
    }

    fn line_at(body: &str, at: usize) -> &str {
        let from = body[..at].rfind('\n').map_or(0, |i| i + 1);
        let to = body[at..].find('\n').map_or(body.len(), |i| at + i);
        body[from..to].trim()
    }

    /// `code` 에서 낱말 `word` 가 선 자리 — 앞뒤가 식별자 글자가 아닌 것만.
    fn word_positions(code: &str, word: &str) -> Vec<usize> {
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        code.match_indices(word)
            .map(|(at, _)| at)
            .filter(|&at| {
                !code[..at].chars().next_back().is_some_and(ident)
                    && !code[at + word.len()..].chars().next().is_some_and(ident)
            })
            .collect()
    }

    /// `?` 말고 함수를 일찍 떠나는 철자 — `return` 과 `let … else` 의 `else`. 둘을 가르는 것은 `else` 바로 앞이다:
    /// `if … else` 는 `} else` 이고, `let … else` 는 초기식이 `}` 로 끝날 수 없다(Rust 문법).
    fn early_exits(code: &str) -> Vec<usize> {
        let mut found = word_positions(code, "return");
        found.extend(
            word_positions(code, "else")
                .into_iter()
                .filter(|&at| !code[..at].trim_end().ends_with('}')),
        );
        found.sort_unstable();
        found
    }

    fn assert_no_early_exit(file: &str, code: &str, what: &str) {
        if let Some(&at) = early_exits(code).first() {
            panic!(
                "{file}: {what}에 `?` 아닌 이른 반환이 있다 — 소스 시험이 실패 걸음으로 못 센다: `{}`",
                line_at(code, at)
            );
        }
    }

    /// `spawned` 의 `?` 뒤에서 — 가드 무장(`ChildGuard::new(`)이 무리 넣기(`new_group_with(`)보다 앞이고, 첫 `?` 가 무리
    /// 넣기의 것이며, 모든 `?` 가 가드 풀기(`.into_inner()`) 앞이다. `?` 말고 일찍 떠나는 철자는 없다.
    fn assert_adoption_is_the_first_fallible_step_after_spawn(
        file: &str,
        body: &str,
        spawned: &str,
    ) {
        let spawn_at = body
            .find(spawned)
            .unwrap_or_else(|| panic!("{file}: `{spawned}` 가 없다 — 이 항목의 전제가 낡았다"));
        let spawn_step = spawn_at + body[spawn_at..].find('?').expect("띄우기의 `?`");
        let armed = body
            .find("ChildGuard::new(")
            .unwrap_or_else(|| panic!("{file}: 가드 무장 지점이 없다"));
        let grouped = body
            .find("new_group_with(")
            .unwrap_or_else(|| panic!("{file}: 무리 넣기가 없다 — 이 항목의 전제가 낡았다"));
        let released = body
            .find(".into_inner()")
            .unwrap_or_else(|| panic!("{file}: 가드 풀기 지점이 없다"));
        assert!(
            spawn_step < armed,
            "{file}: 가드가 띄우기보다 앞에 선다 — 이 항목의 전제가 낡았다"
        );
        assert!(
            armed < grouped,
            "{file}: 무리 넣기가 가드 무장보다 앞이다 — 넣기가 실패하면 띄운 자식을 아무도 안 끈다"
        );
        assert_no_early_exit(file, &body[spawn_step + 1..], "띄운 뒤");
        let later: Vec<usize> = body[spawn_step + 1..]
            .match_indices('?')
            .map(|(i, _)| spawn_step + 1 + i)
            .collect();
        let first = *later.first().unwrap_or_else(|| {
            panic!("{file}: 띄운 뒤에 실패 가능한 걸음이 없다 — 이 항목의 전제가 낡았다")
        });
        assert!(
            grouped < first && !body[grouped..first].contains(';'),
            "{file}: 띄운 뒤 첫 실패 가능 걸음이 무리 넣기가 아니다 — 그 실패에서는 무리가 없어 그때까지 뜬 후손을 \
             아무도 안 끈다: `{}`",
            line_at(body, first)
        );
        for &at in &later {
            assert!(
                at < released,
                "{file}: 가드를 푼 뒤에 실패 가능한 걸음이 있다 — 그 실패가 띄운 자식을 남긴다: `{}`",
                line_at(body, at)
            );
        }
    }

    /// ★띄운 바로 다음에 가드가 서고, 띄운 뒤 첫 실패 가능 걸음이 무리 넣기다 — 세 통로 모두★.
    ///
    /// 소스에서 재는 이유 = 그 배치는 실패를 주입할 수 없는 자리다(`GroupOwner::new` · `adopt` · pty 의 `try_clone_reader`
    /// · `take_writer` 를 실패시키는 이음매가 없다). 아래 실프로세스 항목은 가드와 무리가 끄는 것만 재고 어디에 서
    /// 있는지는 못 본다.
    #[test]
    fn the_guard_is_armed_and_the_child_adopted_first_after_spawn() {
        let stdio = include_str!("stdio.rs");
        let pty = include_str!("pty.rs");
        let codex = include_str!("../backend/codex/transport.rs");

        let own = production(include_str!("spawn.rs"));
        assert_adoption_is_the_first_fallible_step_after_spawn(
            "transport/spawn.rs",
            body(&own, "fn spawn_piped("),
            ".spawn()",
        );
        let pty_code = production(pty);
        assert_adoption_is_the_first_fallible_step_after_spawn(
            "transport/pty.rs",
            body(&pty_code, "fn open("),
            ".spawn_command(",
        );

        for (file, src) in [
            ("transport/stdio.rs", stdio),
            ("backend/codex/transport.rs", codex),
        ] {
            let code = production(src);
            let open = body(&code, "fn open(");
            let spawned = open.find("spawn_piped(").unwrap_or_else(|| {
                panic!(
                    "{file}: `open` 이 `spawn_piped` 를 부르지 않는다 — 가드 밖에서 띄울 수 있다"
                )
            });
            assert!(
                !open.contains(".spawn()") && !open.contains("Command::new("),
                "{file}: `open` 이 스스로 띄운다 — 가드 밖이다"
            );
            let spawn_step = spawned + open[spawned..].find('?').expect("띄우기의 `?`");
            assert_no_early_exit(file, &open[spawn_step + 1..], "`spawn_piped` 뒤");
            assert!(
                !open[spawn_step + 1..].contains('?'),
                "{file}: `spawn_piped` 뒤에 실패 가능한 걸음이 있다 — 거기엔 가드가 없다: `{}`",
                line_at(
                    open,
                    spawn_step + 1 + open[spawn_step + 1..].find('?').unwrap_or(0)
                )
            );
        }

        for (file, src) in [
            ("transport/stdio.rs", stdio),
            ("transport/pty.rs", pty),
            ("backend/codex/transport.rs", codex),
        ] {
            assert!(
                !production(src).contains("GroupOwner::new"),
                "{file}: 통로가 무리를 직접 만든다 — 무리 넣기는 가드 아래의 `new_group_with` 로만 한다"
            );
        }
    }

    /// 위 항목의 이른 반환 찾기가 눈멀지 않았다 — `return` 과 `let … else` 는 잡고 `if … else` 는 안 잡는다.
    #[test]
    fn the_early_exit_scan_sees_return_and_let_else_but_not_if_else() {
        assert_eq!(early_exits("let x = f();\nreturn Err(e);").len(), 1);
        assert_eq!(
            early_exits("let Some(x) = f() else {\n    panic!();\n};").len(),
            1
        );
        assert!(early_exits("let x = if a { 1 } else { 2 };").is_empty());
        assert!(early_exits("let returned = elsewhere();").is_empty());
    }

    /// 끄기는 `Ok` 인데 끝내 「아직 돈다」로 답하는 자식 — Windows 에서 끄기가 실제로 실패한 자식의 모양.
    struct Unending {
        polls: Arc<AtomicUsize>,
    }

    impl Reap for Unending {
        fn pid(&self) -> Option<u32> {
            None
        }

        fn kill_direct(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn try_reap(&mut self) -> io::Result<bool> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            Ok(false)
        }
    }

    /// ★안 끝나는 직속 자식은 시한까지만 기다리고 놓는다★ — 시한 없이 기다리면 띄우기를 부른 쪽이 선다.
    #[test]
    fn dropping_the_guard_gives_up_on_a_child_that_does_not_end() {
        let polls = Arc::new(AtomicUsize::new(0));
        let started = Instant::now();
        drop(ChildGuard::new(Unending {
            polls: polls.clone(),
        }));
        let took = started.elapsed();
        assert!(
            took >= REAP_BUDGET,
            "시한 전에 놓았다 — 기다리지 않았다: {took:?}"
        );
        assert!(
            took < REAP_BUDGET + Duration::from_secs(2),
            "시한을 넘겨 기다렸다: {took:?}"
        );
        assert!(
            polls.load(Ordering::Relaxed) > 1,
            "한 번 묻고 말았다 — 시한 안에서 다시 묻지 않는다"
        );
    }

    // ── 실프로세스 ──────────────────────────────────────────────────────────────────

    /// ★가드를 풀면 아무것도 끄지 않는다★ — 성공 경로의 끄기는 통로 `shutdown()` 몫이다(ADR-0001).
    #[cfg(windows)]
    #[test]
    fn releasing_the_guard_leaves_the_child_running() {
        use engram_dashboard_platform::process::pid_alive;

        let child = ChildGuard::new(ping_child()).into_inner();
        assert!(
            pid_alive(child.id()),
            "풀린 가드가 자식을 껐다 — 성공 경로가 통로 대신 끄기를 한다"
        );
        drop(ChildGuard::new(child));
    }

    /// ★가드를 버리면 직속 자식이 끝난다★ — 무리 넣기가 실패한 자리의 정리가 이것 하나다.
    #[cfg(windows)]
    #[test]
    fn dropping_the_guard_ends_the_child() {
        assert_dropping_the_guard_ends(ping_child());
    }

    /// 같은 규칙이 pty 통로의 자식(portable-pty)에도 선다.
    #[cfg(windows)]
    #[test]
    fn dropping_the_guard_ends_a_pty_child() {
        let pair = open_pty();
        let mut cmd = portable_pty::CommandBuilder::new("ping.exe");
        cmd.args(["-n", "30", "127.0.0.1"]);
        assert_dropping_the_guard_ends(pair.slave.spawn_command(cmd).expect("pty 기동"));
        drop(pair);
    }

    /// ★이미 끝난 직속 자식의 끄기는 `Ok` 다★ — 넣은 뒤의 실패에서 무리가 먼저 끝낸 자식을 가드가 다시 끌 때 warn
    /// 없이 거두기로 간다(`Err` 면 거두지 않는다).
    #[cfg(windows)]
    #[test]
    fn killing_an_already_ended_child_is_ok() {
        let mut child = ping_child();
        let _ = child.kill();
        let _ = child.wait();
        assert!(Reap::kill_direct(&mut child).is_ok());
    }

    /// ★넣은 뒤에 실패하면 직속 자식과 손자가 함께 끝난다(무리)★ — 실제 CLI 자리에 `ping` 을 띄운다. 넣은 뒤에야
    /// `ping` 을 띄우도록 문을 단다(`spawn_gated_cmd`).
    #[cfg(windows)]
    #[test]
    fn a_failure_after_adoption_ends_the_child_and_its_grandchild() {
        use engram_dashboard_platform::testing::{open_gate, spawn_gated_cmd};
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let mut seen = None;
        let failed = fail_after_adoption(
            spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW),
            |guard, group| {
                open_gate(guard.child_mut());
                seen = Some(grandchild_in(guard, group));
            },
        );
        assert_tree_ended(failed, seen);
    }

    /// 같은 규칙이 pty 통로의 자식에도 선다. 문도 같다 — `set /p` 가 콘솔에서 한 줄을 받을 때까지 `ping` 을 띄우지
    /// 않고, 넣은 뒤에 master 쓰기로 그 한 줄을 준다.
    #[cfg(windows)]
    #[test]
    fn a_failure_after_adoption_ends_a_pty_child_and_its_grandchild() {
        use std::io::Write;

        let pair = open_pty();
        let mut cmd = portable_pty::CommandBuilder::new("cmd.exe");
        cmd.args([
            "/d",
            "/c",
            "set",
            "/p",
            "_=",
            "&",
            "ping",
            "-n",
            "30",
            "127.0.0.1",
        ]);
        let child = pair.slave.spawn_command(cmd).expect("pty 기동");
        // 판정이 끝날 때까지 쥔다 — 입력 끝을 닫으면 conhost 가 세션을 내려 손자가 무리 아닌 길로 끝날 수 있다(재 보지
        // 않았다).
        let mut gate = pair.master.take_writer().expect("pty 쓰기");

        let mut seen = None;
        let failed = fail_after_adoption(child, |guard, group| {
            gate.write_all(b"go\r\n").expect("문 열기");
            seen = Some(grandchild_in(guard, group));
        });
        assert_tree_ended(failed, seen);
        drop(gate);
        drop(pair);
    }

    /// 통로 `open` 의 배치 그대로 — 가드를 세우고 곧바로 넣은 뒤 `probe` 를 부르고 실패로 돌아간다. 돌아가며 지역
    /// 변수가 선언 역순(무리 → 가드)으로 버려진다.
    #[cfg(windows)]
    fn fail_after_adoption<C: Reap>(
        child: C,
        probe: impl FnOnce(&mut ChildGuard<C>, &GroupOwner),
    ) -> Result<(), PtyError> {
        let mut guard = ChildGuard::new(child);
        let group = new_group_with(guard.pid())?;
        probe(&mut guard, &group);
        Err(PtyError::SpawnFailed("주입한 실패".into()))
    }

    /// 직속 자식과 그 아래 손자 `ping` 의 번호 — 손자가 무리 밖이면(넣기 전에 떴다) 실험이 성립하지 않아 패닉한다.
    #[cfg(windows)]
    fn grandchild_in<C: Reap>(guard: &ChildGuard<C>, group: &GroupOwner) -> (u32, u32) {
        let root = guard.pid().expect("직속 자식 번호");
        let grandchild = ping_under(root);
        let members = group
            .downgrade()
            .expect("Windows 는 무리가 있다")
            .member_pids()
            .expect("무리 멤버 명단");
        assert!(
            members.contains(&grandchild),
            "손자 ping 이 무리 밖이다 — 넣기 전에 떴다 · 실험이 성립하지 않는다: {members:?}"
        );
        (root, grandchild)
    }

    #[cfg(windows)]
    fn assert_tree_ended(failed: Result<(), PtyError>, seen: Option<(u32, u32)>) {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::pid_alive;
        use std::time::Duration;

        let (root, grandchild) =
            seen.unwrap_or_else(|| panic!("무리 넣기가 실패해 실험이 성립하지 않는다: {failed:?}"));
        assert!(failed.is_err(), "주입한 실패가 안 돌아왔다");
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(root)),
            "넣은 뒤 실패했는데 직속 자식이 산다"
        );
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(grandchild)),
            "넣은 뒤 실패했는데 손자 ping 이 산다 — 무리가 넣은 뒤에 뜬 후손을 못 끝냈다"
        );
    }

    #[cfg(windows)]
    fn assert_dropping_the_guard_ends<C: Reap>(child: C) {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::pid_alive;
        use std::time::Duration;

        let root = child.pid().expect("직속 자식 번호");
        let started = Instant::now();
        drop(ChildGuard::new(child));
        // 바쁜 러너에서 스케줄 지연으로 시한을 살짝 넘겨도 거둠은 성공일 수 있다 — 여유를 둔다.
        assert!(
            started.elapsed() < REAP_BUDGET + Duration::from_secs(3),
            "가드가 끈 직속 자식을 시한 안에 못 거뒀다"
        );
        assert!(
            wait_until(Duration::from_secs(5), || !pid_alive(root)),
            "가드를 버렸는데 직속 자식이 산다"
        );
    }

    /// `ping.exe` 를 직속 자식으로 — 손자를 남기지 않아 가드 하나로 다 거둔다.
    #[cfg(windows)]
    fn ping_child() -> Child {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console_window(&mut cmd);
        cmd.spawn().expect("ping 기동")
    }

    #[cfg(windows)]
    fn open_pty() -> portable_pty::PtyPair {
        portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty")
    }

    /// `root` 아래 손자 `ping` 의 번호 — 10 초 안에 못 보면 실험이 성립하지 않아 패닉한다.
    #[cfg(windows)]
    fn ping_under(root: u32) -> u32 {
        use engram_dashboard_base::testing::wait_until;
        use engram_dashboard_platform::process::child_pids;
        use engram_dashboard_platform::testing::is_ping;
        use std::time::Duration;

        let mut grandchild = None;
        assert!(
            wait_until(Duration::from_secs(10), || {
                grandchild = child_pids(root).into_iter().find(|&pid| is_ping(pid));
                grandchild.is_some()
            }),
            "10 초 안에 손자 ping 을 못 봤다 — 실험이 성립하지 않는다"
        );
        grandchild.expect("방금 봤다")
    }
}
