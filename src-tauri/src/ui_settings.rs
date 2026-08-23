//! 디스크의 UI 설정(`<data_dir>/ui-settings.json`) — ★셸은 읽고, 부팅에 한 번만 쓴다★(아래 두 절).
//!
//! 소유: 파일 위치 · 원문→값 변환 · 못 읽거나 깨졌을 때의 기본값 · **어느 창이 어떤 값을 받나** ·
//! **어느 창 항목이 살아남나**. 진입점은 [`load_settings`](fn.load_settings.html)(seam 위) ·
//! [`parse_settings`](fn.parse_settings.html)(순수) · [`deliver_per_window`](fn.deliver_per_window.html)
//! (창별 배달) · [`sweep_dead_windows`](fn.sweep_dead_windows.html)(부팅 정리). 뒤 둘은 **보내는·쓰는
//! 자리를 호출자가 넣는다** — Tauri 도 파일 쓰기도 이 모듈에 들이지 않는다.
//!
//! ## ★화면발 쓰기를 만들지 마라(되살리지 마라)★
//! 파일은 **밖의 에이전트가 직접 고친다**. 화면이 테마를 파일에 저장하기 시작하면 밖의 편집자와 화면이
//! 같은 파일을 두고 경합하고, 그 조정(누가 마지막에 썼나)은 아무도 안 풀었다 — ADR-0166 이 범위 밖에
//! 둔 자리다. 그래서 화면의 테마 조작은 인메모리로 남고 다음 `ui.refresh` 가 그것을 덮는다.
//! ★금지되는 것은 **화면발 쓰기**이지 이 파일에 쓰는 행위 전부가 아니다★ — 그 오독이 두 번 났다.
//!
//! ## ★쓰는 자리는 하나 — 부팅 쓸기★ // ADR-0167
//! [`sweep_dead_windows`] 가 **죽은 창의 항목**만 지운다([`write_atomic`] — 임시 파일 + rename).
//! 위 경합에 안 닿는 이유 셋: **부팅에 한 번**만 돌고(refresh 마다가 아니다), **화면이 끼지 않으며**
//! (토글을 영속시키는 것이 아니다), 지우는 대상이 밖의 에이전트가 방금 쓴 값이 아니라 **재시작을 넘겨
//! 오적용될 항목**이다. 세 번째 쓰는 자리를 늘리기 전에 ADR-0167 「영향/불변식」의 갈림길(파일 쪼개기 /
//! 쓰기 직렬화)을 먼저 고를 것 — 그때 위 경합이 실재가 된다.
//!
//! ## ★`.corrupt` 사이드카를 만들지 않는다★
//! 에이전트 명부는 잃으면 되살릴 수 없어 깨진 원본을 옆에 남기지만, 이 파일에는 되살릴 것이 없다
//! (한 칸짜리 취향값이고 밖에서 다시 쓰면 그만이다). 깨졌으면 로그 한 줄과 기본값이 전부다.
//! ★사본을 안 남기는 것과 로그 레벨은 별개다★ — 파싱 실패는 그래도 손상 신호라 `error` 로 나간다
//! ([`load_settings`] · 정본 `docs/reference/logging-conventions.md`).
//!
//! ## ★원문 크기 상한이 있다★
//! 밖의 에이전트가 쓰는 파일이라 크기가 우리 손에 없다. 통째로 읽고 나서 재면 상한 검사가 도착하기 전에
//! 메모리가 먼저 바닥나 **기본값 접기도 경고도 못 돌고 프로세스가 죽는다** — 그래서 읽는 양 자체를
//! [`MAX_SETTINGS_BYTES`] 에서 끊는다([`read_capped`]).
//!
//! ## ★값 타입이 하나고 파싱도 한 자리다★
//! 파일이 말하는 것은 [`UiSettings`] 한 벌이다 — 전역 테마와 창별 덮어쓰기 지도. **칸을 늘릴 때는
//! [`parse_settings`] 안에서 그 타입에 칸을 붙인다**: 키를 하나 더 꺼내 보는 함수를 옆에 세우면 모르는 칸
//! 무시·상한·기본값 접기가 함수마다 갈린다.
//!
//! ## ★고장의 무게가 둘로 갈린다★
//! - **전역 칸을 못 쓰면 파일 전체를 못 쓴다** — 접을 바닥이 없어 모든 창이 [`DEFAULT_THEME`] 로 간다.
//! - **창 항목 하나를 못 쓰면 그 창만 전역 값으로 접는다** — 파일은 멀쩡하고 전역 값이 이미 그 창의 답이다.
//!   [`DEFAULT_THEME`] 로 접으면 그 창만 파일에 적힌 어느 값과도 안 맞게 된다. 사유는 로그가 진다
//!   ([`ParsedSettings::refused`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 데이터 폴더 안 파일 이름 — 밖의 에이전트가 이 이름으로 찾는다.
pub const UI_SETTINGS_FILE: &str = "ui-settings.json";

/// 읽기·파싱이 실패한 자리에서 쓰는 값. ★실패 종류를 가르지 않고 전부 이 값으로 접는다★.
pub const DEFAULT_THEME: UiTheme = UiTheme::Dark;

/// 화면 테마 — 세 값이 전부다.
///
/// ★`EInk` 를 빼지 말 것★: e-ink 는 밝기 변형이 아니라 **색을 무력화하는** 별도 의도라 dark/light 로
/// 접히지 않는다(ADR-0062).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Dark,
    Light,
    EInk,
}

