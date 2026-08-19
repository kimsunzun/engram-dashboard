//! 명령 주인 명부의 공유 핸들 — **전 연결이 같은 한 부를 본다**(ADR-0155/0156 · TRD §3-7).
//!
//! 명부의 규칙(등록 전량 last-wins · 차분 · 끊김 제거 · 상한)은 전부 도구 crate 의 [`Roster`] 가
//! 소유한다. 여기가 더하는 것은 **연결 수명과 그 주인에게 닿는 길** 둘이다 — 명부는 이름과 그 주인만
//! 알고 어느 연결이 아직 붙어 있는지도, 그 주인에게 어떻게 보내는지도 모르는데, 둘 다 아는 쪽이
//! 데몬이다(ADR-0154).
//!
//! ★끊긴 연결의 늦은 패킷을 막는 그물은 **여기 하나뿐**이다★(`Shared::refuse_if_detached`):
//! [`Roster`] 는 끊긴 주인의 등록을 지우므로(ADR-0150 결정 3) 그 주인의 죽음을 기억할 자리가 없고, 늦은
//! 등록·차분과 진짜 인수인계를 구분하지 못한다. 이 그물을 지우면 죽은 연결의 패킷이 통과해 그 이름이
//! **없는 주인**을 가리킨 채 데몬 수명 내내 `Available` 로 답한다(그 연결엔 다시 올 정리가 없다).
//! [`Roster`] 쪽에 같은 그물을 다시 세우지 말 것 — 주인 단위 상태를 따로 들면 그 목록이 자취와 똑같이
//! 무한히 자란다(ADR-0150 가 자취를 버린 것과 같은 이유).
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
//! 부르거나 표에서 핸들 사본을 뜬 뒤 즉시 놓는다 — **보내는 것은 호출자가 잠금 밖에서 한다**. 가드를
//! 밖으로 내보내면 답장을 기다리는 배달 하나가 등록·연결 정리·조회를 전부
//! 세운다(근거 정본 = [`engram_dashboard_command::OwnerLookupSource`] 주석).
//! ★알려진 비용★: [`CommandRoster::entries`] 는 잠근 채로 명부 전량을 복제한다 — 상한까지 찬 명부면
//! 그 구간이 짧지 않다. 줄이려면 [`Roster`] 가 빌려주는 모양을 바꿔야 해서 이 슬라이스 밖이다.
//! [`CommandRoster::sink_of`] 도 잠근 채로 표를 선형 훑는다 — 연결 수에 상한이 없고 배달마다 한 번씩
//! 도는 조회다(자세한 것은 그 자리 주석).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use engram_dashboard_command::{
    CommandDecl, CommandError, ErrorCode, OwnerLookup, OwnerLookupSource, OwnerToken, Roster,
    RosterEntry,
};
use engram_dashboard_net::frame_port::{ConnId, FrameSink};

use crate::connection_core::sanitize_for_log;

/// 끊김 한 줄에 **이름을 몇 개까지** 적나.
///
/// ★상한이 없으면 로그 줄 하나의 크기를 상대가 정한다★: 한 주인이 얹을 수 있는 이름은 512개
/// ([`Roster::MAX_NAMES_PER_OWNER`])이고 하나가 128 B 까지라(`Roster::MAX_NAME_BYTES`) 안 자르면 이 한
/// 줄이 64 KiB 가 된다 — 그 값 전부가 클라이언트가 고른 문자열이다.
/// ★자르는 것이 뭘 잃나★: **개수는 안 잃는다**(`names` 필드가 정확한 수를 따로 나른다). 잃는 것은
/// 「어느 이름이었나」의 뒤쪽이고, 진단에 쓰이는 것은 앞쪽 몇 개다.
const MAX_LOGGED_NAMES: usize = 8;

/// 클라이언트가 등록한 이름들을 로그 필드 하나로 — **모양은 손질기가, 개수는 [`MAX_LOGGED_NAMES`] 가** 묶는다.
///
/// ★같은 문으로 들어온 같은 값을 두 모양으로 적지 않는다★: 등록 반려가 이미 같은 형태를 골랐다
/// (`connection_core::ConnectionCore::refuse_names_i_answer` — `'이름'` 을 콤마로 잇는다). 한 값이 자리마다
/// 다른 모양이면 로그를 훑는 사람이 같은 것을 같은 것으로 못 본다.
fn logged_names(names: &[String]) -> String {
    let mut out = names
        .iter()
        .take(MAX_LOGGED_NAMES)
        .map(|name| format!("'{}'", sanitize_for_log(name)))
        .collect::<Vec<_>>()
        .join(", ");
    let elided = names.len().saturating_sub(MAX_LOGGED_NAMES);
    if elided > 0 {
        out.push_str(&format!(" (+{elided} more)"));
    }
    out
}

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
    /// 지금 붙어 있는 연결과 그 각각에 닿는 길. **명부와 한 잠금 아래** 있어야 확인과 변경 사이가
    /// 벌어지지 않는다.
    // ADR-0154
    live: BTreeMap<ConnId, LiveConn>,
}

/// 붙어 있는 연결 하나 — **누구의 것인가**와 **어떻게 닿는가**.
///
/// ★주인 토큰을 붙을 때 계산해 **저장한다** — 조회할 때 되짚지 않는다★: 오늘의 주인 토큰은
/// [`CommandRoster::owner_of`] 가 연결 id 에서 파생하지만 그 파생은 잠정 규칙이고 슬라이스 B 가
/// 클라이언트 자작 식별자로 대체한다(ADR-0150). 토큰 문자열에서 연결 id 를 되짚는 조회를 쓰면 그날
/// 조용히 빈손이 되고, 그 빈손은 「주인이 없다」와 구분되지 않는다. 저장해 두면 파생 규칙이 바뀌어도
/// 조회는 그대로 성립한다.
// ADR-0154
struct LiveConn {
    owner: OwnerToken,
    sink: Arc<dyn FrameSink>,
}

impl CommandRoster {
    /// 주인 토큰의 접두사 — 이 형식은 데몬 안에서만 난다(`CommandRoster::owner_of`).
    pub const OWNER_TOKEN_PREFIX: &'static str = "conn-";

    pub fn new() -> Self {
        Self::default()
    }

