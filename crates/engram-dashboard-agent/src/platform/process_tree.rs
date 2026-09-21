//! 「이 PID 아래 지금 어떤 프로세스들이 살아 있나」를 OS 에 묻는다 — 각자의 신원(PID + 시작시각)과 함께.
//!
//! ★무엇을 찾으려고 묻는지는 이 파일이 모른다(ADR-0004)★ — 받는 것은 뿌리 하나이고 돌려주는 것은
//! 신원 목록뿐이다. 도메인 지식이 0 이라 여기 있고, 소비자가 하나뿐이라 바닥 crate 로는 안 내려간다
//! (ADR-0175 입주 조건 ① · `file_holders` 와 같은 자리·같은 사유).
//!
//! ★셈은 `engram_dashboard_base::platform` 의 두 primitive 를 **조립만** 한다★ — 자식 열거와 시작시각
//! 조회를 여기서 다시 쓰지 않는다. 이 파일이 더하는 것은 「뿌리의 신원을 먼저 확인하고, 거기서부터
//! 걸어 내려간다」는 규칙 하나다.
//!
//! 진입점 = [`subtree`]. 실패는 값으로 돌려준다(panic 없음).

/// 한 프로세스의 신원.
///
/// ★두 칸은 **함께** 대조하라고 있다★ — PID 는 OS 가 재사용하므로 PID 단독 일치는 남의 프로세스를
/// 우리 것으로 본다(ADR-0218 결정 2). 그래서 이 타입에는 `pid` 만 꺼내 쓰는 헬퍼를 두지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: u32,
    /// 프로세스 생성 FILETIME — `engram_dashboard_base::platform::process_creation_time` 이 돌려주는 값과
    /// 같은 척도다.
    pub(crate) start_time: u64,
}