impl UiTheme {
    /// 프론트가 `document.documentElement` 의 `data-theme` 에 그대로 박는 문자열.
    ///
    /// ★`src/styles/theme.css` 의 `:root[data-theme='…']` 셀렉터와 **같은 철자**여야 한다★ — 어긋나면
    /// 오류 하나 없이 스타일만 안 붙어서, 화면만 보고는 철자가 틀린 것인지 파일이 안 읽힌 것인지 모른다.
    pub const fn as_wire(self) -> &'static str {
        match self {
            UiTheme::Dark => "dark",
            UiTheme::Light => "light",
            UiTheme::EInk => "e-ink",
        }
    }

    fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "dark" => Some(UiTheme::Dark),
            "light" => Some(UiTheme::Light),
            "e-ink" => Some(UiTheme::EInk),
            _ => None,
        }
    }
}

/// 파일 한 벌이 말하는 것 — 전역 테마와 **창별 덮어쓰기**.
///
/// 해소 규칙은 하나다: 항목이 있는 창은 그 값, 없는 창은 전역 값([`UiSettings::theme_for`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    global: UiTheme,
    /// 창 label → 그 창만의 테마. **없는 창은 부재가 곧 「전역 값」이다**(빈 값을 따로 두지 않는다).
    ///
    /// ★label 은 신원이 아니다★ — 팝아웃 label 은 앱을 띄울 때마다 1 부터 다시 세는 프로세스-로컬 카운터가
    /// 짓는다(`commands/popout.rs`). 그래서 재시작을 넘긴 `slot-popup-1` 항목은 **다른 창**에 조용히
    /// 적용된다. 쓸어내는 자리는 부팅이고 `ui.refresh` 가 아니다 — 아직 만들어지는 중인 창의 항목을 지운다.
    // ADR-0167
    windows: BTreeMap<String, UiTheme>,
}

impl UiSettings {
    /// 창을 안 가리는 자리(명령 답장)가 싣는 값.
    pub fn global(&self) -> UiTheme {
        self.global
    }

    pub fn theme_for(&self, window: &str) -> UiTheme {
        self.windows.get(window).copied().unwrap_or(self.global)
    }
}

/// [`parse_settings`] 가 내놓는 것 — 값 한 벌과 **반려당한 창 항목**.
///
/// 반려를 값에 안 싣고 여기 따로 두는 이유: 반려는 로그로만 나가고 wire 로는 안 나간다([`ThemeSource`] 가
/// 두 갈래인 채로 남는다). 접힌 결과 자체는 「항목이 없는 창」과 구별되지 않는다 — 그게 접기의 뜻이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSettings {
    pub settings: UiSettings,
    /// 반려 사유 — ★[`MAX_REFUSED_DETAILS`] 건까지만★. 전체 개수는 [`ParsedSettings::refused_total`].
    pub refused: Vec<String>,
    /// 반려된 항목 총수(위 목록은 그중 앞부분).
    pub refused_total: usize,
}

/// 로그에 사유를 펼치는 항목 수 상한.
///
/// ★개수 자체가 증폭 경로다★ — 원문은 [`MAX_SETTINGS_BYTES`] 까지 허용되므로 못 쓰는 항목이 수천 개인
/// 파일이 성립하고, 이 파일은 **창을 열 때마다·refresh 때마다** 다시 읽힌다. 그래서 사유는 앞 몇 건만
/// 펼치고 나머지는 총수로만 센다(값 하나의 길이를 자르는 것은 [`describe_value`] 의 몫 — 다른 축이다).
pub const MAX_REFUSED_DETAILS: usize = 3;

/// 적용된 값이 어디서 왔나 — ★두 갈래뿐이다★.
///
/// 호출자의 질문은 「내가 고친 값이 먹었나」 하나이고, 그 답에 필요한 것은 이 둘이다. **왜** 접혔는지
/// (파일 없음 · 못 읽음 · JSON 깨짐 · 모르는 이름 · 상한 초과)는 밖으로 내보내지 않는다 — 그 다섯은
/// 로그가 진다([`load_settings`]). 다섯을 wire 로 올리면 호출자가 사유별 분기를 짜기 시작하고, 그 순간
/// 이 다섯 갈래가 계약이 돼 버린다.
///
/// ★wire 문자열을 손으로 적지 않는다★ — serde 가 variant 이름을 그대로 낸다. 리터럴을 다시 타이핑하면
/// 같은 어휘가 **세 곳**(이 enum · 그 리터럴 · wire 쪽 `ThemeOrigin`)에 살고, 리터럴만 고쳐도 아무것도
/// 안 깨진다. 남은 두 다리는 `layout::commands` 의 exhaustive `match`(갈래 존재)와 두 직렬화를 맞대는
/// 테스트(철자)가 잡는다. ★여기에 `#[serde(rename)]` 을 달지 말 것★ — 그러면 두 표면이 같은 뜻을 다른
/// 철자로 말한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ThemeSource {
    /// 파일에 적힌 값을 그대로 적용했다.
    File,
    /// 파일을 못 써서 [`DEFAULT_THEME`] 으로 접었다.
    Fallback,
}

/// 한 자리에 적용할 값과 **그 값이 파일에서 온 것인지** — 창 하나 몫이거나 전역 몫이다
/// ([`LoadedSettings::for_window`] · [`LoadedSettings::global`]).
///
/// ★둘을 함께 내는 이유★: `theme` 만으로는 「파일에 dark 라고 적혀 있다」와 「네 값이 반려돼 dark 로
/// 접혔다」가 같아 보인다. 호출자가 그 둘을 못 가르면 편집이 먹었는지 확인할 방법이 없다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadedTheme {
    pub theme: UiTheme,
    pub source: ThemeSource,
}