    /// 연결 하나의 주인 토큰 — **연결 id 에서 파생한다**. 파생 규칙은 여기 하나뿐이고, 부르는 곳은
    /// [`CommandRoster::attach`] 하나다(정책과 그 근거는
    /// `connection_core::ConnectionSession::owner_token`).
    ///
    /// ★등록·정리·조회는 이 함수가 아니라 attach 가 **저장한 값**을 본다★ — 그래야 이 파생이 바뀌는 날
    /// 세 쪽이 갈라지지 않는다(ADR-0154). 그 저장값을 밖에서 물으려면 [`CommandRoster::attached_owner`].
    ///
    /// ★이 파생은 **오늘의 잠정 규칙**이고 슬라이스 B 가 대체한다★: 주인 키는 **클라이언트가 만들어 첫
    /// 인사에 실어 보낸 식별자**가 되고(ADR-0150 결정 1·2), 연결 id 파생은 그 값을 안 보낸 연결의 fail-open
    /// 갈래로만 남는다(그 결정의 영향절). 그 식별자는 아직 코드에 없다 — 그래서 지금은 이 파생이 전부다.
    // ADR-0150
    pub fn owner_of(conn_id: ConnId) -> OwnerToken {
        OwnerToken::new(format!("{}{conn_id}", Self::OWNER_TOKEN_PREFIX))
    }

    /// 연결이 섰다 — 이 뒤로 그 연결의 등록이 받아들여지고, 그 주인 앞으로 온 봉투가 `frames` 로 나간다.
    ///
    /// ★`frames` 를 clone 해 든다★: 연결 수명 훅이 넘겨 주는 것은 **빌린 참조**라 그대로는 연결보다
    /// 오래 못 산다(`engram_dashboard_net::frame_port::ConnectionHandler::on_connect` 계약). 표에 두려면
    /// `'static` 공유 핸들이 필요하고, 그 clone 이 이 한 줄이다.
    ///
    /// ★닿는 길이 명부 등재보다 **먼저**다★: 이름은 등록 패킷으로 오르고 그 패킷은 이 호출 뒤에만
    /// 받아들여지므로([`Shared::refuse_if_detached`]), 명부에 오른 이름은 항상 닿을 수 있다. 순서가
    /// 뒤집히면 「명부엔 있는데 닿을 길이 없는」 이름이 서고 그 창은 런타임에 아무 신호도 내지 않는다
    /// (ADR-0154 삽입 순서).
    ///
    /// ★전제 — 같은 [`ConnId`] 로 두 번 부르지 않는다★: 두 번째가 첫 번째의 출구를 덮으므로, 먼저 붙은
    /// 연결이 얹은 이름의 봉투가 나중 연결로 간다. 그래서 **거절 로직을 두지 않고** 덮어쓰기 그대로 두되
    /// `error!` 한 줄을 남긴다.
    /// ★그 도달 불가는 **호출자가 둘**이라 두 수열이 함께 서야 성립한다★ — 한쪽만 적어 두면 반쪽 근거다:
    /// 소켓 연결은 네트워크 행의 단조 증가 카운터에서 나오고 재사용되지 않으며(`ws.rs` 의
    /// `ConnRegistry::alloc_id`), 제어 라우트의 가짜 호출자 연결은 `u64::MAX` 에서 **내려간다**
    /// (`command_delivery::CommandBus::open_caller` — 두 수열이 만나려면 2^63 번을 할당해야 한다는 셈이
    /// 그 함수 doc 에 있다). 두 발급기 중 하나라도 성질이 바뀌면 여기 전제가 함께 깨진다.
    /// ★여기서 패닉하지 않는다 — 되살리지 마라★: 이 함수는 `on_connect` 안에서 돈다. 거기서 죽으면
    /// 정리 훅도 레지스트리 해제도 통째로 건너뛴 채 죽은 큐가 팬아웃 대상으로 남고(그 사유의 정본 =
    /// 네트워크 행 `ws.rs` 의 같은 금지), 표에는 영영 안 지워지는 항목이 남아 [`CommandRoster::sink_of`]
    /// 가 **죽은 주인에게 `Some`** 을 답한다. `debug_assert` 도 같은 이유로 안 된다 — 릴리스에서 사라져
    /// 관측을 못 주면서 debug 에서만 그 사고를 만든다.
    ///
    /// ★파생은 여기 한 번뿐이다★ — 아래 [`CommandRoster::detach`]·[`CommandRoster::register`]·
    /// [`CommandRoster::update`] 는 [`CommandRoster::owner_of`] 를 다시 부르지 않고 여기서 저장한 값을
    /// 읽는다. 그래야 파생 규칙이 바뀌는 날 명부 등재·제거·조회가 **같은 토큰**을 본다(ADR-0154 재론
    /// 트리거 ②).
    // ADR-0154
    pub fn attach(&self, conn_id: ConnId, frames: &Arc<dyn FrameSink>) {
        // ★강참조는 잠금 **밖에서** 놓는다★ — 소멸자가 무엇을 하는지 이 표는 모른다(근거 = 이 파일 헤더).
        let replaced = {
            let mut shared = self.lock();
            shared.live.insert(
                conn_id,
                LiveConn {
                    owner: Self::owner_of(conn_id),
                    sink: frames.clone(),
                },
            )
        };
        // 덮인 항목이 곧 충돌의 증거다 — 그래서 잠금을 놓은 뒤에 찍어도 사건을 그대로 서술한다(로그를
        //   임계 구역 밖으로 빼는 근거는 [`CommandRoster::detach`] 와 같다).
        // ★갚아야 할 빚 — 이 토큰은 **다듬지 않고** 찍는다★: 오늘은 데몬이 만든 `conn-<id>` 라 길이도
        //   모양도 우리 것이다. 슬라이스 B 가 여기 **클라이언트가 보낸 식별자**를 저장하는 순간, 이 줄은
        //   검증 안 된 상대 문자열을 원문으로 찍는 자리가 된다 — 메가바이트짜리 로그 줄과 제어문자
        //   위조 항목이 그때 열린다. ★`attach` 가 저장하는 값을 바꾸는 커밋이 이 줄을 함께 고쳐야 한다★ —
        //   손질기는 같은 crate 안에 있어(`connection_core::sanitize_for_log` — 이 파일이 아래
        //   [`logged_names`] 에서 이미 쓴다) 그때 할 일은 그 함수를 부르는 것뿐이다.
        if let Some(previous) = &replaced {
            tracing::error!(
                conn = conn_id,
                previous_owner = %previous.owner,
                "같은 연결 id 로 두 번 붙었다 — 먼저 붙은 주인의 출구를 덮었다(연결 id 는 재사용되지 않는다)"
            );
        }
        drop(replaced);
    }

