//! groups — `@`주소 해석 seam(spec §4 · ADR-0104 결정 1 · **ADR-0111 결정 4 · ADR-0112 결정 1 ·
//! ADR-0121 결정 1**).
//!
//! ★역할★: "`@`주소 → 멤버 이름 목록" 을 펼친다. **v1 소스는 내장 하나뿐이고 그 안의 어휘가 둘**이다
//!   (ADR-0121): **`@here`** = 지금 살아 있는 전원 · **`@all`** = 명부 전원(산 것 **+ 잠든 것**). 멤버십은
//!   **이름 기반**(id 아님 — WYSIWYA, ADR-0101 · 재스폰 생존).
//!
//! ★두 어휘가 **다른 명단 풀**을 읽는 것이 결정 그 자체다(load-bearing — ADR-0121 §영향)★: 같은 풀을
//!   보게 만들면 "all" 이 다시 거짓 이름이 되고(잠든 상대를 놓친 채 발신 LLM 은 "전원에게 알렸다" 고 믿는다)
//!   `@here` 는 존재 이유를 잃는다. 그래서 아래 계약 스위트가 두 어휘의 소스 분리를 축으로 단언한다.
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
//!   `BuiltinGroups`(= `@all`·`@here`) 하나다. **over-build 금지** — 폴더/계층 구현은 지금 만들지 않는다.
//!
//! ★순수·기계적 해석(load-bearing 경계, ADR-0104)★: `resolve` 는 이름 목록으로 **펼치기만** 한다 —
//!   살아있음 판정·발신자 제외·중복 제거·로스터 대조는 전부 상위(`MessagingService`)의 몫이다. 특히
//!   **"펼침 결과 0명" 은 여기서 에러가 아니다**(빈 목록을 그대로 돌려준다): `GROUP_EMPTY` 는 펼침 + 명시
//!   지목을 합친 **최종 수신자 집합**이 비었을 때만 나는 판정이라(ADR-0114 결정 3) 이 층에서 알 수 없다.

/// 내장 어휘 `@all` — **명부 전원**(산 것 + 잠든 것) **− 발신자**(spec §4 · ADR-0121 결정 1).
pub const ALL_GROUP: &str = "@all";

/// 내장 어휘 `@here` — **지금 살아 있는 전원** **− 발신자**(spec §4 · ADR-0121 결정 1).
///
/// ★어휘 선택 근거★: Slack 이 같은 구분을 쓴다(`@channel` = 전원 / `@here` = 지금 활성) — 에이전트를
///   구동하는 LLM 이 그 관례를 이미 알고 있을 가능성이 높다(ADR-0121 §근거).
pub const HERE_GROUP: &str = "@here";

/// 해석기가 읽는 **명단 풀 두 개**(ADR-0121 결정 1) — 호출자가 락 밖 스냅샷에서 만들어 주입한다.
///
/// ★왜 한 슬라이스가 아니라 풀 둘인가(load-bearing — 소스 지식이 파이프라인에 새지 않게)★: "어느 어휘가
///   어느 풀을 읽나" 는 **소스의 지식**이다. 상위가 어휘별로 다른 슬라이스를 골라 넘기는 형태였다면
///   파이프라인이 `@all`/`@here` 를 알아야 하고, 그러면 미래 소스(폴더 `@폴더명`)가 자기 풀을 고를 수
///   없어진다(ADR-0104 "소스 지식이 파이프라인에 새면 위반"). 그래서 풀을 **둘 다** 넘기고 선택은 소스가 한다.
/// ★발신자 제외는 호출자가 이미 적용한 상태★로 온다(spec §4 — 펼침에서 발신자를 뺀다. 정본이 ADR-0111 이
///   아니라 spec 이라는 점이 load-bearing: 0111 결정 4 에 그 문구가 없어 거기만 보면 빠뜨린다).
#[derive(Debug, Clone, Copy)]
pub struct MemberPools<'a> {
    /// 지금 살아 있는 이름들 — `@here` 의 명단이자 `@all` 의 앞부분.
    pub live: &'a [String],
    /// 잠든 이름들(프로필 실재·그 세션은 살아 있지 않음) — **`@all` 만** 이걸 함께 읽는다.
    ///   ★중복을 접지 않은 채 온다★: 이 층의 계약이 **호출자가 준 풀을 verbatim 돌려준다**(정렬·dedup·필터
    ///   금지)라서다. 동명 판정은 상위가 `AddressingSources.dormant_names` 를 직접 세어 하므로
    ///   (service.rs `push_recipient`) 여기서 접든 안 접든 결과 행은 같지만, 판정 지식을 이 층에 흘리지
    ///   않는 것 자체가 seam 계약이다.
    pub dormant: &'a [String],
}

