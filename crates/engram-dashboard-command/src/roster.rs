//! 주인 명부 — **런타임 등록만으로** 찬다(TRD §3-7).

use std::collections::{BTreeMap, BTreeSet};

use crate::{CommandDecl, CommandError, OwnerToken};

/// 이름 하나의 명부 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub name: String,
    /// 등록 패킷이 실어 온 모양. ★불투명 문자열이다★ — 파싱·검증·분기 금지(ADR-0135).
    pub help: String,
    pub owner: OwnerToken,
    /// `false` = tombstone(주인이 지금 없다). ★지우지 않는 것이 요점★ — 지우면
    /// `UNKNOWN_COMMAND` 와 `OWNER_UNAVAILABLE` 을 가를 수 없다(TRD §4-②).
    pub available: bool,
}

/// 이름 하나를 물었을 때의 세 답. 뒤 둘이 갈려야 호출자가 **재시도가 의미 있는지**를 안다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLookup {
    Available(OwnerToken),
    Unavailable,
    Unknown,
}

/// ★빌드 목록을 조회하지 않는다★ — 명부가 런타임 등록만으로 차야 「지금 부를 수 있는가」가 사실이 된다
/// (TRD §3-7 금지 조항).
/// ★tombstone 에 만료를 두지 않는다★ — 시간 만료를 두면 같은 질문에 시점에 따라 다른 오류가 나가
/// 두 판정이 호출자 입장에서 구분 불가능해진다(ADR-0135 · TRD §4-②).
///
/// ★이름 하나에 자취도 하나다 — 재등록은 last-wins 로 **덮는다**★
///
/// 만료가 없으므로 자취를 쌓으면 지울 수단이 아무 데도 없다. 주인 토큰은 연결마다 새로 나므로
/// (`OwnerToken`), 같은 셸이 N 번 재연결하면 같은 이름에 N 개의 죽은 항목이 영구히 남고 등록·정리가
/// 매번 그 전부를 훑는다. 이름당 하나면 **재연결 축**이 닫힌다.
/// ★단 그것만으로는 크기가 안 묶인다★ — 이름은 등록 패킷이 실어 온 **문자열**이라(빌드 상수가 아니다)
/// 명부는 「어휘 크기」가 아니라 **주인이 실제로 보낸 서로 다른 이름 수**만큼 자란다. ADR-0135
/// 「바운드 대신」이 어휘 크기를 말한 것은 그 전제 위였고, 남는 축은 개수 상한과 길이 상한이 닫는다
/// ([`Roster::MAX_NAMES_PER_OWNER`] · [`Roster::MAX_NAMES`] · [`Roster::MAX_NAME_BYTES`] ·
/// [`Roster::MAX_HELP_BYTES`]).
/// ★대가★: 서로 다른 두 주인이 같은 이름을 동시에 쥔 상태에서 **나중 주인이 끊기면** 앞 주인이 아직
/// 붙어 있어도 그 이름은 `OWNER_UNAVAILABLE` 이 된다. v1 은 단일 앱 인스턴스 전제이고 정책이
/// last-wins 하나라(TRD §1 「다중 앱 인스턴스」) 이 상태 자체가 v1 범위 밖이다. 실제로 위험한 절반 —
/// **재연결 겹침**(새 연결이 등록한 뒤 옛 연결의 cleanup 이 도착) — 은 그대로 막힌다: 정리는 자기가
/// 아직 주인인 이름만 내린다.
// ADR-0135
#[derive(Debug, Default)]
pub struct Roster {
    /// 이름 → 그 이름을 **마지막으로** 등록한 주인의 자취(가용이거나 tombstone).
    entries: BTreeMap<String, Registration>,
}

#[derive(Debug, Clone)]
struct Registration {
    owner: OwnerToken,
    help: String,
    available: bool,
}

