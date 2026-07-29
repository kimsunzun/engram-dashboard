//! groups — `@`주소 해석 seam(spec §4 · ADR-0104 결정 1 · **ADR-0111 결정 4 · ADR-0112 결정 1**).
//!
//! ★역할★: "`@`주소 → 멤버 이름 목록" 을 펼친다. **v1 소스는 내장 `@all` 하나뿐**이다 — 멤버 = 발송 순간
//!   살아 있는 수신 가능 전원(호출자가 로스터 스냅샷에서 뽑아 넘긴다). 멤버십은 **이름 기반**(id 아님 —
//!   WYSIWYA, ADR-0101 · 재스폰 생존).
//!
//! ★그룹은 발송 경로가 아니라 **주소 해석 매크로**다(ADR-0111 결정 4 — 이 모듈의 존재 이유)★: 펼친
//!   이름들은 직접 지목한 수신자와 **완전히 같은** 경로로 흐른다(spec §5 다중 수신자 fan-out). 그룹 전용
//!   배달 규칙(죽은 멤버 skip · 발송 순간 `(id,epoch)` 결박 · 전용 fan-out 분기)은 **전부 폐지**됐다 —
//!   되살리면 ADR-0111 위반이다.
//!
//! ★사용자 정의 그룹은 제거됐다(ADR-0111 결정 4 · ADR-0112 결정 1)★: 런타임 등록 명단(레지스트리) ·
//!   `group` MCP 툴 · CLI `group` 서브커맨드 · ADR-0109 관리 semantics 일체가 사라졌다. 저장형·이름붙인
//!   그룹은 실수요가 서는 시점(폴더 그룹이 1순위 후보)에 **재설계**한다 — 그룹을 위한 저장 구조를 미리
//!   두지 않는다.
//!
//! ★해석 seam 은 유지한다(ADR-0104 결정 1 의 *방향* 존속, 소스만 축소)★: 미래 소스(폴더 `@폴더명`)를
//!   메시징 파이프라인을 건드리지 않고 추가할 수 있게 `GroupSource` 확장점을 남긴다. v1 구현은
//!   `BuiltinGroups`(= `@all`) 하나다. **over-build 금지** — 폴더/계층 구현은 지금 만들지 않는다.
//!
//! ★순수·기계적 해석(load-bearing 경계, ADR-0104)★: `resolve` 는 이름 목록으로 **펼치기만** 한다 —
//!   살아있음 판정·발신자 제외·중복 제거·로스터 대조는 전부 상위(`MessagingService`)의 몫이다. 특히
//!   **"펼침 결과 0명" 은 여기서 에러가 아니다**(빈 목록을 그대로 돌려준다): `GROUP_EMPTY` 는 펼침 + 명시
//!   지목을 합친 **최종 수신자 집합**이 비었을 때만 나는 판정이라(ADR-0114 결정 3) 이 층에서 알 수 없다.
// ADR-0104
// ADR-0111
// ADR-0112

/// 내장 그룹 `@all` — "발송 순간 살아있는 수신 가능 전원 **− 발신자**"(spec §4 · ADR-0111 결정 4).
///
/// ★내장 불변식(load-bearing)★: 저장소가 없으므로 생성·삭제·증감이라는 개념 자체가 없다(관리 표면 제거 —
///   ADR-0112 영향 절). 해석 시엔 호출자가 넘긴 live 스냅샷을 verbatim 반환한다(레지스트리는 liveness 를
///   모른다 — 발신자 제외는 호출자가 스냅샷을 만들 때 이미 적용한다).
// ADR-0111
pub const ALL_GROUP: &str = "@all";

/// `@`주소 해석 에러 — 상위가 wire 에러 코드로 매핑한다(spec §4).
///
/// ★두 값뿐인 이유(ADR-0111/0114)★: 등록 명단이 사라져 `Builtin`·`InvalidMemberName` 은 표현할 상태가
///   없어졌고, `Empty` 는 판정 시점이 **최종 수신자 집합**으로 올라가 이 층을 떠났다(모듈 헤더).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// 이름이 `@` 네임스페이스 규약을 어김(선행 `@` 없음·`@` 단독·중복 `@`). 정규화 실패.
    InvalidName { name: String },
    /// `@all` 이 아닌 `@이름` — 존재하지 않는 주소 → `GROUP_NOT_FOUND`(발송 단위 전체 반려, ADR-0114 결정 3).
    NotFound { name: String },
}

