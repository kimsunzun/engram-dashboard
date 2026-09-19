//! `/control/hook` 입구 — 에이전트 프로그램이 **자기 자식으로 띄운 훅**이 우리에게 보고하는 자리.
//!
//! ★왜 이 입구가 따로 있나★: 다른 제어 입구는 전부 **에이전트 본인이 치는 동사**라 "나" 가 등장하지
//!   않는다(`control::agent` 헤더). 이 입구는 반대다 — 보고의 내용이 「나는 지금 이 세션이다」이고,
//!   그 "나" 를 **자격증명에서만** 파생한다. 그래서 동사 표(`agent.*`)로 접히지 않는다: 그 표의 핸들러는
//!   호출자 신원을 받지 않는다.
//!
//! ★이 모듈의 **코드**는 어느 백엔드도 이름으로 부르지 않는다(ADR-0004)★ — 받는 것은 「세션 id 문자열
//!   하나」이고 그것을 누가 어떤 훅 이름으로 냈는지 여기서 분기하지 않는다. 자격을 가르는 축도 백엔드
//!   이름이 아니라 **술어**다([`SessionStartRecorder::assigns_own_session_id`]). 아래 주석이 한 프로그램을
//!   이름으로 적는 것은 **위협 모델의 출처와 거부한 수단의 근거**를 적기 위해서다 — 이름을 지우면 「왜 그
//!   피어의 수단을 안 베꼈나」가 검증 불가가 된다. 그 이름이 **코드 분기**로 새면 그때가 위반이다.
//!
//! ★환경변수 상속이 이 입구의 위협 모델이다★: 제어 평면 토큰은 스폰 env 로 심기고 env 는 자식에게
//!   자동 상속된다(`backend::inject_cli_entrance` doc). 그래서 에이전트가 **하위 에이전트**를 띄우면 그
//!   하위 세션의 훅도 **같은 토큰**으로 보고한다 — 신원만 믿으면 자식의 세션 id 가 부모 프로필에 앉는다.
//!
//! ★그 축에 쓰는 수단은 **「이미 찬 칸을 덮지 않는다」 한 줄**이다 — 「누가 자식인가」를 알아내는 것이
//!   아니다★: 식별 수단이 없다. 토큰도 env 도 부모와 글자 그대로 같고, 훅 페이로드에 계보를 말하는 칸이
//!   없다(실측 7 필드: session_id · transcript_path · cwd · hook_event_name · model ·
//!   permission_mode · source). ★성숙 피어(herdr)가 읽는 `CODEX_THREAD_ID` 는 이 환경에 존재하지
//!   않는다★(codex 0.155.0 바이너리 전체 문자열에서 0 건 — 실측 2026-09-18). 그래서 그 수단을 베끼지
//!   않았다.
//!
//! ★**그래서 이 입구는 위 위협을 절반만 막는다 — 없는 방어를 믿지 말 것**★: 막히는 것은 **칸이 이미 찬
//!   뒤**에 오는 보고뿐이고, **칸이 빈 동안에는 먼저 닿은 쪽이 이긴다**. 자식의 훅이 부모의 것보다 먼저
//!   닿으면 자식 값이 앉고 그 뒤 부모가 거절된다 — 오늘 그 순서를 뒤집을 수단이 없다.
//! ★그 빈 칸 창은 **화신마다 한 번씩** 열린다★ — Fresh 스폰이 그 칸을 반납하기 때문이다(`agent` 의
//!   `manager::fresh_spawn_release_session_id` — crate 내부 항목이라 링크가 아니라 이름으로 적는다).
//!   반납이 없으면 창이 한 번뿐인 대신 2 회차 화신부터 **정상 재시작마다 아래 거절 로그가 찍혀** 진짜
//!   중첩 사고를 가린다 — 둘 중 신호를 살리는 쪽을 골랐다.
//! ★app-server 모드에는 그 창이 사실상 없다(실측 2026-09-18)★: 훅은 `thread/start` 가 아니라 **첫
//!   `turn/start` 16ms 뒤**에 뜨므로 통로 sink 가 언제나 먼저 칸을 채우고, 훅이 싣고 오는 값은 그
//!   thread id 와 **같은 값**이다. 그래서 그 모드의 훅 보고는 늘 「이미 같은 값」으로 끝난다.
//!   규칙 본문은
//!   [`ProfileRegistry::adopt_session_id`](engram_dashboard_agent::profile::ProfileRegistry::adopt_session_id)
//!   가 지고, 이 파일은 자격과 사유 기록만 진다.
//!
//! tauri import 0(daemon crate).
// ADR-0004
// ADR-0086
// ADR-0185
// ADR-0208