/// [`load_settings`] 가 내놓는 것 — 파일 한 벌과 그 파일을 실제로 썼는지.
///
/// ★`source` 는 창별로 갈리지 않는다★ — 파일을 읽었으면 모든 창이 `File`, 못 읽었으면 모든 창이
/// `Fallback` 이다. 창 항목 하나가 반려된 것은 여기 안 실린다: 그 창이 받는 값은 여전히 **읽힌 파일에서 온**
/// 전역 값이고, 그 사실을 이 칸으로 물으면 세 번째 갈래가 생긴다(ADR-0166 이 막은 자리 · ADR-0167 이
/// 넓히지 않기로 한 자리). 반려는 로그가 진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSettings {
    settings: UiSettings,
    source: ThemeSource,
}

impl LoadedSettings {
    /// 창을 안 가리는 답 — `ui.refresh` 답장이 싣는 값.
    pub fn global(&self) -> LoadedTheme {
        LoadedTheme {
            theme: self.settings.global(),
            source: self.source,
        }
    }

    pub fn for_window(&self, window: &str) -> LoadedTheme {
        LoadedTheme {
            theme: self.settings.theme_for(window),
            source: self.source,
        }
    }

    pub fn payload_for(&self, window: &str) -> UiSettingsPayload {
        self.for_window(window).into()
    }
}

/// 프론트로 나가는 값 — 부팅 조회(`get_ui_settings`)와 `ui.refresh` 푸시가 **같은 모양**을 쓴다.
///
/// 필드 이름은 wire 계약이다(프론트가 이 이름으로 읽는다 — `src/theme/uiSettings.ts`).
/// `source` 는 그 한 struct 를 두 자리가 나눠 쓰는 덕에 두 경로에 함께 실린다(따로 배선하지 않았다).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UiSettingsPayload {
    pub theme: String,
    /// ★String 이 아니라 enum 을 그대로 싣는다★ — 문자열로 옮기면 그 자리에 철자를 손으로 적게 되고,
    /// 그건 wire 쪽 `ThemeOrigin` 과 어긋나도 아무것도 안 깨지는 네 번째 사본이 된다.
    pub source: ThemeSource,
}

impl From<LoadedTheme> for UiSettingsPayload {
    fn from(loaded: LoadedTheme) -> Self {
        UiSettingsPayload {
            theme: loaded.theme.as_wire().to_string(),
            source: loaded.source,
        }
    }
}

/// 설정 원문을 가져오는 seam — 파싱·기본값 판정을 파일 시스템 없이 세우는 자리(ADR-0012).
pub trait SettingsSource: Send + Sync {
    /// 파일 원문. **부재도 `Err` 다** — 호출자는 종류를 가르지 않고 전부 기본값으로 접는다.
    fn read(&self) -> std::io::Result<String>;

    /// 경고에 실을 출처 표시(경로). 어느 파일을 못 읽었는지가 안 보이면 데이터 폴더가 갈렸을 때
    /// (`ENGRAM_DATA_DIR` · 디버그/릴리즈 분기 — ADR-0024) 엉뚱한 파일을 고치며 헤맨다.
    fn origin(&self) -> String;
}

/// 운영 구현 — `<data_dir>/ui-settings.json`.
pub struct FileSource {
    path: PathBuf,
}

