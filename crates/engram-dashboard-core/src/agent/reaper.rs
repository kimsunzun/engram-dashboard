//! 세션 reaper — 종료 분류(ADR-0019)의 단일 소비자.
//!
//! pump 가 finish 승자일 때 발행한 `ReapMsg` 를 **단일 supervisor 스레드**가 소비해 다음을 수행한다:
//! sessions 맵에서 제거(epoch 일치 검증 후) → 프로필 disposition(auto_restore=false 다운그레이드 /
//! 손 안 댐 — ADR-0083 으로 자동 삭제 폐지) → 목록 통지. kill_agent 가 직접 하던 맵 제거·통지를
//! 여기로 위임해 done 단일 소비자로 만든다.
//!
//! 불변식:
//! - kill 2동사(ADR-0001)·finalize 1회(ADR-0005)는 reaper 가 건드리지 않는다 — done 신호를
//!   소비할 뿐. ReapMsg 발행은 finalize 승자 경로 1회.
//! - 락 순서(ADR-0006): sessions write lock 구간 = epoch 검증 + remove 만. ProfileRegistry
//!   mutate(디스크 IO)·status_sink 통지는 lock 밖.
//! - epoch(ADR-0007/0084): reap 전 epoch 일치 검증 → 재spawn 된 새 세션을 옛 done 이 오삭제 못 함.
//!   같은 epoch-guard 를 apply_disposition 까지 확장(ADR-0084) → stale reap 이 재활성화(epoch bump)로
//!   붙은 산 세션의 auto_restore 를 강등 못 함.
//! - idempotency: sessions.remove() Some 승자 1명만 disposition·통지(같은 done 2회 와도 1회).
//!
//! tauri import 0.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use crate::agent::profile::ProfileRegistry;
use crate::agent::session::AgentSession;
use crate::agent::types::{AgentId, AgentInfo, ControlChannel, Disposition, ReapMsg, StatusSink};

/// reaper 스레드로 보내는 메시지. ReapMsg(정상 종료 이벤트) + 명시 Stop(셧다운).
/// Stop 없이도 모든 Sender drop 시 recv 가 Err 로 끝나 루프가 종료된다(이중 안전).
pub enum ReaperCmd {
    Reap(ReapMsg),
    Stop,
}

/// reaper 가 reap_one 수행에 필요한 공유 핸들 묶음. AgentManager 의 필드 Arc 들을 그대로 공유한다
/// (manager 와 동일 sessions/profiles/status_sink 를 본다 — 두 주체가 같은 모델).
pub struct ReaperDeps {
    pub sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    pub profiles: Arc<ProfileRegistry>,
    pub status_sink: Arc<dyn StatusSink>,
    /// ADR-0086: 제어 채널 seam(manager 와 공유 Arc). terminal 수렴 지점(여기)에서 revoke 를 부른다 —
    /// 크래시·EOF·정상 종료 등 kill 이 아닌 모든 terminal 을 커버한다(kill 은 kill_agent 가 선제 revoke).
    pub control: Arc<dyn ControlChannel>,
}