use engram_dashboard_agent::profile::SessionIdAdoption;
use engram_dashboard_agent::types::AgentId;
use uuid::Uuid;

use super::ingress::ControlQueryResult;
use super::registry::BoundIdentity;

/// 훅이 POST 하는 봉투 — 칸 하나뿐이다.
///
/// ★필드 이름은 [`engram_dashboard_agent::types::CLI_HOOK_SESSION_ID_FIELD`] 가 정본이고, 아래
///   `the_wire_field_name_is_the_shared_constant` 가 그 둘을 묶어 잰다★ — serde 어트리뷰트에는 상수를
///   못 쓰므로(리터럴만 받는다) 컴파일러가 아니라 그 시험이 벽이다.
/// ★`#[serde(default)]` 인 이유★: 없으면 역직렬화가 실패해 **빈 400** 으로 끝나는데, 이 라우트를 부르는
///   것은 사람이 아니라 훅이라 그 응답을 읽는 눈이 없다. 빈 문자열로 받아 아래 판정이 사유를 남기면
///   데몬 로그에 흔적이 남는다 — 그것이 이 입구의 유일한 진단 표면이다. ★CLI 도 같은 규율이라, 읽을 수
///   없는 페이로드를 만나면 빈 값을 실어 보내 이 흔적을 남긴다★(그쪽 훅 동사 주석).
#[derive(Debug, Default, serde::Deserialize)]
pub struct HookSessionStartRequest {
    #[serde(default)]
    pub session_id: String,
}

/// 한 보고의 결말 — 로그 문구와 응답 봉투를 함께 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStartOutcome {
    /// 빈 칸에 받아 적었다.
    Recorded,
    /// 이미 같은 값이 있었다 — 아무 것도 바꾸지 않았다.
    AlreadyKnown,
    /// ★이미 **다른** 값이 있었다 — 덮지 않고 거절했다★.
    Conflict,
    /// 이 에이전트는 자기 세션 id 를 **우리에게서 받는** 쪽이라 밖에서 온 보고를 받지 않는다.
    NotEligible,
    /// 세션 id 칸이 비었거나 uuid 로 읽히지 않는다.
    Unreadable,
    /// 그 사이 프로필이 사라졌거나 화신이 갈렸다.
    NotApplied,
}

impl SessionStartOutcome {
    fn wire(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::AlreadyKnown => "already_known",
            Self::Conflict => "conflict",
            Self::NotEligible => "not_eligible",
            Self::Unreadable => "unreadable",
            Self::NotApplied => "not_applied",
        }
    }

    fn is_ok(self) -> bool {
        matches!(self, Self::Recorded | Self::AlreadyKnown | Self::NotApplied)
    }

    fn code(self) -> &'static str {
        match self {
            Self::Conflict => "CONFLICTING_SESSION",
            Self::NotEligible => "NOT_ELIGIBLE",
            _ => "INVALID_ARGUMENT",
        }
    }
}