impl FileSource {
    /// 데몬·셸이 공유하는 데이터 폴더 안(ADR-0024/0029 — `default_data_dir`).
    pub fn in_data_dir() -> Self {
        Self::at(engram_dashboard_discovery::default_data_dir().join(UI_SETTINGS_FILE))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsSource for FileSource {
    fn read(&self) -> std::io::Result<String> {
        read_capped(std::fs::File::open(&self.path)?, MAX_SETTINGS_BYTES)
    }

    fn origin(&self) -> String {
        self.path.display().to_string()
    }
}

/// 원문 상한. 한 칸짜리 취향 파일이라 실제 크기는 수십 바이트다 — 이 값은 「사람이 손으로 늘려도 여기까진
/// 정상」의 선이지 예상 크기가 아니다.
pub const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// ★상한까지만 읽는다 — 읽고 나서 재지 않는다★.
///
/// 먼저 통째로 읽어 길이를 재면 상한 검사가 도착하기 전에 메모리가 먼저 바닥난다(밖의 에이전트가 쓰는
/// 파일이라 크기가 우리 손에 없다). 그러면 기본값 접기도 경고도 못 돌고 프로세스가 죽는다.
///
/// 상한 초과와 UTF-8 아님은 **둘 다 `Err`** 다 — 이 seam 의 계약은 「원문을 가져왔나」 하나이고, 둘 다
/// 원문을 못 가져온 것이다(호출자는 종류를 안 가른다).
pub fn read_capped(source: impl std::io::Read, cap: u64) -> std::io::Result<String> {
    use std::io::Read;

    let mut buf = Vec::new();
    // cap + 1 = 「넘었나」를 알 수 있는 최소치. 넘었어도 읽는 양은 여기서 멈춘다.
    source.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{cap} 바이트 상한을 넘었다"),
        ));
    }
    String::from_utf8(buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// 원문 → 값 한 벌. ★파일 시스템을 안 탄다★.
///
/// ★모르는 칸은 무시한다(의도 — 앞날을 위한 호환)★: 모르는 키를 반려하면 칸을 하나 더할 때마다 옛 셸이
/// 파일 전체를 거부한다. 빠뜨린 검증이 아니다.
///
/// `Err` = **파일 전체를 못 쓴다**(전역 칸이 없거나 못 쓸 값이다). 창 항목 하나가 못 쓸 것은 `Err` 가
/// 아니라 [`ParsedSettings::refused`] 로 나간다 — 무게가 다르다(모듈 헤더 「고장의 무게」).
///
/// ★오류·반려 문구에 원문을 그대로 싣지 않는다★ — 이 문구는 곧장 로그로 나가는데, 파일을 쓰는 것은 밖의
/// 에이전트라 그 안에 무엇이 들었는지 우리가 정하지 않는다([`describe_value`] · [`json_kind`]).
///
/// 오류는 로그 한 줄에 그대로 실릴 문구다 — 호출자가 종류로 분기하지 않는다(전부 기본값행).
pub fn parse_settings(text: &str) -> Result<ParsedSettings, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON 이 아니다: {e}"))?;
    let global = parse_global_theme(&doc)?;

    let mut windows = BTreeMap::new();
    let mut refused = Vec::new();
    let mut refused_total = 0usize;
    let mut refuse = |reason: String| {
        refused_total += 1;
        if refused.len() < MAX_REFUSED_DETAILS {
            refused.push(reason);
        }
    };
    match doc.get("windows") {
        None => {}
        Some(serde_json::Value::Object(entries)) => {
            for (label, raw) in entries {
                match raw.as_str().and_then(UiTheme::from_wire) {
                    Some(theme) => {
                        windows.insert(label.clone(), theme);
                    }
                    // ★창 이름도 게이트를 통과해야 실린다★ — 지도의 **키**도 밖의 에이전트가 쓴다.
                    None => refuse(format!(
                        "창 {} 의 테마 {}",
                        describe_value(label),
                        describe_theme_value(raw)
                    )),
                }
            }
        }
        Some(other) => refuse(format!(
            "`windows` 는 지도여야 한다(받은 것: {})",
            json_kind(other)
        )),
    }

    Ok(ParsedSettings {
        settings: UiSettings { global, windows },
        refused,
        refused_total,
    })
}

/// 전역 칸 하나 — 이것이 못 쓸 값이면 파일 전체가 못 쓸 것이다(창 항목을 접을 바닥이 없다).
fn parse_global_theme(doc: &serde_json::Value) -> Result<UiTheme, String> {
    let Some(raw) = doc.get("theme") else {
        return Err("`theme` 키가 없다".to_string());
    };
    let Some(name) = raw.as_str() else {
        // 값이 아니라 **종류**만 — 여기 걸리는 것은 객체·배열일 수 있고 그건 통째로 상한 크기다.
        return Err(format!(
            "`theme` 는 문자열이어야 한다(받은 것: {})",
            json_kind(raw)
        ));
    };
    UiTheme::from_wire(name).ok_or_else(|| {
        format!(
            "모르는 테마 이름 {} — 허용: dark, light, e-ink",
            describe_value(name)
        )
    })
}

/// 테마 자리에 온 JSON 값을 로그용으로 — 문자열이면 [`describe_value`], 아니면 종류만.
fn describe_theme_value(raw: &serde_json::Value) -> String {
    match raw.as_str() {
        Some(name) => describe_value(name),
        None => format!("<문자열이 아닌 {}>", json_kind(raw)),
    }
}

/// JSON 값의 **종류만**. 값 자체는 안 싣는다 — 로그로 샐 수 있고 [`MAX_SETTINGS_BYTES`] 까지 커질 수 있다.
fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// 로그에 실을 값을 만든다 — ★**모양으로 걸러서**, 테마 이름·창 label 처럼 생긴 것만 싣는다★.
///
/// 막는 것 둘: 파일에 들어온 값이 로그로 **새는** 것과, 상한(64KiB)까지 허용된 덩치가 창을 열 때마다·
/// refresh 때마다 로그로 **증폭**되는 것. 그런데 값을 아예 안 실으면 오타 진단(무엇을 잘못 적었나)이 죽는다.
/// 둘 중 하나를 고르는 대신 **모양 게이트**를 둔다: 진짜 테마 오타는 전부 통과하고
/// (`Dark`·`darkk`·`e-ink2`), 이메일·URL·토큰·붙여넣은 덩치는 charset 이나 길이에서 걸린다.
/// 걸린 값은 길이만 남긴다 — 「무엇이 들어왔나」 대신 「얼마나 큰 무언가가 들어왔나」.
///
/// ★대가는 알고 치른 것이다(사용자 결정 — 되살리지 마라)★: 테마 이름처럼 **생긴** 짧은 비밀은 그대로
/// 로그에 실린다(`companySecret42` 는 charset·길이를 다 통과한다). 그것까지 막으려면 값을 아예 안 실어야
/// 하고, 그러면 오타 진단이 죽는다 — 그 둘 중 진단을 택했다. 여기를 「구멍」으로 읽고 값을 지우지 말 것.
///
/// ★반환값은 이미 인용·이스케이프된 형태다 — 호출부에서 `{:?}` 를 다시 씌우지 말 것★. 그래서 통과분에
/// `{:?}`(Rust `str` Debug)를 여기서 씌운다: 그것이 `\n`·`\r`·ESC 를 escape 해 **로그 줄 쪼개기와 ANSI
/// 주입**을 막는 자리다. 게이트가 이미 그런 문자를 거르므로 오늘은 이중 방어지만, 게이트를 넓히는 순간
/// 이 한 겹이 유일한 방어가 된다.
///
/// ★잘라 싣는 방식을 쓰지 않는다(그 자리에 게이트를 뒀다)★ — 「앞머리 N 자만 남긴다」는 자르는 순서마다
/// 다른 구멍이 난다. **자르고 마스킹하면** 키 패턴이 반토막 나 정규식을 비켜 가고 그 반토막이 실린다.
/// **마스킹하고 자르면** 마스킹이 문자열을 줄여 뒤쪽 바이트를 앞머리 창으로 끌어올린다
/// (`"sk-proj-"+A*30+"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"` → `"***wJalrXUtnFEMI/K7MDENG/bPxRfiCY…"`).
/// 어느 순서든 어떤 입력에서는 샌다 — 그래서 길이로 자르는 대신 **모양으로 거른다**. 게이트를 통과한 값은
/// 이미 짧아서 자를 일이 없다.
///
/// 마스킹은 코어 공용 헬퍼를 **명시 호출**한다 — 자동 적용이 아니라 호출자 몫이라는 것이 그 헬퍼의 계약이다
/// (`docs/reference/logging-conventions.md` 「보안」 · ADR-0138). ★이 저장소의 유일한 호출처가 아니다★ —
/// `core/src/agent/transport/stdio.rs` 가 외부 프로세스 stderr 에 같은 방식으로 건다.
fn describe_value(raw: &str) -> String {
    if !looks_like_name(raw) {
        return format!(
            "<쓸 수 있는 이름 모양이 아닌 {}자 문자열>",
            raw.chars().count()
        );
    }
    // ★게이트를 통과한 값에도 마스킹은 남긴다★ — 길이·charset 을 다 만족하면서 키 모양인 것이 있다
    // (`AKIA` + 대문자 16자 = 20자 영숫자). 게이트를 넓히면 이 겹이 먼저 받아 준다.
    format!("{:?}", engram_dashboard_core::logging::mask_secrets(raw))
}