/// `root` 와 그 아래 **살아 있는** 후손 전부의 신원.
///
/// - ★`root` 의 시작시각이 `root_start_time` 과 다르면 **빈 목록**이다★ — 그 PID 는 더는 우리가 띄운
///   그 프로세스가 아니다(죽었거나 재사용됐다). 나무 걷기는 뿌리의 신원이 확인된 뒤에만 뜻이 있다:
///   확인 없이 걸으면 남의 프로세스 나무를 우리 것이라 부르게 된다.
/// - **후손의 시작시각은 지금 읽어 채운다** — 우리가 미리 알던 값이 없기 때문이다. 그래서 대조는 언제나
///   「이 순간 그 PID 인 프로세스」와의 대조이고, 열거와 대조 사이에 PID 가 재사용돼도 시작시각이
///   갈라서 걸러진다.
/// - ★**부모보다 먼저 태어난 항목은 후손이 아니다 — 버린다**★. 이것이 이 함수의 유일한 **의미** 규칙이고,
///   없으면 남의 프로세스가 우리 나무에 들어온다. 실제 시퀀스: 사용자가 손으로 띄운 `codex.exe` 가 고아가
///   되어 기록된 ppid `P` 를 그대로 달고 남고, Windows 가 `P` 를 우리 래퍼에 **재사용**한다. 그러면
///   `child_pids(P)` 가 그 남을 우리 자식으로 돌려주고, 그 남은 자기 락의 홀더와 자기 신원이 당연히
///   맞으므로 **우리 codex 가 락을 만들기 전 0.7~21 초 동안 유일한 후보**가 된다 — `Ambiguous` 도 안 뜨고
///   남의 스레드를 우리 손잡이에 적는다(ADR-0218 「영향/불변식」이 이름 붙인 그 조용한 고장).
///   ★`base` 의 `child_pids` 가 자기 doc 에서 이미 경고하는 것이 이것이다★ — 부모가 죽으면 ppid 가
///   stale 이라 「살아 있는 부모의 직계 자식」 용도로만 믿으라고 적혀 있다. 이 규칙이 그 단서를 강제한다.
///   ★같은 시각(`==`)은 통과시킨다★ — FILETIME 눈금 하나 안에서 뜬 부모·자식이 실재할 수 있고, 거기서
///   막으면 정상 자식을 잃는다. 막는 것은 **먼저 태어난** 것뿐이다.
///   ★이 규칙이 **무엇까지** 잡나 — 근거를 적되 「닫혀 있다」고는 하지 않는다★: 어떤 프로세스가 ppid 로
///   우리 뿌리의 PID 를 달고 있는 흔한 경우는 둘이다. ⓐ 정말 우리 후손이거나, ⓑ 그것을 낳은 **다른**
///   프로세스가 그 PID 를 쥐고 있다가 죽고 그 번호가 우리 뿌리에 재사용된 경우다. ⓑ 라면 그 프로세스는
///   재사용 **이전**에 태어났고, 재사용 이전이란 곧 우리 뿌리가 그 번호를 받기 전이므로 **뿌리보다
///   먼저** 태어났다 — 이 비교가 ⓑ 를 전부 잡는다. 뿌리 자신이 죽고 번호가 또 넘어간 경우는 위 첫
///   항목(뿌리 신원 확인)이 막는다.
///   ★단 제3의 경우가 실재한다 — 그래서 「전수」가 아니다★: Windows 는 `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`
///   로 부모를 **명시 지정**해 프로세스를 만들 수 있다. 그렇게 태어난 남은 우리 뿌리보다 **뒤**에
///   태어나고도 우리 뿌리의 PID 를 ppid 로 달 수 있어 이 비교를 지난다. 그래도 이 규칙을 두는 값어치는
///   그대로다 — 흔한 경로(ⓑ)를 닫고, 남는 경로는 그 남이 **우리 `CODEX_HOME` 아래 `<uuid>.lock` 까지
///   쥐고 있어야** 실제 오검출이 된다.
///
/// ★받아들인 잔여(다시 논쟁하지 말 것 — 2026-09-21 결정)★
///   1. **열거와 시작시각 읽기 사이의 PID 재사용** — `child_pids` 가 준 번호를 우리가 읽기 전에 그
///      프로세스가 죽고 번호가 넘어가면, 우리가 읽는 시작시각은 새 주인의 것이다. 창이 마이크로초
///      단위이고 원자적으로 고칠 수단이 없다(열거와 조회가 별개 syscall 이다).
///   2. **같은 눈금(`==`)** — 위 참조.
///   3. **명시 부모 지정** — 바로 위 문단.
/// - 시작시각을 못 읽는 항목은 **빼고, 그 아래로 내려가지도 않는다** — 신원의 절반이 없으면 대조가 PID
///   단독으로 내려앉고, 그 항목을 부모로 쓰면 위 순서 규칙을 적용할 기준이 없어 그 가지 전체가 무검증이 된다.
/// - `root_start_time` 이 0(미상)이면 빈 목록이다. 같은 사유.
/// - **best-effort**: 열거가 실패하면 그만큼 덜 돌려준다. 오류를 올리지 않는 것은 호출자가 그것을
///   「후보 아님」과 다르게 처리할 방법이 없기 때문이다(다음 바퀴에 다시 묻는다).
/// - ★깊이 제한을 두지 않는다 — 대신 **이미 본 신원을 다시 안 내려간다**★. ppid 는 OS 가 즉시 갱신하지
///   않아 순환처럼 보이는 모양이 나올 수 있고, 그때 방문 표시가 없으면 이 함수가 안 끝난다.
///   ★표시의 키는 PID 가 아니라 **신원**이다★ — PID 로만 표시하면 재사용된 PID 가 「이미 봤다」로 건너뛰어,
///   같은 번호를 쓰는 **다른** 프로세스가 통째로 안 보인다.
pub(crate) fn subtree(root: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
    use engram_dashboard_base::platform::{child_pids, process_creation_time};

    if root == 0 || root_start_time == 0 {
        return Vec::new();
    }
    if process_creation_time(root) != Some(root_start_time) {
        return Vec::new();
    }

    walk(
        ProcessIdentity {
            pid: root,
            start_time: root_start_time,
        },
        &|pid| {
            child_pids(pid)
                .into_iter()
                .filter_map(|child| {
                    process_creation_time(child).map(|start_time| ProcessIdentity {
                        pid: child,
                        start_time,
                    })
                })
                .collect()
        },
    )
}