impl ReaperDeps {
    /// reap 1건 처리(ADR-0019 §reap_one). 이 함수는 reaper 스레드(또는 테스트)에서만 호출된다.
    ///
    /// 순서(불변식 고정):
    ///   1) write lock { epoch 불일치면 return; remove } 즉시 해제 — Arc 만 들고 나온다.
    ///   2) None(이미 제거됨=패자) 이면 return(idempotent).
    ///   3) !shutting_down 이면 disposition 적용(lock 밖, ProfileRegistry mutate=디스크 IO).
    ///   4) 목록 통지(lock 밖, 외부 콜백).
    pub fn reap_one(&self, msg: ReapMsg) {
        // 1. write lock 구간 = epoch 검증 + remove 만(ADR-0006). Arc clone 후 즉시 해제.
        //    ★poison-tolerant★: 다른 스레드(pump 등)가 sessions lock 보유 중 panic 해 lock 이
        //    poison 돼도 reaper 는 계속 reap 해야 한다(좀비 방지). 데이터는 HashMap 일 뿐 불변식이
        //    깨진 게 아니므로 into_inner 로 가드를 회수해 진행한다(catch_unwind 와 이중 안전).
        let removed = {
            let mut sessions = self
                .sessions
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // epoch 불일치 = 재spawn 으로 자리 바뀐 유령 done → 새 세션 보존(ADR-0007).
            match sessions.get(&msg.id) {
                Some(s) if s.epoch == msg.epoch => sessions.remove(&msg.id),
                _ => return,
            }
        };

        // 2. 패자(이미 누가 remove) = no-op. remove Some 승자 1명만 아래로 진행(idempotency).
        if removed.is_none() {
            return;
        }
        drop(removed); // Arc<AgentSession> 폐기 — 여기서 transport/core 자원이 마지막으로 끊긴다.

        // 2.5. ADR-0086: 제어 채널 토큰 폐기 + mcp-config 삭제. ★여기가 모든 terminal 의 단일 수렴점★
        //   (ADR-0019 reaper) — 크래시·EOF·정상 exit·유저 kill 어떤 경로든 정확히 1회 이 지점을 지난다.
        //   epoch 검증을 통과한 승자(msg.epoch == session.epoch)만 여기 오므로, stale terminal 이
        //   재활성화(epoch bump)로 새로 붙은 산 토큰을 지우는 일이 없다(remove epoch-guard 와 같은 원리).
        //   revoke 는 idempotent(remove-if-present)라 kill_agent 의 선제 revoke 와 겹쳐도 무해.
        // ADR-0086
        self.control.revoke(msg.id, msg.epoch);

        // 3. 셧다운 종료가 아니면 disposition 적용. 셧다운이면 손대지 않음(auto_restore=true 잔류
        //    → 부팅 복원). lock 밖에서 ProfileRegistry mutate(디스크 IO) — 락 순서 준수.
        //    ★msg.epoch 를 함께 넘긴다(ADR-0084 epoch-guard)★: sessions.remove 는 이미 epoch 검증을
        //    거쳤지만, remove 와 이 lock-free disposition 사이 창에서 재활성화(epoch bump)가 일어나면
        //    stale reap 이 산 세션을 강등할 수 있어 disposition 계층까지 epoch-guard 를 확장한다.
        if !msg.shutting_down_at_finish {
            let disposition = decide(&msg);
            apply_disposition(&self.profiles, msg.id, msg.epoch, disposition);
        }

        // 4. 목록 변경 통지(lock 밖, 외부 콜백). list_agents 와 동치인 스냅샷을 만든다.
        let agents = list_agents(&self.sessions, &self.profiles);
        self.status_sink.agent_list_updated(agents);

        tracing::info!(
            agent = %msg.id,
            epoch = msg.epoch,
            shutting_down = msg.shutting_down_at_finish,
            "reaped session"
        );
    }
}

/// 종료 분류(ADR-0019 §decide, ADR-0083 개정). frozen snapshot(intent/shutting_down)으로만 판정.
///
/// ```text
/// shutting_down_at_finish        => KeepAsIs               // 데몬 셧다운: 부팅 복원(auto_restore 그대로)
/// 그 외 모든 종료(유저 kill·정상 exit·크래시·EOF·signal)
///                                => KeepDisableAutoRestore // 시체 보존 + auto_restore=false
/// ```
/// ★ADR-0083: 자동 삭제 폐지★ — 어떤 종료도 프로필을 지우지 않는다(옛 유저 kill·정상 exit(code0)
///   → DeleteProfile 조항 폐지). 사용자 정책(ADR-0082 계승 "삭제하지마, 시체로라도 남겨")대로
///   모든 런타임 종료는 세션만 맵에서 수거하고 프로필은 시체로 보존(claude_session_id 유지 →
///   재활성화 시 --resume 로 이어받음). 프로필 삭제는 자동 처분이 아니라 **명시적 사용자 명령**
///   (AgentCommand::DeleteProfile / Tauri delete_profile — apply_disposition 을 거치지 않고
///   ProfileRegistry::remove 직접 호출)으로만 일어난다.
// ADR-0083
pub fn decide(msg: &ReapMsg) -> Disposition {
    if msg.shutting_down_at_finish {
        return Disposition::KeepAsIs;
    }
    // 셧다운이 아니면 종료 원인(유저 kill·정상 exit·크래시·EOF 무관) 전부 시체 보존.
    // intent/reason 으로 삭제를 가르던 분기는 ADR-0083 으로 폐지.
    Disposition::KeepDisableAutoRestore
}