/// 테마 이름·창 label 처럼 **생겼나** — ASCII 영숫자와 하이픈만, 그리고 짧을 것.
///
/// ★`_`·`.`·`@`·`/`·`:`·공백을 일부러 뺐다★: 그것들이 이메일·URL·경로·문장을 갈라내는 실질적인 칸막이다
/// (`e_ink` 같은 오타는 그 대가로 길이만 남지만, 그건 통과시켰을 때 이메일이 함께 통과하는 것보다 싸다).
/// 대문자를 넣은 이유는 `Dark` 가 **가장 흔한 오타**이기 때문이다 — `from_wire` 가 대소문자를 가리므로
/// 소문자만 받으면 정작 제일 자주 나는 실수를 못 보여준다.
///
/// 이 앱이 실제로 쓰는 창 label(`main`·`agent-tree`·`slot-popup-N`)은 전부 이 charset 안이다.
fn looks_like_name(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= MAX_LOGGED_VALUE_BYTES
        && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 로그에 실을 수 있는 값의 최대 길이(바이트 = 위 charset 에서는 문자 수와 같다). 테마 이름은 다섯 자라
/// 오타 진단에는 남아돈다.
///
/// ★이 선이 무엇을 보장하나 — 성질로 읽을 것, 예외를 세지 말 것★: **긴** 것을 자를 뿐 「키 모양을 전부
/// 거른다」가 아니다. 이 안에 들어오는 짧은 키 모양이 실제로 있고(`AKIA`+16 = 20자 · `sk-`+20~21 = 23~24자
/// … 세어 봤자 정규식이 늘면 목록이 낡는다) 그것들은 마스킹 겹이 받는다([`describe_value`]).
/// 두 겹의 분담이 그것이다 — **길이·charset 은 상한, 마스킹은 통과분의 방어.**
const MAX_LOGGED_VALUE_BYTES: usize = 24;

/// seam 위 — ★실패는 전부 [`DEFAULT_THEME`] 로 접는다★(패닉도 전파도 없다).
///
/// ★레벨이 셋으로 갈린다★(정본 = `docs/reference/logging-conventions.md`):
/// - **파일 없음 = `debug`.** 아무도 아직 안 만든 상태가 신규 설치의 **정상**이다 — 기본 레벨(warn)에서
///   창을 열 때마다 경고가 뜨면 「릴리스 평상시 거의 무출력」이 깨진다. 무음(명부 로더가 그렇다 —
///   `core/src/persistence/mod.rs`) 대신 `debug` 를 고른 것은 **찾아보러 왔을 때 볼 것이 있게** 하려는 것뿐이다:
///   `RUST_LOG=debug` 로 다시 띄우면 우리가 어느 경로를 봤는지가 이 줄에 실린다(데이터 폴더는
///   `ENGRAM_DATA_DIR`·디버그/릴리즈 분기로 갈릴 수 있다 — ADR-0024).
///   ★기본 레벨에서는 아무 신호도 못 준다★ — 폴더가 어긋난 상태는 기본 설정에서 「파일 없음」과 여전히
///   구별되지 않는다. 그 갭을 메우려면 기동 시 결정된 데이터 폴더를 한 줄 남기는 별개 변경이 필요하다.
/// - **그 밖의 읽기 실패 = `warn`.** 비정상이지만 안전 폴백.
/// - **파싱 실패 = `error`.** 손상 신호(그 문서의 명시 분기).
///
/// ★레벨 자체는 무검증이다★ — 이 스위트에 tracing subscriber 하네스가 없어 반환값만 덮인다. 셋 중 하나를
/// 한 낱말 고쳐 뒤바꿔도 테스트는 초록이다(알려진 갭 · 검증하려면 subscriber 하네스가 먼저 서야 한다).
///
/// 성공도 남긴다 — 파일 IO 는 외부 경계라 계측 의무가 있고(그 문서 「계측 의무」), 그 한 줄이 없으면
/// 「refresh 가 dark 를 읽었다」와 「refresh 가 안 돌았다 / 알림이 이 창에 안 닿았다」가 로그에서 같아 보인다.
pub fn load_settings(source: &dyn SettingsSource) -> LoadedSettings {
    let text = match source.read() {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                module = "ui_settings",
                source = %source.origin(),
                fallback = DEFAULT_THEME.as_wire(),
                "UI 설정 파일이 없어 기본 테마로 둔다"
            );
            return folded();
        }
        Err(e) => {
            tracing::warn!(
                module = "ui_settings",
                source = %source.origin(),
                fallback = DEFAULT_THEME.as_wire(),
                "UI 설정을 못 읽어 기본 테마로 둔다: {e}"
            );
            return folded();
        }
    };
    match parse_settings(&text) {
        Ok(parsed) => {
            if parsed.refused_total > 0 {
                // ★파일이 못 쓸 것이 아니라 **항목 하나가** 못 쓸 것이다★ — 그래서 error 가 아니라 warn 이고,
                //   그 창은 전역 값으로 돈다. 사유 목록은 상한까지만이라 총수를 따로 싣는다.
                tracing::warn!(
                    module = "ui_settings",
                    source = %source.origin(),
                    refused = parsed.refused_total,
                    shown = ?parsed.refused,
                    fallback = parsed.settings.global().as_wire(),
                    "쓸 수 없는 창별 테마 항목이 있어 그 창은 전역 테마로 둔다"
                );
            }
            tracing::debug!(
                module = "ui_settings",
                source = %source.origin(),
                theme = parsed.settings.global().as_wire(),
                windows = parsed.settings.windows.len(),
                "UI 설정에서 테마를 읽었다"
            );
            LoadedSettings {
                settings: parsed.settings,
                source: ThemeSource::File,
            }
        }
        Err(reason) => {
            // 레벨은 손상 신호(error)지만 ★사본은 남기지 않는다★ — 되살릴 것이 없다(모듈 헤더).
            tracing::error!(
                module = "ui_settings",
                source = %source.origin(),
                fallback = DEFAULT_THEME.as_wire(),
                "UI 설정이 쓸 수 없는 모양이라 기본 테마로 둔다: {reason}"
            );
            folded()
        }
    }
}

