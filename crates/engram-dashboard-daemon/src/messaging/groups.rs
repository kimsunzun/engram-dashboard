//! groups — 그룹 명단 레지스트리 + 해석 seam(spec §4 · ADR-0104 결정 1).
//!
//! ★역할★: "그룹 이름 → 멤버 이름 목록" 을 해석한다. v1 소스 = ① 런타임 등록 명단(생성·증감·삭제,
//!   인메모리) ② 내장 `@all`. 멤버십은 **이름 기반**(id 아님 — WYSIWYA, ADR-0101 · 재스폰 생존).
//!
//! ★그룹 해석 seam(load-bearing — ADR-0104 결정 1)★: 미래 소스(폴더 `@폴더명`·계층)를 **메시징 파이프라인
//!   (ingress·MessagingService)을 건드리지 않고** 추가할 수 있게 해석 지점을 seam 으로 분리한다. 그 확장점
//!   = `GroupSource` 트레잇이다 — v1 은 이 레지스트리가 유일 구현이고, 폴더가 데몬 소유로 생기면 다른
//!   `GroupSource` 구현을 추가하고 `Groups::resolve` 가 소스들을 순회하게만 하면 된다(파이프라인 불변).
//!   소스 지식이 파이프라인에 새면 ADR-0104 위반. **over-build 금지** — 폴더/계층 구현은 지금 만들지 않는다.
//!
//! ★순수·기계적 해석(load-bearing 경계, ADR-0104)★: `resolve` 는 그룹을 멤버 이름 목록으로 **펼치기만**
//!   한다 — 살아있음(liveness) 판정·죽은 멤버 skip 은 여기서 하지 않는다. 레지스트리는 순수해서 **누가
//!   살아 있는지 모른다**. `@all` 조차 해석 시 호출자가 넘긴 live 스냅샷을 그대로 돌려줄 뿐이다. 발송 순간
//!   스냅샷·skip 회계는 후속 increment(MessagingService)의 몫이다(경로·책임 분리).
// ADR-0104

use std::collections::HashMap;

/// 내장 그룹 `@all` — "발송 순간 살아있는 수신 가능 전원"(spec §4 · ADR-0103 결정 4).
///
/// ★내장 불변식(load-bearing)★: `@all` 은 생성·삭제·증감이 **불가**하다(관리 불요, 예약 이름). CRUD 가
///   이 이름을 만나면 `Builtin` 에러로 거절한다 — 사용자 등록 명단과 이름이 충돌해 방송 의미가 오염되는 것을
///   막는다. 해석 시엔 호출자가 넘긴 live 스냅샷을 verbatim 반환한다(레지스트리는 liveness 를 모름).
// ADR-0103
// ADR-0104
pub const ALL_GROUP: &str = "@all";

/// 그룹 연산 에러 — 상위가 wire 에러 코드로 매핑한다(spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// 그룹 이름이 `@` 네임스페이스 규약을 어김(선행 `@` 없음 등). 정규화 실패.
    InvalidName { name: String },
    /// 내장 `@all` 을 생성/삭제/증감하려 함 — 거절(spec §4 내장 그룹 보호).
    Builtin,
    /// 지목한 그룹이 등록 명단에 없음 → `GROUP_NOT_FOUND`.
    NotFound { name: String },
    /// 그룹은 있으나 해석 결과 멤버가 0명 → `GROUP_EMPTY`(빈 그룹 발송 반려, spec §4).
    Empty { name: String },
}