impl Roster {
    /// 주인 하나가 명부에 **누적으로** 쥘 수 있는 이름 수 — 지금 보낸 패킷이 아니라 그 주인 앞으로
    /// 남아 있는 자취 전량(가용 + tombstone)에 이번 패킷을 더한 값을 잰다.
    ///
    /// ★이름은 등록 패킷이 실어 온 **문자열**이지 빌드 상수가 아니다★ — 상한이 없으면 오작동하거나
    /// 악의적인 주인 하나가 명부를 무한히 불린다(만료가 없으므로 회수 수단도 없다 — ADR-0135).
    /// 값의 근거: 지금 어휘 전량이 60 남짓이고(화면 33 · core 5 · daemon 3 · `src-tauri` 17) 가장 큰
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
    /// ★만료가 없으므로 이 문자열은 주인이 끊긴 뒤에도 데몬 수명 내내 남는다★(ADR-0135) — 상한이
    /// 없으면 과장된 `help` 하나가 영구 상주 메모리가 된다.
    /// 값의 근거: 지금 가장 큰 항목이 `agent.spawn` 의 591바이트다(`core/bindings/commands.schema.json`
    /// 실측). 4096 = 그 일곱 배로, 중첩 struct 를 몇 겹 더 편 선언도 넉넉히 든다.
    /// 두 상한을 합치면 명부의 상주 상한이 [`Roster::MAX_NAMES`] × (128 + 4096) ≈ 16 MiB 로 닫힌다.
    pub const MAX_HELP_BYTES: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    /// 붙을 때 한 방 — 그 주인의 **전량**이다. 재연결 재전송이 같은 경로를 타므로 이 주인이 이번에
    /// 싣지 않은 옛 이름은 tombstone 으로 내린다(주인 단위 last-wins, ADR-0081 「RegisterRole 재전송」).
    ///
    /// ★상한을 넘으면 **한 이름도 넣지 않고** `INVALID_ARGUMENT` 로 돌려보낸다★ — 넘치는 만큼만 잘라
    /// 넣으면 주인은 성공으로 알고 일부 이름이 조용히 없는 상태가 된다(그 이름은 `UNKNOWN_COMMAND` 로
    /// 나가는데, 등록한 쪽에는 그럴 이유가 없다). 거절은 명부를 **건드리기 전에** 판정하므로 실패한
    /// 등록이 기존 상태를 바꾸지 않는다.
    /// ★이 상한은 ADR-0135 가 거부한 TTL 과 다른 축이다★ — 거기서 거부한 것은 **시간**이라 같은 질문의
    /// 답이 시계에 따라 갈렸다. 개수 상한은 명부에 든 이름의 답을 바꾸지 않는다.
    pub fn register(
        &mut self,
        owner: &OwnerToken,
        decls: Vec<CommandDecl>,
    ) -> Result<(), CommandError> {
        self.check_room_for(owner, &decls)?;
        self.tombstone_owner(owner);
        for decl in decls {
            self.entries.insert(
                decl.name,
                Registration {
                    owner: owner.clone(),
                    help: decl.help,
                    available: true,
                },
            );
        }
        Ok(())
    }

    /// 이 등록이 상한 안인가 — **명부를 건드리기 전에** 본다.
    ///
    /// ★주인당 상한은 패킷 하나가 아니라 그 주인의 **누적**을 잰다★: [`Roster::tombstone_owner`] 는
    /// 이번에 안 실린 옛 이름을 지우지 않고 플래그만 내리므로(만료 없음 — ADR-0135), 패킷만 재면 서로
    /// 겹치지 않는 512개짜리 스냅샷을 잇달아 보내는 주인 하나가 명부 전체를 자기 자취로 채운다. 그러면
    /// 진짜 셸의 등록이 전체 상한에 막히고 `tab.*`·`window.*` 가 전부 `UNKNOWN_COMMAND` 로 나가는데,
    /// 그 코드는 「재시도가 무의미하니 이름을 다시 발견하라」는 뜻이라(TRD §4-②) 호출자는 실재하는
    /// 명령을 영영 포기한다. 그래서 전체 상한이 **주인별 몫 없는 공유 자원**이 되지 않게 여기서 막는다.
    /// 이번 패킷에 실린 이름은 덮어쓰기라 자리를 새로 먹지 않으므로 두 번 세지 않는다.
    /// ★남는 축(형태로 못 막는다)★: 주인 토큰은 연결마다 새로 나므로 재연결하며 매번 다른 512개를
    /// 등록하면 이 상한이 매번 0에서 시작한다. 그 축을 막는 것은 [`Roster::MAX_NAMES`] 하나뿐이다.
    ///
    /// 이름·`help` 는 등록 패킷이 실어 온 문자열이라 길이도 함께 잰다 — 만료가 없어 과장된 `help` 하나가
    /// 주인이 끊긴 뒤에도 데몬 수명 내내 상주한다.
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

    /// 연결이 끊겼다 — **그 주인이 아직 쥐고 있는** 이름만 내린다. 이름은 남는다.
    pub fn disconnect(&mut self, owner: &OwnerToken) {
        self.tombstone_owner(owner);
    }

