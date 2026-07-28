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
    /// ★멤버 이름이 `@` 로 시작 = 중첩 그룹 시도(round-2 리뷰 F5)★ — v1 미지원이라 거절한다.
    ///
    /// ★왜 거절인가(조용한 수용이 더 나쁘다)★: 멤버십은 **에이전트 이름** 기반인데(spec §4), `@` 로
    ///   시작하는 이름을 가진 에이전트는 존재할 수 없다(`@` 는 그룹 네임스페이스 예약). 그대로 등록하면
    ///   그 멤버는 어떤 방송에서도 매치되지 않아 **영원히 `skipped`** 되고, 발신자는 중첩 그룹이 동작한다고
    ///   믿는다(응답에 멤버로 보이니까). 등록 시점에 막아 오해를 원천 차단한다.
    InvalidMemberName { name: String },
}

/// ★그룹 주소 정규화 — **seam 레벨** 자유 함수(C4 리뷰 fix F · ADR-0104 결정 1)★. `@` 네임스페이스 규약을
/// 검증하고 정규화된 이름을 돌려준다. 실패는 `InvalidName`.
///
/// ★왜 `Groups`(v1 구현)의 연관 함수가 아니라 자유 함수인가(load-bearing — seam 경계)★: 발송 파이프라인
///   (`MessagingService::handle_group_send`)은 봉투 `to` 라벨을 만들고 소스에 물어보기 **전에** 이름을
///   정규화한다. 그 정규화가 `Groups::normalize`(= v1 런타임 명단 구현의 메서드)면 파이프라인이 **특정 소스
///   타입에 묶인다** — 폴더 소스처럼 이름 문법이 다른 구현이 들어와도 파이프라인은 영영 v1 레지스트리의
///   규약으로만 이름을 걸러, 새 소스가 도달 불가해진다(ADR-0104 "소스 지식이 파이프라인에 새면 위반").
///   그래서 규약을 소스 구현 밖 seam 레벨에 두고, 파이프라인·모든 `GroupSource` 구현이 **이 한 함수**를
///   공유한다(`Groups` 는 이걸 위임해 쓴다).
/// ★라벨 단일 출처★: 봉투 `to` 속성은 **해석에 실제로 쓰인 이름**과 같아야 한다(수신 LLM 이 보는 방송 대상 =
///   데몬이 편 명단의 출처). 파이프라인이 자기 나름의 trim/보정을 하면 두 문자열이 미묘하게 갈리므로,
///   정규화 지점을 여기 하나로 고정한다.
///
/// ★엄격 규약(load-bearing — @-네임스페이스 계약 · finding 3)★: 그룹 이름은 **반드시 정확히 하나의 선행
///   `@` 로 시작**해야 한다. 선행 `@` 가 없으면(`coders`) `InvalidName` 으로 거부한다 — 예전의 관대 보정
///   (`coders` → `@coders`)은 `@` 네임스페이스 계약을 흐려(사람/그룹 이름 구분이 `@` 하나에 걸려 있음)
///   거부됐다. `@` 뒤 본문(remainder)이 비었거나(`@` 단독) 공백만이면 역시 `InvalidName`.
/// ★본문에 추가 `@` 금지(finding 3)★: 선행 `@` 를 하나 벗긴 뒤 본문에 또 `@` 가 있으면(`@@x`·`@@`·`@a@b`)
///   거부한다. "정확히 하나의 `@`" 계약을 문자 그대로 강제하는 가장 단순한 규칙이다 — 본문 내부 `@`(`@a@b`)를
///   허용하면 어디까지가 네임스페이스 마커고 어디부터가 이름인지 모호해져 사람/그룹 구분이 흐려진다.
/// ★입구 정규화는 별개 층★: 툴 인자를 관대하게 받아 `@` 를 붙여 주는 편의 정규화가 필요하면 그건 **MCP/CLI
///   입구 계층**에서 하고, 이 순수 규약 함수는 계약을 엄격히 강제한다(경계 분리 — 여기서 관대해지면 계약
///   위반을 해석기가 은폐).
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

    /// 이름 검증·정규화 — **seam 함수 `normalize_group_name` 에 위임**한다(C4 리뷰 fix F).
    ///
    /// ★왜 위임인가★: `@` 네임스페이스 규약은 **소스마다 갈리면 안 되는 계약**이라 정본이 이 구현체 밖(모듈
    ///   레벨 자유 함수)에 있다. 여기 남은 건 이 구현의 CRUD 가 같은 규약을 쓰게 하는 얇은 호출부다 —
    ///   규약을 고칠 땐 seam 함수 한 곳만 고친다(구현체마다 복제 금지).
    fn normalize(name: &str) -> Result<String, GroupError> {
        normalize_group_name(name)
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
    /// 무시(집합 의미). `@all` 은 거절. `@` 로 시작하는 **멤버** 이름도 거절(중첩 그룹 미지원 — F5).
    ///
    /// ★구조적 guard(입구 검증과 이중)★: 입구(ingress)가 먼저 걸러 좋은 hint 를 주지만, 여기서도 막아
    ///   **어떤 경로로도** 매치 불가능한 멤버가 명단에 들어가지 못하게 한다(레지스트리 불변식).
    pub fn add_member(&mut self, group: &str, member: &str) -> Result<(), GroupError> {
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        Self::validate_member_name(member)?;
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

    /// ★멤버 이름 규약(round-2 리뷰 F5)★ — `@` 로 시작하면 그룹 네임스페이스라 에이전트 이름일 수 없다.
    /// 규칙 정본을 한 함수에 두고 단건(`add_member`)·배치(`update_members`)가 공유한다(두 경로가 갈리면
    /// 한쪽으로만 유령 멤버가 새어 들어온다).
    fn validate_member_name(member: &str) -> Result<(), GroupError> {
        if member.trim_start().starts_with('@') {
            return Err(GroupError::InvalidMemberName {
                name: member.to_string(),
            });
        }
        Ok(())
    }

    /// ★배치 증감 — **전부 검증한 뒤 전부 적용**(round-3 리뷰 G2 · load-bearing)★. 반환 = 적용 후 명단.
    ///
    /// ★왜 원자적이어야 하나★: 예전엔 상위(`MessagingService::group_update`)가 `add_member` 를 루프로
    ///   돌렸다. 그러면 `["alice", "@all"]` 같은 배치가 **alice 를 넣은 뒤** 두 번째에서 에러를 내
    ///   호출자에게는 실패를 돌려주면서 레지스트리는 이미 바뀐 상태로 남는다(부분 반영). 입구(ingress)가
    ///   먼저 걸러 주는 덕에 운영 경로에선 안 보이지만, 그건 **입구를 반드시 거친다**는 가정에 기댄 안전이라
    ///   내부 호출자 하나가 우회하면 곧바로 깨진다. 원자성을 자료구조 쪽에 두면 그 가정이 필요 없다.
    /// ★단계★: ① 그룹 이름 규약·내장 보호 ② 모든 멤버 이름 규약 ③ "생성되지 않을 그룹에 대한 조작인가"
    ///   (`add` 가 비었는데 그룹이 없으면 `NotFound`) — 여기까지 통과하면 ④ 적용은 **실패할 수 없다**.
    /// ★순서 = add 먼저, remove 나중★: 한 호출에 같은 이름이 양쪽에 있으면 최종 상태는 "빠진 것" 이다
    ///   (제거 의도를 무시하는 쪽이 더 위험 — 다음 방송이 그 멤버에게 나간다).
    /// ★암묵 생성★: `add` 가 있으면 없는 그룹이 그 자리에서 생긴다. `remove` 만으로는 절대 생기지 않는다.
    // round-3 리뷰 G2
    // ADR-0109 (배치 원자성 — 전검증 후 일괄 적용)
    pub fn update_members(
        &mut self,
        group: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>, GroupError> {
        // ── 1~3단계: 검증만(레지스트리 무변경) ──────────────────────────────────────────
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        for m in add.iter().chain(remove.iter()) {
            Self::validate_member_name(m)?;
        }
        let exists = self.registered.contains_key(&norm);
        if !exists && add.is_empty() {
            // 생성 트리거(add)가 없으면 없는 그룹은 그대로 없는 그룹이다 — 조회든 remove 든 `NotFound`.
            //   (빈 add 가 유령 그룹을 만들지 않게 하는 규칙과 같은 자리.)
            return Err(GroupError::NotFound { name: norm });
        }

        // ── 4단계: 적용(여기서부터 실패 없음) ────────────────────────────────────────────
        let members = self.registered.entry(norm).or_default();
        for m in add {
            if !members.iter().any(|x| x == m) {
                members.push(m.clone());
            }
        }
        for m in remove {
            members.retain(|x| x != m);
        }
        Ok(members.clone())
    }

    /// ★관리 조회용 멤버 목록(S18 D — `group { group }`)★. 등록 그룹이면 그 명단(**빈 그룹은 `Ok(vec![])`**),
    /// 없으면 `NotFound`. `@all` 은 여기서 처리하지 않는다(`Builtin` — liveness 를 모르므로 상위가 `resolve`
    /// 에 live 스냅샷을 넘겨 푼다).
    ///
    /// ★왜 `resolve` 로 대신하지 않나(load-bearing — 두 질문이 다르다)★: `resolve` 는 **발송용**이라 멤버 0명을
    ///   `Empty` 로 **거부**한다(빈 그룹에 방송하면 아무에게도 안 가므로 반려가 맞다). 그런데 **관리**에서는
    ///   "방금 만든 빈 그룹" 이 정상 상태다 — 그걸 에러로 답하면 `group @g --add x` 직후의 조회가 실패하고,
    ///   사용자는 그룹이 안 만들어진 줄 안다. 두 질문(보낼 수 있나 / 명단이 뭐냐)을 한 함수로 뭉개면 한쪽이
    ///   반드시 거짓말을 하므로 조회 전용 출구를 따로 둔다.
    pub fn members_of(&self, group: &str) -> Result<Vec<String>, GroupError> {
        let norm = Self::normalize(group)?;
        if Self::is_builtin(&norm) {
            return Err(GroupError::Builtin);
        }
        self.registered
            .get(&norm)
            .cloned()
            .ok_or(GroupError::NotFound { name: norm })
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

/// ★`GroupSource` 계약 스위트 — 소스 교체 내구성 그물(사용자 지시 2026-07-26)★
///
/// ★왜 이게 있나(load-bearing — 이 모듈의 존재 이유)★: `GroupSource` 는 **미래에 갈아끼울 것을 전제로**
///   깐 seam 이다(ADR-0104 결정 1 — 폴더가 데몬 소유로 생기면 `@폴더명` 소스가 추가된다). 그때 새 구현이
///   조용히 다른 의미를 갖게 되는 게 최악이다: `@all` 이 정렬돼 나오거나, 빈 그룹이 `NotFound` 로 접히거나,
///   등록 명단이 live 스냅샷과 교차되거나 — 전부 **컴파일은 되고 기존 테스트는 초록**인 채로 방송 의미만
///   뒤집힌다. 그래서 해석 의미론을 구현이 아니라 **트레잇에 대해** 단언하는 재사용 스위트를 둔다.
///
/// ★쓰는 법(미래 소스 작성자에게)★: 새 `GroupSource` 를 만들면 그 소스의 테스트에서 이 스위트를 **그대로**
///   돌린다 — 픽스처만 자기 소스에 맞게 채우면 된다:
///   ```ignore
///   let src = MyFolderSource::new(/* … */);
///   assert_group_source_contract(&src, &GroupSourceFixture {
///       known: "@proj".into(), members: vec!["carol".into()],
///       // Empty 축은 건너뛸 수 있지만 **이유를 적어야** 한다(조용한 생략 금지 — EmptyAxis 주석).
///       empty: EmptyAxis::Representable("@empty-folder".into()),
///       unknown: "@nope".into(), handles_all: false,
///   });
///   ```
/// ★이 스위트가 빨개지면★: 고칠 대상은 **스위트가 아니라 결정**이다. 해석 의미론은 spec §4 · ADR-0103
///   결정 4 · ADR-0104 결정 1 이 정한 사용자 결정 사항이라, 여기 단언을 느슨하게 만드는 수정은 곧 계약
///   변경이다 — 사용자 재가 없이 완화하지 말 것(그물을 잘라 통과시키는 건 그물의 목적을 없앤다).
/// ★스위트가 실제로 무는지 자체 검증★: 아래 테스트 모듈에 **일부러 어긋난 소스**(정렬하는 소스 · 빈 그룹을
///   NotFound 로 접는 소스)를 두고 `#[should_panic]` 으로 이 스위트가 잡는지 단언한다 — 그물이 공허하지
///   (vacuous) 않다는 증거다.
// ADR-0104
#[cfg(any(test, feature = "test-harness"))]
pub mod contract {
    use super::{GroupError, GroupSource, ALL_GROUP};

    /// ★`Empty` 축 표현(C4 리뷰 fix E)★ — "이 소스에서 '아는데 멤버 0명' 상태를 만들 수 있나" 를 **명시적
    /// 선택**으로 강제한다.
    ///
    /// ★왜 `Option<String>` 을 버렸나(load-bearing — 그물 구멍)★: 옛 픽스처는 `empty: Option<String>` 이라
    ///   `None` 이면 스위트가 그 축을 **조용히 건너뛰었다**. 미래 소스 작성자가 `Empty`/`NotFound` 구분을
    ///   구현하지 않았거나 픽스처를 채우기 귀찮아 `None` 을 넣으면, 계약의 핵심 축 하나가 아무 흔적 없이
    ///   빠진 채 스위트는 초록이 된다 — "돌렸으니 계약을 지킨다" 는 잘못된 확신만 남는다. 이제 건너뛰려면
    ///   **왜 불가능한지 이유를 문자열로 적어야** 하고, 스위트가 그 사실을 stderr 로 남긴다(생략은 기록된다).
    #[derive(Debug, Clone)]
    pub enum EmptyAxis {
        /// 이 소스에서 "알지만 멤버 0명" 상태를 만들 수 있다 — 그 그룹의 (정규화된) 이름.
        Representable(String),
        /// 소스 구조상 그 상태가 **표현 불가**하다. `why` = 왜 불가한지(예: "폴더 소스는 빈 폴더를 그룹으로
        ///   노출하지 않는다"). 스위트가 이 문자열을 로그로 남겨 축 생략이 기록에 남게 한다.
        Unrepresentable { why: &'static str },
    }

    /// 계약 스위트가 소스를 두드리는 데 필요한 **소스별 픽스처**. 이름은 전부 **이미 정규화된 형태**
    /// (선행 `@` 하나 + 본문)여야 한다 — 스위트가 에러의 `name` 필드를 이 값과 그대로 비교하기 때문이다.
    #[derive(Debug, Clone)]
    pub struct GroupSourceFixture {
        /// 이 소스가 아는 그룹(멤버 ≥1). 스위트는 이 이름의 해석 결과가 `members` 와 **verbatim** 같은지 본다.
        pub known: String,
        /// `known` 의 정답 멤버 목록 — **순서까지** 정본이다(방송 순서는 결정적이어야 한다).
        pub members: Vec<String>,
        /// `Empty` 축 — 표현 가능하면 그 그룹 이름, 불가하면 **이유를 적어** 명시적으로 건너뛴다(위 enum).
        pub empty: EmptyAxis,
        /// 이 소스가 **모르는** 그룹 이름(`NotFound` 축).
        pub unknown: String,
        /// 이 소스가 내장 `@all` 을 **소유**하는가. v1 런타임 명단 소스 = true. 폴더 등 추가 소스는 보통
        ///   false 이고, 그 경우 스위트는 "`@all` 을 제 것인 양 해석하지 않는다"(= `NotFound`)를 요구한다 —
        ///   소스 순회 구조에서 방송 의미를 가로채면 안 되기 때문이다.
        pub handles_all: bool,
    }

    fn owned(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// ★`GroupSource` 해석 의미론 계약 — 어떤 구현이든 이걸 통과해야 한다(spec §4 · ADR-0103/0104)★.
    ///
    /// 단언하는 축(각 축이 어긴 순간 방송 의미가 어떻게 뒤집히는지는 본문 주석에 붙였다):
    ///   1. `@all` = 호출자가 넘긴 live 스냅샷 **verbatim**(정렬·dedup·필터 금지)
    ///   2. `@all` + 빈 스냅샷 = `Empty`(≠ `NotFound`)
    ///   3. 등록 그룹 = 명단 **verbatim**, 그리고 **live 스냅샷과 교차하지 않음**(순수 해석 — liveness 는 상위)
    ///   4. `NotFound` vs `Empty` 는 **구별**된다(둘이 서로 다른 wire 코드로 나가므로)
    ///   5. `@` 네임스페이스 규약 위반은 `InvalidName`(조용한 보정 금지)
    ///   6. 결정적 — 같은 입력을 두 번 물으면 같은 답(내부 가변 상태·시계 의존 금지)
    ///
    /// ★단언 문구 앞의 `axisN-…:` 토큰(C4 리뷰 fix E · load-bearing)★: 모든 실패 메시지는 **축마다 유일한**
    ///   토큰으로 시작한다. 아래 자체검증 테스트가 `#[should_panic(expected = …)]` 로 "그물이 무는지" 를
    ///   확인하는데, 옛 부분문자열(`"verbatim"`·`"Empty"`)은 **여러 축의 문구에 동시에 등장**해서 엉뚱한 축이
    ///   먼저 터져도 테스트가 초록이었다(= 자체검증이 자기 대상을 확인하지 못함). 토큰을 유일하게 두고 거기에
    ///   핀을 박으면 "의도한 그 축이 물었다" 가 보장된다. **토큰을 바꾸면 `should_panic` 핀도 함께 고칠 것.**
    pub fn assert_group_source_contract(src: &dyn GroupSource, fx: &GroupSourceFixture) {
        // ── 1·2. 내장 `@all` 축 ────────────────────────────────────────────────────────────
        // live 스냅샷은 "발송 순간 살아있는 이름들" 이고, 소스는 그걸 **그대로** 돌려줘야 한다. 정렬하면
        //   방송 순서가 로스터 순서와 갈리고, dedup 하면 동명 다수(skip 사유)가 소스 단계에서 지워져
        //   상위의 멤버별 회계가 그 사실을 볼 수 없게 된다.
        let live = owned(&["zeta", "alpha", "zeta", "mid"]);
        if fx.handles_all {
            assert_eq!(
                src.resolve(ALL_GROUP, &live),
                Ok(live.clone()),
                "axis1-all-verbatim: @all 은 live 스냅샷 그대로여야 한다(정렬·dedup·필터 금지 — spec §4)"
            );
            assert_eq!(
                src.resolve(ALL_GROUP, &[]),
                Err(GroupError::Empty {
                    name: ALL_GROUP.to_string()
                }),
                "axis2-all-empty: 산 수신자 0명인 @all 은 Empty(GROUP_EMPTY) — NotFound 로 접으면 발신자가 '그룹이 없다'고 오독한다"
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

        // ── 3. 등록(알려진) 그룹 축 — 명단 verbatim + live 무관 ─────────────────────────────
        let resolved = src
            .resolve(&fx.known, &[])
            .expect("axis3-known-resolves: 소스가 안다고 선언한 그룹은 해석돼야 한다");
        assert_eq!(
            resolved, fx.members,
            "axis3-known-order: 알려진 그룹은 명단 그대로(순서 포함) — 방송 순서는 결정적이어야 한다"
        );
        // ★live 교차 금지(ADR-0104 순수성)★: 죽은 멤버 skip 은 상위(MessagingService)의 회계다. 소스가
        //   여기서 live 와 교차해 걸러 버리면 상위가 "이 멤버는 왜 결과에 없나" 를 영영 알 수 없고,
        //   응답 `results[]` 에서 skipped 줄이 통째로 사라진다(조용한 유실).
        assert_eq!(
            src.resolve(&fx.known, &owned(&["아무-상관-없는-이름"])),
            Ok(fx.members.clone()),
            "axis3-known-live-independent: 등록 그룹 해석은 live 스냅샷에 영향받지 않아야 한다(liveness 판정은 상위 소관 — ADR-0104)"
        );

        // ── 4. NotFound vs Empty 구별 ──────────────────────────────────────────────────────
        assert_eq!(
            src.resolve(&fx.unknown, &live),
            Err(GroupError::NotFound {
                name: fx.unknown.clone()
            }),
            "axis4-unknown-notfound: 모르는 그룹은 NotFound 이고 name 은 정규화된 이름을 실어야 한다(상위가 hint 에 되쓴다)"
        );
        match &fx.empty {
            EmptyAxis::Representable(empty) => assert_eq!(
                src.resolve(empty, &live),
                Err(GroupError::Empty {
                    name: empty.clone()
                }),
                "axis4-empty-distinct: 아는데 멤버 0명이면 Empty — NotFound 와 뭉개면 GROUP_EMPTY/GROUP_NOT_FOUND 가 한 코드로 붕괴한다"
            ),
            // ★생략은 조용하지 않다(fix E)★ — 축을 건너뛰는 유일한 길은 "왜 불가한지" 를 남기는 것이고,
            //   그 사실은 테스트 출력에 찍힌다(회수자가 어떤 축이 미검증인지 본다).
            EmptyAxis::Unrepresentable { why } => eprintln!(
                "axis4-empty-skipped: 이 소스는 '아는데 멤버 0명' 상태를 표현할 수 없어 Empty 축을 건너뜀 — 사유: {why}"
            ),
        }

        // ── 5. `@` 네임스페이스 규약 ───────────────────────────────────────────────────────
        // 관대한 보정(`coders` → `@coders`)은 사람 이름과 그룹 이름의 구분이 `@` 하나에 걸려 있는 계약을
        //   흐린다 — 소스가 조용히 보정하면 오타 하나로 엉뚱한 방송이 나간다.
        let unprefixed = fx.known.trim_start_matches('@').to_string();
        for bad in ["", "@", "  @  ", "@@x", "@@", "@a@b", unprefixed.as_str()] {
            assert!(
                matches!(src.resolve(bad, &live), Err(GroupError::InvalidName { .. })),
                "axis5-namespace-strict: '{bad}' 은 @ 네임스페이스 규약 위반이라 InvalidName 이어야 한다(조용한 보정 금지)"
            );
        }

        // ── 6. 결정성 — 같은 입력 두 번, 같은 답 ────────────────────────────────────────────
        // 소스는 순수해야 한다(시계·내부 가변 상태 금지 — 모듈 헤더). 이게 깨지면 "발송 순간 스냅샷" 이
        //   의미를 잃는다(같은 순간의 두 질문이 다른 세계를 본다).
        assert_eq!(
            src.resolve(&fx.known, &live),
            src.resolve(&fx.known, &live),
            "axis6-deterministic-known: 해석은 결정적이어야 한다(순수 — 시계·내부 가변 상태 금지)"
        );
        if fx.handles_all {
            assert_eq!(
                src.resolve(ALL_GROUP, &live),
                src.resolve(ALL_GROUP, &live),
                "axis6-deterministic-all: @all 해석도 결정적이어야 한다"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contract::{assert_group_source_contract, EmptyAxis, GroupSourceFixture};
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
    fn add_member_rejects_group_like_names_so_nesting_cannot_be_registered() {
        // ★round-2 리뷰 F5 — 구조적 guard★: 입구가 먼저 거르지만, 레지스트리도 스스로 막아 **어떤 경로로도**
        //   매치 불가능한 멤버(= `@` 로 시작하는 이름)가 명단에 들어가지 못하게 한다.
        let mut g = Groups::new();
        assert_eq!(
            g.add_member("@t", "@coders"),
            Err(GroupError::InvalidMemberName {
                name: "@coders".to_string()
            })
        );
        assert_eq!(
            g.add_member("@t", " @all"),
            Err(GroupError::InvalidMemberName {
                name: " @all".to_string()
            }),
            "앞 공백으로 위장해도 막는다"
        );
        // 반려는 그룹을 만들지도 않는다(암묵 생성보다 이름 검증이 먼저).
        assert!(matches!(
            g.members_of("@t"),
            Err(GroupError::NotFound { .. })
        ));
        // 대조군: 평범한 이름·이름 안의 `@` 는 정상 수용(과잉 차단 아님).
        assert!(g.add_member("@t", "alice").is_ok());
        assert!(g.add_member("@t", "e@mail").is_ok());
    }

    #[test]
    fn update_members_is_all_or_nothing_when_a_name_in_the_batch_is_invalid() {
        // ★round-3 리뷰 G2★: 입구 검증을 우회하는 내부 호출자가 잘못된 배치를 넣어도 **부분 반영이 없어야**
        //   한다. 옛 구현(상위의 add_member 루프)은 alice 를 넣은 뒤 두 번째에서 에러를 내, 호출자에겐
        //   실패인데 레지스트리는 바뀐 상태로 남겼다.
        let mut g = Groups::new();
        let err = g
            .update_members("@t", &names(&["alice", " @all"]), &[])
            .expect_err("배치 안의 잘못된 이름은 반려");
        assert_eq!(
            err,
            GroupError::InvalidMemberName {
                name: " @all".to_string()
            }
        );
        assert!(
            matches!(g.members_of("@t"), Err(GroupError::NotFound { .. })),
            "실패한 배치는 그룹조차 만들지 않는다(부분 변경 0)"
        );
        assert_eq!(g.list(), vec![ALL_GROUP], "레지스트리 무변경");

        // 기존 그룹에 대한 실패도 명단을 건드리지 않는다.
        g.update_members("@t", &names(&["alice"]), &[]).unwrap();
        let err = g
            .update_members("@t", &names(&["bob", "@nested"]), &names(&["alice"]))
            .expect_err("반려");
        assert!(matches!(err, GroupError::InvalidMemberName { .. }));
        assert_eq!(
            g.members_of("@t"),
            Ok(names(&["alice"])),
            "bob 이 들어가지도, alice 가 빠지지도 않아야: 부분 반영 0"
        );
    }

    #[test]
    fn update_members_applies_adds_then_removes_and_creates_implicitly() {
        let mut g = Groups::new();
        // 암묵 생성 + 순서(add 먼저, remove 나중 — 한 배치에 같은 이름이면 remove 가 이긴다).
        assert_eq!(
            g.update_members("@t", &names(&["a", "b", "c"]), &names(&["b"])),
            Ok(names(&["a", "c"]))
        );
        // remove 만으로는 없는 그룹이 생기지 않는다.
        assert!(matches!(
            g.update_members("@ghost", &[], &names(&["a"])),
            Err(GroupError::NotFound { .. })
        ));
        // 빈 배치 = 순수 조회(부작용 0), 없는 그룹이면 NotFound.
        assert_eq!(g.update_members("@t", &[], &[]), Ok(names(&["a", "c"])));
        assert!(matches!(
            g.update_members("@ghost", &[], &[]),
            Err(GroupError::NotFound { .. })
        ));
        // 내장 그룹은 배치 경로에서도 보호된다.
        assert_eq!(
            g.update_members("@all", &names(&["a"]), &[]),
            Err(GroupError::Builtin)
        );
    }

    #[test]
    fn members_of_returns_empty_list_for_a_known_but_empty_group() {
        // ★관리 조회 ≠ 발송 해석★: resolve 는 빈 그룹을 Empty 로 거부하지만(방송 반려), members_of 는
        //   "방금 만든 빈 그룹" 을 정상 상태로 답해야 한다(안 그러면 add 직후 조회가 실패한다).
        let mut g = Groups::new();
        g.create("@hollow").unwrap();
        assert_eq!(g.members_of("@hollow"), Ok(vec![]));
        assert!(matches!(
            g.resolve("@hollow", &[]),
            Err(GroupError::Empty { .. })
        ));
    }

    #[test]
    fn members_of_rejects_unknown_builtin_and_bad_names() {
        let mut g = Groups::new();
        g.add_member("@x", "a").unwrap();
        assert_eq!(g.members_of("@x"), Ok(names(&["a"])));
        assert!(matches!(
            g.members_of("@ghost"),
            Err(GroupError::NotFound { .. })
        ));
        // @all 은 liveness 가 필요해 이 순수 조회로는 못 푼다 — 상위가 resolve(live) 로 간다.
        assert_eq!(g.members_of("@all"), Err(GroupError::Builtin));
        assert!(matches!(
            g.members_of("nope"),
            Err(GroupError::InvalidName { .. })
        ));
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

    // ── GroupSource 계약 스위트(소스 교체 내구성 그물 — 사용자 지시 2026-07-26) ────────────────
    // 아래 3종은 한 세트다: ① v1 소스가 계약을 지킨다 ② **다른 구조의 소스**로도 같은 스위트가 돈다
    // (스위트가 `Groups` 에 유착돼 있지 않다는 증거) ③ 어긋난 소스는 **실제로 잡힌다**(그물이 공허하지 않다).

    /// 미래 소스 모사 — 폴더처럼 "이름 → 자식들" 을 **Vec 선형 탐색**으로 푸는 완전히 다른 구조이고,
    /// 내장 `@all` 은 소유하지 않는다(그건 런타임 명단 소스 몫). ADR-0104 가 예고한 `@폴더명` 소스의 형태다.
    ///
    /// ★이 더블의 목적★: 계약 스위트가 `Groups` 의 구현 디테일(HashMap·정규화 호출 위치)에 유착되지 않았음을
    ///   보인다 — 저장 구조가 달라도 **같은 스위트가 그대로 돈다**는 게 seam 교체 가능성의 실증이다.
    struct FolderLikeSource {
        folders: Vec<(String, Vec<String>)>,
    }
    impl GroupSource for FolderLikeSource {
        fn resolve(&self, group: &str, _live: &[String]) -> Result<Vec<String>, GroupError> {
            // `@` 규약은 소스가 바뀌어도 그대로다 — **seam 함수**를 쓴다(v1 구현체의 메서드가 아니라).
            //   미래 소스가 `Groups::normalize` 를 부르면 그 순간 v1 레지스트리 타입에 묶인다(fix F).
            let norm = normalize_group_name(group)?;
            // 이 소스는 `@all` 을 소유하지 않는다 — 모른다고 답해 상위 순회가 내장 소스로 넘어가게 한다.
            let Some((_, children)) = self.folders.iter().find(|(n, _)| n == &norm) else {
                return Err(GroupError::NotFound { name: norm });
            };
            if children.is_empty() {
                return Err(GroupError::Empty { name: norm });
            }
            Ok(children.clone())
        }
    }

    #[test]
    fn v1_runtime_registry_satisfies_the_group_source_contract() {
        let mut g = Groups::new();
        g.add_member("@coders", "alice").unwrap();
        g.add_member("@coders", "bob").unwrap();
        g.create("@hollow").unwrap();
        assert_group_source_contract(
            &g,
            &GroupSourceFixture {
                known: "@coders".to_string(),
                members: names(&["alice", "bob"]),
                empty: EmptyAxis::Representable("@hollow".to_string()),
                unknown: "@ghost".to_string(),
                handles_all: true,
            },
        );
    }

    #[test]
    fn a_differently_shaped_source_runs_the_same_contract() {
        // 소스 구조가 달라도(Vec 선형 탐색·@all 미소유) 같은 스위트를 통과해야 한다 — 미래 폴더 소스가
        //   이 테스트를 자기 이름으로 복제해 쓰면 된다(스위트 재사용 경로 실증).
        let src = FolderLikeSource {
            folders: vec![
                ("@proj".to_string(), names(&["carol", "dave"])),
                ("@empty-folder".to_string(), vec![]),
            ],
        };
        assert_group_source_contract(
            &src,
            &GroupSourceFixture {
                known: "@proj".to_string(),
                members: names(&["carol", "dave"]),
                empty: EmptyAxis::Representable("@empty-folder".to_string()),
                unknown: "@nope".to_string(),
                handles_all: false,
            },
        );
    }

    /// 일부러 어긋난 소스 ①: `@all` 을 **정렬**해서 돌려준다(verbatim 위반). 정렬은 무해해 보이지만 방송
    /// 순서를 로스터 순서와 갈라놓고, 동명 다수 판정을 상위에서 흐린다.
    struct SortingSource;
    impl GroupSource for SortingSource {
        fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP {
                if live.is_empty() {
                    return Err(GroupError::Empty { name: norm });
                }
                let mut sorted = live.to_vec();
                sorted.sort(); // ← 계약 위반(verbatim 아님).
                return Ok(sorted);
            }
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    // ★핀은 **축 유일 토큰**에 박는다(fix E)★: 옛 핀(`"verbatim"`)은 여러 축 문구에 등장해, 엉뚱한 축이
    //   먼저 터져도 이 테스트가 초록이었다(자체검증이 자기 대상을 확인 못 함). 토큰이 바뀌면 여기도 바꾼다.
    #[should_panic(expected = "axis1-all-verbatim")]
    fn contract_suite_catches_a_source_that_reorders_all() {
        // ★그물이 무는지 확인★ — 스위트가 통과만 시키는 장식이면 미래 교체 때 아무 것도 못 잡는다.
        assert_group_source_contract(
            &SortingSource,
            &GroupSourceFixture {
                known: "@x".to_string(),
                members: names(&["a"]),
                // 이 더블은 @all 만 아는 최소 소스라 "아는데 멤버 0명" 을 만들 수 없다 — 사유를 남겨 축을
                //   명시적으로 건너뛴다(조용한 None 금지 — EmptyAxis).
                empty: EmptyAxis::Unrepresentable {
                    why: "이 더블은 @all 만 해석하고 등록 명단이 없어 빈 그룹 상태를 만들 수 없다",
                },
                unknown: "@nope".to_string(),
                handles_all: true,
            },
        );
    }

    /// 일부러 어긋난 소스 ②: 빈 그룹을 `NotFound` 로 접는다 — 두 사실이 한 코드로 붕괴하면 발신자는
    /// "명단이 비었다"(멤버를 넣어라)와 "그런 그룹 없다"(만들어라)를 구별할 수 없다.
    struct ConflatingSource {
        members: Vec<String>,
    }
    impl GroupSource for ConflatingSource {
        fn resolve(&self, group: &str, live: &[String]) -> Result<Vec<String>, GroupError> {
            let norm = normalize_group_name(group)?;
            if norm == ALL_GROUP {
                if live.is_empty() {
                    return Err(GroupError::Empty { name: norm });
                }
                return Ok(live.to_vec());
            }
            if norm == "@known" {
                return Ok(self.members.clone());
            }
            // ← 계약 위반: 빈 그룹도 "없는 그룹" 과 같은 에러로 접는다.
            Err(GroupError::NotFound { name: norm })
        }
    }

    #[test]
    #[should_panic(expected = "axis4-empty-distinct")]
    fn contract_suite_catches_a_source_that_conflates_empty_with_not_found() {
        assert_group_source_contract(
            &ConflatingSource {
                members: names(&["a"]),
            },
            &GroupSourceFixture {
                known: "@known".to_string(),
                members: names(&["a"]),
                empty: EmptyAxis::Representable("@hollow".to_string()),
                unknown: "@nope".to_string(),
                handles_all: true,
            },
        );
    }
}