    /// 그 연결이 붙을 때 저장한 주인 토큰. **부재 = 지금 붙어 있지 않다**(그래서 이 연결 앞으로는 어떤
    /// 이름도 설 수 없다 — [`Shared::refuse_if_detached`]).
    ///
    /// ★명부의 주인이 **누구인가**를 묻는 곳은 여기다★ — [`CommandRoster::owner_of`] 를 밖에서 다시 부르면
    /// 그 파생이 바뀌는 날 물은 값과 저장된 값이 갈라진다(ADR-0154).
    ///
    /// ★답은 **잠금을 놓는 순간 낡는다** — 판정 경로에 놓지 마라★: 이 호출과 그다음 호출 사이에 그 연결의
    /// `detach` 가 낄 수 있다(겹침이 실재하는 근거는 이 파일 헤더). 그러니 「붙어 있으니 등록이 통과할
    /// 것」·「붙어 있으니 봉투가 나갈 것」처럼 **읽은 값으로 뒷일을 결정하면** 안 된다 — 그 결정들은
    /// [`CommandRoster::register`]·[`CommandRoster::sink_of`] 안에서 잠금과 함께 원자적으로 난다.
    /// 오늘 유일한 소비자(`connection_core` 의 `note_claimed_owner`)가 안전한 이유도 그것이다: 고르는 것이
    /// **로그 갈래**뿐이고, 등록의 성사 여부는 여전히 [`Shared::refuse_if_detached`] 가 한 임계 구역 안에서
    /// 정한다.
    // ADR-0154
    pub fn attached_owner(&self, conn_id: ConnId) -> Option<OwnerToken> {
        self.lock()
            .live
            .get(&conn_id)
            .map(|conn| conn.owner.clone())
    }

    /// 주인 토큰이 지금 붙어 있는 연결의 프레임 출구. **부재(`None`) = 지금 붙어 있는 어느 연결도 그
    /// 토큰을 들고 있지 않다** — 이 함수가 아는 것은 그게 전부라 「한 번도 붙은 적 없다」와 「왕복 도중
    /// 끊겼다」를 **구분하지 못한다**. 조회가 `Available` 을 답한 뒤 여기서 부재가 나오는 경합에서
    /// 호출자는 `OUTCOME_UNKNOWN` 으로 답한다(ADR-0154). ★소유자 부재로 접지 않는다 — 그건 거짓
    /// 확신이다★(같은 결의 선례 = 도구 crate `route.rs` 의 `forwarding_unknown` 과 상관 키 불일치 갈래).
    ///
    /// ★핸들 사본만 내보내고 잠금은 여기서 끝난다★ — 보내는 것은 호출자가 잠금 밖에서 한다(ADR-0006 ·
    /// 이 파일 헤더). 가드째 내보내면 답장을 기다리는 배달 하나가 등록·연결 정리·조회를 전부 세운다.
    ///
    /// ★조회는 **저장된 토큰과의 동등 비교**다★ — 토큰에서 연결 id 를 되짚지 않는다(근거 = [`LiveConn`]).
    ///
    /// ★받은 핸들은 **즉시 쓰고 버린다** — 대기표·상관 표에 보관하지 않는다★: 명부는 내보낸 사본을
    /// 회수할 수단이 없으므로, 보관하는 순간 그 연결의 송신 큐가 명부보다 오래 살아남는다(살아남은
    /// `Sender` 사본이 무엇을 깨는지는 네트워크 행 `ws.rs` 의 「사본 전수」가 적는다).
    ///
    /// ★한 주인에 연결이 여럿이면 **어느 것이 나오는지 보장하지 않는다**★: 오늘은
    /// [`CommandRoster::owner_of`] 가 연결과 1:1 이라 도달 불가고, 슬라이스 B 가 주인 키를 클라이언트
    /// 자작 식별자로 바꾸면 도달 가능해진다. 그때의 선택 규칙은 그 슬라이스 몫이라 여기서 특정 승자를
    /// 계약으로 굳히지 않는다 — 지금 나오는 순서는 표의 순회 순서일 뿐 약속이 아니다(같은 뿌리의
    /// 어긋남 = [`CommandRoster::detach`] 의 「알려진 어긋남」).
    ///
    /// ★알려진 비용★: 잠근 채로 표를 **선형 훑는다**. 붙는 연결 수에 상한이 없고(네트워크 행 수락
    /// 루프에 동시 연결 제한이 없다) 배달은 명령마다 이 조회를 한 번씩 하므로, 그 둘이 함께 커지면
    /// 여기가 등록·연결 정리와 같은 잠금을 두고 다툰다. 역인덱스는 이 슬라이스 밖이다.
    // ADR-0154
    pub fn sink_of(&self, owner: &OwnerToken) -> Option<Arc<dyn FrameSink>> {
        self.lock()
            .live
            .values()
            .find(|conn| conn.owner == *owner)
            .map(|conn| conn.sink.clone())
    }

    /// **연결 하나**의 프레임 출구 — 배달이 답장을 되돌릴 때 쓴다(형제 [`CommandRoster::sink_of`] 는 주인
    /// 토큰으로 찾는다).
    ///
    /// ★두 조회를 하나로 합치지 않는 이유★: 나가는 봉투의 목적지는 **주인**이고 돌아오는 답장의 목적지는
    /// **물어본 연결**이라 키가 다르다. 오늘 둘이 1:1 이라 같은 표를 보지만, 슬라이스 B 가 주인 키를
    /// 클라이언트 자작 식별자로 바꾸면 한 주인에 연결이 여럿이 되고 그때 답장이 엉뚱한 연결로 간다.
    ///
    /// ★받은 핸들은 즉시 쓰고 버린다★ — 보관 금지의 근거는 [`CommandRoster::sink_of`] 와 같다.
    /// **부재 = 그 연결이 이미 끊겼다**(답장을 낼 곳이 없다는 뜻이고, 오류가 아니다).
    ///
    /// ★[`ConnId`] 하나로 지목하는 것이 안전한 근거는 **할당기가 그 값을 재사용하지 않는다**는 사실
    /// 뿐이다★ — 세대 번호가 없으므로, 재사용이 생기는 날 오래 남은 상관 표 항목이 같은 번호를 새로 받은
    /// 다른 피어에게 남의 답장을 건넨다. 실측과 그때의 처방은 `command_delivery` 의 `Pending::origin`.
    // ADR-0154
    pub fn sink_for_conn(&self, conn_id: ConnId) -> Option<Arc<dyn FrameSink>> {
        self.lock().live.get(&conn_id).map(|conn| conn.sink.clone())
    }