/// ★그룹 주소 정규화 — **seam 레벨** 자유 함수(C4 리뷰 fix F · ADR-0104 결정 1)★. `@` 네임스페이스 규약을
/// 검증하고 정규화된 이름을 돌려준다. 실패는 `InvalidName`.
///
/// ★왜 소스 구현의 메서드가 아니라 자유 함수인가(load-bearing — seam 경계)★: 발송 파이프라인은 소스에
///   물어보기 **전에** 이름을 정규화한다(봉투 `to` 토큰·에러 hint 가 그 값을 쓴다). 그 정규화가 특정 소스
///   구현의 메서드면 파이프라인이 그 타입에 묶여, 이름 문법이 다른 미래 소스가 도달 불가해진다(ADR-0104
///   "소스 지식이 파이프라인에 새면 위반"). 그래서 규약을 소스 밖 seam 레벨에 두고 전원이 공유한다.
///
/// ★엄격 규약(load-bearing — @-네임스페이스 계약)★: 그룹 이름은 **반드시 정확히 하나의 선행 `@`** 로
/// 시작해야 한다. 선행 `@` 가 없으면(`coders`) `InvalidName` — 관대 보정(`coders` → `@coders`)은 사람/그룹
/// 구분이 `@` 하나에 걸린 계약을 흐려 거부됐다. `@` 뒤 본문이 비었거나 공백만이면, 또 본문에 `@` 가 더
/// 있으면(`@@x`·`@a@b`) 역시 `InvalidName`.
// ADR-0104
pub fn normalize_group_name(name: &str) -> Result<String, GroupError> {
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

/// 그룹 해석 소스(seam — ADR-0104 결정 1). "`@`주소 → 멤버 이름 목록" 해석기의 확장점.
///
/// ★왜 트레잇인가★: v1 소스는 내장 `@all` 하나뿐이지만, 폴더(`@폴더명`)가 데몬 소유로 생기면 **다른
///   구현**을 추가하고 상위가 소스들을 순회하게만 하면 된다 — 메시징 파이프라인은 그대로다. 지금은
///   확장점만 깔고(저위험·장기, CLAUDE.md §0) 폴더/계층 구현은 만들지 않는다.
/// ★live 스냅샷 주입★: 해석은 순수해야 하므로 `@all` 처리에 필요한 "지금 살아있는 이름들"을 **호출자가**
///   넘긴다(소스가 프로세스 생사를 조회하지 않음 — 순수성·seam 격리). **발신자 제외는 호출자가 이미
///   적용한 상태**로 온다(spec §4 — 펼침에서 발신자를 뺀다).
pub trait GroupSource {
    /// `group` 주소를 멤버 이름 목록으로 펼친다. 이 소스가 그 주소를 모르면 `NotFound`.
    /// ★빈 결과는 에러가 아니다★ — `Ok(vec![])` 로 돌려준다(모듈 헤더 "GROUP_EMPTY 는 최종 집합 판정").
    fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError>;
}

/// v1 유일 소스 — 내장 `@all` 만 안다(무저장 즉석 계산, ADR-0111 결정 4).
///
/// ★상태가 없다(ZST)★: 저장할 명단이 없으므로 단일 락 아래 둘 이유도 없다 —
/// `MessagingService` 가 필드로 들고 락 밖에서 부른다(로스터 스냅샷도 락 밖이라 자연스럽다).
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinGroups;

impl GroupSource for BuiltinGroups {
    /// `@all` = live 스냅샷 verbatim(정렬·dedup·필터 금지 — 그건 전부 상위 몫). 그 밖의 `@이름` = `NotFound`.
    fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
        let norm = normalize_group_name(group)?;
        if norm == ALL_GROUP {
            return Ok(live.to_vec());
        }
        Err(GroupError::NotFound { name: norm })
    }
}

/// ★`GroupSource` 계약 스위트 — 소스 교체 내구성 그물(사용자 지시 2026-07-26)★
///
/// ★왜 이게 있나(load-bearing — 이 모듈의 존재 이유)★: `GroupSource` 는 **미래에 갈아끼울 것을 전제로**
///   깐 seam 이다(ADR-0104 결정 1 — 폴더가 데몬 소유로 생기면 `@폴더명` 소스가 추가된다). 그때 새 구현이
///   조용히 다른 의미를 갖게 되는 게 최악이다: `@all` 이 정렬돼 나오거나, 빈 스냅샷이 에러로 접히거나,
///   `@` 규약이 관대해지거나 — 전부 **컴파일은 되고 기존 테스트는 초록**인 채로 방송 의미만 뒤집힌다.
///   그래서 해석 의미론을 구현이 아니라 **트레잇에 대해** 단언하는 재사용 스위트를 둔다.
///
/// ★이 스위트가 빨개지면★: 고칠 대상은 **스위트가 아니라 결정**이다. 해석 의미론은 spec §4 · ADR-0111
///   결정 4 · ADR-0114 결정 3 이 정한 사용자 결정 사항이라, 여기 단언을 느슨하게 만드는 수정은 곧 계약
///   변경이다 — 사용자 재가 없이 완화하지 말 것.
// ADR-0104
// ADR-0111
#[cfg(any(test, feature = "test-harness"))]
pub mod contract {
    use super::{GroupError, GroupSource, ALL_GROUP};