/// `@`주소 해석 에러 — 상위가 wire 에러 코드로 매핑한다(spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// `@` 네임스페이스 규약 위반 — 정규화 실패.
    InvalidName { name: String },
    /// 내장 어휘가 아닌 `@이름` — 존재하지 않는 주소 → `GROUP_NOT_FOUND`(발송 단위 전체 반려, ADR-0114 결정 3).
    NotFound { name: String },
}

/// ★그룹 주소 정규화 — **seam 레벨** 자유 함수(C4 리뷰 fix F · ADR-0104 결정 1)★
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
pub fn normalize_group_name(name: &str) -> Result<String, GroupError> {
    let trimmed = name.trim();
    let Some(body) = trimmed.strip_prefix('@') else {
        return Err(GroupError::InvalidName {
            name: name.to_string(),
        });
    };
    if body.is_empty() || body.chars().all(char::is_whitespace) || body.contains('@') {
        return Err(GroupError::InvalidName {
            name: name.to_string(),
        });
    }
    Ok(format!("@{body}"))
}

/// 그룹 해석 소스(seam — ADR-0104 결정 1). "`@`주소 → 멤버 이름 목록" 해석기의 확장점.
pub trait GroupSource {
    /// 이 소스가 그 주소를 모르면 `NotFound`.
    fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError>;
}

/// v1 유일 소스 — 내장 두 어휘(`@all`·`@here`)만 안다(무저장 즉석 계산, ADR-0111 결정 4 · ADR-0121 결정 1).
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinGroups;

impl GroupSource for BuiltinGroups {
    fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError> {
        let norm = normalize_group_name(group)?;
        if norm == HERE_GROUP {
            return Ok(pools.live.to_vec());
        }
        if norm == ALL_GROUP {
            let mut members = Vec::with_capacity(pools.live.len() + pools.dormant.len());
            members.extend_from_slice(pools.live);
            members.extend_from_slice(pools.dormant);
            return Ok(members);
        }
        Err(GroupError::NotFound { name: norm })
    }
}

/// ★`GroupSource` 계약 스위트 — 소스 교체 내구성 그물(사용자 지시 2026-07-26)★
///
/// ★왜 이게 있나(load-bearing — 이 모듈의 존재 이유)★: `GroupSource` 는 **미래에 갈아끼울 것을 전제로**
///   깐 seam 이다(ADR-0104 결정 1 — 폴더가 데몬 소유로 생기면 `@폴더명` 소스가 추가된다). 그때 새 구현이
///   조용히 다른 의미를 갖게 되는 게 최악이다: `@all` 이 정렬돼 나오거나, **`@here` 가 잠든 이름까지 읽거나**,
///   빈 스냅샷이 에러로 접히거나, `@` 규약이 관대해지거나 — 전부 **컴파일은 되고 기존 테스트는 초록**인
///   채로 방송 의미만 뒤집힌다. 그래서 해석 의미론을 구현이 아니라 **트레잇에 대해** 단언하는 재사용
///   스위트를 둔다.
///
/// ★이 스위트가 빨개지면★: 고칠 대상은 **스위트가 아니라 결정**이다. 해석 의미론은 spec §4 · ADR-0111
///   결정 4 · ADR-0114 결정 3 · ADR-0121 결정 1 이 정한 사용자 결정 사항이라, 여기 단언을 느슨하게 만드는
///   수정은 곧 계약 변경이다 — 사용자 재가 없이 완화하지 말 것.
#[cfg(any(test, feature = "test-harness"))]
pub mod contract {
    use super::{GroupError, GroupSource, MemberPools, ALL_GROUP, HERE_GROUP};

    #[derive(Debug, Clone)]
    pub struct GroupSourceFixture {
        /// 이 소스가 **모르는** 주소 이름(`NotFound` 축). 이미 정규화된 형태(`@nope`)여야 한다.
        pub unknown: String,
        /// 이 소스가 내장 어휘(`@all`·`@here`)를 **소유**하는가. v1 `BuiltinGroups` = true.
        ///   미래 폴더 소스 = 보통 false.
        pub handles_builtins: bool,
    }

