//! 주인 명부 — **런타임 등록만으로** 차고, 주인이 끊기면 그 몫이 자취 없이 사라진다
//! (TRD §3-7 · ADR-0150).

use std::collections::{BTreeMap, BTreeSet};

use crate::{CommandDecl, CommandError, ErrorCode, OwnerToken};

/// 이름 하나의 명부 항목 — 명부에 있는 것은 **주인이 있는 이름뿐**이다(ADR-0150).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub name: String,
    /// 등록 패킷이 실어 온 모양. ★불투명 문자열이다★ — 파싱·검증·분기 금지(ADR-0156).
    pub help: String,
    pub owner: OwnerToken,
}

/// 이름 하나를 물었을 때의 두 답.
///
/// ★「주인이 자리 비움」이라는 답은 없다★ — 끊긴 주인의 등록이 명부에서 사라지므로 그 이름은 **한 번도
/// 본 적 없는 이름**과 같은 답을 받는다(ADR-0150 가 감수한 것 — 자취를 두지 않는 근거는 [`Roster`]).
/// 그 구분이 실제로 필요해지면 자취를 되살리는 것이 아니라 연결 목록을 응답에 싣는 쪽으로 푼다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLookup {
    Available(OwnerToken),
    Unknown,
}

/// 「이 이름의 주인은 누구인가」 하나만 묻는 창구 — [`crate::route`] 가 명부에 대해 필요로 하는 전부다.
///
/// ★배달이 명부를 **왕복 내내** 붙들지 않게 하는 자리다★: 공유 명부를 쥔 조립부가 `route` 에 참조를
/// 넘기려면 답장이 올 때까지 락을 들고 있어야 하고, 그동안 등록도 연결 정리도 다른 배달도 전부 멈춘다
/// (느린 상대 하나가 명부 전체를 세운다). 구현이 락을 **이 호출 안에서만** 잡으면 그 정지가 없어진다.
pub trait OwnerLookupSource: Send + Sync {
    fn lookup(&self, name: &str) -> OwnerLookup;
}

impl OwnerLookupSource for Roster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        Roster::lookup(self, name)
    }
}

/// ★빌드 목록을 조회하지 않는다★ — 명부가 런타임 등록만으로 차야 「지금 부를 수 있는가」가 사실이 된다
/// (TRD §3-7 금지 조항).
///
/// ★주인이 끊기면 그 주인의 항목이 사라진다 — 자취도 표식도 남기지 않는다★([`Roster::disconnect`])
///
/// 자취를 남기는 형태로 되돌리지 말 것. 주인 토큰은 연결마다 새로 나므로(`OwnerToken`) 같은 클라이언트가
/// 재접속하면 명부 눈에는 남남이고, 그래서 자취는 덮이지 않고 **쌓인다**. 만료도 회수 경로도 없어
/// [`Roster::MAX_NAMES`] 에 닿으면 명부에 없던 이름을 실은 등록이 전부 거절되고 — 그 이름들이
/// `UNKNOWN_COMMAND`(= 「재시도 말고 다시 발견하라」, TRD §4-②)로 나간다 — 데몬 재시작 말고는 안 풀린다
/// (ADR-0150 결정 3).
/// ★단 그것만으로 크기가 묶이지는 않는다★ — 이름은 등록 패킷이 실어 온 **문자열**이라(빌드 상수가
/// 아니다) 붙어 있는 주인 하나가 얼마든 많은 이름을 보낼 수 있다. 그 축은 개수·길이 상한이 닫는다
/// ([`Roster::MAX_NAMES_PER_OWNER`] · [`Roster::MAX_NAMES`] · [`Roster::MAX_NAME_BYTES`] ·
/// [`Roster::MAX_HELP_BYTES`]).
/// ★이름 하나에 주인은 하나다★ — 뒤에 등록한 쪽이 이긴다. 서로 다른 두 주인이 같은 이름을 얹으면 앞
/// 주인은 그 이름을 잃고, 나중 주인이 끊길 때 그 이름은 앞 주인에게 돌아가지 않고 사라진다. 이름 하나에
/// 주인 여럿은 유보된 안건이다(ADR-0150 대안 G).
// ADR-0156
// ADR-0150
#[derive(Debug, Default)]
pub struct Roster {
    /// 이름 → 그 이름을 **마지막으로** 등록한 주인. 끊긴 주인의 항목은 남지 않는다.
    entries: BTreeMap<String, Registration>,
}

#[derive(Debug, Clone)]
struct Registration {
    owner: OwnerToken,
    help: String,
}

impl Roster {
    /// 주인 하나가 명부에 **누적으로** 쥘 수 있는 이름 수 — 지금 보낸 패킷이 아니라 그 주인 앞으로
    /// 명부에 있는 전량에 이번 패킷을 더한 값을 잰다([`Roster::check_room_for`]).
    ///
    /// ★이름은 등록 패킷이 실어 온 **문자열**이지 빌드 상수가 아니다★ — 상한이 없으면 오작동하거나
    /// 악의적인 주인 하나가 붙어 있는 동안 명부를 무한히 불린다.
    /// 값의 근거: 지금 어휘 전량이 60 남짓이고(화면 33 · agent 5 · daemon 3 · `src-tauri` 17) 가장 큰
    /// 주인이 33이다. 512 = 그 여덟 배 — 실무가 닿지 않으면서 폭주는 막는 자리다.
    pub const MAX_NAMES_PER_OWNER: usize = 512;

