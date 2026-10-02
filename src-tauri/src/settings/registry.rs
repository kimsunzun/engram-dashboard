//! 설정 스키마 표 — 키 · 종류 · 기본값 · 설명의 정본이고, 입력을 정규 문자열로 바꾸는 규칙(TRD §5-2)도
//! 여기 있다. 설정 하나를 더하는 일 = [`SETTINGS`] 에 한 줄. 파일은 모른다(`store`).

/// 값의 종류 — 받는 입력과 정규형이 종류마다 갈린다([`SettingDef::normalize`]).
pub(super) enum Kind {
    /// 표의 철자가 정규형이다. 입력은 대소문자를 가리지 않는다.
    Choice(&'static [&'static str]),
    /// CSS 길이. ★범위는 단위마다 따로다★ — 숫자 범위 하나를 단위와 무관하게 대면 `48rem` 이 통과한다.
    /// `px` 는 늘 받고, `rem` 이 `Some` 인 키만 rem·em 을 같은 범위로 받는다. 양 끝 포함.
    CssLength {
        px: (f64, f64),
        rem: Option<(f64, f64)>,
    },
    /// 단위 없는 CSS 수. 양 끝 포함.
    CssNumber { range: (f64, f64) },
}

pub(super) struct SettingDef {
    pub key: &'static str,
    pub kind: Kind,
    /// 정규형으로 적는다 — 시험이 「기본값이 자기 자신으로 정규화된다」를 잰다.
    pub default: &'static str,
    pub desc: &'static str,
}

/// px · rem(em) 범위 묶음 셋. 지금 값이 통과하는 넉넉한 선이지 다듬은 값이 아니다(아래 표 머리).
const FONT_PX: (f64, f64) = (8.0, 48.0);
const FONT_REM: (f64, f64) = (0.5, 3.0);
const SPACE_PX: (f64, f64) = (0.0, 200.0);
const SPACE_REM: (f64, f64) = (0.0, 12.5);
const OFFSET_PX: (f64, f64) = (-200.0, 200.0);
const OFFSET_REM: (f64, f64) = (-12.5, 12.5);

const fn spacing(key: &'static str, default: &'static str, desc: &'static str) -> SettingDef {
    SettingDef {
        key,
        kind: Kind::CssLength {
            px: SPACE_PX,
            rem: Some(SPACE_REM),
        },
        default,
        desc,
    }
}

/// 설정 전부 — 이 순서가 `get`·`schema` 의 답 순서다.
///
/// ★`chat.style.*` 는 임시 이름공간이다(사용자 결정 2026-10-02 · U2)★ — 프론트의 11키와 기본값을 그대로
/// 옮겼고, 챗 영역 재작성과 플러그인 배치로 갈려 나갈 예정이다. 키 이름 · 묶음 · 범위를 다듬지 않는다.
/// 이 키들을 표에서 빼는 변경이 파일에 남은 값을 지우는 일까지 맡는다 — 모르는 키는 파일에 남고 명령으로
/// 못 지운다(TRD §5-1).
///
/// ADR-0051: `chat.style.*` 기본값은 `src/styles/theme.css` `:root` 의 `--chat-*` fallback 과 같아야 한다 —
/// 하나만 바뀌면 부팅 첫 프레임과 설정 적용이 어긋난다. 그 대조를 재는 시험은 없다(U2). 숫자는 사용자가
/// 라이브 튜닝으로 확정한 값이다 — 임의로 「정리」하지 말 것.
// ADR-0265
pub(super) const SETTINGS: &[SettingDef] = &[
    SettingDef {
        key: "theme.default",
        kind: Kind::Choice(&["dark", "light", "e-ink"]),
        default: "dark",
        desc: "창별 덮어쓰기가 없는 창의 테마",
    },
    spacing(
        "chat.style.railRowPt",
        "0.8rem",
        "챗 rail 행 위쪽 여백(행간)",
    ),
    spacing(
        "chat.style.plainRowPt",
        "0.7rem",
        "챗 rail 이 아닌 행(사용자 말풍선·구분선) 위쪽 여백",
    ),
    spacing("chat.style.userPy", "7px", "사용자 말풍선 세로 안쪽 여백"),
    spacing(
        "chat.style.userPx",
        "0.9rem",
        "사용자 말풍선 가로 안쪽 여백",
    ),
    spacing(
        "chat.style.userMy",
        "0.375rem",
        "사용자 말풍선 세로 바깥 여백",
    ),
    spacing("chat.style.railGutter", "1.5rem", "챗 rail 칸 폭"),
    SettingDef {
        key: "chat.style.railLineOffset",
        kind: Kind::CssLength {
            px: OFFSET_PX,
            rem: Some(OFFSET_REM),
        },
        default: "-1rem",
        desc: "챗 rail 연결선 위쪽 오프셋(보통 음수 — 위 행으로 잇는다)",
    },
    spacing(
        "chat.style.railDotTop",
        "0.5625rem",
        "챗 rail 점 표식의 위쪽 위치",
    ),
    SettingDef {
        key: "chat.style.fontSize",
        kind: Kind::CssLength {
            px: FONT_PX,
            rem: Some(FONT_REM),
        },
        default: "13px",
        desc: "챗 기본 글자 크기",
    },
    SettingDef {
        key: "chat.style.lineHeight",
        kind: Kind::CssNumber { range: (0.5, 4.0) },
        default: "1.45",
        desc: "챗 기본 줄 높이(글자 크기의 배수)",
    },
    spacing(
        "chat.style.waitStripH",
        "1.75rem",
        "입력창 위 대기 표시 줄 높이",
    ),
];

