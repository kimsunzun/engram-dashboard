//! 프로필 편집 포트의 `AgentManager` 구현.
//!
//! 붙임 쓰기는 명부 락 한 번씩이고(저장 포함 — 순서는 `ProfileRegistry` 의 것 그대로), 잠김 읽기는 예약 ·
//! 세션 · 명부를 **하나씩 따로** 잠깐 잡고 놓는다(겹쳐 쥐지 않는다 — ADR-0006).
// ADR-0311
// TRD S25 §1-5 「프로필 편집 포트」

use super::AgentManager;
use crate::profile_edit::{
    Attachments, PresetPathRename, ProfileEditError, ProfileEditPort, WorkspaceLock,
};
use crate::types::AgentId;

impl ProfileEditPort for AgentManager {
    fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError> {
        if self.profiles.try_set_attachments(id, next)? {
            Ok(())
        } else {
            Err(ProfileEditError::NotFound(id))
        }
    }

    fn rewrite_preset_paths(
        &self,
        renames: &[PresetPathRename],
    ) -> Result<Vec<AgentId>, ProfileEditError> {
        Ok(self.profiles.try_rewrite_preset_paths(renames)?)
    }

    fn remove_preset_paths(&self, paths: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
        Ok(self.profiles.try_remove_preset_paths(paths)?)
    }