    /// 계약 스위트가 소스를 두드리는 데 필요한 **소스별 픽스처**.
    #[derive(Debug, Clone)]
    pub struct GroupSourceFixture {
        /// 이 소스가 **모르는** 주소 이름(`NotFound` 축). 이미 정규화된 형태(`@nope`)여야 한다.
        pub unknown: String,
        /// 이 소스가 내장 `@all` 을 **소유**하는가. v1 `BuiltinGroups` = true. 미래 폴더 소스 = 보통 false.
        pub handles_all: bool,
    }

    fn owned(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// ★`GroupSource` 해석 의미론 계약 — 어떤 구현이든 이걸 통과해야 한다(spec §4 · ADR-0111/0114)★.
    ///
    /// 단언하는 축:
    ///   1. `@all` = 호출자가 넘긴 live 스냅샷 **verbatim**(정렬·dedup·필터 금지 — 상위가 판정한다)
    ///   2. `@all` + 빈 스냅샷 = **`Ok(vec![])`**(에러 아님 — `GROUP_EMPTY` 는 최종 집합 판정, ADR-0114 결정 3)
    ///   3. 모르는 주소 = `NotFound`(전체 반려 층 — 주소 공간 오류)
    ///   4. `@` 네임스페이스 규약 위반 = `InvalidName`(조용한 보정 금지)
    ///   5. 결정적 — 같은 입력을 두 번 물으면 같은 답(내부 가변 상태·시계 의존 금지)
    ///
    /// ★단언 문구 앞의 `axisN-…:` 토큰(load-bearing)★: 모든 실패 메시지는 **축마다 유일한** 토큰으로
    ///   시작한다. 자체검증 테스트가 `#[should_panic(expected = …)]` 로 "그물이 무는지" 확인하는데, 부분
    ///   문자열이 여러 축에 겹치면 엉뚱한 축이 먼저 터져도 초록이 된다. **토큰을 바꾸면 핀도 함께 고칠 것.**
    pub fn assert_group_source_contract(src: &dyn GroupSource, fx: &GroupSourceFixture) {
        // ── 1·2. 내장 `@all` 축 ────────────────────────────────────────────────────────────
        // live 스냅샷은 "발송 순간 살아있는 이름들(발신자 제외 적용 후)" 이고, 소스는 그걸 **그대로**
        //   돌려줘야 한다. 정렬하면 상위의 행 순서 규칙(명시 토큰 → 펼침 사전순)이 두 층에 흩어지고,
        //   dedup 하면 동명 다수(RECIPIENT_AMBIGUOUS 사유)가 소스 단계에서 지워져 상위가 그 사실을 못 본다.
        let live = owned(&["zeta", "alpha", "zeta", "mid"]);
        if fx.handles_all {
            assert_eq!(
                src.resolve(ALL_GROUP, &live),
                Ok(live.clone()),
                "axis1-all-verbatim: @all 은 live 스냅샷 그대로여야 한다(정렬·dedup·필터 금지 — spec §4)"
            );
            assert_eq!(
                src.resolve(ALL_GROUP, &[]),
                Ok(Vec::new()),
                "axis2-all-empty-ok: 산 수신자 0명인 @all 은 빈 목록(에러 아님) — GROUP_EMPTY 는 명시 지목까지 합친 최종 집합으로 상위가 판정한다(ADR-0114 결정 3)"
            );
        } else {
            assert!(
                matches!(
                    src.resolve(ALL_GROUP, &live),
                    Err(GroupError::NotFound { .. })
                ),
                "axis2b-all-not-owned: @all 을 소유하지 않는 소스는 그걸 제 것인 양 해석하면 안 된다(소스 순회에서 방송 의미 가로채기 금지)"
            );
        }

        // ── 3. NotFound 축 ────────────────────────────────────────────────────────────────
        assert_eq!(
            src.resolve(&fx.unknown, &live),
            Err(GroupError::NotFound {
                name: fx.unknown.clone()
            }),
            "axis3-unknown-notfound: 모르는 주소는 NotFound 이고 name 은 정규화된 이름을 실어야 한다(상위가 hint 에 되쓴다)"
        );

        // ── 4. `@` 네임스페이스 규약 ───────────────────────────────────────────────────────
        // 관대한 보정(`coders` → `@coders`)은 사람 이름과 그룹 이름의 구분이 `@` 하나에 걸려 있는 계약을
        //   흐린다 — 소스가 조용히 보정하면 오타 하나로 엉뚱한 방송이 나간다.
        for bad in ["", "@", "  @  ", "@@x", "@@", "@a@b", "plain-name"] {
            assert!(
                matches!(src.resolve(bad, &live), Err(GroupError::InvalidName { .. })),
                "axis4-namespace-strict: '{bad}' 은 @ 네임스페이스 규약 위반이라 InvalidName 이어야 한다(조용한 보정 금지)"
            );
        }

        // ── 5. 결정성 — 같은 입력 두 번, 같은 답 ────────────────────────────────────────────
        // 소스는 순수해야 한다(시계·내부 가변 상태 금지). 이게 깨지면 "발송 순간 스냅샷 한 장" 계약이
        //   의미를 잃는다(같은 순간의 두 질문이 다른 세계를 본다).
        assert_eq!(
            src.resolve(&fx.unknown, &live),
            src.resolve(&fx.unknown, &live),
            "axis5-deterministic: 해석은 결정적이어야 한다(순수 — 시계·내부 가변 상태 금지)"
        );
        if fx.handles_all {
            assert_eq!(
                src.resolve(ALL_GROUP, &live),
                src.resolve(ALL_GROUP, &live),
                "axis5-deterministic-all: @all 해석도 결정적이어야 한다"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contract::{assert_group_source_contract, GroupSourceFixture};
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn all_group_resolves_to_the_live_snapshot_verbatim() {
        let live = names(&["zeta", "alpha", "zeta"]);
        // 정렬·dedup 없음 — 상위(MessagingService)가 행 순서·중복 제거·동명 판정을 소유한다.
        assert_eq!(BuiltinGroups.resolve("@all", &live), Ok(live));
    }

    #[test]
    fn all_group_with_no_live_names_is_an_empty_list_not_an_error() {
        // ADR-0114 결정 3: 펼침 0명은 에러가 아니다 — `["@all", "<자기이름>"]` 이 명시 지목 1행으로
        //   살아남는 규칙이 여기 성립한다(최종 집합이 비어야만 GROUP_EMPTY).
        assert_eq!(BuiltinGroups.resolve("@all", &[]), Ok(Vec::new()));
    }

    #[test]
    fn unknown_at_address_is_not_found() {
        assert_eq!(
            BuiltinGroups.resolve("@coders", &names(&["alice"])),
            Err(GroupError::NotFound {
                name: "@coders".to_string()
            }),
            "사용자 정의 그룹은 제거됐다(ADR-0111 결정 4) — @all 외의 @주소는 전부 주소 공간 오류"
        );
    }

    #[test]
    fn namespace_rules_are_strict() {
        for bad in ["", "@", " @ ", "@@x", "@a@b", "coders"] {
            assert!(
                matches!(
                    normalize_group_name(bad),
                    Err(GroupError::InvalidName { .. })
                ),
                "'{bad}' 은 규약 위반이어야 한다"
            );
        }
        assert_eq!(
            normalize_group_name(" @all ").unwrap(),
            "@all",
            "trim 만 한다"
        );
    }

    #[test]
    fn builtin_source_satisfies_the_group_source_contract() {
        assert_group_source_contract(
            &BuiltinGroups,
            &GroupSourceFixture {
                unknown: "@nope".to_string(),
                handles_all: true,
            },
        );
    }

    /// ★그물이 공허하지 않다는 증거★ — 일부러 어긋난 소스(정렬하는 `@all`)를 스위트에 물린다.
    struct SortingSource;
    impl GroupSource for SortingSource {
        fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP {
                let mut v = live.to_vec();
                v.sort();
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_a_source_that_sorts_all() {
        assert_group_source_contract(
            &SortingSource,
            &GroupSourceFixture {
                unknown: "@nope".to_string(),
                handles_all: true,
            },
        );
    }

    /// 빈 스냅샷을 에러로 접는 소스 — axis2 가 물어야 한다(옛 `GroupError::Empty` 회귀 그물).
    struct EmptyRejectingSource;
    impl GroupSource for EmptyRejectingSource {
        fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP {
                if live.is_empty() {
                    return Err(GroupError::NotFound { name: norm });
                }
                return Ok(live.to_vec());
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis2-all-empty-ok")]
    fn contract_suite_catches_a_source_that_rejects_an_empty_snapshot() {
        assert_group_source_contract(
            &EmptyRejectingSource,
            &GroupSourceFixture {
                unknown: "@nope".to_string(),
                handles_all: true,
            },
        );
    }
}
