//! 명령 주인 명부의 공유 핸들 — **전 연결이 같은 한 부를 본다**(ADR-0134/0135 · TRD §3-7).
//!
//! 명부의 규칙(등록 전량 last-wins · 차분 · tombstone · 상한)은 전부 도구 crate 의 [`Roster`] 가
//! 소유한다. 여기가 더하는 것은 **연결 수명**뿐이다 — 명부는 이름만 알고 어느 연결이 아직 붙어
//! 있는지를 모르는데, 그것을 아는 쪽이 데몬이다([`Roster::owner_is_gone`] 주석이 가리키는 자리).
//!
//! ★살아 있는지 확인과 명부 변경은 **같은 임계 구역** 안에서 일어난다★: 둘을 나누면 확인과 변경
//! 사이에 정리가 끼어들어, 이미 끊긴 연결의 등록이 그 뒤에 내려앉는다. `on_disconnect` 는 조용한
//! 시점이 아니라 **`on_text` 와 겹칠 수 있는** 시점이므로(`frame_port::ConnectionHandler` 계약 —
//! 네트워크 행은 abort 를 걸 뿐 완료를 기다리지 않는다) 그 겹침은 이론이 아니다. 내려앉으면 그 이름은
//! **없는 주인**을 가리킨 채 데몬 수명 내내 `Available` 로 답한다(그 연결엔 다시 올 정리가 없고 명부엔
//! 만료가 없다).
//!
//! ★이름이 겹치는 다른 것과 헷갈리지 말 것★: 이 crate 의 `RosterBroadcast`·`RosterChanged`·`RosterDiff`
//! 는 **에이전트/프로필 명단**의 통지 포트다(ADR-0132). 여기 명부는 **명령 이름 → 주인**이고 둘은
//! 아무 관계가 없다.
//!
//! ★락은 이 파일 안에서만 잡고 이 파일 안에서 푼다★(ADR-0006): 아래 메서드는 잠그고 [`Roster`] 를 한 번
//! 부른 뒤 즉시 놓는다. 가드를 밖으로 내보내면 답장을 기다리는 배달 하나가 등록·연결 정리·조회를 전부
//! 세운다(근거 정본 = [`engram_dashboard_command::OwnerLookupSource`] 주석).
//! ★알려진 비용★: [`CommandRoster::entries`] 는 잠근 채로 명부 전량을 복제한다 — 상한까지 찬 명부면
//! 그 구간이 짧지 않다. 줄이려면 [`Roster`] 가 빌려주는 모양을 바꿔야 해서 이 슬라이스 밖이다.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use engram_dashboard_command::{
    CommandDecl, CommandError, ErrorCode, OwnerToken, Roster, RosterEntry,
};
use engram_dashboard_net::frame_port::ConnId;

/// 붙어 있지 않은 연결의 등록·차분을 반려할 때의 문구.
///
/// ★상수로 두는 이유는 테스트다★: 같은 `CONFLICT` 를 [`Roster`] 쪽 두 검사도 내므로, 문구를 안 보면
/// 「연결 수명 그물이 잡았다」와 「명부 검사가 잡았다」가 구분되지 않는다 — 그러면 이 파일의 그물을
/// 통째로 지워도 초록인 테스트가 생긴다.
pub(crate) const DETACHED_REFUSAL: &str =
    "this connection is not attached — a registration lands only while its connection is up";

#[derive(Clone, Default)]
pub struct CommandRoster {
    inner: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    roster: Roster,
    /// 지금 붙어 있는 연결. **명부와 한 잠금 아래** 있어야 확인과 변경 사이가 벌어지지 않는다.
    live: BTreeSet<ConnId>,
}

impl CommandRoster {
    /// 주인 토큰의 접두사 — 이 형식은 데몬 안에서만 난다(`CommandRoster::owner_of`).
    pub const OWNER_TOKEN_PREFIX: &'static str = "conn-";