    // ADR-0321
    fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
        // 읽는 순서 = 예약 → 세션 → 명부. 실행은 세션을 올린 **뒤에** 예약을 놓으므로, 이 호출과 겹친 실행은 둘
        //   중 하나에 반드시 걸린다(거꾸로 읽으면 그 사이 끝난 실행을 둘 다 놓친다). 세션 id 는 그 실행의 첫 제출이
        //   적으므로 맨 뒤에 읽는다.
        // TODO(TRD S25 10a `agent.setWorkspace`): 바꾸기가 자기 예약을 쥔 채 이 판정을 부르면 그 예약이 `Preparing`
        //   으로 보인다 — 예약 표가 쥔 쪽을 가르게 될 때 함께 고친다.
        let preparing = self.activation_in_flight(id);
        // ADR-0116: 시체(reaper 가 거두기 전)는 떠 있지 않다 — 명부와 같은 술어.
        let running = self
            .get_session(id)
            .is_ok_and(|session| session.status().is_live());
        let conversed = self
            .profiles
            .get(id)
            .ok_or(ProfileEditError::NotFound(id))?
            .backend_session_id
            .is_some();
        Ok(if running {
            Some(WorkspaceLock::Running)
        } else if preparing {
            Some(WorkspaceLock::Preparing)
        } else if conversed {
            Some(WorkspaceLock::Conversed)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::super::tests::{agent_profile, put_live_session, scripted_manager};
    use super::super::SpawnReservation;
    use super::*;
    use crate::profile::scripted_store::{Outcome, FAILURES};
    use crate::profile::{AttachedPreset, StoreError};
    use crate::types::TerminalReason;

    fn attachments(default_preset: bool, paths: &[&str]) -> Attachments {
        Attachments {
            default_preset,
            presets: paths
                .iter()
                .map(|p| AttachedPreset {
                    path: (*p).to_string(),
                })
                .collect(),
        }
    }

    /// 명부에 프로필 하나를 심은 매니저 — 저장 결과는 시험이 정한다.
    fn seeded() -> (
        AgentManager,
        Arc<crate::profile::scripted_store::ScriptedStore>,
        AgentId,
    ) {
        let (manager, store) = scripted_manager();
        let p = agent_profile("C:/edit/a", Some("a"));
        let id = p.id;
        manager.profiles.upsert(p);
        (manager, store, id)
    }

    fn converse(manager: &AgentManager, id: AgentId) {
        assert!(manager
            .profiles
            .update_with(id, |p| p.backend_session_id = Some(Uuid::new_v4())));
    }

    // ── 붙임 쓰기 ──

    #[test]
    fn set_attachments_writes_through_and_tells_a_missing_agent_apart() {
        let (manager, store, id) = seeded();
        let next = attachments(false, &["/b.md", "/a.md"]);

        manager.set_attachments(id, next.clone()).unwrap();
        assert_eq!(manager.profiles.attachments_of(id), Some(next.clone()));
        assert_eq!(store.disk(id).unwrap().presets, next.presets);
        manager
            .set_attachments(id, next.clone())
            .expect("같은 값은 오류가 아니다");

        let missing = Uuid::new_v4();
        assert!(matches!(
            manager.set_attachments(missing, next),
            Err(ProfileEditError::NotFound(got)) if got == missing
        ));
    }

    #[test]
    fn rewrite_preset_paths_returns_the_changed_agents() {
        let (manager, _store, id) = seeded();
        manager
            .set_attachments(id, attachments(true, &["/a.md"]))
            .unwrap();

        let changed = manager
            .rewrite_preset_paths(&[PresetPathRename {
                from: "/a.md".into(),
                to: "/b.md".into(),
            }])
            .unwrap();

        assert_eq!(changed, vec![id]);
        assert_eq!(
            manager.profiles.attachments_of(id),
            Some(attachments(true, &["/b.md"]))
        );
    }

    #[test]
    fn remove_preset_paths_returns_the_changed_agents() {
        let (manager, _store, id) = seeded();
        manager
            .set_attachments(id, attachments(true, &["/a.md", "/b.md"]))
            .unwrap();

        let changed = manager.remove_preset_paths(&["/a.md".into()]).unwrap();

        assert_eq!(changed, vec![id]);
        assert_eq!(
            manager.profiles.attachments_of(id),
            Some(attachments(true, &["/b.md"]))
        );
    }

    #[test]
    fn a_store_failure_surfaces_as_store_and_changes_nothing() {
        for failure in FAILURES {
            let (manager, store, id) = seeded();
            manager
                .set_attachments(id, attachments(true, &["/a.md"]))
                .unwrap();
            let before = manager.profiles.attachments_of(id);
            store.set(failure);

            let set = manager
                .set_attachments(id, attachments(false, &[]))
                .unwrap_err();
            let rewrite = manager
                .rewrite_preset_paths(&[PresetPathRename {
                    from: "/a.md".into(),
                    to: "/b.md".into(),
                }])
                .unwrap_err();
            let remove = manager.remove_preset_paths(&["/a.md".into()]).unwrap_err();

            for err in [set, rewrite, remove] {
                assert!(
                    matches!(
                        (failure, &err),
                        (Outcome::FailsIo, ProfileEditError::Store(StoreError::Io(_)))
                            | (
                                Outcome::Refuses(_),
                                ProfileEditError::Store(StoreError::Refused(_))
                            )
                    ),
                    "{failure:?} → {err}"
                );
            }
            assert_eq!(manager.profiles.attachments_of(id), before, "{failure:?}");
            assert!(!manager.profiles.is_dirty(), "{failure:?}");
        }
    }

    // ── 작업 폴더 잠김 ──

    #[test]
    fn workspace_lock_reports_each_reason_alone() {
        let (manager, _store, id) = seeded();
        assert_eq!(manager.workspace_lock(id).unwrap(), None);

        {
            let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("예약");
            assert_eq!(
                manager.workspace_lock(id).unwrap(),
                Some(WorkspaceLock::Preparing)
            );
        }
        assert_eq!(manager.workspace_lock(id).unwrap(), None, "예약을 놓았다");

        let (manager, _store, id) = seeded();
        put_live_session(&manager, id, 1);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running)
        );

        let (manager, _store, id) = seeded();
        converse(&manager, id);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Conversed)
        );
    }

    #[test]
    fn workspace_lock_reports_the_first_of_running_preparing_conversed() {
        let (manager, _store, id) = seeded();
        converse(&manager, id);
        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("예약");
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Preparing),
            "준비 중 > 대화함"
        );

        put_live_session(&manager, id, 1);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running),
            "떠 있음 > 준비 중 · 대화함"
        );
    }

    /// 끝나는 중(`Exiting`)은 아직 떠 있다 — 명부와 같은 술어(ADR-0116).
    #[test]
    fn an_exiting_session_still_locks_as_running() {
        let (manager, _store, id) = seeded();
        let core = put_live_session(&manager, id, 1);
        assert!(core.enter_exiting());

        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Running)
        );
    }

    /// ★읽는 순서 예약 → 세션 → 명부를 소스에서 못 박는다★ — 위 항목들은 한 스레드에서 차례로 재므로 순서를
    /// 뒤집어도 초록이다. 뒤집히면 겹친 실행(세션을 올린 뒤 예약을 놓는다)을 둘 다 놓쳐 잠김이 풀려 보인다.
    #[test]
    fn workspace_lock_reads_reservation_then_session_then_roster() {
        let body = squashed_production_body("fn workspace_lock(", "#[cfg(test)]");
        let at = |call: &str| {
            body.find(call).unwrap_or_else(|| {
                panic!("`{call}` 를 안 읽는다 — 이 항목의 전제가 낡았다: {body}")
            })
        };
        let reservation = at("self.activation_in_flight(id)");
        let session = at("self.get_session(id)");
        let roster = at("self.profiles.get(id)");
        assert!(
            reservation < session && session < roster,
            "읽는 순서가 예약 → 세션 → 명부가 아니다: {body}"
        );
    }

    /// 운영 구획에서 `start` 와 `end` 사이를 잘라 주석 줄을 빼고 공백을 전부 지운다 — `manager.rs` 시험의 같은
    /// 이름 도우미와 같은 꼴(이 파일을 읽는다).
    fn squashed_production_body(start: &str, end: &str) -> String {
        let src = include_str!("profile_edit.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("`{start}` 가 운영 구획에 없다 — 이 항목의 전제가 낡았다"))
            .split(end)
            .next()
            .expect("다음 구획까지");
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect::<String>()
    }

    #[test]
    fn a_dead_session_awaiting_the_reaper_does_not_lock() {
        let (manager, _store, id) = seeded();
        let core = put_live_session(&manager, id, 1);
        core.finish(TerminalReason::Killed);

        assert_eq!(manager.workspace_lock(id).unwrap(), None);
        converse(&manager, id);
        assert_eq!(
            manager.workspace_lock(id).unwrap(),
            Some(WorkspaceLock::Conversed)
        );
    }

    #[test]
    fn workspace_lock_of_an_agent_not_in_the_roster_is_not_found() {
        let (manager, _store, _id) = seeded();
        let stranger = Uuid::new_v4();
        assert!(matches!(
            manager.workspace_lock(stranger),
            Err(ProfileEditError::NotFound(got)) if got == stranger
        ));

        // 명부에 없으면 세션이 떠 있어도 없다고 답한다.
        put_live_session(&manager, stranger, 1);
        assert!(matches!(
            manager.workspace_lock(stranger),
            Err(ProfileEditError::NotFound(_))
        ));
    }
}