/// disposition 을 ProfileRegistry 에 적용(ADR-0019, ADR-0083 개정, ADR-0084 epoch-guard).
/// **downgrade-only**: auto_restore 를 절대 true 로 올리지 않는다 — KeepDisableAutoRestore 는 false 로만
/// 내린다. KeepAsIs 는 무동작. ADR-0083: 자동 삭제(옛 DeleteProfile) 폐지 — reaper 는 프로필을 지우지
/// 않는다(수거 + 다운그레이드만).
///
/// ★ADR-0084 epoch-guard★: `reaped_epoch`(= ReapMsg.epoch = 죽은 세션이 spawn 될 때 읽은 프로필
///   epoch. session.epoch 과 동일 값)와 **현재 프로필 epoch 이 일치할 때만** auto_restore 를 내린다.
///   sessions.remove 후 이 lock-free disposition 사이에 재활성화가 `bump_epoch`(manager.rs Resume
///   갈래)로 프로필 epoch 를 올렸다면, `p.epoch != reaped_epoch` → 다운그레이드를 **건너뛴다**(그
///   사이 새로 붙은 산 세션을 stale reap 이 강등하지 못하게). sessions.remove 의 epoch-guard(ADR-0007)
///   와 같은 원리를 disposition 계층까지 확장한 것이다.
/// ★lock 순서(ADR-0006)★: 비교를 **update_with 클로저 안**(프로필 락 보유 중)에서 한다 —
///   sessions 락은 여기서 절대 잡지 않는다(disposition 은 sessions lock-free 유지). epoch 판정을
///   프로필의 in-memory 필드로만 하므로 sessions 맵을 볼 필요가 없다.
fn apply_disposition(
    profiles: &ProfileRegistry,
    id: AgentId,
    reaped_epoch: u32,
    disposition: Disposition,
) {
    match disposition {
        Disposition::KeepDisableAutoRestore => {
            // 존재 + epoch 일치할 때만 false 로 내린다(이미 false 면 그대로 — 올리지 않음).
            // epoch 불일치 = 그 사이 재활성화로 epoch 가 올라간 새 산 세션 → 손대지 않는다(ADR-0084).
            profiles.update_with(id, |p| {
                if p.epoch == reaped_epoch {
                    p.auto_restore = false;
                }
            });
        }
        Disposition::KeepAsIs => {}
    }
}

/// 현재 살아있는 세션 목록 스냅샷 → AgentInfo. manager.list_agents 와 동일 로직을 reaper 가
/// lock 밖에서 만들 수 있게 분리(통지용). sessions read lock 으로 Arc 만 모아 즉시 해제한 뒤,
/// 각 세션의 AgentInfo 를 조립한다(profiles lock 과 sessions lock 비중첩 — ADR-0006).
fn list_agents(
    sessions: &Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    profiles: &Arc<ProfileRegistry>,
) -> Vec<AgentInfo> {
    let snapshot: Vec<Arc<AgentSession>> = {
        // poison-tolerant(reap_one 1과 동일 이유): 통지용 스냅샷이라 가드 회수로 진행한다.
        let guard = sessions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.values().cloned().collect()
    };
    snapshot.iter().map(|s| session_info(s, profiles)).collect()
}

/// session → AgentInfo(manager.agent_info 와 **byte-identical** 매핑). sessions lock 미보유에서만 호출.
///
/// ★ADR-0101 (WYSIWYA): manager.agent_info 와 반드시 같은 이름 사슬을 써야 한다★ — reap_one 은 세션
///   종료마다 이 스냅샷을 agent_list_updated 로 브로드캐스트하는 hot path 라, 여기서 옛 규칙(profile.name)
///   을 쓰면 아무 reap 때마다 트리 이름이 예전 full-path 라벨로 되돌아간다. 그래서 canonical name 을
///   agent_info 와 동일하게 display_name(override) ?? basename(session.cwd) 로 파생하고, 프로필 부재
///   fallback 도 같은 공유 코어(name::canonical_name_or_id_fallback)로 맞춘다(로직 복제 금지).
///
/// ★cwd 출처 = session.cwd★: AgentInfo.cwd 에 넣는 값과 동일(canonical). profile.cwd(raw)에서
///   파생하면 트리 basename 과 어긋난다 — agent_info 와 같은 이유(manager.rs resolve_canonical_name 참조).
// ADR-0101
fn session_info(session: &Arc<AgentSession>, profiles: &Arc<ProfileRegistry>) -> AgentInfo {
    use std::sync::atomic::Ordering;
    let cwd = session.cwd.to_string_lossy();
    let display_name = profiles.get(session.id).and_then(|p| p.display_name);
    let name = crate::agent::name::canonical_name_or_id_fallback(
        display_name.as_deref(),
        &cwd,
        session.id,
    );
    AgentInfo {
        id: session.id,
        name,
        cwd: session.cwd.to_string_lossy().to_string(),
        status: session.status(),
        cols: session.cols.load(Ordering::Relaxed),
        rows: session.rows.load(Ordering::Relaxed),
        epoch: session.epoch,
        capabilities: session.capabilities(),
    }
}