    pub fn new() -> Self {
        Self::default()
    }

    /// 연결 하나의 주인 토큰 — **연결 id 에서 파생한다**. 파생 규칙이 여기 하나뿐이라 등록·정리·조회가
    /// 같은 값을 본다(정책과 그 근거는 `connection_core::ConnectionSession::owner_token`).
    // ADR-0134
    pub fn owner_of(conn_id: ConnId) -> OwnerToken {
        OwnerToken::new(format!("{}{conn_id}", Self::OWNER_TOKEN_PREFIX))
    }

    /// 연결이 섰다 — 이 뒤로 그 연결의 등록이 받아들여진다.
    pub fn attach(&self, conn_id: ConnId) {
        self.lock().live.insert(conn_id);
    }

    /// 연결이 끊겼다 — 살아 있는 명단에서 빼고 그 주인의 이름을 자취로 내린다. **둘은 한 잠금 안에서**
    /// 일어난다(겹쳐 도는 등록이 그 사이로 못 들어온다).
    ///
    /// 이름은 **지우지 않는다** — 지우면 「모르는 이름」과 「주인이 지금 없는 이름」이 같은 답이 된다
    /// (ADR-0135 · TRD §4-②).
    pub fn detach(&self, conn_id: ConnId) {
        let mut shared = self.lock();
        shared.live.remove(&conn_id);
        shared.roster.disconnect(&Self::owner_of(conn_id));
    }

    /// 붙을 때의 전량 등록. 끊긴 연결의 늦은 패킷은 `CONFLICT` 로 반려한다.
    pub fn register(&self, conn_id: ConnId, decls: Vec<CommandDecl>) -> Result<(), CommandError> {
        let mut shared = self.lock();
        shared.refuse_if_detached(conn_id)?;
        shared.roster.register(&Self::owner_of(conn_id), decls)
    }

    /// 붙어 있는 동안의 차분. 반려 규칙은 [`CommandRoster::register`] 와 같다.
    pub fn update(
        &self,
        conn_id: ConnId,
        added: Vec<CommandDecl>,
        removed: Vec<String>,
    ) -> Result<(), CommandError> {
        let mut shared = self.lock();
        shared.refuse_if_detached(conn_id)?;
        shared
            .roster
            .update(&Self::owner_of(conn_id), added, removed)
    }

    /// 명부 전량의 스냅샷. [`Roster::entries`] 의 iterator 를 그대로 내보내면 호출자가 순회하는 동안
    /// 락이 잡혀 있으므로 여기서 걷어 낸다.
    pub fn entries(&self) -> Vec<RosterEntry> {
        self.lock().roster.entries().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.lock().expect("command roster poisoned")
    }
}