    /// 명부 전체의 이름 수 — 주인이 여럿이어도 여기서 멈춘다(주인당 상한만 두면 주인 수로 우회된다).
    pub const MAX_NAMES: usize = 4096;

    /// 이름 한 개의 최대 바이트 수.
    ///
    /// 값의 근거: 지금 실재하는 가장 긴 이름이 `agentlist.createTerminal`(24바이트)이다. 128 = 그 다섯
    /// 배 남짓 — 계열을 더 깊게 파도 닿지 않으면서, 이름 하나로 명부를 부풀리는 길은 닫는다.
    pub const MAX_NAME_BYTES: usize = 128;

    /// `help` 한 칸의 최대 바이트 수.
    ///
    /// ★이 문자열은 그 주인이 붙어 있는 동안 상주한다★ — 상한이 없으면 과장된 `help` 하나가 명부의
    /// 메모리를 통째로 먹는다.
    /// 값의 근거: 지금 가장 큰 항목이 `agent.spawn` 의 591바이트다(`agent/bindings/commands.schema.json`
    /// 실측). 4096 = 그 일곱 배로, 중첩 struct 를 몇 겹 더 편 선언도 넉넉히 든다.
    /// 두 상한을 합치면 명부의 상주 상한이 [`Roster::MAX_NAMES`] × (128 + 4096) ≈ 16 MiB 로 닫힌다.
    pub const MAX_HELP_BYTES: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    /// 붙을 때 한 방 — 그 주인의 **전량**이다. 재연결 재전송이 같은 경로를 타므로 이 주인이 이번에
    /// 싣지 않은 옛 이름은 명부에서 내려간다(주인 단위 last-wins, ADR-0081 「RegisterRole 재전송」).
    /// 그래서 빈 목록으로 부르면 그 주인의 자리가 온전히 빈다.
    ///
    /// ★상한을 넘으면 **한 이름도 넣지 않고** `INVALID_ARGUMENT` 로 돌려보낸다★ — 넘치는 만큼만 잘라
    /// 넣으면 주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다(그 이름은 `UNKNOWN_COMMAND` 로
    /// 나가는데, 등록한 쪽에는 그럴 이유가 없다). 거절은 명부를 **건드리기 전에** 판정하므로 실패한
    /// 등록이 기존 상태를 바꾸지 않는다.
    /// ★남이 쥔 이름을 가져가는 것이 여기서는 적법하다★ — 등록은 **붙는 순간**에 매인 전량 선언이라
    /// (TRD §3-7 조항 1) 새 연결이 옛 연결의 이름을 이어받는 것이 인수인계 그 자체다. 붙어 있는 동안의
    /// 차분([`Roster::update`])에는 그 닻이 없어 같은 인수인계가 허용되지 않는다.
    /// ★끊긴 주인의 늦은 등록을 **여기서 막지 않는다**★ — 명부에는 그 주인의 흔적이 없어(끊길 때 지운다)
    /// 산 주인이 보낸 것과 구분할 근거가 없다. 그 그물은 연결 수명을 아는 쪽 **한 곳뿐**이다
    /// (`engram-dashboard-daemon` 의 `CommandRoster::refuse_if_detached`) — 여기에 두 번째 그물을 세우면
    /// 주인 단위 상태를 따로 들어야 하고, 그 목록이 자취와 똑같이 무한히 자란다.
    // ADR-0150
    pub fn register(
        &mut self,
        owner: &OwnerToken,
        decls: Vec<CommandDecl>,
    ) -> Result<(), CommandError> {
        self.check_room_for(owner, &decls)?;
        // 전량 선언이라 이번에 안 실린 이 주인의 이름은 자리째 내려간다.
        self.remove_owner(owner);
        for decl in decls {
            self.entries.insert(
                decl.name,
                Registration {
                    owner: owner.clone(),
                    help: decl.help,
                },
            );
        }
        Ok(())
    }

