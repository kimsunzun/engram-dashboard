//! 명령 주인 명부의 공유 핸들 — **전 연결이 같은 한 부를 본다**(ADR-0140/0141 · TRD §3-7).
//!
//! 명부의 규칙(등록 전량 last-wins · 차분 · 끊김 제거 · 상한)은 전부 도구 crate 의 [`Roster`] 가
//! 소유한다. 여기가 더하는 것은 **연결 수명**뿐이다 — 명부는 이름과 그 주인만 알고 어느 연결이 아직 붙어
//! 있는지를 모르는데, 그것을 아는 쪽이 데몬이다.
//!
//! ★끊긴 연결의 늦은 패킷을 막는 그물은 **여기 하나뿐**이다★(`Shared::refuse_if_detached`):
//! [`Roster`] 는 끊긴 주인의 등록을 지우므로(ADR-0144 결정 3) 그 주인의 죽음을 기억할 자리가 없고, 늦은
//! 등록·차분과 진짜 인수인계를 구분하지 못한다. 이 그물을 지우면 죽은 연결의 패킷이 통과해 그 이름이
//! **없는 주인**을 가리킨 채 데몬 수명 내내 `Available` 로 답한다(그 연결엔 다시 올 정리가 없다).
//! [`Roster`] 쪽에 같은 그물을 다시 세우지 말 것 — 주인 단위 상태를 따로 들면 그 목록이 자취와 똑같이
//! 무한히 자란다(ADR-0144 가 자취를 버린 것과 같은 이유).
//!
//! ★살아 있는지 확인과 명부 변경은 **같은 임계 구역** 안에서 일어난다★: 둘을 나누면 확인과 변경
//! 사이에 정리가 끼어들어, 이미 끊긴 연결의 등록이 그 뒤에 내려앉는다. `on_disconnect` 는 조용한
//! 시점이 아니라 **`on_text` 와 겹칠 수 있는** 시점이므로(`frame_port::ConnectionHandler` 계약 —
//! 네트워크 행은 abort 를 걸 뿐 완료를 기다리지 않는다) 그 겹침은 이론이 아니다.
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
/// ★상수로 두는 이유는 테스트다★: 같은 `CONFLICT` 를 [`Roster`] 의 남의 이름 검사도 내므로, 문구를 안
/// 보면 「연결 수명 그물이 잡았다」와 「명부 검사가 잡았다」가 구분되지 않는다 — 그러면 이 파일의 그물을
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
    ///
    /// ★이 파생은 **오늘의 잠정 규칙**이고 슬라이스 B 가 대체한다★: 주인 키는 **클라이언트가 만들어 첫
    /// 인사에 실어 보낸 식별자**가 되고(ADR-0144 결정 1·2), 연결 id 파생은 그 값을 안 보낸 연결의 fail-open
    /// 갈래로만 남는다(그 결정의 영향절). 그 식별자는 아직 코드에 없다 — 그래서 지금은 이 파생이 전부다.
    // ADR-0144
    pub fn owner_of(conn_id: ConnId) -> OwnerToken {
        OwnerToken::new(format!("{}{conn_id}", Self::OWNER_TOKEN_PREFIX))
    }

    /// 연결이 섰다 — 이 뒤로 그 연결의 등록이 받아들여진다.
    pub fn attach(&self, conn_id: ConnId) {
        self.lock().live.insert(conn_id);
    }

    /// 연결이 끊겼다 — 살아 있는 명단에서 빼고 그 주인의 이름을 명부에서 지운다. **둘은 한 잠금 안에서**
    /// 일어난다(겹쳐 도는 등록이 그 사이로 못 들어온다).
    ///
    /// ★제거 지점은 이 한 곳뿐이다★ — 끊김을 아는 경로가 `AgentConnections::on_disconnect` 하나이고, 두
    /// 번째 제거 지점을 만들면 인과가 갈라진다(ADR-0144).
    /// ★지운 것을 로그로 남긴다 — 이 줄이 그 사건의 **유일한 진단 표면**이다★: 명부에는 끊긴 주인의 자취가
    /// 남지 않으므로(ADR-0144 결정 3) 사라진 이름을 조회로 되짚을 길이 없다. 이 줄을 지우면 「어느 명령이 왜
    /// 없어졌나」에 답할 자료가 아무 데도 없다(반려 로그는 **거절된 패킷**만 말한다). 레벨은 연결 수명
    /// 사건이라 `info!` 다(`docs/reference/logging-conventions.md` 레벨 표 · 같은 문서 「계측 의무」의 연결
    /// 수명 항목). 내릴 이름이 없던 끊김만 `debug!` 로 내린다 — 지운 것이 없으면 이 줄이 말할 사건이 없고
    /// 연결 자체의 종료는 네트워크 행이 이미 남기는데, **등록하는 클라이언트가 아직 0건이라 그 갈래가
    /// 평상시 전부**다.
    /// ★로그는 잠금을 **놓은 뒤** 부른다★ — 파일 sink 가 동기 쓰기라(로깅 컨벤션 「인프라」) 임계 구역 안에서
    /// 부르면 등록·조회·다른 연결의 정리가 그 IO 만큼 함께 멈춘다.
    /// `help` 는 싣지 않는다 — 클라이언트가 실어 온 문자열이고 상한이 이름의 32배다(`Roster::MAX_HELP_BYTES`).
    ///
    /// ★알려진 어긋남 — 제거 단위는 **주인 토큰**인데 생존 명단(`Shared::live`)은 **[`ConnId`]** 다★:
    /// 오늘은 [`CommandRoster::owner_of`] 가 연결과 1:1 이라 무해하다. 슬라이스 B 가 주인 키를 클라이언트
    /// 자작 식별자로 바꿔 **여러 연결이 한 주인을 공유**하면 `detach(conn1)` 이 아직 산 `conn2` 의 등록까지
    /// 지우고, `conn2` 는 `Shared::refuse_if_detached` 를 계속 통과하므로 **자기 이름이 지워진 것을
    /// 모른다**(등록은 붙을 때 1회뿐이라 다시 얹을 계기가 없다). ★슬라이스 B 가 이것을 닫아야 한다★ — 지금
    /// 고치려면 제거 단위를 연결로 좁혀야 하는데 [`ConnId`] 는 도구 crate 가 모르는 타입이라, 고치는 행위
    /// 자체가 그 슬라이스의 주인 모델을 선결한다(사용자 결정 2026-08-17 — `docs/process/step-log.md` ㉲).
    // ADR-0144
    pub fn detach(&self, conn_id: ConnId) {
        let removed = {
            let mut shared = self.lock();
            shared.live.remove(&conn_id);
            shared.roster.disconnect(&Self::owner_of(conn_id))
        };
        if removed.is_empty() {
            tracing::debug!(conn = conn_id, "연결 끊김 — 명부에서 내릴 이름이 없다");
        } else {
            tracing::info!(
                conn = conn_id,
                names = removed.len(),
                removed = %removed.join(" "),
                "연결 끊김 — 명령 명부에서 이 주인의 이름을 지웠다"
            );
        }
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
    /// ★끊긴 연결의 늦은 패킷을 막는 **유일한** 그물이다★ — 근거와 「두 번째를 세우지 말 것」은 이 파일
    /// 헤더.
    ///
    /// ★코드 선택★: 패킷은 멀쩡하고(`INVALID_ARGUMENT` 아님) 부르는 쪽 잘못도 아니다 — 거절하는 것은
    /// **상태**(그 연결은 지금 명단에 없다)라서 `CONFLICT` 다. 그 코드의 재시도 지시는 `never` 다 — 같은
    /// 연결로 다시 보내 봐야 그 연결은 이미 없고, 다시 붙으면 **새 토큰**을 받으므로 재시도가 아니라
    /// 재등록이다.
    ///
    /// ★문구가 「끊겼다」로 좁지 않은 이유★: 같은 갈래가 **한 번도 붙은 적 없는** 연결에도 선다. 오늘
    /// 운영 경로에서는 안 나지만(`on_connect` 이 dispatch 보다 앞이라는 네트워크 행 순서) 타입이 막는
    /// 것은 아니다 — `register`/`update` 는 맨 [`ConnId`] 를 받는다.
    ///
    /// ★이 반려에 로그를 따로 남기지 않는다★ — 부르는 쪽(`connection_core` 의 `reply_roster`)이 명부 거절
    /// 전량을 `warn!`(conn·verb·code·문구)으로 이미 남긴다. 여기서 한 줄 더 내면 같은 사건이 두 번 적히고,
    /// 그 줄은 **잠금을 쥔 채** 나간다([`CommandRoster::detach`] 가 로그를 잠금 밖으로 뺀 것과 같은 이유).
    /// ★알려진 어긋남 — 이 판정 단위는 [`ConnId`] 인데 제거 단위는 주인 토큰이다★: 오늘은 둘이 1:1 이라
    /// 무해하나, 슬라이스 B 가 여러 연결이 한 주인을 공유하게 만들면 **자기 등록이 남의 `detach` 에 지워진
    /// 뒤에도 이 그물을 통과하는** 연결이 생긴다 — 그 연결은 자기 이름이 없어진 것을 모르고, 등록은 붙을 때
    /// 1회뿐이라 되돌릴 계기도 없다. 근거 정본과 「슬라이스 B 가 닫는다」는 [`CommandRoster::detach`] 주석.
    // ADR-0144
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
    // 네 갈래를 못으로 박는다: {등록·차분} × {아무도 안 쥔 이름·산 연결이 쥔 이름}. 명부에는 끊긴 주인의
    // 흔적이 없으므로(ADR-0144) 이 그물은 **연결 수명**으로만 설 수 있다.
    //
    // ★코드만 보면 안 된다 — 문구까지 본다★: 같은 `CONFLICT` 를 `Roster::check_added_are_not_taken` 도
    // 내므로, 코드만 단언하면 이 파일의 그물을 통째로 지워도 몇몇은 **다른 이유로** 초록을 유지한다.

    /// 이름이 되살아나는지까지 본다 — 정리가 지운 이름을 늦은 등록이 다시 얹으면, 그 이름은 **없는 주인**
    /// 앞으로 데몬 수명 내내 `Available` 이 된다(그 연결엔 다시 올 정리가 없다).
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
            OwnerLookup::Unknown,
            "정리가 지운 이름이 되살아나면 안 된다"
        );
        assert!(roster.entries().is_empty(), "명부에 남는 것이 없다");
    }

    /// ★명부만으로는 못 막는 갈래★ — 끊긴 주인의 등록은 지워지므로 `Roster` 에는 그 죽음을 기억할 자리가
    /// 없고, 늦은 등록이 산 연결의 이름을 인수인계로 이어받는 것처럼 통과한다. 여기서 막는 것은 연결
    /// 수명이다.
    #[test]
    fn a_late_registration_cannot_take_a_live_connections_name() {
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
    fn a_late_delta_cannot_take_a_live_connections_name() {
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

    /// ★명부 검사가 통째로 비켜가는 자리 — 여기선 연결 수명이 유일한 그물이다★
    ///
    /// 아무도 안 쥔 이름을 더하는 차분은 `Roster::check_added_are_not_taken` 이 통과시키고, 끊긴 주인을
    /// 가릴 검사는 명부에 아예 없다(ADR-0144). 그물이 빠지면 `tab.split` 이 **없는 주인** 앞으로
    /// `Available` 이 되어 데몬 수명 내내 굳는다.
    #[test]
    fn a_late_delta_adding_an_unclaimed_name_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1);
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2);
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 같은 이름을 새로 얹는다");

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

    /// ★끊김이 **무엇을 지웠는지** 로그가 말해야 한다★
    ///
    /// 자취를 남기지 않으므로(ADR-0144 결정 3) 사라진 이름은 **자료에 남지 않는다** — 조회로 되짚을 길이
    /// 없어 「어느 명령이 왜 없어졌나」에 답하는 표면이 이 한 줄뿐이다. 반려 로그(`connection_core` 의
    /// `reply_roster`)는 **거절된 패킷**만 말하므로 대체가 못 된다.
    ///
    /// 문구가 아니라 **레벨과 필드**를 본다: 연결 수명 사건이라 `info!` 이고
    /// (`docs/reference/logging-conventions.md` 레벨 표), 개수만으로는 어느 명령이 사라졌는지 못 짚으므로
    /// 이름까지 실린다.
    #[test]
    fn a_detach_logs_the_names_it_removed() {
        let roster = CommandRoster::new();
        roster.attach(7);
        roster
            .register(7, vec![decl("tab.create"), decl("tab.close")])
            .expect("등록");

        let logged = capture_info(|| roster.detach(7));

        assert!(logged.contains("conn=7"), "어느 연결인지: {logged:?}");
        assert!(
            logged.contains("tab.create") && logged.contains("tab.close"),
            "지운 이름이 실려야 한다: {logged:?}"
        );
    }

    /// 내릴 이름이 없던 끊김은 `info!` 를 내지 않는다 — 지운 것이 없으면 이 줄이 말할 사건도 없고, 연결
    /// 자체의 종료는 네트워크 행이 이미 남긴다. 등록하는 클라이언트가 아직 0건이라(TRD §3-7) 이 갈래가
    /// **평상시 전부**이므로, 여기서 안 가르면 기본 경로가 무의미한 줄로 덮인다.
    #[test]
    fn a_detach_with_nothing_to_remove_stays_quiet_at_info() {
        let roster = CommandRoster::new();
        roster.attach(7);

        let logged = capture_info(|| roster.detach(7));

        assert!(
            logged.is_empty(),
            "지운 것이 없으면 info 는 비어야 한다: {logged:?}"
        );
    }

    /// INFO 이벤트의 필드만 모으는 최소 수집기 — 포맷 레이어를 쓰지 않는 이유는 이 테스트가 보는 것이
    /// 「그 필드가 실린 INFO 이벤트가 났는가」 하나뿐이라서다(`control::tests` 의 같은 형태).
    /// `with_default` 는 **이 스레드에서만** 걸리므로 병렬 테스트와 섞이지 않는다.
    fn capture_info(body: impl FnOnce()) -> String {
        use tracing::subscriber;

        struct InfoCollector {
            lines: Arc<Mutex<Vec<String>>>,
        }
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", f.name(), v));
            }
        }
        impl subscriber::Subscriber for InfoCollector {
            fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
                *m.level() == tracing::Level::INFO
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut buf = String::new();
                event.record(&mut Visit(&mut buf));
                self.lines.lock().expect("lines poisoned").push(buf);
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        subscriber::with_default(
            InfoCollector {
                lines: lines.clone(),
            },
            body,
        );
        let captured = lines.lock().expect("lines poisoned");
        captured.join("\n")
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