    /// 그 연결이 아직 붙어 있나 — ★출구 **사본을 만들지 않는** 조회다★.
    ///
    /// 살아 있는지만 알면 되는 자리(부수효과를 내기 전 호출자 확인)가 형제 [`CommandRoster::sink_for_conn`]
    /// 을 쓰면 강참조 하나가 그 자리의 수명만큼 산다. 그 자리가 수 초짜리 blocking 본문이면 사본이 그동안
    /// 살아 있어 **끊긴 연결의 writer 가 자기 종료를 못 한다**(ADR-0154 의 「내준 사본을 왕복 너머로 들지
    /// 않는다」 — 그 문장이 막는 누수 그대로다). 그래서 판정만 필요한 호출자는 이쪽을 쓴다.
    /// ★`ConnId` 는 재사용되지 않는다★ — 이 술어가 「같은 피어인가」로도 읽히는 근거이고, 그 실측의 정본은
    /// `command_delivery` 의 `Pending::origin`.
    // ADR-0154
    pub fn is_attached(&self, conn_id: ConnId) -> bool {
        self.lock().live.contains_key(&conn_id)
    }

    /// 시험 전용 — 저장된 주인 토큰만 갈아 끼워 **찢어진 창**(명부 조회는 `Available` 인데 닿는 길이 없다)을
    /// 결정적으로 만든다. 운영 경로에서 그 상태는 조회와 전달 사이에 낀 `detach` 로만 생겨 손으로 못 세운다.
    #[cfg(test)]
    pub(crate) fn overwrite_stored_owner(&self, conn_id: ConnId, owner: OwnerToken) {
        self.lock()
            .live
            .get_mut(&conn_id)
            .expect("붙어 있는 연결이어야 한다")
            .owner = owner;
    }

    /// 연결이 끊겼다 — 그 주인의 이름을 명부에서 지우고 그 연결에 닿는 길을 거둔다. **둘은 한 잠금
    /// 안에서** 일어난다(겹쳐 도는 등록이 그 사이로 못 들어온다).
    ///
    /// ★ADR-0154 이 못 박은 제거 순서(명부가 먼저, 닿는 길이 나중)를 그대로 쓴다★ — 오늘은 둘이 한
    /// 잠금 안이라 「명부엔 있는데 닿을 수 없는」 중간 상태를 아무도 관측하지 못하지만, 순서를 코드에
    /// 남겨 두어야 표가 잠금 밖으로 갈리는 날 그 창이 열리지 않는다.
    ///
    /// ## ★부르는 자리는 둘이고, 그 둘은 **다른 종류의 연결**을 거둔다★
    ///
    /// ① **실 연결의 끊김** — `AgentConnection::on_disconnect` 하나뿐이다. 끊김을 아는 경로가 그것 하나라
    ///    이 종류의 두 번째 제거 지점을 만들면 인과가 갈라진다(ADR-0150). 그 조항은 그대로 유효하다.
    /// ② **가짜 호출자 연결의 왕복 끝** — `command_delivery::CallerSeat` 의 소멸자(ADR-0160). 제어 라우트
    ///    호출 하나가 답장을 받으려고 세우는 연결이고, **`on_disconnect` 에 영영 닿지 않는다**(소켓이 없다).
    ///
    /// ★그래서 ①의 불변식을 「전체 하나」로 되돌려 적지 말 것★ — 그 문장은 이제 거짓이고, 거짓인 채로
    /// 두면 ②를 지우는 편집이 「단일 제거 지점」을 근거로 정당해 보인다. ②를 지우면 그 칸은 프로세스
    /// 수명 내내 남아 [`CommandRoster::sink_of`] 의 선형 훑기를 영구히 늘린다.
    /// ★카브아웃이 성립하는 근거 = **두 종류가 서로의 항목을 못 건드린다**★: 가짜 연결의 번호는 `u64::MAX`
    /// 에서 내려가고 실 연결은 1부터 올라가며(그 발급기 doc), 가짜 연결은 이름을 등록하지 않아 지울 명부
    /// 항목이 아예 없다. 즉 ②는 `live` 에서 자기 칸 하나를 빼는 것 이상을 하지 않는다.
    /// ★지운 것을 로그로 남긴다 — 이 줄이 그 사건의 **유일한 진단 표면**이다★: 명부에는 끊긴 주인의 자취가
    /// 남지 않으므로(ADR-0150 결정 3) 사라진 이름을 조회로 되짚을 길이 없다. 이 줄을 지우면 「어느 명령이 왜
    /// 없어졌나」에 답할 자료가 아무 데도 없다(반려 로그는 **거절된 패킷**만 말한다). 레벨은 연결 수명
    /// 사건이라 `info!` 다(`docs/reference/logging-conventions.md` 레벨 표 · 같은 문서 「계측 의무」의 연결
    /// 수명 항목). 내릴 이름이 없던 끊김만 `debug!` 로 내린다 — 지운 것이 없으면 이 줄이 말할 사건이 없고
    /// 연결 자체의 종료는 네트워크 행이 이미 남기는데, **등록하는 클라이언트가 아직 0건이라 그 갈래가
    /// 평상시 전부**다.
    /// ★로그는 잠금을 **놓은 뒤** 부른다★ — 파일 sink 가 동기 쓰기라(로깅 컨벤션 「인프라」) 임계 구역 안에서
    /// 부르면 등록·조회·다른 연결의 정리가 그 IO 만큼 함께 멈춘다.
    /// `help` 는 싣지 않는다 — 클라이언트가 실어 온 문자열이고 상한이 이름의 32배다(`Roster::MAX_HELP_BYTES`).
    ///
    /// ★알려진 어긋남 — 제거 단위는 **주인 토큰**인데 생존 표(`Shared::live`)의 키는 **[`ConnId`]** 다★:
    /// 오늘은 [`CommandRoster::owner_of`] 가 연결과 1:1 이라 무해하다. 슬라이스 B 가 주인 키를 클라이언트
    /// 자작 식별자로 바꿔 **여러 연결이 한 주인을 공유**하면 `detach(conn1)` 이 아직 산 `conn2` 의 등록까지
    /// 지우고, `conn2` 는 `Shared::refuse_if_detached` 를 계속 통과하므로 **자기 이름이 지워진 것을
    /// 모른다**(등록은 붙을 때 1회뿐이라 다시 얹을 계기가 없다). ★슬라이스 B 가 이것을 닫아야 한다★ — 지금
    /// 고치려면 제거 단위를 연결로 좁혀야 하는데 [`ConnId`] 는 도구 crate 가 모르는 타입이라, 고치는 행위
    /// 자체가 그 슬라이스의 주인 모델을 선결한다(사용자 결정 2026-08-17 — `docs/process/step-log.md` ㉲).
    ///
    /// ★지울 주인은 **표에서 읽는다** — 여기서 다시 파생하지 않는다★: 파생하면 [`CommandRoster::attach`]
    /// 가 저장한 토큰과 어긋나는 날 `disconnect` 가 아무것도 못 지우고, 그 이름은 **죽은 주인 앞으로**
    /// 데몬 수명 내내 `Available` 로 남는다(ADR-0154). 표에 없는 연결이면 지울 것도 없다 — 등록 자체가
    /// [`Shared::refuse_if_detached`] 를 통과해야만 서기 때문이다.
    // ADR-0150
    // ADR-0154
    pub fn detach(&self, conn_id: ConnId) {
        // ★거둔 항목을 잠금 **밖에서** 놓는다★: 여기가 프레임 출구의 마지막 강참조일 수 있고(등록만 하고
        //   아무 에이전트도 구독하지 않은 클라이언트가 그렇다), 그러면 채널 닫기·waker 깨우기가 임계 구역
        //   안에서 돈다. 이 파일 헤더가 약속한 「잠그고 한 번 부른 뒤 즉시 놓는다」 밖의 일이다.
        let (removed, reclaimed) = {
            // ★잠금 실패에 패닉하지 않는다 — 이 함수의 호출자 둘이 **전부 정리 경로**다★(위 ①②). 그중
            //   ②는 소멸자라, 여기서 패닉하면 되감기 중이면 곧바로 abort 이고 그렇지 않아도 그 소멸자의
            //   **다음 줄**(상관 표 정리)이 통째로 안 돌아 두 표가 함께 샌다. 그리고 이 함수가 하는 일은
            //   **제거뿐**이라 오염된 표를 계속 쓰는 위험도 형제 동사들(등록·조회)보다 작다 —
            //   형제는 [`CommandRoster::lock`] 의 패닉을 그대로 진다.
            //   짝 규율의 정본 = `command_delivery::CommandDeliveries::lock_for_cleanup`.
            let mut shared = self.lock_for_cleanup();
            let owner = shared.live.get(&conn_id).map(|conn| conn.owner.clone());
            let removed = match owner {
                Some(owner) => shared.roster.disconnect(&owner),
                None => Vec::new(),
            };
            let reclaimed = shared.live.remove(&conn_id);
            (removed, reclaimed)
        };
        drop(reclaimed);
        if removed.is_empty() {
            tracing::debug!(conn = conn_id, "연결 끊김 — 명부에서 내릴 이름이 없다");
        } else {
            tracing::info!(
                conn = conn_id,
                names = removed.len(),
                removed = %logged_names(&removed),
                "연결 끊김 — 명령 명부에서 이 주인의 이름을 지웠다"
            );
        }
    }