    pub fn lookup(&self, name: &str) -> OwnerLookup {
        match self.entries.get(name) {
            None => OwnerLookup::Unknown,
            Some(reg) if reg.available => OwnerLookup::Available(reg.owner.clone()),
            Some(_) => OwnerLookup::Unavailable,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = RosterEntry> + '_ {
        self.entries.iter().map(|(name, reg)| RosterEntry {
            name: name.clone(),
            help: reg.help.clone(),
            owner: reg.owner.clone(),
            available: reg.available,
        })
    }

    /// 이름 수 — 자취가 이름당 하나라 이 값이 곧 명부의 크기다.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn tombstone_owner(&mut self, owner: &OwnerToken) {
        for reg in self.entries.values_mut() {
            if &reg.owner == owner {
                reg.available = false;
            }
        }
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

    #[test]
    fn disconnect_keeps_the_name_and_its_shape() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, vec![decl("tab.create")])
            .expect("상한 안");
        roster.disconnect(&owner);

        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unavailable);
        let entry = roster.entries().next().expect("tombstone 이 남는다");
        assert_eq!(entry.help, "{\"name\":\"tab.create\"}");
    }

    /// ★재연결 겹침 — 옛 연결의 cleanup 이 새 등록을 내리면 안 된다★
    ///
    /// 주인 토큰은 연결마다 새로 나므로 셸이 재연결하면 등록(새 토큰)이 먼저 오고 옛 연결의 정리가
    /// 뒤늦게 온다. 정리가 이름 단위로 내려가면 방금 붙은 셸의 명령이 `OWNER_UNAVAILABLE` 이 된다.
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
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unavailable);
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

    /// ★만료가 없으므로 자취가 쌓이면 지울 수단이 없다★ — 이름당 하나여야 크기가 어휘로 묶인다.
    #[test]
    fn reconnecting_many_times_leaves_one_trace_per_name() {
        let mut roster = Roster::new();
        for connection in 0..50 {
            let owner = OwnerToken::new(format!("shell-conn-{connection}"));
            roster
                .register(&owner, vec![decl("tab.create"), decl("tab.close")])
                .expect("상한 안");
            roster.disconnect(&owner);
        }

        assert_eq!(roster.len(), 2, "이름 수만큼만 남는다");
        assert_eq!(roster.entries().count(), 2);
        assert_eq!(roster.lookup("tab.create"), OwnerLookup::Unavailable);
    }

    /// 마지막 자취의 `help` 가 남는다 — 주인이 꺼져 있어도 모양은 조회된다(ADR-0135).
    #[test]
    fn the_surviving_trace_is_the_latest_one() {
        let old = OwnerToken::new("shell-conn-1");
        let new = OwnerToken::new("shell-conn-2");
        let mut roster = Roster::new();
        roster
            .register(&old, vec![decl("tab.create")])
            .expect("상한 안");
        roster
            .register(
                &new,
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: "{\"name\":\"tab.create\",\"since\":2}".to_string(),
                }],
            )
            .expect("상한 안");
        roster.disconnect(&new);

        let entry = roster.entries().next().expect("tombstone 이 남는다");
        assert_eq!(entry.help, "{\"name\":\"tab.create\",\"since\":2}");
        assert_eq!(entry.owner, new);
        assert!(!entry.available);
    }

    #[test]
    fn never_seen_name_is_unknown() {
        assert_eq!(Roster::new().lookup("tab.create"), OwnerLookup::Unknown);
    }

    fn decls(prefix: &str, count: usize) -> Vec<CommandDecl> {
        (0..count).map(|i| decl(&format!("{prefix}.{i}"))).collect()
    }

    /// ★한 주인이 명부를 무한히 불릴 수 없다★ — 이름은 등록 패킷이 보낸 문자열이고 만료가 없으므로
    /// (ADR-0135) 상한이 유일한 회수 수단이다.
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

    /// ★주인당 상한은 **누적**이다 — 자기 tombstone 이 자기 몫을 먹는다★
    ///
    /// 자취는 만료되지 않으므로(ADR-0135) 패킷 하나만 재면 서로 겹치지 않는 스냅샷을 잇달아 보내는
    /// 주인이 전체 상한까지 혼자 차지한다. 그러면 진짜 셸의 등록이 막히고, 실재하는 `tab.*` 가
    /// `UNKNOWN_COMMAND`(= 「재시도 무의미, 이름을 다시 발견하라」 — TRD §4-②)로 나가 호출자가 그
    /// 명령을 영영 포기한다.
    #[test]
    fn an_owners_own_tombstones_count_against_its_cap() {
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
        assert!(
            roster.len() < Roster::MAX_NAMES,
            "한 주인이 전체 상한을 혼자 먹지 못한다"
        );
    }

    /// 끊긴 뒤에도 마찬가지다 — tombstone 은 지워지지 않으므로 같은 주인의 몫을 계속 쥔다.
    #[test]
    fn a_disconnected_owners_names_still_count_when_it_registers_again() {
        let owner = OwnerToken::new("shell-1");
        let mut roster = Roster::new();
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("상한 안");
        roster.disconnect(&owner);

        roster
            .register(&owner, vec![decl("wave1.0")])
            .expect_err("자취가 자리를 쥐고 있다");

        // 같은 이름을 다시 실으면 통과한다 — 덮어쓰기는 자리를 새로 먹지 않는다.
        roster
            .register(&owner, decls("wave0", Roster::MAX_NAMES_PER_OWNER))
            .expect("재등록은 상한 안");
        assert_eq!(roster.len(), Roster::MAX_NAMES_PER_OWNER);
    }

    /// ★이름·`help` 는 등록 패킷이 실어 온 문자열이고 만료가 없다★ — 길이를 안 재면 과장된 `help` 하나가
    /// 주인이 끊긴 뒤에도 데몬 수명 내내 상주한다(ADR-0135).
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
        assert_eq!(roster.lookup("tab.legacy"), OwnerLookup::Unavailable);
    }
}