/// 접힌 결과 하나 — 세 실패 갈래가 같은 값을 낸다는 것을 한 자리에 둔다(갈래를 늘리는 것이 아니다).
///
/// ★창별 지도도 함께 비운다★ — 파일을 못 썼으면 창별 항목도 못 쓴 것이다(그 항목은 그 파일에서 온다).
fn folded() -> LoadedSettings {
    LoadedSettings {
        settings: UiSettings {
            global: DEFAULT_THEME,
            windows: BTreeMap::new(),
        },
        source: ThemeSource::Fallback,
    }
}

// ── 부팅 쓸기 — 죽은 창의 항목 지우기 ────────────────────────────────────────

/// [`sweep_dead_windows`] 가 하고 온 일 — 로그가 읽고 하네스가 잰다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepOutcome {
    /// 지울 항목이 없어 ★파일을 다시 쓰지 않았다★ — 평상시 부팅이 이 길이다.
    Untouched,
    /// 지우고 다시 썼다.
    Swept { removed: Vec<String> },
    /// 원문을 못 가져왔거나 JSON 이 아니라 손대지 않았다. ★실패 종류를 가르지 않는다★ — 어느 쪽이든 이
    /// 함수가 할 일은 「아무것도 안 한다」 하나다([`load_settings`] 와 같은 분담).
    Skipped,
    /// 지울 것은 있었는데 쓰기가 실패했다 — 파일은 옛 상태 그대로이고 앱은 계속 뜬다.
    NotWritten(String),
}