    /// 붙을 때의 전량 등록. 끊긴 연결의 늦은 패킷은 `CONFLICT` 로 반려한다. 이름이 서는 주인은
    /// [`CommandRoster::attach`] 가 저장한 토큰이다 — 여기서 다시 파생하지 않는다(ADR-0154).
    pub fn register(&self, conn_id: ConnId, decls: Vec<CommandDecl>) -> Result<(), CommandError> {
        let mut shared = self.lock();
        let owner = shared.refuse_if_detached(conn_id)?;
        shared.roster.register(&owner, decls)
    }

    /// 붙어 있는 동안의 차분. 반려 규칙과 주인 토큰의 출처는 [`CommandRoster::register`] 와 같다.
    pub fn update(
        &self,
        conn_id: ConnId,
        added: Vec<CommandDecl>,
        removed: Vec<String>,
    ) -> Result<(), CommandError> {
        let mut shared = self.lock();
        let owner = shared.refuse_if_detached(conn_id)?;
        shared.roster.update(&owner, added, removed)
    }

    /// 명부 전량의 스냅샷. [`Roster::entries`] 의 iterator 를 그대로 내보내면 호출자가 순회하는 동안
    /// 락이 잡혀 있으므로 여기서 걷어 낸다.
    pub fn entries(&self) -> Vec<RosterEntry> {
        self.lock().roster.entries().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.lock().expect("command roster poisoned")
    }

    /// 잠금 실패에 패닉하지 않는 획득 — ★**정리 경로 전용**★. 유일한 호출자가
    /// [`CommandRoster::detach`] 이고, 사유는 그 함수 안 주석이다.
    ///
    /// ★릴리스에서는 이 갈래에 닿지 못한다(알려진 범위)★ — 워크스페이스 `[profile.release]` 가
    /// `panic = "abort"` 라 오염이 아예 생기지 않는다. 그래도 두는 이유는 debug·테스트 빌드에서 그 갈래가
    /// 실재하고, 거기서 새는 표가 그대로 회귀 시험의 관측을 망치기 때문이다.
    fn lock_for_cleanup(&self) -> std::sync::MutexGuard<'_, Shared> {
        match self.inner.lock() {
            Ok(shared) => shared,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// 배달 3단계의 2단계가 명부에 묻는 **한 가지**(`engram_dashboard_command::route`).
///
/// ★잠금은 이 호출 안에서 끝난다 — 그것이 이 trait 의 존재 이유다★: 참조를 통째로 넘기면 배달이 답장을
/// 기다리는 내내 명부가 잠겨 등록·연결 정리·다른 배달이 전부 선다(근거 정본 = 그 trait 주석).
// ADR-0154
impl OwnerLookupSource for CommandRoster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        self.lock().roster.lookup(name)
    }
}