    /// 붙어 있는 동안의 **차분** — 늦게 뜬 기능이 이름을 더하고 꺼진 기능이 이름을 내린다(TRD §3-7 조항 3).
    /// 전량 재전송은 [`Roster::register`] 뿐이다.
    ///
    /// ★`removed` 는 이름을 **지운다** — 자취로 남기지 않는다★: 자취로 남기면 끊기지 않은 주인이
    /// `added`=새 이름 · `removed`=옛 이름을 되풀이해 자기 몫([`Roster::MAX_NAMES_PER_OWNER`])을 영구히
    /// 채울 수 있고, 그러면 연결을 끊지 않고도 자취가 명부를 영구히 메우는 같은 사고가 재현된다
    /// (ADR-0150 결정 3이 끊김에 대해 닫은 축의 나머지 절반).
    /// ★`removed` 는 **자기가 쥔 이름만** 지운다★ — [`Roster::disconnect`] 와 같은 규칙이다. 지나간
    /// 주인의 늦은 차분이 방금 붙은 주인의 등록을 지우면 살아 있는 명령이 사라진다(재연결 겹침). 남의
    /// 이름·없는 이름은 조용히 지나간다 — 차분은 「내 몫을 이렇게 바꿔라」이지 남의 상태에 대한 주장이
    /// 아니다.
    /// ★`added` 는 남이 쥔 이름을 가져가지 못한다 — 여기가 [`Roster::register`] 와 갈리는 자리다★.
    /// 등록의 last-wins 는 **붙는 순간**에 매여 있어(TRD §3-7 조항 1) 인수인계로 읽히지만, 차분은 정의상
    /// 「붙어 있는 동안」이라 그 닻이 없다. 닻 없는 덮어쓰기를 허용하면 죽은 연결의 늦은 차분이 방금 붙은
    /// 셸의 등록을 **자기 토큰으로 덮어** 조회가 죽은 주인을 `Available` 로 답하고, 산 셸은 다시 등록할
    /// 계기가 없어(등록은 붙을 때 한 번뿐) 그 상태가 그대로 굳는다.
    /// ★조용히 넘기지 않고 반려하는 이유★(`removed` 의 남의 이름은 조용히 지나간다): 남의 이름을 안
    /// 지우는 것은 차분이 말한 「내 몫」이 이미 그러하다는 뜻이라 잃는 것이 없지만, `added` 를 조용히
    /// 무시하면 **성공을 보고하면서** 호출자는 그 이름이 자기 앞으로 온다고 믿는다. 반려는 `CONFLICT`
    /// 다 — 패킷이 틀린 것이(`INVALID_ARGUMENT`) 아니라 명부의 **상태**가 아니라고 답한다.
    /// ★끊긴 주인의 차분을 **여기서 막지 않는다**★ — [`Roster::register`] 와 같은 이유이고, 그 그물이 선
    /// 곳도 같은 한 곳이다.
    /// ★한 이름이 `added` 와 `removed` 양쪽에 실린 패킷은 **`INVALID_ARGUMENT` 로 통째 반려한다**★ —
    /// 적용 순서를 정하지 않는다. 어느 순서를 골라도 호출자는 결과를 알 수 없다: 더한 뒤 지우면 **성공을
    /// 받고도** 그 이름이 명부에 없어 `UNKNOWN_COMMAND` 로 나가고([`Roster::register`] 의 상한 반려가 막는
    /// 그 사고 그대로 —
    /// 「주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다」), 지운 뒤 더하면 내리라는 지시가 조용히
    /// 무시된다. **성공 보고와 실제 상태가 갈리는 쪽이 순서를 잘못 고르는 것보다 나쁘다** — 호출자는 조용한
    /// 삭제를 성공한 등록과 구분할 수단이 없다. 순서를 정한 조항이 ADR-0150 에도 TRD §3-7 조항 3 에도 없으니
    /// 그 자리를 여기서 임의로 메우지 않는다.
    ///
    /// 상한은 `added` 만 세어 다시 본다 — 넘치면 **한 이름도 건드리지 않는다**([`Roster::register`] 와
    /// 같은 계약이고, 위 반려 둘도 같다).
    // ADR-0150
    pub fn update(
        &mut self,
        owner: &OwnerToken,
        added: Vec<CommandDecl>,
        removed: Vec<String>,
    ) -> Result<(), CommandError> {
        // 길이 상한을 먼저 통과시킨다 — 아래 반려들이 이름을 문구에 인용한다(`check_room_for` 안의
        //   같은 순서와 같은 이유).
        self.check_room_for(owner, &added)?;
        Self::check_lists_are_disjoint(&added, &removed)?;
        self.check_added_are_not_taken(owner, &added)?;
        for decl in added {
            self.entries.insert(
                decl.name,
                Registration {
                    owner: owner.clone(),
                    help: decl.help,
                },
            );
        }
        for name in removed {
            if self
                .entries
                .get(&name)
                .is_some_and(|reg| &reg.owner == owner)
            {
                self.entries.remove(&name);
            }
        }
        Ok(())
    }

    /// 한 이름이 두 목록에 함께 실려 있지 않나 — **명부를 건드리기 전에** 본다([`Roster::check_room_for`] 와
    /// 같은 계약). 반려하는 이유와 「적용 순서를 정하지 않는다」는 [`Roster::update`] 주석.
    ///
    /// ★명부를 안 본다 — 그래서 `self` 를 받지 않는다★: 판정 대상이 **패킷 하나**이고 그 코드가
    /// `INVALID_ARGUMENT` 인 것도 같은 이유다(같은 왕복을 다시 보내도 같은 답이 난다). 형제
    /// [`Roster::check_added_are_not_taken`] 는 반대로 명부 **상태**를 말하므로 `CONFLICT` 이고 상태가
    /// 바뀌면 답이 바뀐다.
    /// ★순회는 `added` 쪽이다★ — 문구에 인용할 이름은 [`Roster::MAX_NAME_BYTES`] 를 통과한 것이어야 하는데
    /// 길이를 재는 것은 `decls` 뿐이다(`removed` 는 안 잰다). 겹친 이름은 정의상 `added` 에도 있으므로 이
    /// 순회가 인용하는 값은 항상 그 상한 안이다.
    fn check_lists_are_disjoint(
        added: &[CommandDecl],
        removed: &[String],
    ) -> Result<(), CommandError> {
        let dropped: BTreeSet<&str> = removed.iter().map(|name| name.as_str()).collect();
        for decl in added {
            if dropped.contains(decl.name.as_str()) {
                return Err(CommandError::invalid_argument(format!(
                    "'{}' is in both added and removed — this packet does not say which one wins",
                    decl.name
                )));
            }
        }
        Ok(())
    }

