//! ★canonical 표시명 파생(ADR-0101 WYSIWYA)★ — cwd 의 마지막 경로 세그먼트(basename)를 파생한다.
//!
//! ★단일 출처(백엔드)★: 프론트 `src/util/basename.ts` 와 **정확히 같은 규칙**을 Rust 로 포팅한 것이다.
//!   백엔드(라우팅·로스터·봉투 sender)와 프론트(트리·팝업 표시)가 같은 문자열을 파생해야 "보이는 이름
//!   = 주소로 쓰는 이름"(WYSIWYA) 불변식이 선다. 각자 복제하면 win/posix·root/drive-root/UNC 엣지가
//!   갈려 트리엔 "engram" 인데 라우팅은 다른 문자열을 기대하는 어긋남이 생긴다 — 그래서 규칙을 한 곳에
//!   박고 프론트 규칙과 1:1 로 맞춘다(대응 테스트로 봉인).
//!
//! 아래 엣지 동작은 `basename.ts` 를 verbatim 미러한다 — 프론트 규칙이 바뀌면 이쪽도 함께 바꾼다.

/// cwd 가 비었거나 파생할 세그먼트가 없을 때의 안정적 placeholder(blank 라벨 방지).
/// 프론트 `PATH_NAME_PLACEHOLDER` 와 동일 문자열이어야 한다(표시 일치).
pub const PATH_NAME_PLACEHOLDER: &str = "(경로 없음)";

/// ★반환값은 절대 blank(빈/공백-only) 가 아니다★: 상위(표시명)는 이 값 하나로만 그리므로 blank 면
///   빈 칸으로 보인다. 파생할 basename 도 raw cwd 도 없을 때는 placeholder 로 degrade 한다.
///
/// 엣지 케이스(basename 이 없거나 misleading 할 때)는 파생하지 않고 raw cwd 로 degrade 한다(프론트 동형):
///   - 빈/공백-only 문자열 → `PATH_NAME_PLACEHOLDER`
///   - drive-root `C:\` / `C:/` / `C:` → 원본 유지("C:" 로 collapse 하면 오해 소지)
///   - posix root `/` · UNC `\\server\share` 처럼 후행 구분자 제거 후 세그먼트가 없거나 원본과
///     같아지는 경우 → raw cwd 반환(잘못된 세그먼트로 붕괴 방지).
pub fn cwd_basename(cwd: &str) -> String {
    // ★JS trim 과의 공백집합 미세 차이(수용된 divergence)★: Rust str::trim(White_Space)과
    //   JS String.trim 이 인식하는 공백이 U+FEFF·U+0085 등에서 조금 다르다 — 그런 exotic 공백만으로
    //   이뤄진 cwd 는 한쪽만 placeholder 로 떨어져 갈릴 수 있다. 그러나 실제 파일시스템 cwd 는 절대
    //   이런 문자열이 아니므로 JS 공백집합 완전 일치는 쫓지 않는다(현실 경로엔 무영향).
    if cwd.trim().is_empty() {
        return PATH_NAME_PLACEHOLDER.to_string();
    }
    // 후행 구분자 제거(`/`·`\` 모두). 프론트 정규식 `/[\\/]+$/` 와 동형.
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return cwd.to_string();
    }
    // 마지막 구분자 위치(둘 중 큰 인덱스). 프론트 `Math.max(lastIndexOf('/'), lastIndexOf('\\'))` 동형.
    let idx = trimmed.rfind(['/', '\\']).map(|i| i as isize).unwrap_or(-1);
    let base = if idx >= 0 {
        // idx 는 유효 바이트 경계(구분자 = ASCII 1바이트)라 슬라이싱 안전.
        &trimmed[(idx as usize) + 1..]
    } else {
        trimmed
    };
    if base.is_empty() || is_drive_root(base) {
        return cwd.to_string();
    }
    base.to_string()
}

/// ★프로필 있는 에이전트의 canonical 표시명 파생(ADR-0101)★ — display_name(override) 이 있으면 그대로,
///   없으면 cwd basename. **manager(agent_info)·daemon(sender_display_name) 공유 사슬의 순수 코어**다.
///   프로필 부재(ad-hoc) 시의 id-prefix fallback 은 호출부가 각자 붙인다(id 를 여기 넘기지 않기 위함 —
///   이 함수는 프론트 트리 규칙 `display_name ?? basename(cwd)` 과 1:1 대응하는 순수 파생으로 유지).
pub fn resolve_display_name(display_name: Option<&str>, cwd: &str) -> String {
    match display_name {
        Some(n) => n.to_string(),
        None => cwd_basename(cwd),
    }
}