impl Shared {
    /// ★끊긴 연결의 늦은 패킷을 막는 **유일한** 그물이다★ — 근거와 「두 번째를 세우지 말 것」은 이 파일
    /// 헤더.
    ///
    /// 통과하면 그 연결이 붙을 때 저장한 **주인 토큰**을 낸다 — 부르는 쪽이 다시 파생하지 않게 하려는
    /// 것이다(파생 지점은 [`CommandRoster::attach`] 하나. ADR-0154). 반려 갈래에서는 토큰이 없고, 그것이
    /// 곧 이 그물의 뜻이다.
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
    // ADR-0150
    fn refuse_if_detached(&self, conn_id: ConnId) -> Result<OwnerToken, CommandError> {
        match self.live.get(&conn_id) {
            Some(conn) => Ok(conn.owner.clone()),
            None => Err(CommandError::of(ErrorCode::Conflict, DETACHED_REFUSAL)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doubles::FakeFrameSink;
    use engram_dashboard_command::OwnerLookup;
    use engram_dashboard_net::frame_port::{Frame, FrameError};
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: "{}".to_string(),
        }
    }

    /// 표에 들 출구 하나와 **그 출구가 닿는 곳**. 핸들끼리를 `Arc` 포인터로 견주지 않는 이유는 이 표가
    /// 지키는 것이 「그 핸들로 보내면 그 연결로 간다」이지 「같은 객체가 나왔다」가 아니라서다.
    fn sink_with_inbox() -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel::<Frame>(4);
        (Arc::new(FakeFrameSink::new(tx)), rx)
    }

    /// 닿는 길을 보지 않는 테스트가 자리만 채울 때 쓴다.
    fn some_sink() -> Arc<dyn FrameSink> {
        sink_with_inbox().0
    }

    /// 소멸 시점에 **명부 잠금이 풀려 있었는지**를 기록하는 출구. 프레임은 버린다 — 이 더블이 보는 것은
    /// 배달이 아니라 자기가 언제 죽는가다.
    struct LockProbeSink {
        roster: CommandRoster,
        unlocked_at_drop: Arc<AtomicBool>,
    }

    impl FrameSink for LockProbeSink {
        fn try_send(&self, _frame: Frame) -> Result<(), FrameError> {
            Ok(())
        }

        fn send(&self, _frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl Drop for LockProbeSink {
        fn drop(&mut self) {
            let unlocked = self.roster.inner.try_lock().is_ok();
            self.unlocked_at_drop.store(unlocked, Ordering::Release);
        }
    }

    /// 핸들에 표식을 흘려 어느 받는 곳으로 나갔는지 가른다.
    fn ping(sink: &Arc<dyn FrameSink>, mark: &str) {
        sink.try_send(Frame::Text(mark.to_string()))
            .expect("가짜 출구는 비어 있다");
    }

    fn delivered(rx: &mut mpsc::Receiver<Frame>) -> Option<String> {
        match rx.try_recv() {
            Ok(Frame::Text(text)) => Some(text),
            Ok(other) => panic!("Text 여야 함: {other:?}"),
            Err(_) => None,
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
    // 흔적이 없으므로(ADR-0150) 이 그물은 **연결 수명**으로만 설 수 있다.
    //
    // ★코드만 보면 안 된다 — 문구까지 본다★: 같은 `CONFLICT` 를 `Roster::check_added_are_not_taken` 도
    // 내므로, 코드만 단언하면 이 파일의 그물을 통째로 지워도 몇몇은 **다른 이유로** 초록을 유지한다.

    /// 이름이 되살아나는지까지 본다 — 정리가 지운 이름을 늦은 등록이 다시 얹으면, 그 이름은 **없는 주인**
    /// 앞으로 데몬 수명 내내 `Available` 이 된다(그 연결엔 다시 올 정리가 없다).
    #[test]
    fn a_registration_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
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
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
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
        roster.attach(1, &some_sink());
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
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
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
    /// 가릴 검사는 명부에 아예 없다(ADR-0150). 그물이 빠지면 `tab.split` 이 **없는 주인** 앞으로
    /// `Available` 이 되어 데몬 수명 내내 굳는다.
    #[test]
    fn a_late_delta_adding_an_unclaimed_name_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
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
    /// 자취를 남기지 않으므로(ADR-0150 결정 3) 사라진 이름은 **자료에 남지 않는다** — 조회로 되짚을 길이
    /// 없어 「어느 명령이 왜 없어졌나」에 답하는 표면이 이 한 줄뿐이다. 반려 로그(`connection_core` 의
    /// `reply_roster`)는 **거절된 패킷**만 말하므로 대체가 못 된다.
    ///
    /// 문구가 아니라 **레벨과 필드**를 본다: 연결 수명 사건이라 `info!` 이고
    /// (`docs/reference/logging-conventions.md` 레벨 표), 개수만으로는 어느 명령이 사라졌는지 못 짚으므로
    /// 이름까지 실린다.
    #[test]
    fn a_detach_logs_the_names_it_removed() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());
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

    /// ★이 줄의 **모양**을 상대가 정하지 못한다★ — 개행 하나면 로그 줄이 둘로 쪼개져 뒤 줄이 우리가 찍은
    /// 항목처럼 보인다(위조 항목).
    ///
    /// 명부는 이름의 **바이트 길이만** 보고 문자셋을 안 보므로(도구 crate `Roster` 의 등록 검사) 개행이 든
    /// 이름은 실제로 등록된다 — 이 시험의 전제가 그것이다.
    /// ★이름을 **하나만** 얹는다 — 이 축을 개수 축과 한 시험에 합치지 말 것★: 실제로 합쳐 봤더니
    /// [`MAX_LOGGED_NAMES`] 를 넘겨 채운 순간 위조 이름이 이름순 정렬에서 **잘리는 쪽에 들어가** 손질기를
    /// 걷어내도 초록이었다(즉 그 형태는 모양 축을 한 번도 재지 않는다). 개수 축은 아래 형제가 잰다.
    #[test]
    fn a_control_character_in_a_removed_name_never_reaches_the_log_field_verbatim() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let forged = "tab.forged\n2026-08-19T00:00:00Z  WARN forged entry";
        roster.register(9, vec![decl(forged)]).expect("등록");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains("tab.forged"),
            "전제: 그 이름이 이 줄에 실렸다: {logged:?}"
        );
        assert!(
            !logged.contains('\n'),
            "개행이 원문으로 실렸다 — 로그 줄이 쪼개진다: {logged:?}"
        );
    }