/// reaper supervisor 스레드를 기동하고 핸들 + Sender 를 반환한다. AgentManager 가 생성 시 1회 호출.
/// 스레드는 `while let Ok(cmd) = rx.recv()` 로 ReapMsg 를 직렬 소비하며, Stop 또는 모든 Sender
/// drop 시 종료한다.
pub fn spawn_reaper(deps: ReaperDeps) -> (Sender<ReaperCmd>, JoinHandle<()>) {
    let (tx, rx): (Sender<ReaperCmd>, Receiver<ReaperCmd>) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("engram-reaper".into())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    // ★단일 장애점 격리(reviewer-deep blocker)★: reaper 는 전역 단일 스레드라
                    //   reap_one 한 건의 panic(예: lock poison→expect, decide/apply 내부 패닉)이
                    //   스레드 전체를 죽이면 **이후 모든 세션이 맵에서 영영 안 빠져 좀비화**한다.
                    //   pump 는 agent 별 catch_unwind 로 이미 격리돼 있으니 reaper 도 메시지 1건
                    //   처리 실패가 루프를 못 죽이게 catch_unwind 로 감싼다. &deps 는 unwind 후에도
                    //   재사용하므로 AssertUnwindSafe 로 감싼다(여기서 deps 를 옮기지 않음).
                    ReaperCmd::Reap(msg) => {
                        let deps = &deps;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            move || deps.reap_one(msg),
                        ));
                        if let Err(e) = result {
                            let detail = e
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| e.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "<non-string panic>".to_string());
                            // 다음 메시지로 계속 — reaper 생존이 좀비 방지의 핵심.
                            tracing::error!(panic = %detail, "reap_one panicked — reaper 루프 생존, 다음 메시지 계속");
                        }
                    }
                    ReaperCmd::Stop => break,
                }
            }
            tracing::debug!("reaper thread stopped");
        })
        .expect("spawn reaper thread");
    (tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    // decide 는 이제 셧다운 여부만 보므로 lib 본문은 이 두 타입을 쓰지 않는다(테스트에서만 구성).
    use crate::agent::types::{TerminalReason, TerminationIntent};

    fn msg(intent: TerminationIntent, shutting_down: bool, reason: TerminalReason) -> ReapMsg {
        ReapMsg {
            id: uuid::Uuid::new_v4(),
            epoch: 0,
            reason,
            intent_at_finish: intent,
            shutting_down_at_finish: shutting_down,
        }
    }

    #[test]
    fn decide_user_kill_keeps_corpse() {
        // ADR-0083: 유저 kill 도 삭제 아님 — 시체 보존(KeepDisableAutoRestore). 재활성화 resume 가능.
        let m = msg(TerminationIntent::UserKill, false, TerminalReason::Killed);
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_clean_exit_keeps_corpse() {
        // ADR-0083: 정상 exit(code0) 도 삭제 아님 — 시체 보존. code-0 갭(ADR-0082 §열린항목 ②) 닫힘.
        let m = msg(
            TerminationIntent::None,
            false,
            TerminalReason::Exited { code: Some(0) },
        );
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_crash_keeps_and_disables() {
        // exit 1 = 크래시.
        let m = msg(
            TerminationIntent::None,
            false,
            TerminalReason::Exited { code: Some(1) },
        );
        assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
    }

    #[test]
    fn decide_unknown_code_is_crash() {
        // EOF/StreamClosed/Error/code 불명 → 보수적으로 크래시.
        for reason in [
            TerminalReason::Exited { code: None },
            TerminalReason::StreamClosed,
            TerminalReason::Error("boom".into()),
        ] {
            let m = msg(TerminationIntent::None, false, reason);
            assert_eq!(decide(&m), Disposition::KeepDisableAutoRestore);
        }
    }

    #[test]
    fn decide_shutting_down_keeps_as_is() {
        // 셧다운이면 intent/reason 무관하게 KeepAsIs(부팅 복원 대상 유지).
        let m = msg(
            TerminationIntent::UserKill,
            true,
            TerminalReason::Exited { code: Some(1) },
        );
        assert_eq!(decide(&m), Disposition::KeepAsIs);
    }
}