pub(super) fn find(key: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|def| def.key == key)
}

/// 정확한 키 하나, 또는 `.` 으로 끝나는 접두가 덮는 키 전부(표 순서). 맞는 것이 없으면 빈 목록.
pub(super) fn select(selector: &str) -> Vec<&'static SettingDef> {
    if selector.ends_with('.') {
        SETTINGS
            .iter()
            .filter(|def| def.key.starts_with(selector))
            .collect()
    } else {
        find(selector).into_iter().collect()
    }
}

impl SettingDef {
    /// 입력 → 정규 문자열(TRD §5-2). 모든 종류가 앞뒤 공백을 받는다.
    ///
    /// 수는 `[+-]? (숫자+ (. 숫자+)? | . 숫자+)` 만 받는다 — 지수 표기 · `inf` · `NaN` · `5.` 은 거절이다.
    /// 정규형은 최단 십진(지수 없음) · `+` 없음 · `-0` 은 `0` · 단위는 소문자.
    ///
    /// `Err` = 호출자에게 그대로 보일 문구(무엇이 틀렸나 + 기대 형식). ★받은 값은 싣지 않는다★ — 보낸 쪽이
    /// 이미 알고, 상한 없는 문자열이 오류 문구와 로그로 번진다.
    pub fn normalize(&self, input: &str) -> Result<String, String> {
        let input = input.trim();
        let refuse = |problem: &str| Err(format!("{problem} — 기대: {}", self.expected()));
        match &self.kind {
            Kind::Choice(choices) => match choices.iter().find(|c| c.eq_ignore_ascii_case(input)) {
                Some(choice) => Ok((*choice).to_string()),
                None => refuse("선택지에 없다"),
            },
            Kind::CssLength { px, rem } => {
                let Some((value, unit)) = split_number(input) else {
                    return refuse("수가 아니다");
                };
                let unit = unit.to_ascii_lowercase();
                let range = match (unit.as_str(), rem) {
                    ("px", _) => *px,
                    ("rem" | "em", Some(rem)) => *rem,
                    _ => return refuse("받지 않는 단위다"),
                };
                if !within(value, range) {
                    return refuse("범위 밖이다");
                }
                Ok(format!("{}{unit}", canonical_number(value)))
            }
            Kind::CssNumber { range } => match split_number(input) {
                Some((value, "")) if within(value, *range) => Ok(canonical_number(value)),
                Some((_, "")) => refuse("범위 밖이다"),
                _ => refuse("단위 없는 수가 아니다"),
            },
        }
    }