/// [`subtree`] 의 **규칙만** — OS 는 `children` 뒤에 있다(ADR-0012). 규칙 둘(부모보다 먼저 태어난 것은
/// 버린다 · 이미 본 **신원**은 다시 안 내려간다)의 사유 정본은 [`subtree`] 의 doc 이고 여기 되풀어
/// 적지 않는다.
///
/// `children` 은 **신원을 못 읽은 항목을 이미 걸러서** 준다 — 그 거르기가 이 규칙 밖인 것은, 신원 없는
/// 항목에는 적용할 순서 기준 자체가 없기 때문이다.
fn walk(
    root: ProcessIdentity,
    children: &dyn Fn(u32) -> Vec<ProcessIdentity>,
) -> Vec<ProcessIdentity> {
    let mut out = vec![root];
    let mut visited = vec![root];
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for child in children(parent.pid) {
            if child.start_time < parent.start_time {
                continue;
            }
            if visited.contains(&child) {
                continue;
            }
            visited.push(child);
            frontier.push(child);
            out.push(child);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRAPPER: ProcessIdentity = ProcessIdentity {
        pid: 4242,
        start_time: 1_000,
    };

    fn id(pid: u32, start_time: u64) -> ProcessIdentity {
        ProcessIdentity { pid, start_time }
    }

    /// 부모 PID → 그 아래 신원들. 표에 없으면 자식 없음.
    fn table(rows: &[(u32, Vec<ProcessIdentity>)]) -> impl Fn(u32) -> Vec<ProcessIdentity> + '_ {
        move |pid| {
            rows.iter()
                .find(|(parent, _)| *parent == pid)
                .map(|(_, kids)| kids.clone())
                .unwrap_or_default()
        }
    }

    // ── 순서 규칙(G1) ────────────────────────────────────────────────────────────

    /// ★이 항목이 지키는 것 = 「남의 고아가 재사용된 PID 를 타고 우리 나무에 들어오지 않는다」★.
    /// 그 남은 우리 래퍼보다 **먼저** 떠 있었으므로 시작시각이 앞선다 — 그것이 유일한 구분 표식이다.
    /// 규칙이 빠지면 우리 codex 가 락을 만들기 전 구간에 **그 남이 유일한 후보**가 되어, `Ambiguous`
    /// 조차 안 뜨고 남의 스레드를 우리 손잡이에 적는다.
    #[test]
    fn a_child_that_predates_its_parent_is_not_ours() {
        let stranger = id(777, WRAPPER.start_time - 1);
        let got = walk(WRAPPER, &table(&[(WRAPPER.pid, vec![stranger])]));
        assert_eq!(
            got,
            vec![WRAPPER],
            "부모보다 먼저 태어난 항목이 들어왔다: {got:?}"
        );
    }

    /// 그 남 아래 매달린 것까지 통째로 안 들어온다 — 가지째 끊는다.
    #[test]
    fn nothing_under_a_predating_child_gets_in_either() {
        let stranger = id(777, WRAPPER.start_time - 1);
        let grandchild = id(778, WRAPPER.start_time + 5);
        let got = walk(
            WRAPPER,
            &table(&[
                (WRAPPER.pid, vec![stranger]),
                (stranger.pid, vec![grandchild]),
            ]),
        );
        assert_eq!(got, vec![WRAPPER], "끊긴 가지 아래가 새 들어왔다: {got:?}");
    }

    /// 같은 눈금에 뜬 자식은 정상이다 — 막는 것은 **먼저** 태어난 것뿐이다.
    #[test]
    fn a_child_born_on_the_same_tick_is_ours() {
        let twin = id(4816, WRAPPER.start_time);
        let got = walk(WRAPPER, &table(&[(WRAPPER.pid, vec![twin])]));
        assert_eq!(got, vec![WRAPPER, twin]);
    }

    #[test]
    fn the_walk_reaches_grandchildren() {
        let codex = id(4816, WRAPPER.start_time + 1);
        let tool = id(19_468, WRAPPER.start_time + 2);
        let got = walk(
            WRAPPER,
            &table(&[(WRAPPER.pid, vec![codex]), (codex.pid, vec![tool])]),
        );
        assert_eq!(got, vec![WRAPPER, codex, tool]);
    }

    // ── 방문 표시(G2) ────────────────────────────────────────────────────────────

    /// ★표시가 PID 키면 재사용된 번호가 「이미 봤다」로 건너뛰어 **다른** 프로세스가 통째로 사라진다★.
    #[test]
    fn a_recycled_pid_is_a_different_process_not_a_repeat() {
        let first = id(4816, WRAPPER.start_time + 1);
        let recycled = id(4816, WRAPPER.start_time + 9);
        let got = walk(
            WRAPPER,
            &table(&[(WRAPPER.pid, vec![first]), (first.pid, vec![recycled])]),
        );
        assert!(
            got.contains(&recycled),
            "같은 번호의 다른 프로세스가 표시에 먹혔다: {got:?}"
        );
    }

    /// stale ppid 가 순환처럼 보여도 끝난다 — 같은 신원을 두 번 안 내려간다.
    #[test]
    fn a_cycle_shaped_table_terminates() {
        let a = id(10, WRAPPER.start_time + 1);
        let b = id(11, WRAPPER.start_time + 1);
        let got = walk(
            WRAPPER,
            &table(&[
                (WRAPPER.pid, vec![a]),
                (a.pid, vec![b]),
                (b.pid, vec![a, b]),
            ]),
        );
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn an_unknown_root_has_no_subtree() {
        assert!(subtree(0, 1).is_empty());
        assert!(
            subtree(std::process::id(), 0).is_empty(),
            "시작시각 미상이면 남는 것이 PID 단독 대조다"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_wrong_start_time_disowns_the_root() {
        let me = std::process::id();
        let start = engram_dashboard_base::platform::process_creation_time(me)
            .expect("자기 creation time 조회 가능");
        assert!(
            subtree(me, start.wrapping_add(1)).is_empty(),
            "시작시각이 어긋난 PID 를 우리 것으로 봤다"
        );
    }

    /// ★이 저장소의 스폰은 Windows 에서 `cmd.exe /c <program>` 한 겹을 지나므로, 우리가 쥔 PID 는 늘
    /// 래퍼다★ — 그래서 뿌리 자신뿐 아니라 **후손이 목록에 들어오는 것**이 이 함수의 존재 이유다.
    #[cfg(windows)]
    #[test]
    fn the_subtree_reaches_past_the_root() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping", "-n", "4", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("cmd.exe 기동");
        let root = child.id();
        let start = engram_dashboard_base::platform::process_creation_time(root)
            .expect("자식 creation time 조회 가능");

        // 손자(`ping`)가 뜰 때까지 짧게 기다린다 — cmd 가 먼저 뜨고 그다음에 띄운다.
        let mut found = Vec::new();
        for _ in 0..40 {
            found = subtree(root, start);
            if found.len() > 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let _ = child.kill();
        let _ = child.wait();

        assert!(
            found.iter().any(|p| p.pid == root),
            "뿌리 자신이 목록에 없다: {found:?}"
        );
        assert!(
            found.len() > 1,
            "후손이 하나도 안 잡혔다 — 래퍼 PID 만 대조하면 codex 를 영영 못 찾는다: {found:?}"
        );
        assert!(
            found.iter().all(|p| p.start_time != 0),
            "신원의 절반이 빈 항목이 섞였다: {found:?}"
        );
    }
}