/// 그룹 해석 소스(seam — ADR-0104 결정 1). "그룹 이름 → 멤버 이름 목록" 해석기의 확장점.
///
/// ★왜 트레잇인가★: v1 소스는 런타임 명단(이 파일의 `Groups`)뿐이지만, 폴더(`@폴더명`)가 데몬 소유로
///   생기면 **다른 구현**(폴더 트리를 훑는 소스)을 추가하고 상위 해석기가 소스들을 순회하게만 하면 된다 —
///   메시징 파이프라인은 그대로다. 지금은 확장점만 깔고(저위험·장기, CLAUDE.md §0) 폴더/계층 구현은 만들지
///   않는다(over-build 금지 — ADR-0104 "확장점만").
/// ★live 스냅샷 주입★: 해석은 순수해야 하므로 `@all` 처리에 필요한 "지금 살아있는 이름들"을 **호출자가**
///   넘긴다(소스가 프로세스 생사를 조회하지 않음 — 순수성·seam 격리).
pub trait GroupSource {
    /// `group` 이름을 멤버 이름 목록으로 펼친다. 이 소스가 그 그룹을 모르면 `NotFound`.
    /// `live` = 발송 순간 살아있는 수신 가능 이름 스냅샷(`@all` 등 동적 그룹 해석용, 호출자 주입).
    fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError>;
}

/// 런타임 등록 그룹 명단(인메모리, spec §4). 데몬 재시작 시 소멸(인메모리 단계 정합).
///
/// ★순수★: 프로세스 생사·시계를 모른다(모듈 헤더 순수성). `@all` 해석은 호출자 live 스냅샷에만 의존한다.
#[derive(Debug, Default)]
pub struct Groups {
    /// 그룹 이름(정규화된 `@…`) → 멤버 이름 목록(등록 순서 보존 — 방송 순서 결정적).
    /// ★`@all` 은 여기 없다★: 내장이라 저장하지 않고 해석 시 live 스냅샷으로 특수 처리한다.
    registered: HashMap<String, Vec<String>>,
}

impl Groups {
    /// 빈 레지스트리.
    pub fn new() -> Self {
        Self::default()
    }

    /// 이름을 검증·정규화한다(`@` 네임스페이스). 성공 시 정규화된 이름을 돌려준다.
    ///
    /// ★엄격 규약(load-bearing — @-네임스페이스 계약 · finding 3)★: 그룹 이름은 **반드시 정확히 하나의
    ///   선행 `@` 로 시작**해야 한다. 선행 `@` 가 없으면(`coders`) `InvalidName` 으로 거부한다 — 예전의 관대
    ///   보정(`coders` → `@coders`)은 `@` 네임스페이스 계약을 흐려(사람/그룹 이름 구분이 `@` 하나에 걸려
    ///   있음) 거부됐다. `@` 뒤 본문(remainder)이 비었거나(`@` 단독) 공백만이면 역시 `InvalidName`.
    /// ★본문에 추가 `@` 금지(finding 3)★: 선행 `@` 를 하나 벗긴 뒤 본문에 또 `@` 가 있으면(`@@x`·`@@`·
    ///   `@a@b`) 거부한다. "정확히 하나의 `@`" 계약을 문자 그대로 강제하는 가장 단순한 규칙이다 — 본문 내부
    ///   `@`(`@a@b`)를 허용하면 어디까지가 네임스페이스 마커고 어디부터가 이름인지 모호해져 사람/그룹 구분이
    ///   흐려진다. 그래서 이름에 `@` 는 **선행 하나만** 허용한다(내부·중복 `@` 전부 거부).
    /// ★입구 정규화는 후속 increment★: 툴 인자를 관대하게 받아 `@` 를 붙여 주는 편의 정규화가 필요하면
    ///   그건 **MCP/CLI 입구 계층**에서 하고(entrance-layer), 이 순수 레지스트리는 계약을 엄격히 강제한다
    ///   (경계 분리 — 여기서 관대해지면 계약 위반을 저장소가 은폐).
    fn normalize(name: &str) -> Result<String, GroupError> {
        let trimmed = name.trim();
        // 정확히 하나의 선행 `@` 요구 — 없으면 거부.
        let Some(body) = trimmed.strip_prefix('@') else {
            return Err(GroupError::InvalidName {
                name: name.to_string(),
            });
        };
        // 본문이 비었거나(`@` 단독) 공백만 → 거부. 그리고 본문에 또 `@` 가 있으면(`@@x`·`@a@b`) 거부 —
        // "정확히 하나의 `@`" 계약(선행 하나만, 내부·중복 `@` 불가).
        if body.is_empty() || body.chars().all(char::is_whitespace) || body.contains('@') {
            return Err(GroupError::InvalidName {
                name: name.to_string(),
            });
        }
        Ok(format!("@{body}"))
    }