    /// `settings.schema` 가 싣는 종류 이름.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Choice(_) => "choice",
            Kind::CssLength { .. } => "css-length",
            Kind::CssNumber { .. } => "css-number",
        }
    }

    pub fn choices(&self) -> Option<Vec<String>> {
        match self.kind {
            Kind::Choice(choices) => Some(choices.iter().map(|c| c.to_string()).collect()),
            _ => None,
        }
    }

    /// (하한, 상한) — 정규형 문자열. `CssLength` 는 받는 단위마다 한 값씩 `, ` 로 잇는다
    /// (`"8px, 0.5rem, 0.5em"`). `Choice` 는 둘 다 `None`.
    pub fn bounds(&self) -> (Option<String>, Option<String>) {
        match &self.kind {
            Kind::Choice(_) => (None, None),
            Kind::CssLength { px, rem } => {
                let units = length_units(*px, *rem);
                let side = |pick: fn((f64, f64)) -> f64| {
                    units
                        .iter()
                        .map(|(unit, range)| format!("{}{unit}", canonical_number(pick(*range))))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                (Some(side(|r| r.0)), Some(side(|r| r.1)))
            }
            Kind::CssNumber { range } => (
                Some(canonical_number(range.0)),
                Some(canonical_number(range.1)),
            ),
        }
    }

    fn expected(&self) -> String {
        match &self.kind {
            Kind::Choice(choices) => format!("{} 중 하나(대소문자 무시)", choices.join(" · ")),
            Kind::CssLength { px, rem } => {
                let ranges = length_units(*px, *rem)
                    .iter()
                    .map(|(unit, (lo, hi))| {
                        format!("{unit} {}~{}", canonical_number(*lo), canonical_number(*hi))
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                format!(
                    "수 바로 뒤에 단위, 지수 표기 없이 — {ranges} (예: {})",
                    self.default
                )
            }
            Kind::CssNumber { range } => format!(
                "단위 없는 수, 지수 표기 없이 — {}~{} (예: {})",
                canonical_number(range.0),
                canonical_number(range.1),
                self.default
            ),
        }
    }
}

fn length_units(px: (f64, f64), rem: Option<(f64, f64)>) -> Vec<(&'static str, (f64, f64))> {
    let mut units = vec![("px", px)];
    if let Some(rem) = rem {
        units.push(("rem", rem));
        units.push(("em", rem));
    }
    units
}

fn within(value: f64, (lo, hi): (f64, f64)) -> bool {
    lo <= value && value <= hi
}

/// 앞머리의 수와 나머지(단위 자리). 수 문법은 [`SettingDef::normalize`] 문서.
///
/// 지수를 따로 막지 않아도 거절된다 — `1e2px` 의 나머지는 `e2px` 라 단위가 아니다.
fn split_number(text: &str) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    let digits_from = |start: usize| {
        bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let int_digits = digits_from(end);
    end += int_digits;
    let mut frac_digits = 0;
    if bytes.get(end) == Some(&b'.') {
        frac_digits = digits_from(end + 1);
        if frac_digits == 0 {
            return None;
        }
        end += 1 + frac_digits;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    // 위 문법을 지난 문자열은 `f64` 파서가 늘 받는다. 아주 긴 수는 무한대로 올라가 범위 검사에서 걸린다.
    let value = text[..end].parse::<f64>().ok()?;
    Some((value, &text[end..]))
}

/// 최단 십진 — `f64` 의 `Display` 는 왕복하는 가장 짧은 자릿수를 지수 없이 낸다.
fn canonical_number(value: f64) -> String {
    // `-0` 을 `0` 으로 — 안 그러면 `-0px` 가 `-0px` 로 남는다.
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(key: &str) -> &'static SettingDef {
        find(key).expect("표에 있는 키")
    }

    fn ok(key: &str, input: &str) -> String {
        def(key)
            .normalize(input)
            .unwrap_or_else(|e| panic!("{key} = {input:?} 가 거절됐다: {e}"))
    }

    fn refused(key: &str, input: &str) -> String {
        match def(key).normalize(input) {
            Ok(v) => panic!("{key} = {input:?} 가 {v:?} 로 통과했다"),
            Err(e) => e,
        }
    }

    // ── 표 ──

    #[test]
    fn the_table_has_twelve_unique_keys() {
        assert_eq!(SETTINGS.len(), 12);
        let mut keys: Vec<&str> = SETTINGS.iter().map(|d| d.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 12, "키가 겹친다");
    }

    #[test]
    fn every_default_is_already_canonical() {
        for d in SETTINGS {
            assert_eq!(
                d.normalize(d.default).as_deref(),
                Ok(d.default),
                "{} 의 기본값이 정규형이 아니거나 범위 밖이다",
                d.key
            );
        }
    }

    #[test]
    fn chat_style_defaults_match_the_frontend_values() {
        let expected = [
            ("railRowPt", "0.8rem"),
            ("plainRowPt", "0.7rem"),
            ("userPy", "7px"),
            ("userPx", "0.9rem"),
            ("userMy", "0.375rem"),
            ("railGutter", "1.5rem"),
            ("railLineOffset", "-1rem"),
            ("railDotTop", "0.5625rem"),
            ("fontSize", "13px"),
            ("lineHeight", "1.45"),
            ("waitStripH", "1.75rem"),
        ];
        for (name, default) in expected {
            assert_eq!(def(&format!("chat.style.{name}")).default, default);
        }
        assert_eq!(def("theme.default").default, "dark");
    }

    // ── 선택자 ──

    #[test]
    fn a_selector_is_an_exact_key_or_a_prefix_ending_in_a_dot() {
        assert_eq!(select("theme.default").len(), 1);
        assert_eq!(select("chat.style.").len(), 11);
        assert_eq!(select("chat.").len(), 11);
        assert!(
            select("chat.style").is_empty(),
            "점 없는 접두는 키가 아니다"
        );
        assert!(select("theme.").iter().all(|d| d.key == "theme.default"));
        assert!(select("").is_empty());
        assert!(select(".").is_empty());
        assert!(select("THEME.DEFAULT").is_empty(), "키는 대소문자를 가린다");
    }

    // ── Choice ──

    #[test]
    fn choice_ignores_case_and_surrounding_space_and_returns_the_table_spelling() {
        assert_eq!(ok("theme.default", "light"), "light");
        assert_eq!(ok("theme.default", "E-INK"), "e-ink");
        assert_eq!(ok("theme.default", "  Dark\t"), "dark");
        refused("theme.default", "eink");
        refused("theme.default", "");
        refused("theme.default", "dark light");
    }

    // ── CssLength ──

    #[test]
    fn length_canonical_form_is_shortest_decimal_with_lowercase_unit() {
        let k = "chat.style.fontSize";
        assert_eq!(ok(k, "15px"), "15px");
        assert_eq!(ok(k, "15.0px"), "15px");
        assert_eq!(ok(k, "015.50PX"), "15.5px");
        assert_eq!(ok(k, " 0.90Rem "), "0.9rem");
        assert_eq!(ok(k, ".5rem"), "0.5rem");
        assert_eq!(ok(k, "1EM"), "1em");
        assert_eq!(ok(k, "+13px"), "13px", "부호 + 는 받고 정규형에서 뺀다");
    }

    #[test]
    fn length_range_is_per_unit_and_inclusive() {
        let k = "chat.style.fontSize";
        assert_eq!(ok(k, "8px"), "8px");
        assert_eq!(ok(k, "48px"), "48px");
        assert_eq!(ok(k, "0.5rem"), "0.5rem");
        assert_eq!(ok(k, "3rem"), "3rem");
        assert_eq!(ok(k, "3em"), "3em");
        refused(k, "7.999px");
        refused(k, "48.001px");
        refused(k, "48rem");
        refused(k, "3.1em");
        refused(k, "0.4rem");
    }

    #[test]
    fn spacing_keys_take_zero_and_reject_negatives() {
        let k = "chat.style.userPy";
        assert_eq!(ok(k, "0px"), "0px");
        assert_eq!(ok(k, "-0px"), "0px", "-0 은 0 으로 접는다");
        assert_eq!(ok(k, "-0.0rem"), "0rem");
        assert_eq!(ok(k, "200px"), "200px");
        assert_eq!(ok(k, "12.5rem"), "12.5rem");
        refused(k, "-1px");
        refused(k, "200.5px");
        refused(k, "12.6rem");
        assert_eq!(ok("chat.style.waitStripH", "3rem"), "3rem");
    }

    #[test]
    fn the_rail_line_offset_takes_negatives_to_its_edges() {
        let k = "chat.style.railLineOffset";
        assert_eq!(ok(k, "-1rem"), "-1rem");
        assert_eq!(ok(k, "-200px"), "-200px");
        assert_eq!(ok(k, "-12.5rem"), "-12.5rem");
        assert_eq!(ok(k, "-.25em"), "-0.25em");
        refused(k, "-200.1px");
        refused(k, "-12.51rem");
        refused(k, "12.51rem");
    }

    #[test]
    fn length_refuses_malformed_numbers_and_units() {
        let k = "chat.style.fontSize";
        for bad in [
            "13", "px", "13 px", "13pt", "13%", "13px;", "1e1px", "1E1px", "1e+1px", "13.px",
            "13.", ".px", "-", "+", "+-13px", "--13px", "inf", "infpx", "NaNpx", "nanpx", "１３px",
            "13px px", "",
        ] {
            refused(k, bad);
        }
    }

    #[test]
    fn a_huge_number_is_out_of_range_not_infinite() {
        let huge = format!("1{}px", "0".repeat(400));
        let e = refused("chat.style.fontSize", &huge);
        assert!(e.contains("범위"), "{e}");
    }

    #[test]
    fn the_refusal_names_the_expected_format_and_not_the_input() {
        let e = refused("chat.style.fontSize", "secret-token-48rem");
        assert!(e.contains("px 8~48"), "{e}");
        assert!(e.contains("rem 0.5~3"), "{e}");
        assert!(!e.contains("secret"), "받은 값을 싣지 않는다: {e}");
        let e = refused("theme.default", "x");
        assert!(e.contains("dark · light · e-ink"), "{e}");
    }

    // ── CssNumber ──

    #[test]
    fn number_canonical_form_and_range() {
        let k = "chat.style.lineHeight";
        assert_eq!(ok(k, "1.45"), "1.45");
        assert_eq!(ok(k, " 1.450 "), "1.45");
        assert_eq!(ok(k, "+2"), "2");
        assert_eq!(ok(k, "0.5"), "0.5");
        assert_eq!(ok(k, "4"), "4");
        refused(k, "0.49");
        refused(k, "4.01");
        refused(k, "1.45px");
        refused(k, "1e0");
        refused(k, "NaN");
        refused(k, "inf");
        refused(k, "normal");
    }

    // ── 스키마 ──

    #[test]
    fn schema_kinds_and_bounds() {
        let font = def("chat.style.fontSize");
        assert_eq!(font.kind_name(), "css-length");
        assert_eq!(
            font.bounds(),
            (
                Some("8px, 0.5rem, 0.5em".to_string()),
                Some("48px, 3rem, 3em".to_string())
            )
        );
        assert_eq!(
            def("chat.style.railLineOffset").bounds().0.as_deref(),
            Some("-200px, -12.5rem, -12.5em")
        );
        let line = def("chat.style.lineHeight");
        assert_eq!(line.kind_name(), "css-number");
        assert_eq!(
            line.bounds(),
            (Some("0.5".to_string()), Some("4".to_string()))
        );
        let theme = def("theme.default");
        assert_eq!(theme.kind_name(), "choice");
        assert_eq!(theme.bounds(), (None, None));
        assert_eq!(
            theme.choices(),
            Some(vec!["dark".into(), "light".into(), "e-ink".into()])
        );
        assert_eq!(font.choices(), None);
    }
}