    /// `added` 가 **남의 등록**을 덮지 않나 — 명부에 남는 항목은 주인이 있는 것뿐이라(ADR-0150) 주인
    /// 비교 하나로 판정된다.
    fn check_added_are_not_taken(
        &self,
        owner: &OwnerToken,
        decls: &[CommandDecl],
    ) -> Result<(), CommandError> {
        for decl in decls {
            let taken = self
                .entries
                .get(&decl.name)
                .is_some_and(|reg| &reg.owner != owner);
            if taken {
                return Err(CommandError::of(
                    ErrorCode::Conflict,
                    format!("'{}' is registered to another owner", decl.name),
                ));
            }
        }
        Ok(())
    }

    /// 이 등록이 상한 안인가 — **명부를 건드리기 전에** 본다.
    ///
    /// ★주인당 상한은 패킷 하나가 아니라 그 주인의 **누적**을 잰다★: 패킷만 재면 서로 겹치지 않는
    /// 512개짜리 차분을 잇달아 보내는 주인 하나가 명부 전체를 혼자 채운다. 그러면 진짜 셸의 등록이 전체
    /// 상한에 막히고 `tab.*`·`window.*` 가 전부 `UNKNOWN_COMMAND` 로 나가는데, 그 코드는 「재시도가
    /// 무의미하니 이름을 다시 발견하라」는 뜻이라(TRD §4-②) 호출자는 실재하는 명령을 영영 포기한다.
    /// 그래서 전체 상한이 **주인별 몫 없는 공유 자원**이 되지 않게 여기서 막는다. 이번 패킷에 실린 이름은
    /// 덮어쓰기라 자리를 새로 먹지 않으므로 두 번 세지 않는다.
    /// ★이 상한은 거부된 TTL 과 다른 축이다★ — 거기서 거부한 것은 **시간**이라 같은 질문의 답이 시계에
    /// 따라 갈렸다(ADR-0156 대안 C · ADR-0150 대안 F). 개수 상한은 명부에 든 이름의 답을 바꾸지 않는다.
    /// ★판정이 변경보다 앞이라 **과대측정**이다(알고 남긴 것)★: 이번 왕복에서 내려갈 이름도 아직 명부에
    /// 있으므로 함께 센다 — [`Roster::register`] 가 갈아치울 자기 이름과 [`Roster::update`] 의 `removed`
    /// 가 그렇다. 대가는 「몫이 꽉 찬 주인은 이름 전량을 한 패킷에 갈아치우지 못한다」뿐이고, 빈 전량
    /// 등록을 한 번 보내면 자기 자리가 온전히 비어 그 길이 열린다. 명부가 자라는 축이 아니라서 정밀하게
    /// 고치지 않았다 — 정확히 재려면 상한 판정이 변경 결과를 모델링해야 하고, 그러면 「명부를 건드리기
    /// 전에 판정한다」는 이 함수의 계약이 흐려진다.
    ///
    /// 이름·`help` 는 등록 패킷이 실어 온 문자열이라 길이도 함께 잰다 — 과장된 `help` 하나가 그 주인이
    /// 붙어 있는 동안 명부 메모리를 통째로 먹는다.
    fn check_room_for(
        &self,
        owner: &OwnerToken,
        decls: &[CommandDecl],
    ) -> Result<(), CommandError> {
        for decl in decls {
            // 이름을 먼저 잰다 — 그래야 아래 `help` 오류가 이름을 안전하게 인용할 수 있다.
            if decl.name.len() > Self::MAX_NAME_BYTES {
                return Err(CommandError::invalid_argument(format!(
                    "a command name may be at most {} bytes (got {})",
                    Self::MAX_NAME_BYTES,
                    decl.name.len()
                )));
            }
            if decl.help.len() > Self::MAX_HELP_BYTES {
                return Err(CommandError::invalid_argument(format!(
                    "the shape sent with '{}' may be at most {} bytes (got {})",
                    decl.name,
                    Self::MAX_HELP_BYTES,
                    decl.help.len()
                )));
            }
        }
        let names: BTreeSet<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        let kept = self
            .entries
            .iter()
            .filter(|(name, reg)| &reg.owner == owner && !names.contains(name.as_str()))
            .count();
        let held = kept + names.len();
        if held > Self::MAX_NAMES_PER_OWNER {
            return Err(CommandError::invalid_argument(format!(
                "a single owner may hold at most {} command names ({kept} still on the roster from this owner, {} in this packet)",
                Self::MAX_NAMES_PER_OWNER,
                names.len()
            )));
        }
        let added = names
            .iter()
            .filter(|name| !self.entries.contains_key(**name))
            .count();
        if self.entries.len() + added > Self::MAX_NAMES {
            return Err(CommandError::invalid_argument(format!(
                "the roster holds at most {} command names ({} in use, {added} new)",
                Self::MAX_NAMES,
                self.entries.len()
            )));
        }
        Ok(())
    }