    /// ★이 줄의 **크기**를 상대가 정하지 못한다★ — 한 주인이 512개까지 얹을 수 있어(도구 crate 의
    /// `Roster::MAX_NAMES_PER_OWNER`) 안 자르면 이 한 줄이 64 KiB 가 된다.
    ///
    /// ★`names` 는 자르지 않는다★ — 정확한 개수가 남아야 「몇 개가 사라졌나」가 안 흐려진다. 그래서 잘리는
    /// 것은 「어느 이름이었나」의 뒤쪽뿐이고, 그 사실도 줄에 적힌다.
    #[test]
    fn the_removed_names_line_says_how_many_it_elided() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let total = MAX_LOGGED_NAMES + 3;
        let decls: Vec<CommandDecl> = (0..total).map(|i| decl(&format!("tab.n{i:02}"))).collect();
        roster.register(9, decls).expect("등록");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains(&format!("names={total}")),
            "개수는 정확해야 한다: {logged:?}"
        );
        assert!(
            logged.contains(&format!("(+{} more)", total - MAX_LOGGED_NAMES)),
            "자른 사실을 말해야 한다: {logged:?}"
        );
        assert!(
            !logged.contains(&format!("tab.n{:02}", total - 1)),
            "마지막 이름은 잘려 나가야 한다: {logged:?}"
        );
        // 넉넉한 상한으로 「상대가 크기를 정하지 못한다」만 못박는다 — 정확한 수를 박으면 필드 하나 늘 때마다
        //   깨진다.
        assert!(
            logged.len() < 1024,
            "줄 크기를 상대가 정한다: {} bytes",
            logged.len()
        );
    }

    /// 내릴 이름이 없던 끊김은 `info!` 를 내지 않는다 — 지운 것이 없으면 이 줄이 말할 사건도 없고, 연결
    /// 자체의 종료는 네트워크 행이 이미 남긴다. 등록하는 클라이언트가 아직 0건이라(TRD §3-7) 이 갈래가
    /// **평상시 전부**이므로, 여기서 안 가르면 기본 경로가 무의미한 줄로 덮인다.
    #[test]
    fn a_detach_with_nothing_to_remove_stays_quiet_at_info() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());

        let logged = capture_info(|| roster.detach(7));

        assert!(
            logged.is_empty(),
            "지운 것이 없으면 info 는 비어야 한다: {logged:?}"
        );
    }

    /// 붙은 적 없는 연결의 정리 — 표에 항목이 없으니 읽을 주인도 지울 이름도 없다. 이 갈래는 `detach` 가
    /// 주인 토큰을 **표에서 읽게** 되면서 생겼다. 조용해야 하고, 무엇보다 산 연결의 이름을 건드리면 안
    /// 된다.
    #[test]
    fn a_detach_for_a_connection_that_never_attached_is_quiet() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");

        let logged = capture_info(|| roster.detach(2));

        assert!(
            logged.is_empty(),
            "지울 것이 없으면 info 는 비어야 한다: {logged:?}"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(1)),
            "남의 정리가 산 연결의 이름을 건드리면 안 된다"
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
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("재연결은 새 토큰으로 온다");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    // ── 주인 토큰 → 그 주인에게 닿는 길(ADR-0154) ──────────────────────────────────
    //
    // 배달이 밟는 순서 그대로 잰다: 이름으로 명부에 물어 주인을 얻고(`lookup`), 그 주인으로 출구를
    // 얻는다(`sink_of`). 그래서 **조회 키는 명부가 실제로 쥔 값**이지 시험이 손으로 지어낸 문자열이
    // 아니다.

    #[test]
    fn an_attached_owner_resolves_to_that_connections_sink() {
        let roster = CommandRoster::new();
        let (sink, mut inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        roster.register(1, vec![decl("tab.create")]).expect("등록");

        let owner = match lookup(&roster, "tab.create") {
            OwnerLookup::Available(owner) => owner,
            other => panic!("주인이 있어야 함: {other:?}"),
        };
        let found = roster
            .sink_of(&owner)
            .expect("붙어 있는 주인은 닿는 길이 있다");
        ping(&found, "envelope");

        assert_eq!(
            delivered(&mut inbox),
            Some("envelope".to_string()),
            "그 연결의 출구로 나가야 한다"
        );
    }

    /// 끊긴 주인은 닿는 길이 없다 — 그리고 ★안 보이는 것으로는 부족하고 **놓아야** 한다★.
    ///
    /// 그래서 조회의 부재만이 아니라 강참조가 실제로 사라졌는지까지 잰다. 묘비만 남기고 `Arc` 를 쥐고
    /// 있는 구현은 앞의 단언만으로는 초록이다. 그런 구현이 깨는 것 둘: 채널 할당과 `Notify` 가 데몬
    /// 수명 내내 남고 — 그리고 이쪽이 하류에 중한데 — **살아남은 표 항목은 죽은 주인에게
    /// [`CommandRoster::sink_of`] 가 `Some` 을 답하게 만들어**, 배달이 정직한 「주인 없음」 대신 조용한
    /// 블랙홀이 된다.
    /// (writer task 의 자기종료와는 무관하다 — 네트워크 행은 `on_disconnect` **앞에서** 진 task 에
    /// `abort()` 를 걸므로 명부의 사본이 writer 를 좀비로 만들지는 못한다.)
    #[test]
    fn a_detached_owner_releases_its_sink() {
        let roster = CommandRoster::new();
        let (sink, _inbox) = sink_with_inbox();
        let reclaimed = Arc::downgrade(&sink);
        roster.attach(1, &sink);
        drop(sink); // 이제 표가 유일한 강참조다.

        roster.detach(1);

        assert!(roster.sink_of(&CommandRoster::owner_of(1)).is_none());
        assert!(
            reclaimed.upgrade().is_none(),
            "표가 핸들을 실제로 놓아야 한다 — 안 보이기만 하면 묘비다"
        );
        assert!(
            !roster.lock().live.contains_key(&1),
            "항목 자체가 사라져야 한다 — 핸들만 비운 묘비를 남기면 연결이 오갈수록 표가 무한히 자란다"
        );
    }

    /// ★거둔 핸들의 소멸자가 **임계 구역 밖**에서 돈다★ — `detach` 의 제거를 잠금 블록 안으로 되돌리면
    /// 소멸자가 그 안에서 돈다. 그 자리에서 도는 코드가 무엇을 하는지 명부는 모르고(`dyn` 이다), 이 파일
    /// 헤더가 약속한 「잠그고 한 번 부른 뒤 즉시 놓는다」 밖의 일이다. 그런 되돌림은 저장소 어디도 빨개지지
    /// 않으므로 이 테스트가 그 유일한 그물이다.
    ///
    /// 재는 방법: 소멸자가 그 시점에 명부 잠금을 잡아 본다. 같은 스레드가 이미 쥐고 있으면 `try_lock` 은
    /// `WouldBlock` 이라 **교착 없이** 갈린다.
    #[test]
    fn the_reclaimed_sink_is_dropped_outside_the_lock() {
        let roster = CommandRoster::new();
        let unlocked_at_drop = Arc::new(AtomicBool::new(false));
        let sink: Arc<dyn FrameSink> = Arc::new(LockProbeSink {
            roster: roster.clone(),
            unlocked_at_drop: unlocked_at_drop.clone(),
        });
        roster.attach(1, &sink);
        drop(sink); // 표가 유일한 강참조 — 소멸자는 `detach` 가 돌린다.

        roster.detach(1);

        assert!(
            unlocked_at_drop.load(Ordering::Acquire),
            "소멸자가 명부 잠금을 쥔 채 돌았다"
        );
    }

    /// ★저장된 토큰이 파생형이 **아닌** 상태에서도 등재·조회·제거가 전부 그 값을 쓰는지 잰다★
    ///
    /// 셋 중 하나라도 [`CommandRoster::owner_of`] 로 재파생하는 구현은 여기서 빨개진다 — 재파생하는
    /// `register` 는 이름을 `conn-1` 앞에 세우고, 재파생하는 `detach` 는 아무것도 못 지운다.
    /// ★슬라이스 B 를 선결하지 않는다★: 바꾸는 것은 표에 든 값 하나이고 [`CommandRoster::attach`] 의
    /// 시그니처가 아니다 — 운영 경로는 여전히 토큰을 스스로 계산한다.
    #[test]
    fn the_roster_files_and_removes_under_the_stored_token() {
        let roster = CommandRoster::new();
        let (sink, _inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        let stored = OwnerToken::new("shell-a1");
        roster.lock().live.get_mut(&1).expect("붙어 있다").owner = stored.clone();

        roster.register(1, vec![decl("tab.create")]).expect("등록");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(stored.clone()),
            "명부는 저장된 토큰 앞으로 이름을 세운다"
        );
        assert!(
            roster.sink_of(&stored).is_some(),
            "조회도 저장된 토큰으로 닿는다"
        );
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_none(),
            "파생형으로는 닿지 않는다"
        );
        assert_eq!(roster.attached_owner(1), Some(stored));

        // ★`capture_info` 로 감싼다 — 이 `detach` 는 지울 이름이 있어 `info!` 를 낸다★: 구독자 없는
        //   스레드가 그 callsite 를 **처음** 등록하면 「관심 없음」이 박혀, 뒤에 그 줄을 재는
        //   `a_detach_logs_the_names_it_removed` 가 빈손이 된다(4-스레드 실행에서 실제로 그렇게 깨졌다,
        //   2026-08-18). ★「전역 캐시라 한 번 때리면 굳는다」로 적지 말 것 — 그 진술은 틀렸다★: 하자의
        //   실제 조건(스레드를 가로지르는 최초 등록)과 실측 근거는 `log_capture` 모듈 헤더가 정본이다.
        capture_info(|| roster.detach(1));

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unknown,
            "제거도 저장된 토큰으로 한다 — 재파생하면 이 이름이 죽은 주인 앞에 남는다"
        );
    }

    #[test]
    fn an_owner_that_never_attached_has_no_sink() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        assert!(
            roster.sink_of(&CommandRoster::owner_of(2)).is_none(),
            "붙은 적 없는 연결"
        );
        assert!(
            roster.sink_of(&OwnerToken::new("shell")).is_none(),
            "이 데몬이 낸 적 없는 모양의 토큰"
        );
    }

    /// 여럿이 붙어 있을 때 **각자 자기 출구**가 나온다 — 섞이면 한 클라이언트의 명령이 다른
    /// 클라이언트로 배달된다(그쪽은 자기가 안 시킨 것을 실행한다).
    #[test]
    fn each_owner_gets_its_own_sink() {
        let roster = CommandRoster::new();
        let (first, mut first_inbox) = sink_with_inbox();
        let (second, mut second_inbox) = sink_with_inbox();
        roster.attach(1, &first);
        roster.attach(2, &second);

        ping(
            &roster.sink_of(&CommandRoster::owner_of(1)).expect("주인 1"),
            "to-1",
        );
        ping(
            &roster.sink_of(&CommandRoster::owner_of(2)).expect("주인 2"),
            "to-2",
        );

        assert_eq!(delivered(&mut first_inbox), Some("to-1".to_string()));
        assert_eq!(delivered(&mut second_inbox), Some("to-2".to_string()));
        assert_eq!(delivered(&mut first_inbox), None, "1 번에 남의 것이 섞였다");
        assert_eq!(
            delivered(&mut second_inbox),
            None,
            "2 번에 남의 것이 섞였다"
        );
    }

    /// ★조회가 토큰을 **파싱하지 않는다**는 것을 문다★
    ///
    /// 아래 문자열은 전부 `"conn-"` 을 떼고 수로 읽으면 **1** 이 된다(`"01"`·`"+1"` 은 Rust 의 정수
    /// 파서가 받는다). 즉 접두를 떼어 연결 id 를 되짚는 조회는 이것들에 conn 1 의 출구를 내주고, 저장된
    /// 토큰과 동등 비교하는 조회는 빈손을 낸다. 그 파생이 잠정 규칙이라 되짚기를 금지한 근거는
    /// [`LiveConn`].
    ///
    /// ★이 테스트가 못 무는 변종이 하나 있다★: 파싱한 뒤 [`CommandRoster::owner_of`] 로 **되돌려
    /// 비교하는** 구현은 위 문자열에도 빈손을 내므로 여기는 통과한다. 그 변종은 저장된 토큰이 파생형이
    /// 아닌 상태에서만 갈리므로 [`the_roster_files_and_removes_under_the_stored_token`] 이 잡는다 — 그쪽이
    /// 표의 값을 직접 바꿔 그 상태를 만든다.
    #[test]
    fn a_lookup_compares_the_stored_token_and_never_parses_it() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        for impostor in ["conn-01", "conn-+1"] {
            assert!(
                roster.sink_of(&OwnerToken::new(impostor)).is_none(),
                "저장된 토큰과 다른 문자열이다: {impostor}"
            );
        }
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_some(),
            "저장된 그 토큰은 여전히 닿는다"
        );
    }
}