/// ★프로필 부재(ad-hoc / 산 세션에 DeleteProfile) 시의 canonical 이름 파생(ADR-0101)★ —
///   override 없이 session cwd basename 으로 파생하되, cwd 가 placeholder/빈값을 낼 때만 id 앞 8자로
///   degrade 한다. manager·reaper 가 **같은 fallback** 을 쓰게 하는 공유 코어(로직 복제 금지):
///   프론트 트리는 프로필이 사라진 산 세션도 basename(AgentInfo.cwd)로 그리므로, 백엔드가 곧장
///   id-prefix 로 떨어지면 트리 ≠ 라우팅 이 생긴다. 그래서 cwd basename 을 먼저 시도한다.
///   공백-only display_name 은 override 없음으로 취급한다.
pub fn canonical_name_or_id_fallback(
    display_name: Option<&str>,
    cwd: &str,
    id: crate::types::AgentId,
) -> String {
    if let Some(n) = display_name {
        if !n.trim().is_empty() {
            return n.to_string();
        }
    }
    let base = cwd_basename(cwd);
    // 실 파일시스템 cwd 는 이 갈래에 안 온다 — DeleteProfile 후 cwd 가 빈 엣지 방어용이다.
    if base == PATH_NAME_PLACEHOLDER {
        let s = id.to_string();
        return s[..8.min(s.len())].to_string();
    }
    base
}

/// `C:` 처럼 [영문자]+콜론 단독인지. 프론트 정규식 `/^[A-Za-z]:$/` 와 동형.
fn is_drive_root(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    // 아래 케이스는 프론트 `src/util/basename.test.ts` 와 1:1 대응한다(WYSIWYA 봉인 — 한쪽 변경 시
    //   대응 케이스가 깨져 두 규칙이 갈렸음을 즉시 드러낸다).

    #[test]
    fn posix_path_basename() {
        assert_eq!(cwd_basename("/home/me/project"), "project");
    }

    #[test]
    fn windows_path_basename() {
        assert_eq!(cwd_basename("C:\\work\\engram"), "engram");
    }

    #[test]
    fn trailing_separator_ignored() {
        assert_eq!(cwd_basename("C:/proj/"), "proj");
        assert_eq!(cwd_basename("/a/b/c/"), "c");
    }

    #[test]
    fn no_segment_root_falls_back_to_raw() {
        assert_eq!(cwd_basename("/"), "/");
        assert_eq!(cwd_basename("projectonly"), "projectonly");
    }

    #[test]
    fn drive_root_kept_raw() {
        assert_eq!(cwd_basename("C:\\"), "C:\\");
        assert_eq!(cwd_basename("C:/"), "C:/");
        assert_eq!(cwd_basename("C:"), "C:");
    }

    #[test]
    fn empty_or_whitespace_returns_placeholder() {
        assert_eq!(cwd_basename(""), PATH_NAME_PLACEHOLDER);
        assert_eq!(cwd_basename("   "), PATH_NAME_PLACEHOLDER);
        assert!(!PATH_NAME_PLACEHOLDER.trim().is_empty());
    }

    #[test]
    fn unc_share_derives_last_segment() {
        assert_eq!(cwd_basename("\\\\server\\share"), "share");
        assert_eq!(cwd_basename("\\\\server\\share\\"), "share");
    }

    // ── resolve_display_name (canonical 이름 사슬 — 프론트 트리 `display_name ?? basename(cwd)`) ──

    #[test]
    fn resolve_prefers_display_name_override() {
        // 트리 rename 시나리오.
        assert_eq!(
            resolve_display_name(Some("ABC"), "C:\\work\\Filter Library"),
            "ABC"
        );
    }

    #[test]
    fn resolve_falls_back_to_cwd_basename_when_no_override() {
        // profile.name 이 아니라 cwd basename 이다.
        assert_eq!(resolve_display_name(None, "C:\\work\\engram"), "engram");
        assert_eq!(resolve_display_name(None, "/home/me/project"), "project");
    }

    #[test]
    fn resolve_empty_cwd_falls_back_to_placeholder() {
        assert_eq!(resolve_display_name(None, ""), PATH_NAME_PLACEHOLDER);
    }

    // ── canonical_name_or_id_fallback (프로필 부재 fallback — manager·reaper 공유 코어, ADR-0101) ──

    #[test]
    fn fallback_prefers_display_name_override() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(Some("Alice"), "C:\\work\\engram", id),
            "Alice"
        );
    }

    #[test]
    fn fallback_uses_session_cwd_basename_when_no_override() {
        // ★프로필 부재라도 트리는 basename(cwd)를 그리므로 id-prefix 로 바로 안 떨어진다★
        //   — DeleteProfile 로 프로필만 사라진 산 세션에서 트리 == 라우팅 유지.
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(None, "C:\\work\\engram", id),
            "engram"
        );
        assert_eq!(
            canonical_name_or_id_fallback(None, "/home/me/project", id),
            "project"
        );
    }

    #[test]
    fn fallback_blank_override_ignored_uses_cwd() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            canonical_name_or_id_fallback(Some("   "), "/home/me/project", id),
            "project"
        );
    }

    #[test]
    fn fallback_id_prefix_only_when_cwd_empty() {
        let id = uuid::Uuid::new_v4();
        let expected: String = id.to_string().chars().take(8).collect();
        assert_eq!(canonical_name_or_id_fallback(None, "", id), expected);
        assert_eq!(canonical_name_or_id_fallback(None, "   ", id), expected);
    }
}