    /// 연결이 끊겼다 — **그 주인 앞으로 남은 항목을 자리째 지운다.** 자취도 표식도 남기지 않으므로 그
    /// 이름들은 한 번도 등록되지 않았던 것과 같은 답을 받는다([`OwnerLookup`]).
    ///
    /// ★**자기가 아직 주인인 이름만** 내린다★ — 재연결 겹침(새 연결이 등록한 뒤 옛 연결의 정리가 도착)에서
    /// 방금 붙은 주인의 등록을 지우지 않기 위해서다.
    /// ★끊김을 처리하는 자리는 이것 하나다★ — 부르는 경로도 데몬의 `on_disconnect → detach` 하나뿐이고
    /// (ADR-0150), 두 번째를 만들면 인과가 갈라진다.
    /// ★반환 = **이 호출이 지운 이름 전량**(오름차순)★ — 지운 뒤에는 명부에 그 사건의 자취가 없으므로
    /// (자취를 두지 않는 근거는 이 타입 주석) 무엇이 사라졌는지 말할 수 있는 값이 이 반환뿐이다. 계측은
    /// **연결 수명을 아는 쪽**이 한다(`engram-dashboard-daemon` 의 `CommandRoster::detach`) — 이 crate 는
    /// 로깅 의존을 지지 않는다(워크스페이스·서드파티 의존 상한 = `lib.rs` 헤더).
    // ADR-0150
    pub fn disconnect(&mut self, owner: &OwnerToken) -> Vec<String> {
        self.remove_owner(owner)
    }

    pub fn lookup(&self, name: &str) -> OwnerLookup {
        match self.entries.get(name) {
            Some(reg) => OwnerLookup::Available(reg.owner.clone()),
            None => OwnerLookup::Unknown,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = RosterEntry> + '_ {
        self.entries.iter().map(|(name, reg)| RosterEntry {
            name: name.clone(),
            help: reg.help.clone(),
            owner: reg.owner.clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 반환 = 내려간 이름들(오름차순 — `BTreeMap` 순회 순서). [`Roster::register`] 는 그 값을 쓰지 않는다
    /// (전량 선언이 자기 자리를 갈아치우는 것은 사건이 아니라 그 호출의 정의다).
    fn remove_owner(&mut self, owner: &OwnerToken) -> Vec<String> {
        let mut removed = Vec::new();
        self.entries.retain(|name, reg| {
            if &reg.owner == owner {
                removed.push(name.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: format!("{{\"name\":\"{name}\"}}"),
        }
    }

    /// ★재연결 겹침 — 옛 연결의 cleanup 이 새 등록을 내리면 안 된다★
    ///
    /// 주인 토큰은 연결마다 새로 나므로 셸이 재연결하면 등록(새 토큰)이 먼저 오고 옛 연결의 정리가
    /// 뒤늦게 온다. 정리가 이름 단위로 내려가면 방금 붙은 셸의 명령이 조용히 사라진다.
    #[test]
    fn a_superseded_owners_cleanup_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        roster.disconnect(&old);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new.clone()),
            "방금 등록한 주인이 답해야 한다"
        );

        roster.disconnect(&new);
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);
    }

    #[test]
    fn the_latest_registration_answers_while_both_are_up() {
        let first = OwnerToken::new("shell-1");
        let second = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&first, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&second, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(second));
    }

    /// ★재접속을 되풀이해도 명부에 쌓이는 것이 없다★ — 주인 토큰은 연결마다 새로 나므로 자취를 남기는
    /// 형태에서는 이 되풀이가 명부를 상한까지 영구히 채웠다(ADR-0150 결정 3).
    #[test]
    fn reconnecting_many_times_leaves_nothing_behind() {
        let mut roster = Roster::new();
        for connection in 0..50 {
            let owner = OwnerToken::new(format!("shell-conn-{connection}"));
            roster
                .register(&owner, vec![decl("tab.create"), decl("tab.close")])
                .expect("상한 안");
            roster.disconnect(&owner);
        }

        assert!(roster.is_empty(), "쌓이는 것이 없다");
        assert_eq!(roster.entries().count(), 0);
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);
    }

    /// 나중에 등록한 주인의 `help` 가 그 이름의 모양이다 — 이름 하나에 항목도 하나다.
    #[test]
    fn the_surviving_entry_carries_the_latest_shape() {
        let first = OwnerToken::new("shell-1");
        let second = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&first, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(
                &second,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{\"name\":\"tab.create\",\"since\":2}".to_string(),
                }],
            )
            .expect("상한 안");

        assert_eq!(roster.len(), 1);
        let entry = roster.entries().next().expect("한 항목");
        assert_eq!(entry.help, "{\"name\":\"tab.create\",\"since\":2}");
        assert_eq!(entry.owner, second);
    }

    #[test]
    fn never_seen_name_is_unknown() {
        assert_eq!(Roster::new().lookup("tab.create"), OwnerLookup::Unknown);
    }

    fn decls(prefix: &str, count: usize) -> Vec<CommandDecl> {
        (0..count).map(|i| decl(&format!("{prefix}.{i}"))).collect()
    }

    /// ★붙어 있는 한 주인이 명부를 무한히 불릴 수 없다★ — 이름은 등록 패킷이 보낸 문자열이고, 그 주인이
    /// 끊기기 전에는 회수가 없으므로 상한이 유일한 제동이다.
    #[test]
    fn a_registration_over_the_per_owner_cap_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();