/// 설정 파일에서 **죽은 창의 항목**을 지운다 — ★부팅에서 한 번만 부른다★.
///
/// `declared` = 앱 설정이 선언한 창 label 전량(오늘 `main`·`agent-tree`). ★손으로 적지 말 것★ — 뽑는
/// 자리는 `commands::settings::declared_window_labels` 이고 재료는 `tauri.conf.json` 이다.
///
/// ## ★왜 부팅에서만인가 — 생사 확인을 덧대지 마라★
/// 레이아웃은 디스크에 영속되지 않는다(`layout::manager`). 그래서 **부팅 순간 팝아웃은 하나도 없고**, 그때
/// 파일에 있는 비-선언 label 은 생사를 물을 것도 없이 **정의상 전부 죽은 것**이다 — 판정이 공짜인 유일한
/// 시점이 여기다. 여기에 「살아 있나」를 묻는 확인을 덧대거나 이 호출을 `ui.refresh` 로 옮기면, 곧 열릴 창의
/// 항목을 밖의 에이전트가 미리 써 둔 경우 **창이 도착하기 전에 그 항목이 지워진다**(ADR-0167 이 그 경합을
/// 이유로 기각한 대안 그대로다). 선언된 창을 면제하는 것이 남은 위험까지 없앤다 — 그 창들은 항상 있으므로
/// 「아직 안 떴을 수 있다」를 물을 일이 없다.
///
/// ## ★지울 것이 없으면 안 쓴다★
/// 평상시 부팅은 읽기로 끝난다. 매번 쓰면 밖의 에이전트 편집기와 다투는 창이 부팅마다 열린다(모듈 헤더
/// 「쓰는 자리는 하나」).
///
/// ## ★지우기만 한다 — 고치지 않는다★
/// 못 쓸 값·모르는 칸·JSON 이 아닌 원문은 전부 **그대로 둔다**. 읽기가 모르는 칸을 무시하는 것이 앞날의 칸을
/// 받아 주는 장치인데([`parse_settings`]), 쓸기가 문서를 자기가 아는 모양으로 다시 그리면 그 호환이 죽는다.
/// 깨진 파일을 다시 쓰지 않는 이유도 같다 — 사람이 고치던 원문이 사라진다.
///
/// ★남는 키의 **순서**는 보장하지 않는다★ — `serde_json` 이 `preserve_order` 없이 붙어 있어 `Value` 의
/// 지도가 `BTreeMap` 이고, 실제로 쓰는 부팅에서 남는 키가 사전순으로 다시 적힌다(값과 키 집합은 그대로).
/// 순서까지 지키려면 그 feature 를 켜야 하는데 그건 워크스페이스 전체의 `Value` 동작을 바꾼다.
///
/// 쓰는 자리는 호출자가 넣는다 — 이 모듈에 파일 쓰기를 들이지 않는다([`deliver_per_window`] 와 같은 분담).
/// 운영 구현은 [`write_atomic`].
// ADR-0167
pub fn sweep_dead_windows<W>(
    source: &dyn SettingsSource,
    declared: &BTreeSet<String>,
    write: W,
) -> SweepOutcome
where
    W: FnOnce(&str) -> std::io::Result<()>,
{
    let Ok(text) = source.read() else {
        // ★여기서 사유를 로그로 올리지 않는다★ — 곧 첫 창의 부팅 조회가 같은 파일을 같은 이유로 못 읽고,
        //   그때 `load_settings` 가 레벨까지 갈라 남긴다. 여기서 한 번 더 쓰면 같은 사실이 두 줄이 된다.
        return SweepOutcome::Skipped;
    };
    let swept = match plan_sweep(&text, declared) {
        Ok(None) => return SweepOutcome::Untouched,
        Ok(Some(swept)) => swept,
        Err(reason) => {
            // 손상 신호(error)를 여기서 내지 않는 것도 같은 이유다 — `load_settings` 가 곧 그 레벨로 낸다.
            tracing::debug!(
                module = "ui_settings",
                source = %source.origin(),
                "UI 설정이 쓸 수 없는 모양이라 죽은 창 항목을 손대지 않는다: {reason}"
            );
            return SweepOutcome::Skipped;
        }
    };

    // label 도 밖의 에이전트가 쓰는 값이라 로그에 그대로 싣지 않는다(`describe_value` · 상한은
    // `MAX_REFUSED_DETAILS` 의 증폭 사유와 같다 — 이 줄은 부팅에 한 번이지만 항목 수는 파일이 정한다).
    let shown: Vec<String> = swept
        .removed
        .iter()
        .take(MAX_REFUSED_DETAILS)
        .map(|label| describe_value(label))
        .collect();
    match write(&swept.text) {
        Ok(()) => {
            tracing::info!(
                module = "ui_settings",
                source = %source.origin(),
                removed = swept.removed.len(),
                shown = ?shown,
                "재시작을 넘긴 창별 테마 항목을 지웠다"
            );
            SweepOutcome::Swept {
                removed: swept.removed,
            }
        }
        Err(e) => {
            tracing::warn!(
                module = "ui_settings",
                source = %source.origin(),
                removed = swept.removed.len(),
                shown = ?shown,
                "죽은 창별 테마 항목을 못 지웠다(앱은 계속 뜬다): {e}"
            );
            SweepOutcome::NotWritten(e.to_string())
        }
    }
}

/// 지울 것을 뺀 새 원문과 지운 label — [`plan_sweep`] 의 산출물.
struct SweptDocument {
    text: String,
    removed: Vec<String>,
}

/// 원문 → 지울 것을 뺀 원문. ★파일 시스템을 안 탄다★.
///
/// `Ok(None)` = 지울 것이 없다(새 원문을 아예 안 만든다 — 그래야 호출자가 「안 쓴다」를 고를 수 있다).
/// `Err` = 원문이 JSON 이 아니다.
fn plan_sweep(text: &str, declared: &BTreeSet<String>) -> Result<Option<SweptDocument>, String> {
    let mut doc: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON 이 아니다: {e}"))?;
    // `windows` 가 없거나 지도가 아니면 지울 항목이 없다 — 모양 반려는 읽기 쪽 몫이고 여기서 고쳐 쓰지 않는다.
    let Some(entries) = doc
        .get_mut("windows")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(None);
    };
    let removed: Vec<String> = entries
        .keys()
        .filter(|label| !declared.contains(label.as_str()))
        .cloned()
        .collect();
    if removed.is_empty() {
        return Ok(None);
    }
    for label in &removed {
        entries.remove(label);
    }

    let mut text =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("다시 쓸 수 없다: {e}"))?;
    // 사람이 손으로도 고치는 파일이라 줄 끝을 남긴다.
    text.push('\n');
    Ok(Some(SweptDocument { text, removed }))
}