impl Shared {
    /// ★코드 선택★: 패킷은 멀쩡하고(`INVALID_ARGUMENT` 아님) 부르는 쪽 잘못도 아니다 — 거절하는 것은
    /// **상태**(그 연결은 지금 명단에 없다)라서 `CONFLICT` 다. 도구 crate 가 같은 상황(끊긴 주인의 늦은
    /// 차분)에 쓰는 코드와 같게 맞췄고, 그 코드의 재시도 지시는 `never` 다 — 같은 연결로 다시 보내 봐야
    /// 그 연결은 이미 없고, 다시 붙으면 **새 토큰**을 받으므로 재시도가 아니라 재등록이다.
    ///
    /// ★문구가 「끊겼다」로 좁지 않은 이유★: 같은 갈래가 **한 번도 붙은 적 없는** 연결에도 선다. 오늘
    /// 운영 경로에서는 안 나지만(`on_connect` 이 dispatch 보다 앞이라는 네트워크 행 순서) 타입이 막는
    /// 것은 아니다 — `register`/`update` 는 맨 [`ConnId`] 를 받는다.
    fn refuse_if_detached(&self, conn_id: ConnId) -> Result<(), CommandError> {
        if self.live.contains(&conn_id) {
            return Ok(());
        }
        Err(CommandError::of(ErrorCode::Conflict, DETACHED_REFUSAL))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_command::OwnerLookup;

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: "{}".to_string(),
        }
    }

    /// 명부에 직접 묻는다 — 배달이 보는 것이 이 답이다(`entries` 의 투영이 아니라).
    fn lookup(roster: &CommandRoster, name: &str) -> OwnerLookup {
        let shared = roster.lock();
        shared.roster.lookup(name)
    }

    // ── 연결 정리와 겹쳐 도는 등록(frame_port::ConnectionHandler 계약의 잔여 경쟁) ──────
    //
    // 다섯 갈래를 못으로 박는다: {등록·차분} × {자기 이름·마지막 자취를 남이 가져간 뒤} + 두 검사가
    // 모두 비켜가는 차분 하나. 그래서 이 그물은 명부가 아니라 **연결 수명**으로만 설 수 있다.
    //
    // ★코드만 보면 안 된다 — 문구까지 본다★: 같은 `CONFLICT` 를 `Roster` 쪽 두 검사도 내므로
    // (`check_owner_attached` · `check_added_are_not_taken`), 코드만 단언하면 이 파일의 그물을 통째로
    // 지워도 몇몇은 **다른 이유로** 초록을 유지한다.

    #[test]
    fn a_registration_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("정리 뒤에 내려앉은 등록");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unavailable,
            "없는 주인이 `Available` 로 부활하면 안 된다"
        );
    }

    /// ★명부만으로는 못 막는 갈래★ — 산 주인이 죽은 주인의 **마지막** 자취를 가져가면 그 죽음을 기억할
    /// 자리가 안 남아 `Roster` 의 `Gone` 그물이 통째로 비켜간다. 여기서 막는 것은 연결 수명이다.
    #[test]
    fn a_late_registration_cannot_take_a_live_connections_name_even_with_no_trace_left() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2);
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 이름을 이어받는다");

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("죽은 연결의 늦은 등록");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2)),
            "산 연결이 그대로 주인이다"
        );
    }

    #[test]
    fn a_delta_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("정리 뒤에 내려앉은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(lookup(&roster, "tab.split"), OwnerLookup::Unknown);
    }

    #[test]
    fn a_late_delta_cannot_take_a_live_connections_name_even_with_no_trace_left() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2);
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 이름을 이어받는다");

        let err = roster
            .update(1, vec![decl("tab.create")], vec![])
            .expect_err("죽은 연결의 늦은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    /// ★두 검사가 **모두** 비켜가는 자리 — 여기선 연결 수명이 유일한 그물이다★
    ///
    /// 산 연결이 죽은 연결의 **마지막** 자취를 가져가면 「끊겼다」를 기억할 자리가 없어
    /// `Roster::check_owner_attached` 가 통과하고, 더하는 이름은 아무도 안 쥐었으니
    /// `Roster::check_added_are_not_taken` 도 통과한다. 그물이 빠지면 `tab.split` 이 **없는 주인** 앞으로
    /// `Available` 이 되어 데몬 수명 내내 굳는다.
    #[test]
    fn a_late_delta_adding_an_unclaimed_name_is_refused_when_no_trace_is_left() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2);
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 마지막 자취를 가져간다");

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("죽은 연결의 늦은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.split"),
            OwnerLookup::Unknown,
            "죽은 연결 앞으로 새 이름이 서면 안 된다"
        );
    }

    /// 재연결은 **새 연결 id** 로 오므로 막히지 않는다 — 위 반려가 정상 경로를 잠그지 않는다는 경계.
    #[test]
    fn reconnecting_under_a_new_connection_id_registers_normally() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        roster.attach(2);
        roster
            .register(2, vec![decl("tab.create")])
            .expect("재연결은 새 토큰으로 온다");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }
}