    /// 정규화된 이름이 내장 `@all` 인가 — CRUD 보호 검사용.
    fn is_builtin(normalized: &str) -> bool {
        normalized == ALL_GROUP
    }

    /// 그룹 생성(빈 멤버). 이미 있으면 no-op(멱등 — 재등록으로 기존 명단을 지우지 않음). `@all` 은 거절.
    ///
    /// ★멱등 근거★: 오케스트레이터가 스폰마다 같은 그룹을 재선언할 수 있어(런타임 등록), 재-create 가 기존
    ///   멤버를 날리면 사고다. 그래서 있으면 그대로 둔다(멤버 조작은 add/remove 로).
    pub fn create(&mut self, name: &str) -> Result<(), GroupError> {
        let norm = Self::normalize(name)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        self.registered.entry(norm).or_default();
        Ok(())
    }

    /// 그룹에 멤버(이름) 추가. 그룹이 없으면 **생성 후 추가**(등록 편의 — spec §4 "생성·증감"). 중복 이름은
    /// 무시(집합 의미). `@all` 은 거절.
    pub fn add_member(&mut self, group: &str, member: &str) -> Result<(), GroupError> {
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        let members = self.registered.entry(norm).or_default();
        if !members.iter().any(|m| m == member) {
            members.push(member.to_string());
        }
        Ok(())
    }

    /// 그룹에서 멤버 제거. 그룹이 없으면 `NotFound`. 없는 멤버 제거는 no-op(멱등). `@all` 은 거절.
    pub fn remove_member(&mut self, group: &str, member: &str) -> Result<(), GroupError> {
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        let members = self
            .registered
            .get_mut(&norm)
            .ok_or_else(|| GroupError::NotFound { name: norm.clone() })?;
        members.retain(|m| m != member);
        Ok(())
    }

    /// 그룹 삭제. 없으면 `NotFound`. `@all` 은 거절(내장 삭제 불가).
    pub fn delete(&mut self, group: &str) -> Result<(), GroupError> {
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        self.registered
            .remove(&norm)
            .map(|_| ())
            .ok_or(GroupError::NotFound { name: norm })
    }

    /// 등록 그룹 이름 목록(내장 `@all` 포함). MCP `group` 무인자 조회 = 목록(spec §6). 순서 비결정(HashMap)
    /// 이라 상위가 표시 시 정렬한다(여기선 값만).
    ///
    /// ★`@all` 은 항상 목록에 포함★(내장이라 저장은 안 하지만 사용자에겐 존재하는 그룹으로 보여야 — spec §6
    ///   "무인자=목록(@all 포함)"). 등록 명단 앞에 붙여 반환한다.
    pub fn list(&self) -> Vec<String> {
        let mut names = vec![ALL_GROUP.to_string()];
        names.extend(self.registered.keys().cloned());
        names
    }
}