    fn owned(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// ★`GroupSource` 해석 의미론 계약 — 어떤 구현이든 이걸 통과해야 한다(spec §4 · ADR-0111/0114/0121)★.
    ///
    /// ★단언 문구 앞의 `axisN-…:` 토큰(load-bearing)★: 모든 실패 메시지는 **축마다 유일한** 토큰으로
    ///   시작한다. 자체검증 테스트가 `#[should_panic(expected = …)]` 로 "그물이 무는지" 확인하는데, 부분
    ///   문자열이 여러 축에 겹치면 엉뚱한 축이 먼저 터져도 초록이 된다. **토큰을 바꾸면 핀도 함께 고칠 것.**
    pub fn assert_group_source_contract(src: &dyn GroupSource, fx: &GroupSourceFixture) {
        // ── 1·1b·1c·2. 내장 어휘 축 ────────────────────────────────────────────────────────
        // 정렬하면 상위의 행 순서 규칙(명시 토큰 → 펼침 사전순)이 두 층에 흩어지고, dedup 하면 산 층
        //   동명 다수(RECIPIENT_AMBIGUOUS 사유)가 소스 단계에서 지워져 상위가 그 사실을 못 본다.
        let live = owned(&["zeta", "alpha", "zeta", "mid"]);
        let dormant = owned(&["sleepy", "twin", "twin"]);
        let pools = MemberPools {
            live: &live,
            dormant: &dormant,
        };
        let empty = MemberPools {
            live: &[],
            dormant: &[],
        };
        if fx.handles_builtins {
            let mut all_expected = live.clone();
            all_expected.extend(dormant.clone());
            assert_eq!(
                src.resolve(ALL_GROUP, pools),
                Ok(all_expected),
                "axis1-all-verbatim: @all 은 live + dormant 를 그대로 이어 붙여야 한다(잠든 이름 포함·정렬·dedup·필터 금지 — spec §4 · ADR-0121 결정 1)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, pools),
                Ok(live.clone()),
                "axis1b-here-live-only: @here 는 live 스냅샷만 그대로여야 한다(잠든 이름이 끼면 '지금 여기' 가 아니다 — ADR-0121 결정 1)"
            );
            assert_ne!(
                src.resolve(ALL_GROUP, pools),
                src.resolve(HERE_GROUP, pools),
                "axis1c-vocab-split: 잠든 이름이 있으면 @all 과 @here 는 **다른 명단**이어야 한다 — 같은 소스를 보면 'all' 이 다시 거짓 이름이 된다(ADR-0121 §영향)"
            );
            assert_eq!(
                src.resolve(ALL_GROUP, empty),
                Ok(Vec::new()),
                "axis2-all-empty-ok: 명부가 빈 @all 은 빈 목록(에러 아님) — GROUP_EMPTY 는 명시 지목까지 합친 최종 집합으로 상위가 판정한다(ADR-0114 결정 3)"
            );
            assert_eq!(
                src.resolve(HERE_GROUP, empty),
                Ok(Vec::new()),
                "axis2c-here-empty-ok: 산 수신자 0명인 @here 도 빈 목록(에러 아님 — 같은 근거)"
            );
        } else {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert!(
                    matches!(
                        src.resolve(builtin, pools),
                        Err(GroupError::NotFound { .. })
                    ),
                    "axis2b-builtin-not-owned: 내장 어휘({builtin})를 소유하지 않는 소스는 그걸 제 것인 양 해석하면 안 된다(소스 순회에서 방송 의미 가로채기 금지)"
                );
            }
        }

        // ── 3. NotFound 축 ────────────────────────────────────────────────────────────────
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            Err(GroupError::NotFound {
                name: fx.unknown.clone()
            }),
            "axis3-unknown-notfound: 모르는 주소는 NotFound 이고 name 은 정규화된 이름을 실어야 한다(상위가 hint 에 되쓴다)"
        );

        // ── 4. `@` 네임스페이스 규약 ───────────────────────────────────────────────────────
        for bad in ["", "@", "  @  ", "@@x", "@@", "@a@b", "plain-name"] {
            assert!(
                matches!(src.resolve(bad, pools), Err(GroupError::InvalidName { .. })),
                "axis4-namespace-strict: '{bad}' 은 @ 네임스페이스 규약 위반이라 InvalidName 이어야 한다(조용한 보정 금지)"
            );
        }