/// 임시 파일에 쓰고 **rename 으로 갈아끼운다** — 쓰다 죽어도 반쪽 파일이 안 남는다.
///
/// ★임시 파일은 같은 폴더에 만든다★ — rename 이 갈아끼우기로 도는 것은 같은 볼륨 안에서다. 이름에 pid 를
/// 붙이는 것은 같은 폴더를 보는 다른 프로세스와 임시 이름이 겹치면 서로의 반쪽을 rename 하기 때문이다.
///
/// 실패하면 임시 파일을 치우고 원본을 그대로 둔다 — 안 치우면 데이터 폴더에 쓰레기가 쌓인다.
// ADR-0167
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let invalid = |what: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{what} 를 못 고르는 경로"),
        )
    };
    let dir = path.parent().ok_or_else(|| invalid("부모 폴더"))?;
    let mut name = path
        .file_name()
        .ok_or_else(|| invalid("파일 이름"))?
        .to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    let tmp = dir.join(name);

    let staged = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        // ★flush 로는 부족하다★ — 그건 프로세스 버퍼만 비운다. rename 이 가리키게 될 내용이 실제로
        //   디스크에 있어야 「반쪽이 안 남는다」가 성립한다.
        file.sync_all()
    })();
    let outcome = staged.and_then(|()| std::fs::rename(&tmp, path));
    if outcome.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

/// 창마다 **그 창의 값**을 보낸다 — 보내는 자리는 호출자가 넣는다(Tauri 를 이 모듈에 들이지 않는다).
///
/// ★한 봉투를 전 창에 뿌리지 않는다★: 창마다 값이 다를 수 있으므로 목적지를 지목해 보내야 한다. 받는 쪽도
/// 자기 label 로 구독해야 한다 — Tauri 는 `Any` 로 등록된 리스너를 **필터와 무관하게 전부** 깨우고 JS
/// `listen()` 의 기본 타깃이 그 `Any` 다(`view_commands::TauriViewDispatch` 가 같은 쌍을 진다).
///
/// `Err` 가 되는 자리 둘, 둘 다 「알림이 안 닿았다」다(ADR-0166 결정 6):
/// - **보낼 창이 없다** — 이 명령이 하는 일은 알림뿐이라 아무 창도 없으면 아무 일도 안 일어난 것이다.
/// - **한 창이라도 못 받았다** — ★그래도 남은 창까지 다 시도하고 나서 실패로 답한다★. 죽은 창 하나에서
///   멈추면 나머지 창이 옛 값으로 남고, 그 답장은 실패라 **어느 창이 갱신됐는지 아무도 모른다.**
///
/// 실패 문구도 [`MAX_REFUSED_DETAILS`] 까지만 펼친다(창이 많을 때의 증폭 — 그 상수의 사유와 같다).
pub fn deliver_per_window<E>(
    loaded: &LoadedSettings,
    windows: &[String],
    mut emit: E,
) -> Result<(), String>
where
    E: FnMut(&str, UiSettingsPayload) -> Result<(), String>,
{
    if windows.is_empty() {
        return Err("알림을 받을 창이 하나도 없다".to_string());
    }

    let mut failed = 0usize;
    let mut detail: Vec<String> = Vec::new();
    for label in windows {
        if let Err(e) = emit(label, loaded.payload_for(label)) {
            failed += 1;
            if detail.len() < MAX_REFUSED_DETAILS {
                detail.push(format!("{label}: {e}"));
            }
        }
    }
    if failed > 0 {
        return Err(format!(
            "{failed}/{} 창에 못 보냈다 — {}",
            windows.len(),
            detail.join(" · ")
        ));
    }
    Ok(())
}

/// `ui.refresh` 가 잡는 실물 — 파일을 다시 읽어 화면에 밀어 넣는다(조립 때 주입, ADR-0155 규칙 T-1).
///
/// 돌려주는 값에 **파일에서 온 것인지**가 함께 실린다([`LoadedTheme`]) — 호출자가 「내 편집이 먹었나」를
/// 답의 `theme` 만으로는 못 가르기 때문이다.
///
/// ★실패하는 자리는 하나뿐이다 — **알림을 못 보낸 것**★.
///
/// 읽기·파싱 실패는 [`load_settings`] 이 기본값으로 접으므로 `Err` 로 나가지 않는다(그건 `Fallback` 이다).
/// 하지만 알림을 못 보내면 값이 **어느 창에도 안 닿았다** — 그 경우 `Ok` 를 돌려주면 호출자는 화면이 바뀐
/// 줄 안다. `source` 는 「값이 어디서 왔나」를 말하지 「화면이 바뀌었나」를 말하지 않으므로, 그 구분을
/// enum 에 세 번째 값으로 넣지 않고 **성공/실패로** 가른다.
///
/// ★`LayoutEvents` 는 알림 실패를 삼킨다 — 여기는 안 삼킨다★. 저쪽은 레이아웃 **변형이 이미 일어난 뒤**라
/// 알림 유실이 변형을 되돌릴 사유가 아니고 프론트가 read-only pull 로 복구한다. 이 명령은 반대로 **알림이
/// 곧 그 명령의 결과 전부**다(다른 부수효과가 없다) — 못 보냈으면 아무 일도 안 일어난 것이다.
pub trait UiSettingsRefresh: Send + Sync {
    /// 다시 읽어 적용한 값. `Err` = 알림을 못 보냈다(값은 정해졌으나 화면에 안 닿았다).
    fn refresh(&self) -> Result<LoadedTheme, String>;
}
