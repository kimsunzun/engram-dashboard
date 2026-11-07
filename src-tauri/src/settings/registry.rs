//! Settings schema table — the canonical source of key · kind · default · description, and the rules that turn input into a canonical string (TRD §5-2) are
//! here too. Adding one setting = one line in [`SETTINGS`]. It does not know the file (`store`).

/// Kind of value — the accepted input and canonical form differ per kind ([`SettingDef::normalize`]).
pub(super) enum Kind {
    /// The spelling in the table is the canonical form. Input is case-insensitive.
    Choice(&'static [&'static str]),
    /// CSS length. ★The range is separate per unit★ — if a single numeric range is applied regardless of unit, `48rem` passes.
    /// `px` is always accepted, and only keys where `rem` is `Some` accept rem·em with the same range. Both ends inclusive.
    CssLength {
        px: (f64, f64),
        rem: Option<(f64, f64)>,
    },
    /// Unitless CSS number. Both ends inclusive.
    CssNumber { range: (f64, f64) },
}

pub(super) struct SettingDef {
    pub key: &'static str,
    pub kind: Kind,
    /// Write it in canonical form — a test measures 「기본값이 자기 자신으로 정규화된다」(the default normalizes to itself).
    pub default: &'static str,
    pub desc: &'static str,
}

/// Three sets of px · rem(em) range bundles. A generous line that current values pass, not a refined value (table header below).
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

/// All settings — this order is the answer order of `get`·`schema`.
///
/// ★`chat.style.*` is a temporary namespace (user decision 2026-10-02 · U2)★ — the frontend's 11 keys and defaults were carried over
/// as they are, and it is due to be replaced by the chat area rewrite and the plugin layout. Key names · bundling · ranges are not refined.
/// A change that removes these keys from the table also takes on deleting the values left in the file — an unknown key stays in the file and cannot be
/// deleted by a command (TRD §5-1).
///
/// ADR-0051: the `chat.style.*` defaults must equal the `--chat-*` fallback of `:root` in `src/styles/theme.css` —
/// if only one changes, the first boot frame and the settings application diverge. There is no test that measures that comparison (U2). The numbers are values the user
/// settled by live tuning — do not 「정리」(tidy up) them arbitrarily.
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

/// One exact key, or all keys covered by a prefix ending in `.` (table order). An empty list if none match.
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
    /// Input → canonical string (TRD §5-2). All kinds accept leading and trailing whitespace.
    ///
    /// A number accepts only `[+-]? (숫자+ (. 숫자+)? | . 숫자+)` — exponent notation · `inf` · `NaN` · `5.` are rejected.
    /// The canonical form is the shortest decimal (no exponent) · no `+` · `-0` becomes `0` · unit in lowercase.
    ///
    /// `Err` = a message shown to the caller as is (what is wrong + the expected format). ★The received value is not included★ — the sender
    /// already knows it, and an unbounded string would spread into the error message and log.
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

    /// Kind name that `settings.schema` carries.
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

    /// (lower bound, upper bound) — canonical form strings. `CssLength` joins one value per accepted unit with `, `
    /// (`"8px, 0.5rem, 0.5em"`). `Choice` is `None` for both.
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

/// The leading number and the rest (the unit place). For the number grammar see the docs of [`SettingDef::normalize`].
///
/// It is rejected even without blocking exponents separately — the rest of `1e2px` is `e2px`, which is not a unit.
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
    // A string that passed the grammar above is always accepted by the `f64` parser. A very long number rises to infinity and is caught by the range check.
    let value = text[..end].parse::<f64>().ok()?;
    Some((value, &text[end..]))
}

/// Shortest decimal — `Display` of `f64` gives the shortest round-tripping digits with no exponent.
fn canonical_number(value: f64) -> String {
    // `-0` to `0` — otherwise `-0px` stays as `-0px`.
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

    // ── table ──

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

    // ── selector ──

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

    // ── schema ──

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