/// 프로필을 보는 쪽(포트).
///
/// ★`AgentManager` 를 그대로 받지 않는 이유★: 이 판정을 단언하려고 실제 PTY 프로세스가 딸려오면
///   ADR-0012 위반이다. 실물 어댑터는 조립부가 준다.
// ADR-0012
pub trait SessionStartRecorder: Send + Sync {
    /// 이 에이전트가 **자기 세션 id 를 우리에게서 받나**. `None` = 그 프로필이 없다.
    ///
    /// ★`true` 면 보고를 받지 않는다 — 이것이 이 입구의 자격 축이다★: 그 에이전트의 손잡이는 우리가
    ///   정한 값이라 밖에서 온 보고가 그것을 건드릴 이유가 하나도 없다. 받아 주면 그 에이전트가 **자기
    ///   이어받기를 스스로 깨는** 경로가 열리고, 그 훅이 실제로는 그 프로세스가 띄운 **다른 프로그램의**
    ///   세션일 때(중첩) 남의 값이 그 자리에 앉는다.
    /// ★술어를 새로 짜지 말고 backend dispatch 의 그것을 읽을 것★(ADR-0185의 두 축 중 발급 축).
    fn assigns_own_session_id(&self, id: AgentId) -> Option<bool>;

    /// 빈 칸에만 적는다 — 규칙의 정본은
    /// [`ProfileRegistry::adopt_session_id`](engram_dashboard_agent::profile::ProfileRegistry::adopt_session_id).
    fn adopt(&self, id: AgentId, epoch: u32, sid: Uuid) -> SessionIdAdoption;
}

/// 한 보고를 처리한다 — 이 파일의 진입점.
///
/// ★신원은 **인자로 받은 것만** 쓴다★: 봉투에는 "누구" 칸이 없고 앞으로도 두지 않는다. 두는 순간
///   env 를 물려받은 임의의 자손이 남의 이름으로 보고할 수 있다(ADR-0086 「from 은 토큰에서만 파생」).
/// ★응답 봉투는 훅이 읽지 않는다 — 그래도 성공/반려를 가른다★: 이 라우트를 사람이 직접 쳐서 배선을
///   확인하는 경우가 유일한 독자다. 훅에게 남는 신호는 **데몬 로그** 하나다.
/// ★자격 판정이 값 판정보다 **먼저**다★: 받을 자격이 없는 에이전트의 보고는 그 값이 무엇이든 로그 한
///   줄로 끝나야 한다 — 순서를 뒤집으면 자격 없는 호출이 uuid 파싱 결과에 따라 다른 답을 받아, 밖에서
///   그 축을 더듬을 수 있게 된다.
pub fn handle_session_start(
    recorder: &dyn SessionStartRecorder,
    caller: BoundIdentity,
    req: &HookSessionStartRequest,
) -> (SessionStartOutcome, ControlQueryResult) {
    match recorder.assigns_own_session_id(caller.agent_id) {
        Some(true) => {
            tracing::warn!(
                agent = %caller.agent_id,
                epoch = caller.epoch,
                "세션 시작 보고를 버렸다 — 이 에이전트의 세션 id 는 우리가 발급한다(밖에서 온 보고를 받지 않는다)"
            );
            return finish(
                SessionStartOutcome::NotEligible,
                "this agent's session id is issued by the daemon; reported ids are not accepted",
            );
        }
        // 프로필이 없으면 적을 곳도 없다 — 아래 `adopt` 가 같은 답을 내지만, 여기서 끊어야 자격을 못
        //   가린 채로 값 판정에 들어가지 않는다.
        None => {
            tracing::info!(
                agent = %caller.agent_id,
                epoch = caller.epoch,
                "세션 시작 보고를 적지 못했다 — 그 사이 프로필이 사라졌다"
            );
            return finish(SessionStartOutcome::NotApplied, "no such agent");
        }
        Some(false) => {}
    }

    let raw = req.session_id.trim();
    let Ok(sid) = Uuid::parse_str(raw) else {
        // ★값을 로그에 싣지 않는다★ — 임의 문자열이라 개행 하나로 로그 줄이 위조된다(`sanitize_for_log`
        //   의 실패 모드). 길이만 적으면 「빈 칸인가 깨진 값인가」는 갈린다.
        tracing::warn!(
            agent = %caller.agent_id,
            epoch = caller.epoch,
            len = raw.len(),
            "세션 시작 보고를 버렸다 — 세션 id 를 uuid 로 읽지 못했다(훅 배선이나 페이로드를 볼 것)"
        );
        return finish(SessionStartOutcome::Unreadable, "session_id is not a uuid");
    };

    match recorder.adopt(caller.agent_id, caller.epoch, sid) {
        SessionIdAdoption::Adopted => {
            tracing::info!(
                agent = %caller.agent_id,
                epoch = caller.epoch,
                "세션 시작 보고를 프로필에 적었다(빈 칸)"
            );
            finish(SessionStartOutcome::Recorded, "")
        }
        SessionIdAdoption::AlreadyKnown => finish(SessionStartOutcome::AlreadyKnown, ""),
        SessionIdAdoption::Conflict => {
            // ★이 줄이 이 입구의 가장 중요한 기록이다★: 「하위 세션이 같은 토큰으로 보고했다」와 「그
            //   에이전트가 정말로 새 대화를 열었다」가 **여기서 같은 모양으로 온다**. 둘을 가를 축이
            //   없으므로 거절하고, 그 사실을 남겨 사람이 뒤에서 가를 수 있게 한다.
            tracing::warn!(
                agent = %caller.agent_id,
                epoch = caller.epoch,
                "세션 시작 보고를 거절했다 — 이 프로필엔 이미 다른 세션 id 가 있다(하위 세션이 같은 토큰을 물려받았거나, 그 에이전트가 새 대화를 열었다). 덮지 않는다"
            );
            finish(
                SessionStartOutcome::Conflict,
                "a different session id is already recorded for this agent; not overwriting",
            )
        }
        SessionIdAdoption::StaleIncarnation | SessionIdAdoption::Vanished => {
            tracing::info!(
                agent = %caller.agent_id,
                epoch = caller.epoch,
                "세션 시작 보고가 프로필을 바꾸지 않았다(그 사이 화신이 갈렸거나 프로필이 사라졌다)"
            );
            finish(SessionStartOutcome::NotApplied, "")
        }
    }
}