impl GroupSource for Groups {
    /// 그룹 이름 → 멤버 이름 목록(순수·기계적, ADR-0104). `@all` = live 스냅샷 verbatim, 등록 그룹 = 그
    /// 멤버 목록. **liveness 교차·skip 없음**(발송 순간 스냅샷/skip 은 후속 increment — 경계 문서 참조).
    ///
    /// ★결과 비어있음(load-bearing 구분)★: 등록 그룹인데 멤버 0명(또는 `@all` 인데 live 스냅샷 0명) →
    /// `Empty`(빈 그룹 발송 반려, spec §4) / 등록 명단에 없는 이름(그리고 `@all` 아님) → `NotFound`. 두
    /// 에러는 상위가 `GROUP_EMPTY`/`GROUP_NOT_FOUND` 로 각각 매핑하므로 반드시 구분한다(spec §4).
    ///
    /// ★`@all` 이 등록 명단과 격리★: `@all` 은 저장소에 없고 여기서 특수 분기로만 처리한다(내장 불변식) —
    ///   그래서 사용자가 `@all` 을 등록해 의미를 덮어쓸 수 없다(create/add 가 이미 Builtin 으로 거절).
    fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
        let norm = Self::normalize(group)?;
        // 내장 @all — live 스냅샷 verbatim(레지스트리는 liveness 를 모름, 호출자가 주입).
        if Self::is_builtin(&norm) {
            if live.is_empty() {
                return Err(GroupError::Empty { name: norm });
            }
            return Ok(live.to_vec());
        }
        // 등록 그룹 — 멤버 목록 그대로(liveness 교차·skip 없음, 경계 문서 참조).
        let members = self
            .registered
            .get(&norm)
            .ok_or_else(|| GroupError::NotFound { name: norm.clone() })?;
        if members.is_empty() {
            return Err(GroupError::Empty { name: norm });
        }
        Ok(members.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn create_add_remove_delete_roundtrip() {
        let mut g = Groups::new();
        g.create("@coders").unwrap();
        g.add_member("@coders", "alice").unwrap();
        g.add_member("@coders", "bob").unwrap();
        let members = g.resolve("@coders", &[]).unwrap();
        assert_eq!(members, names(&["alice", "bob"]), "등록 순서 보존");
        g.remove_member("@coders", "alice").unwrap();
        assert_eq!(g.resolve("@coders", &[]).unwrap(), names(&["bob"]));
        g.delete("@coders").unwrap();
        assert_eq!(
            g.resolve("@coders", &[]),
            Err(GroupError::NotFound {
                name: "@coders".to_string()
            }),
            "삭제 후엔 NotFound"
        );
    }

    #[test]
    fn add_member_creates_group_if_absent() {
        let mut g = Groups::new();
        // create 없이 add 만 해도 그룹이 생겨야(등록 편의, spec §4).
        g.add_member("@qa", "carol").unwrap();
        assert_eq!(g.resolve("@qa", &[]).unwrap(), names(&["carol"]));
    }

    #[test]
    fn add_member_dedups() {
        let mut g = Groups::new();
        g.add_member("@x", "a").unwrap();
        g.add_member("@x", "a").unwrap();
        assert_eq!(
            g.resolve("@x", &[]).unwrap(),
            names(&["a"]),
            "중복 이름 무시"
        );
    }

    #[test]
    fn create_is_idempotent_and_preserves_members() {
        let mut g = Groups::new();
        g.add_member("@x", "a").unwrap();
        // 재-create 가 기존 멤버를 날리면 안 됨(멱등).
        g.create("@x").unwrap();
        assert_eq!(g.resolve("@x", &[]).unwrap(), names(&["a"]));
    }

    #[test]
    fn all_group_is_builtin_protected() {
        let mut g = Groups::new();
        assert_eq!(g.create("@all"), Err(GroupError::Builtin), "create 거절");
        assert_eq!(
            g.add_member("@all", "x"),
            Err(GroupError::Builtin),
            "add 거절"
        );
        assert_eq!(
            g.remove_member("@all", "x"),
            Err(GroupError::Builtin),
            "remove 거절"
        );
        assert_eq!(g.delete("@all"), Err(GroupError::Builtin), "delete 거절");
    }

    #[test]
    fn all_group_resolves_to_live_snapshot_verbatim() {
        let g = Groups::new();
        let live = names(&["a", "b", "c"]);
        let resolved = g.resolve("@all", &live).unwrap();
        assert_eq!(
            resolved, live,
            "@all = live 스냅샷 verbatim(교차·정렬 없음)"
        );
    }

    #[test]
    fn all_group_empty_live_is_group_empty() {
        let g = Groups::new();
        assert_eq!(
            g.resolve("@all", &[]),
            Err(GroupError::Empty {
                name: "@all".to_string()
            }),
            "live 0명이면 @all 도 GROUP_EMPTY"
        );
    }

    #[test]
    fn unprefixed_name_is_rejected() {
        // 선행 @ 없는 이름은 엄격 거부(관대 보정 폐기 — @-네임스페이스 계약, finding 7).
        let mut g = Groups::new();
        assert!(
            matches!(
                g.add_member("coders", "a"),
                Err(GroupError::InvalidName { .. })
            ),
            "선행 @ 없는 add 는 InvalidName"
        );
        assert!(
            matches!(g.create("coders"), Err(GroupError::InvalidName { .. })),
            "선행 @ 없는 create 는 InvalidName"
        );
        assert!(
            matches!(
                g.resolve("coders", &[]),
                Err(GroupError::InvalidName { .. })
            ),
            "선행 @ 없는 resolve 는 InvalidName"
        );
    }

    #[test]
    fn empty_or_at_only_name_is_invalid() {
        let mut g = Groups::new();
        assert!(matches!(g.create("@"), Err(GroupError::InvalidName { .. })));
        assert!(matches!(g.create(""), Err(GroupError::InvalidName { .. })));
        assert!(matches!(
            g.create("  @  "),
            Err(GroupError::InvalidName { .. })
        ));
    }

    #[test]
    fn double_at_or_inner_at_name_is_invalid() {
        // finding 3: "정확히 하나의 `@`" — 선행 `@` 벗긴 본문에 또 `@` 가 있으면 거부.
        let mut g = Groups::new();
        assert!(
            matches!(g.create("@@x"), Err(GroupError::InvalidName { .. })),
            "@@x 는 InvalidName(둘째 @)"
        );
        assert!(
            matches!(g.create("@@"), Err(GroupError::InvalidName { .. })),
            "@@ 는 InvalidName"
        );
        assert!(
            matches!(g.create("@a@b"), Err(GroupError::InvalidName { .. })),
            "@a@b 는 InvalidName(내부 @)"
        );
        // 대조: 정확히 하나의 선행 @ 는 정상 수용.
        assert!(g.create("@coders").is_ok(), "@coders 는 정상 수용");
    }

    #[test]
    fn resolve_not_found_vs_empty_are_distinct() {
        let mut g = Groups::new();
        // 미등록 이름 → NotFound.
        assert!(matches!(
            g.resolve("@ghost", &[]),
            Err(GroupError::NotFound { .. })
        ));
        // 등록됐으나 멤버 0명 → Empty(둘은 구별돼야 — GROUP_NOT_FOUND vs GROUP_EMPTY 매핑).
        g.create("@hollow").unwrap();
        assert!(matches!(
            g.resolve("@hollow", &[]),
            Err(GroupError::Empty { .. })
        ));
    }

    #[test]
    fn list_includes_all_and_registered() {
        let mut g = Groups::new();
        g.create("@team").unwrap();
        let listed = g.list();
        assert!(listed.contains(&ALL_GROUP.to_string()), "@all 항상 포함");
        assert!(listed.contains(&"@team".to_string()));
    }

    #[test]
    fn registered_group_ignores_live_snapshot() {
        // 등록 그룹 해석은 순수·기계적 — live 스냅샷과 교차하지 않는다(경계: skip 은 후속 increment).
        let mut g = Groups::new();
        g.add_member("@x", "dead-member").unwrap();
        // live 에 없는(죽은) 멤버여도 resolve 는 그대로 반환(skip 안 함).
        let resolved = g.resolve("@x", &names(&["someone-else"])).unwrap();
        assert_eq!(resolved, names(&["dead-member"]));
    }

    #[test]
    fn resolve_via_trait_object_seam() {
        // seam 확인: GroupSource 트레잇 객체로 해석해도 동일(파이프라인이 트레잇에만 의존 가능).
        let mut g = Groups::new();
        g.add_member("@x", "a").unwrap();
        let src: &dyn GroupSource = &g;
        assert_eq!(src.resolve("@x", &[]).unwrap(), names(&["a"]));
    }
}