        let err = roster
            .register(&owner, decls("flood", Roster::MAX_NAMES_PER_OWNER + 1))
            .expect_err("상한을 넘는다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(roster.is_empty(), "실패한 등록은 명부를 건드리지 않는다");
        roster
            .register(&owner, decls("fits", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 자체는 통과한다");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// ★상한 판정이 변경보다 앞이라 이번 패킷이 밀어낼 자기 이름도 함께 센다★
    ///
    /// 과대측정이고 알고 남긴 것이다(근거 = `check_room_for`). 대가가 「몫이 꽉 찬 주인은 이름 전량을 한
    /// 패킷에 갈아치우지 못한다」에 그치는 것과, **빈 전량 등록 한 번이 그 길을 연다**는 것을 함께 박는다.
    #[test]
    fn a_full_owners_disjoint_snapshot_is_measured_before_the_old_names_come_down() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("첫 스냅샷은 상한 안");

        let err = roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect_err("겹치지 않는 두 번째 스냅샷은 누적으로 상한을 넘는다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "실패한 등록은 명부를 건드리지 않는다"
        );

        roster.register(&owner, vec![]).expect("빈 전량 등록");
        assert!(roster.is_empty(), "전량 선언이 비면 자기 자리도 빈다");
        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("자리가 비면 새 전량이 들어온다");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// 끊기면 그 주인의 몫이 온전히 빈다 — 상한을 잡고 있던 이름이 자리째 사라진다(ADR-0150 결정 3).
    #[test]
    fn a_disconnect_frees_that_owners_whole_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 안");
        roster.disconnect(&owner);

        roster
            .register(&owner, decls("wave1", Roster::MAX_NAMES_PER_OWNER))
            .expect("자리를 쥐고 있는 것이 없다");

        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("wave0.0"), OwnerLookup::Unknown);
    }

    /// ★이름·`help` 는 등록 패킷이 실어 온 문자열이다★ — 길이를 안 재면 과장된 `help` 하나가 그 주인이
    /// 붙어 있는 동안 명부 메모리를 통째로 먹는다.
    #[test]
    fn an_oversized_name_or_help_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();

        let at_cap = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES),
            help: "h".repeat(Roster::MAX_HELP_BYTES),
        };
        roster
            .register(&owner, vec![at_cap])
            .expect("상한 자체는 통과한다");
        assert_eq!(roster.len(), 1);

        let long_name = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES + 1),
            help: "{}".to_string(),
        };
        let err = roster
            .register(&owner, vec![long_name])
            .expect_err("이름이 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        let long_help = CommandDecl {
            name: "tab.create".to_string(),
            help: "h".repeat(Roster::MAX_HELP_BYTES + 1),
        };
        let err = roster
            .register(&owner, vec![decl("tab.close"), long_help])
            .expect_err("help 가 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);

        assert_eq!(roster.len(), 1, "실패한 등록은 명부를 건드리지 않는다");
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
    }

    /// 주인당 상한만 두면 주인 수로 우회된다 — 전체 상한이 그 축을 닫는다.
    #[test]
    fn the_total_cap_holds_across_owners() {
        let mut roster = Roster::new();
        let owners = Roster::MAX_NAMES / Roster::MAX_NAMES_PER_OWNER;
        for i in 0..owners {
            let owner = OwnerToken::new(format!("shell-{i}"));
            roster
                .register(
                    &owner,
                    decls(&format!("owner{i}"), Roster::MAX_NAMES_PER_OWNER),
                )
                .expect("전체 상한 안");
        }
        assert_eq!(roster.len(), Roster::MAX_NAMES);

        let newcomer = OwnerToken::new("shell-late");
        let err = roster
            .register(&newcomer, vec![decl("one.more")])
            .expect_err("전체 상한을 넘는다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES, "명부는 그대로다");

        // ★이미 있는 이름은 자리를 새로 먹지 않는다★ — 꽉 찬 명부에서도 재등록은 통과해야 한다.
        roster
            .register(&newcomer, vec![decl("owner0.0")])
            .expect("덮어쓰기는 크기를 안 늘린다");
        assert_eq!(roster.len(), Roster::MAX_NAMES);
    }

    /// 같은 이름을 여러 번 실어 보내도 자리는 하나다 — 중복으로 상한을 태우지 않는다.
    #[test]
    fn duplicate_names_in_one_packet_count_once() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        let repeated = vec![decl("tab.create"); Roster::MAX_NAMES_PER_OWNER + 8];

        roster.register(&owner, repeated).expect("이름은 하나다");

        assert_eq!(roster.len(), 1);
    }

    /// ★사라진 방어선의 등록 쪽 — 끊긴 주인의 늦은 **등록**도 명부는 막지 않는다★
    ///
    /// 등록은 「붙는 순간의 전량 선언」이라 산 남의 이름까지 이어받는 것이 적법한데(인수인계), 명부에는
    /// 그 주인이 이미 끊겼다는 기억이 없으므로 죽은 연결의 늦은 등록과 진짜 인수인계가 구분되지 않는다.
    /// 그래서 이 패킷이 통과하면 그 이름은 **없는 주인** 앞으로 `Available` 이 되고, 그 연결엔 다시 올
    /// 정리가 없어 데몬 수명 내내 굳는다(배달이 끊긴 링크로 나간다). 막는 그물은 연결 수명을 아는 쪽
    /// **한 곳뿐**이다 — `engram-dashboard-daemon` 의 `CommandRoster::refuse_if_detached`.
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_registration_from_a_disconnected_owner() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("산 주인이 그 이름을 새로 얹는다");

        roster
            .register(&old, vec![decl("tab.create")])
            .expect("명부에는 반려할 근거가 없다");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(old),
            "없는 주인 앞으로 선다 — 이것을 막는 곳은 데몬 하나뿐이다"
        );
    }

    // ── 차분 등록(TRD §3-7 조항 3) ────────────────────────────────────────────────────────────

    /// ★차분은 `removed` 만 지운다★ — 자취를 남기면 끊기지 않은 주인이 add/remove 를 되풀이해 자기 몫을
    /// 영구히 채운다(ADR-0150). 안 실린 이름을 손대지 않는 것이 `register` 의 전량 last-wins 와 갈리는
    /// 자리다.
    #[test]
    fn an_update_removes_only_the_names_it_lists() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");

        roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.close".to_string()],
            )
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "차분에 안 실린 이름은 그대로다"
        );
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Available(owner));
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
        assert_eq!(roster.len(), 2, "내린 이름은 자리째 사라진다");
    }

    /// ★차분의 `removed` 가 자리를 비운다★ — 자취로 남기면 붙어 있는 주인이 이름을 바꿔 가며 자기 몫
    /// 상한을 영구히 채울 수 있어, 끊김 없이도 같은 무한 증식이 재현된다(ADR-0150).
    #[test]
    fn swapping_names_forever_does_not_eat_an_owners_cap() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave", Roster::MAX_NAMES_PER_OWNER))
            .expect("몫을 꽉 채운다");

        for round in 0..8 {
            roster
                .update(
                    &owner,
                    vec![],
                    vec![format!("wave.{}", Roster::MAX_NAMES_PER_OWNER - 1 - round)],
                )
                .expect("내리는 것은 언제나 통과한다");
            roster
                .update(&owner, vec![decl(&format!("swap.{round}"))], vec![])
                .expect("자리가 비었으므로 더할 수 있다");
        }

        assert_eq!(
            roster.len(),
            Roster::MAX_NAMES_PER_OWNER,
            "바꿔 끼워도 크기는 그대로다"
        );
    }

    /// ★재연결 겹침 — 차분에도 같은 그물이 서야 한다★
    ///
    /// 옛 연결의 늦은 차분이 이름 단위로 내려가면, 같은 이름을 방금 등록한 새 연결의 명령이 조용히
    /// 사라진다(`disconnect` 가 막는 것과 같은 사고).
    #[test]
    fn an_update_from_a_superseded_owner_does_not_take_down_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        roster
            .update(&old, vec![], vec!["tab.create".to_string()])
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "방금 등록한 주인이 답해야 한다"
        );
    }

    /// ★재연결 겹침 — `added` 쪽에도 같은 그물이 서야 한다★
    ///
    /// 위 테스트의 거울이다. 옛 연결의 늦은 차분이 같은 이름을 **더하면** 항목이 죽은 토큰으로 덮이고,
    /// 조회는 그 죽은 주인을 `Available` 로 답해 배달이 끊긴 링크로 나간다. 산 셸은 다시 등록할 계기가
    /// 없으므로(등록은 붙을 때 한 번) 그 상태는 저절로 풀리지 않는다.
    #[test]
    fn an_update_from_a_superseded_owner_does_not_add_over_the_current_registration() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        let err = roster
            .update(&old, vec![decl("tab.create")], vec![])
            .expect_err("산 남의 등록은 못 가져간다");

        assert_eq!(err.code(), crate::ErrorCode::Conflict);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(new),
            "방금 등록한 주인이 답해야 한다"
        );
    }

    /// ★한 이름이 `added` 와 `removed` 양쪽에 실리면 **패킷째** 반려한다★
    ///
    /// 적용 순서를 고르면 어느 쪽으로 골라도 호출자가 결과를 알 수 없다: 더한 뒤 지우면 **성공을 받고도**
    /// 그 이름이 명부에 없어 `UNKNOWN_COMMAND` 로 나가고(`register` 가 상한에서 막는 그 사고 그대로 —
    /// 「주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다」), 지운 뒤 더하면 내리라는 지시가
    /// 조용히 무시된다. 순서를 정한 조항이 ADR-0150 에도 TRD §3-7 조항 3 에도 없어 반려가 답이다.
    #[test]
    fn an_update_carrying_one_name_in_both_lists_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");

        // 이미 쥔 이름 — 옛 적용 순서에서는 넣었다가 지워 **성공과 함께 사라졌다**.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.create")],
                vec!["tab.create".to_string()],
            )
            .expect_err("같은 이름이 양쪽에 실렸다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("tab.create"),
            "어느 이름이 걸렸는지 문구가 말해야 한다: {}",
            err.message()
        );
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone()),
            "반려는 명부를 건드리지 않는다"
        );

        // 새 이름 — 이쪽은 「넣었다가 지운다」가 처음부터 없던 이름을 만들지도 않지만, 판정은 같다.
        let err = roster
            .update(
                &owner,
                vec![decl("tab.split")],
                vec!["tab.split".to_string()],
            )
            .expect_err("같은 이름이 양쪽에 실렸다");
        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.lookup("tab.split"), OwnerLookup::Unknown);
        assert_eq!(roster.len(), 1);
    }

    /// 끊김이 **무엇을 지웠는지** 부르는 쪽에 돌려준다 — 명부에 자취가 남지 않으므로(ADR-0150) 그 사건을
    /// 나중에 재구성할 값은 이 반환뿐이다. 계측은 연결 수명을 아는 데몬이 한다
    /// (`CommandRoster::detach` — 도구 crate 에 로깅 의존을 들이지 않는다).
    #[test]
    fn a_disconnect_reports_the_names_it_removed() {
        let owner = OwnerToken::new("shell-1");
        let other = OwnerToken::new("shell-2");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");
        roster
            .register(&other, vec![decl("window.focus")])
            .expect("상한 안");

        let removed = roster.disconnect(&owner);

        assert_eq!(
            removed,
            vec!["tab.close".to_string(), "tab.create".to_string()],
            "자기가 쥔 이름만 실린다"
        );
        assert!(
            roster.disconnect(&owner).is_empty(),
            "두 번째 정리에는 지울 것이 없다"
        );
        assert_eq!(
            roster.lookup("window.focus"),
            OwnerLookup::Available(other),
            "남의 등록은 그대로다"
        );
    }

    /// 끊긴 주인이 쥐었던 이름은 **아무도 안 쥔 이름**이 된다 — 산 등록을 뺏는 것이 아니므로 차분이
    /// 그대로 가져간다.
    #[test]
    fn an_update_may_claim_a_name_whose_owner_has_disconnected() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&old);
        roster
            .register(&new, vec![decl("tab.close")])
            .expect("상한 안");
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unknown);

        roster
            .update(&new, vec![decl("tab.create")], vec![])
            .expect("주인 없는 이름은 가져갈 수 있다");

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Available(new));
    }

    /// 상한은 차분에서도 **누적**으로 재고, 넘치면 한 이름도 안 들어간다.
    #[test]
    fn an_update_over_the_per_owner_cap_is_refused_whole() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 안");

        let err = roster
            .update(&owner, vec![decl("one.more")], vec![])
            .expect_err("이미 쥔 이름이 몫을 다 쓰고 있다");

        assert_eq!(err.code(), crate::ErrorCode::InvalidArgument);
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
        assert_eq!(roster.lookup("one.more"), OwnerLookup::Unknown);
    }

    /// ★끊기면 자취 없이 사라진다★ — 주인 토큰은 연결마다 새로 나므로 자취를 남기면 같은 클라이언트의
    /// 재접속이 덮기가 아니라 **쌓기**가 되고, 만료도 회수 경로도 없어 명부가 상한까지 영구히 찬다
    /// (ADR-0150 결정 3).
    #[test]
    fn a_disconnect_empties_that_owners_share_of_the_roster() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");

        roster.disconnect(&owner);

        assert!(roster.is_empty(), "항목이 남지 않는다");
        assert_eq!(roster.entries().count(), 0);
        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Unknown,
            "모르는 이름과 같은 답이다"
        );
    }

    /// 재접속은 **완전히 새 항목**이다 — 새 연결이 안 실은 옛 이름은 어디에도 남지 않는다.
    #[test]
    fn a_reconnect_registers_as_a_wholly_new_entry() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create"), decl("tab.close")])
            .expect("상한 안");
        roster.disconnect(&old);

        roster
            .register(&new, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(roster.len(), 1, "옛 연결의 나머지 이름은 남지 않는다");
        assert_eq!(roster.entries().next().expect("한 항목").owner, new);
        assert_eq!(roster.lookup("tab.close"), OwnerLookup::Unknown);
    }

    /// ★사라진 방어선을 못으로 박는다 — 끊긴 주인의 늦은 패킷을 **명부는 못 막는다**★
    ///
    /// 자취가 없어지면 명부에는 그 주인의 죽음을 기억할 자리가 없다. 그래서 늦은 차분이 그대로 통과하고,
    /// 그 이름은 **없는 주인** 앞으로 `Available` 이 되어 데몬 수명 내내 굳는다(그 연결엔 다시 올 정리가
    /// 없다). 막는 그물은 연결 수명을 아는 쪽 **한 곳뿐**이다 — `engram-dashboard-daemon` 의
    /// `CommandRoster::refuse_if_detached`(살아 있는 연결 명단). 여기에 두 번째 그물을 세우지 말 것:
    /// 명부가 주인 단위 상태를 따로 들면 만료 없는 자료에 무한히 자라는 목록이 하나 더 생긴다(자취를
    /// 버린 것과 같은 이유 — ADR-0150).
    #[test]
    fn the_roster_alone_cannot_refuse_a_late_delta_from_a_disconnected_owner() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&owner);

        roster
            .update(&owner, vec![decl("tab.split")], vec![])
            .expect("명부에는 반려할 근거가 없다");

        assert_eq!(
            roster.lookup("tab.split"),
            OwnerLookup::Available(owner),
            "없는 주인 앞으로 선다 — 이것을 막는 곳은 데몬 하나뿐이다"
        );
    }

    #[test]
    fn reregistration_is_last_wins_and_drops_stale_names() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create"), decl("tab.legacy")])
            .expect("상한 안");
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");

        assert_eq!(
            roster.lookup("tab.create"),
            OwnerLookup::Available(owner.clone())
        );
        assert_eq!(roster.lookup("tab.legacy"), OwnerLookup::Unknown);
    }
}