fn finish(outcome: SessionStartOutcome, hint: &str) -> (SessionStartOutcome, ControlQueryResult) {
    let result = if outcome.is_ok() {
        ControlQueryResult::Ok(serde_json::json!({ "outcome": outcome.wire() }))
    } else {
        ControlQueryResult::Error {
            code: outcome.code(),
            hint: hint.to_string(),
        }
    };
    (outcome, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::CLI_HOOK_SESSION_ID_FIELD;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 실 레지스트리의 세 축(자격·화신·빈 칸)을 같은 모양으로 흉내 낸다.
    #[derive(Default)]
    struct FakeProfiles {
        /// AgentId → (화신, 우리가 sid 를 발급하는 쪽인가, 적혀 있는 sid)
        rows: Mutex<HashMap<AgentId, (u32, bool, Option<Uuid>)>>,
    }

    impl FakeProfiles {
        fn with(id: AgentId, epoch: u32, assigns: bool, sid: Option<Uuid>) -> Self {
            let f = Self::default();
            f.rows.lock().unwrap().insert(id, (epoch, assigns, sid));
            f
        }
        fn sid(&self, id: AgentId) -> Option<Uuid> {
            self.rows.lock().unwrap().get(&id).and_then(|r| r.2)
        }
    }

    impl SessionStartRecorder for FakeProfiles {
        fn assigns_own_session_id(&self, id: AgentId) -> Option<bool> {
            self.rows.lock().unwrap().get(&id).map(|r| r.1)
        }
        fn adopt(&self, id: AgentId, epoch: u32, sid: Uuid) -> SessionIdAdoption {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&id) {
                None => SessionIdAdoption::Vanished,
                Some(row) if row.0 != epoch => SessionIdAdoption::StaleIncarnation,
                Some(row) => match row.2 {
                    Some(existing) if existing == sid => SessionIdAdoption::AlreadyKnown,
                    Some(_) => SessionIdAdoption::Conflict,
                    None => {
                        row.2 = Some(sid);
                        SessionIdAdoption::Adopted
                    }
                },
            }
        }
    }

    fn caller(id: AgentId, epoch: u32) -> BoundIdentity {
        BoundIdentity {
            agent_id: id,
            epoch,
        }
    }

    fn req(sid: &str) -> HookSessionStartRequest {
        HookSessionStartRequest {
            session_id: sid.to_string(),
        }
    }

    // ── 봉투 ────────────────────────────────────────────────────────────────

    /// ★훅이 실제로 보내는 **바이트 모양**으로 들어와야 한다★ — 봉투 칸 이름이 갈리면 보고가 통째로
    /// 사라지는데 그 실패는 훅이 삼켜 화면에 아무 신호도 없다.
    #[test]
    fn the_wire_field_name_is_the_shared_constant() {
        let sid = Uuid::new_v4();
        let body = serde_json::json!({ CLI_HOOK_SESSION_ID_FIELD: sid.to_string() });
        let parsed: HookSessionStartRequest = serde_json::from_value(body).expect("봉투 역직렬화");
        assert_eq!(parsed.session_id, sid.to_string());
    }

    // ── 세 갈래(없음 / 같음 / 다름) ──────────────────────────────────────────

    #[test]
    fn an_empty_slot_takes_the_report() {
        let id = AgentId::new_v4();
        let sid = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, None);

        let (outcome, result) =
            handle_session_start(&profiles, caller(id, 7), &req(&sid.to_string()));

        assert_eq!(outcome, SessionStartOutcome::Recorded);
        assert!(result.is_ok(), "{:?}", result.to_json());
        assert_eq!(profiles.sid(id), Some(sid));
    }

    /// 같은 값을 다시 내는 것(훅 재발화 · 통로가 이미 적은 값을 훅이 뒤따라 보고)은 사고가 아니다.
    #[test]
    fn the_same_value_is_a_no_op_success() {
        let id = AgentId::new_v4();
        let sid = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, Some(sid));

        let (outcome, result) =
            handle_session_start(&profiles, caller(id, 7), &req(&sid.to_string()));

        assert_eq!(outcome, SessionStartOutcome::AlreadyKnown);
        assert!(result.is_ok());
        assert_eq!(profiles.sid(id), Some(sid));
    }

    /// ★이 시험이 재는 사고 둘이 같은 모양으로 온다★: 하위 세션이 부모 토큰을 물려받아 보고하는 것과,
    /// 통로가 이미 적어 둔 진짜 손잡이를 훅이 다른 값으로 덮으려는 것. 어느 쪽이든 **기존 값이 남는다**.
    #[test]
    fn a_different_value_is_refused_and_the_stored_one_survives() {
        let id = AgentId::new_v4();
        let stored = Uuid::new_v4();
        let reported = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, Some(stored));

        let (outcome, result) =
            handle_session_start(&profiles, caller(id, 7), &req(&reported.to_string()));

        assert_eq!(outcome, SessionStartOutcome::Conflict);
        assert!(!result.is_ok());
        assert_eq!(
            result.to_json()["code"],
            "CONFLICTING_SESSION",
            "{:?}",
            result.to_json()
        );
        assert_eq!(
            profiles.sid(id),
            Some(stored),
            "거절했는데 저장된 값이 바뀌었다"
        );
    }

    /// ★**칸이 찬 뒤로는** 순서가 판정을 바꾸지 않는다★ — 재는 것은 「첫 값이 끝까지 남는다」이고,
    /// 그것은 곧 **빈 칸 갈래가 first-wins** 라는 뜻이다(헤더의 「절반만 막는다」가 이 사실이다).
    #[test]
    fn the_first_value_to_land_is_the_one_that_stays() {
        let id = AgentId::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, None);

        handle_session_start(&profiles, caller(id, 7), &req(&a.to_string()));
        handle_session_start(&profiles, caller(id, 7), &req(&b.to_string()));
        handle_session_start(&profiles, caller(id, 7), &req(&a.to_string()));

        assert_eq!(profiles.sid(id), Some(a), "첫 값이 끝까지 남아야 한다");
    }

    // ── 자격 ────────────────────────────────────────────────────────────────

    /// ★우리가 sid 를 발급하는 에이전트의 보고는 값을 보기도 전에 끝난다★ — 그 손잡이를 밖에서 건드릴
    /// 이유가 없다. 이 한 줄이 「그 에이전트가 자기 이어받기를 스스로 깬다」를 구조적으로 닫는다.
    #[test]
    fn an_agent_whose_session_id_we_issue_is_refused() {
        let id = AgentId::new_v4();
        let stored = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, true, Some(stored));

        let (outcome, result) =
            handle_session_start(&profiles, caller(id, 7), &req(&Uuid::new_v4().to_string()));

        assert_eq!(outcome, SessionStartOutcome::NotEligible);
        assert!(!result.is_ok());
        assert_eq!(profiles.sid(id), Some(stored));
    }

    /// 자격이 없으면 **빈 칸이어도** 적지 않는다 — 값 판정에 도달하지 않는 것이 계약이다.
    #[test]
    fn eligibility_is_decided_before_the_value() {
        let id = AgentId::new_v4();
        let profiles = FakeProfiles::with(id, 7, true, None);

        let (outcome, _) =
            handle_session_start(&profiles, caller(id, 7), &req("this-is-not-a-uuid-either"));

        assert_eq!(
            outcome,
            SessionStartOutcome::NotEligible,
            "읽을 수 없는 값이어도 자격 답이 먼저 나와야 한다"
        );
        assert_eq!(profiles.sid(id), None);
    }

    // ── 화신·부재·읽을 수 없는 값 ────────────────────────────────────────────

    #[test]
    fn a_report_from_a_dead_incarnation_writes_nothing() {
        let id = AgentId::new_v4();
        let profiles = FakeProfiles::with(id, 9, false, None);

        let (outcome, _) =
            handle_session_start(&profiles, caller(id, 7), &req(&Uuid::new_v4().to_string()));

        assert_eq!(outcome, SessionStartOutcome::NotApplied);
        assert_eq!(profiles.sid(id), None);
    }

    #[test]
    fn a_report_for_a_vanished_profile_ends_quietly() {
        let profiles = FakeProfiles::default();
        let (outcome, _) = handle_session_start(
            &profiles,
            caller(AgentId::new_v4(), 7),
            &req(&Uuid::new_v4().to_string()),
        );
        assert_eq!(outcome, SessionStartOutcome::NotApplied);
    }

    #[test]
    fn an_unreadable_session_id_writes_nothing() {
        let id = AgentId::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, None);
        for raw in ["", "   ", "not-a-uuid", "01a0b3bb"] {
            let (outcome, result) = handle_session_start(&profiles, caller(id, 7), &req(raw));
            assert_eq!(outcome, SessionStartOutcome::Unreadable, "raw={raw:?}");
            assert!(!result.is_ok());
        }
        assert_eq!(profiles.sid(id), None);
    }

    /// 앞뒤 공백은 훅 배선이 개행을 덧붙이는 흔한 경로라 벗겨서 읽는다.
    #[test]
    fn surrounding_whitespace_is_tolerated() {
        let id = AgentId::new_v4();
        let sid = Uuid::new_v4();
        let profiles = FakeProfiles::with(id, 7, false, None);
        let (outcome, _) =
            handle_session_start(&profiles, caller(id, 7), &req(&format!("  {sid}\n")));
        assert_eq!(outcome, SessionStartOutcome::Recorded);
        assert_eq!(profiles.sid(id), Some(sid));
    }
}