        // ── 5. 결정성 — 같은 입력 두 번, 같은 답 ────────────────────────────────────────────
        // 이게 깨지면 "발송 순간 스냅샷 한 장" 계약이 의미를 잃는다(같은 순간의 두 질문이 다른 세계를 본다).
        assert_eq!(
            src.resolve(&fx.unknown, pools),
            src.resolve(&fx.unknown, pools),
            "axis5-deterministic: 해석은 결정적이어야 한다(순수 — 시계·내부 가변 상태 금지)"
        );
        if fx.handles_builtins {
            for builtin in [ALL_GROUP, HERE_GROUP] {
                assert_eq!(
                    src.resolve(builtin, pools),
                    src.resolve(builtin, pools),
                    "axis5-deterministic-builtin: 내장 어휘({builtin}) 해석도 결정적이어야 한다"
                );
            }
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

    fn pools<'a>(live: &'a [String], dormant: &'a [String]) -> MemberPools<'a> {
        MemberPools { live, dormant }
    }

    #[test]
    fn here_resolves_to_the_live_snapshot_verbatim() {
        let live = names(&["zeta", "alpha", "zeta"]);
        let dormant = names(&["sleepy"]);
        assert_eq!(
            BuiltinGroups.resolve("@here", pools(&live, &dormant)),
            Ok(live),
            "@here 는 산 명단만 본다(잠든 이름 불포함 — ADR-0121 결정 1)"
        );
    }

    #[test]
    fn all_appends_the_dormant_names_to_the_live_snapshot() {
        let live = names(&["zeta", "alpha"]);
        let dormant = names(&["sleepy", "twin", "twin"]);
        assert_eq!(
            BuiltinGroups.resolve("@all", pools(&live, &dormant)),
            Ok(names(&["zeta", "alpha", "sleepy", "twin", "twin"])),
            "풀은 verbatim 이다 — 이 층은 가공하지 않는다(동명 판정은 상위가 dormant_names 로 직접 한다)"
        );
    }

    #[test]
    fn builtin_vocabularies_with_no_names_are_empty_lists_not_errors() {
        assert_eq!(
            BuiltinGroups.resolve("@all", pools(&[], &[])),
            Ok(Vec::new())
        );
        assert_eq!(
            BuiltinGroups.resolve("@here", pools(&[], &[])),
            Ok(Vec::new())
        );
    }

    #[test]
    fn unknown_at_address_is_not_found() {
        let live = names(&["alice"]);
        assert_eq!(
            BuiltinGroups.resolve("@coders", pools(&live, &[])),
            Err(GroupError::NotFound {
                name: "@coders".to_string()
            }),
            "사용자 정의 그룹은 제거됐다(ADR-0111 결정 4) — 내장 어휘 외의 @주소는 전부 주소 공간 오류"
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
        assert_group_source_contract(&BuiltinGroups, &builtin_fixture());
    }

    fn builtin_fixture() -> GroupSourceFixture {
        GroupSourceFixture {
            unknown: "@nope".to_string(),
            handles_builtins: true,
        }
    }

    struct SortingSource;
    impl GroupSource for SortingSource {
        fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                let mut v = pools.live.to_vec();
                if norm == ALL_GROUP {
                    v.extend_from_slice(pools.dormant);
                }
                v.sort();
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_a_source_that_sorts_all() {
        assert_group_source_contract(&SortingSource, &builtin_fixture());
    }

    /// ★ADR-0121 그물★ — 옛 `@all`(로스터만 읽는 것) 회귀.
    struct LiveOnlyAllSource;
    impl GroupSource for LiveOnlyAllSource {
        fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                return Ok(pools.live.to_vec());
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_an_all_that_ignores_dormant_names() {
        assert_group_source_contract(&LiveOnlyAllSource, &builtin_fixture());
    }

    /// ★ADR-0121 그물(반대 방향)★ — `@here` 가 dormant 를 읽는 회귀.
    struct HereReadsDormantSource;
    impl GroupSource for HereReadsDormantSource {
        fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                let mut v = pools.live.to_vec();
                v.extend_from_slice(pools.dormant);
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis1b-here-live-only")]
    fn contract_suite_catches_a_here_that_reads_dormant_names() {
        assert_group_source_contract(&HereReadsDormantSource, &builtin_fixture());
    }

    /// 옛 `GroupError::Empty` 회귀 그물 — axis2 가 물어야 한다.
    struct EmptyRejectingSource;
    impl GroupSource for EmptyRejectingSource {
        fn resolve(&self, group: &str, pools: MemberPools<'_>) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP || norm == HERE_GROUP {
                if pools.live.is_empty() && pools.dormant.is_empty() {
                    return Err(GroupError::NotFound { name: norm });
                }
                let mut v = pools.live.to_vec();
                if norm == ALL_GROUP {
                    v.extend_from_slice(pools.dormant);
                }
                return Ok(v);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis2-all-empty-ok")]
    fn contract_suite_catches_a_source_that_rejects_an_empty_snapshot() {
        assert_group_source_contract(&EmptyRejectingSource, &builtin_fixture());
    }
}
