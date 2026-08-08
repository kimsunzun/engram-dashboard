//! saturation-pilot CLI 인자 파서 (ADR-0090 Stage 2 파일럿).
//!
//! ## 역할
//! `saturation_pilot` bin 의 커맨드라인을 파싱해 검증된 `PilotConfig` 로 만든다. clap 등 인자 파싱
//! 크레이트를 **추가하지 않는다**(ADR-0090 제약: no new deps) — 플래그 종류가 적고 손조립으로 충분.
//!
//! ## 핵심 불변식 (ADR-0090)
//! - **하드 캡 클램프**: `fill_target_tokens` 는 파싱 단계에서 하드 클램프된다. 사용자가 상한보다
//!   크게 줘도 조용히 줄인다(폭주 실험 방지 — 런타임 캡과 이중 방어).
//! - **미지 플래그 = 거부**: 모르는 플래그는 `ParseError::Unknown` 으로 거부하고 호출자가 exit 2 로
//!   끝낸다(오타로 인한 잘못된 파라미터 침묵 실행 방지).
// ADR-0090

use std::path::PathBuf;

/// fill 목표 토큰 하드 클램프(ADR-0090 불변식).
pub const FILL_TARGET_CLAMP: u64 = 180_000;

/// ★doc_chars 하드 클램프(ADR-0090 불변식 — finding 7)★: 한 문서가 fill-target(문자 환산 상한)을
///   넘길 이유가 없으므로 넉넉한 고정 천장을 둔다. 초과 값은 파싱 단계에서 조용히 줄인다.
pub const DOC_CHARS_CLAMP: usize = 200_000;

/// 파일럿 런 설정 — 전 파라미터 + seed. JSONL 헤더에 그대로 직렬화(재현성 핀 = ADR-0088 d5a).
#[derive(Debug, Clone, PartialEq)]
pub struct PilotConfig {
    /// 반복 런 수(각 런 = 새 에이전트 1스폰).
    pub runs: u32,
    /// 포화 목표 토큰(usage 의 context 크기 기준).
    pub fill_target_tokens: u64,
    /// 주입 시점(fill 진행 분율).
    pub inject_at: Vec<f64>,
    /// 주입 후 프로브까지 대기할 fill 턴 수.
    pub probe_gap_turns: u32,
    /// doc 1개의 근사 문자 수.
    pub doc_chars: usize,
    pub model: String,
    /// JSONL 출력 디렉토리. None 이면 driver 가 기본(`target/experiments/pilot-<UTC>/`) 생성.
    pub out: Option<PathBuf>,
    /// 결정적 필러 seed(고정 기본값 — 재현성).
    pub seed: u64,
    /// 워크스페이스 임시 디렉토리를 런 종료 후 보존할지(디버깅용).
    pub keep_workspace: bool,
}

impl Default for PilotConfig {
    fn default() -> Self {
        Self {
            runs: 1,
            fill_target_tokens: 150_000,
            inject_at: vec![0.5, 0.9],
            probe_gap_turns: 6,
            doc_chars: 12_000,
            model: "sonnet".to_string(),
            out: None,
            seed: 0x5EED_0000_0000_0001,
            keep_workspace: false,
        }
    }
}

/// 파싱 실패 종류. Help 는 정상 종료(exit 0), Unknown/Invalid 는 exit 2.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    Help,
    Unknown(String),
    Invalid(String),
}

pub fn usage() -> String {
    "\
saturation-pilot — ADR-0090 Stage 2 컨텍스트 포화 파일럿 드라이버

USAGE:
  saturation-pilot [OPTIONS]

OPTIONS:
  --runs <N>                  반복 런 수 (기본 1)
  --fill-target-tokens <N>    포화 목표 토큰 (기본 150000, 하드 클램프 180000)
  --inject-at <\"0.5,0.9\">      주입 시점 fill 분율 CSV (기본 \"0.5,0.9\")
  --probe-gap-turns <N>       주입 후 프로브까지 fill 턴 수 (기본 6)
  --doc-chars <N>             doc 1개 근사 문자 수 (기본 12000)
  --model <NAME>              모델 핀 (기본 sonnet)
  --out <DIR>                 JSONL 출력 디렉토리 (기본 target/experiments/pilot-<UTC>)
  --seed <U64>                필러 seed (기본 고정값)
  --keep-workspace            런 종료 후 임시 워크스페이스 보존
  -h, --help                  이 도움말
"
    .to_string()
}

/// argv(프로그램명 제외)를 파싱한다. 값이 붙는 플래그는 `--flag value` 형식(등호 `--flag=value` 도 허용).
pub fn parse_args<I, S>(args: I) -> Result<PilotConfig, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut cfg = PilotConfig::default();
    let mut tokens: Vec<String> = Vec::new();
    for a in args {
        let a = a.as_ref();
        if let Some((flag, val)) = a.strip_prefix("--").and_then(|body| {
            body.split_once('=')
                .map(|(f, v)| (format!("--{f}"), v.to_string()))
        }) {
            tokens.push(flag);
            tokens.push(val);
        } else {
            tokens.push(a.to_string());
        }
    }

    let mut it = tokens.into_iter().peekable();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "-h" | "--help" => return Err(ParseError::Help),
            "--keep-workspace" => cfg.keep_workspace = true,
            "--runs" => cfg.runs = take_parse(&mut it, "--runs")?,
            "--fill-target-tokens" => {
                let raw: u64 = take_parse(&mut it, "--fill-target-tokens")?;
                cfg.fill_target_tokens = raw.min(FILL_TARGET_CLAMP);
            }
            "--inject-at" => {
                let raw = take_value(&mut it, "--inject-at")?;
                cfg.inject_at = parse_inject_at(&raw)?;
            }
            "--probe-gap-turns" => cfg.probe_gap_turns = take_parse(&mut it, "--probe-gap-turns")?,
            "--doc-chars" => {
                let raw: usize = take_parse(&mut it, "--doc-chars")?;
                cfg.doc_chars = raw.min(DOC_CHARS_CLAMP);
            }
            "--model" => cfg.model = take_value(&mut it, "--model")?,
            "--out" => cfg.out = Some(PathBuf::from(take_value(&mut it, "--out")?)),
            "--seed" => cfg.seed = take_parse(&mut it, "--seed")?,
            other => return Err(ParseError::Unknown(other.to_string())),
        }
    }
    Ok(cfg)
}

/// ★finding 15 fix★: `--out --keep-workspace` 처럼 값 자리에 또 다른 플래그가 오면, 이전엔 다음 플래그를
///   값으로 삼켜(out="--keep-workspace") 오타를 침묵 실행했다. (경로가 `--` 로 시작하는 병리적 케이스는
///   지원 대상 아님 — 실험 인자엔 그런 값이 없다.)
fn take_value<I: Iterator<Item = String>>(
    it: &mut std::iter::Peekable<I>,
    flag: &str,
) -> Result<String, ParseError> {
    match it.next() {
        None => Err(ParseError::Invalid(format!("{flag} 는 값이 필요합니다"))),
        Some(v) if v.starts_with("--") => Err(ParseError::Invalid(format!(
            "{flag} 값 누락 — 값 자리에 플래그가 왔습니다: {v:?}"
        ))),
        Some(v) => Ok(v),
    }
}

fn take_parse<I: Iterator<Item = String>, T: std::str::FromStr>(
    it: &mut std::iter::Peekable<I>,
    flag: &str,
) -> Result<T, ParseError> {
    let v = take_value(it, flag)?;
    v.parse::<T>()
        .map_err(|_| ParseError::Invalid(format!("{flag} 값 파싱 실패: {v:?}")))
}

/// 각 원소는 (0,1] 범위여야 한다 — 0 은 fill 전이라 무의미, 1 은 포화 도달점. 빈 문자열은 빈
/// 목록(주입 없음).
pub fn parse_inject_at(raw: &str) -> Result<Vec<f64>, ParseError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in raw.split(',') {
        let p = part.trim();
        let f: f64 = p
            .parse()
            .map_err(|_| ParseError::Invalid(format!("--inject-at 원소 파싱 실패: {p:?}")))?;
        if !(f > 0.0 && f <= 1.0) {
            return Err(ParseError::Invalid(format!(
                "--inject-at 원소는 (0,1] 범위여야: {f}"
            )));
        }
        out.push(f);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = parse_args(Vec::<String>::new()).unwrap();
        assert_eq!(cfg, PilotConfig::default());
        assert_eq!(cfg.runs, 1);
        assert_eq!(cfg.fill_target_tokens, 150_000);
        assert_eq!(cfg.inject_at, vec![0.5, 0.9]);
        assert_eq!(cfg.model, "sonnet");
    }

    #[test]
    fn parses_all_flags_space_form() {
        let cfg = parse_args([
            "--runs",
            "3",
            "--fill-target-tokens",
            "100000",
            "--inject-at",
            "0.3,0.7",
            "--probe-gap-turns",
            "4",
            "--doc-chars",
            "8000",
            "--model",
            "haiku",
            "--out",
            "C:/tmp/x",
            "--seed",
            "42",
            "--keep-workspace",
        ])
        .unwrap();
        assert_eq!(cfg.runs, 3);
        assert_eq!(cfg.fill_target_tokens, 100_000);
        assert_eq!(cfg.inject_at, vec![0.3, 0.7]);
        assert_eq!(cfg.probe_gap_turns, 4);
        assert_eq!(cfg.doc_chars, 8000);
        assert_eq!(cfg.model, "haiku");
        assert_eq!(cfg.out, Some(PathBuf::from("C:/tmp/x")));
        assert_eq!(cfg.seed, 42);
        assert!(cfg.keep_workspace);
    }

    #[test]
    fn parses_equals_form() {
        let cfg = parse_args(["--runs=5", "--model=opus"]).unwrap();
        assert_eq!(cfg.runs, 5);
        assert_eq!(cfg.model, "opus");
    }

    #[test]
    fn fill_target_is_hard_clamped() {
        let cfg = parse_args(["--fill-target-tokens", "999999"]).unwrap();
        assert_eq!(cfg.fill_target_tokens, FILL_TARGET_CLAMP);
    }

    #[test]
    fn doc_chars_is_hard_clamped() {
        let huge = usize::MAX.to_string();
        let cfg = parse_args(["--doc-chars", &huge]).unwrap();
        assert_eq!(cfg.doc_chars, DOC_CHARS_CLAMP);
        let cfg2 = parse_args(["--doc-chars", "8000"]).unwrap();
        assert_eq!(cfg2.doc_chars, 8000);
    }

    #[test]
    fn string_flag_rejects_next_flag_as_value() {
        assert!(matches!(
            parse_args(["--out", "--keep-workspace"]).unwrap_err(),
            ParseError::Invalid(_)
        ));
        assert!(matches!(
            parse_args(["--model", "--seed"]).unwrap_err(),
            ParseError::Invalid(_)
        ));
        let cfg = parse_args(["--out", "C:/tmp/x", "--keep-workspace"]).unwrap();
        assert_eq!(cfg.out, Some(PathBuf::from("C:/tmp/x")));
        assert!(cfg.keep_workspace);
    }

    #[test]
    fn unknown_flag_is_rejected() {
        let err = parse_args(["--bogus"]).unwrap_err();
        assert_eq!(err, ParseError::Unknown("--bogus".to_string()));
    }

    #[test]
    fn help_flag_returns_help() {
        assert_eq!(parse_args(["--help"]).unwrap_err(), ParseError::Help);
        assert_eq!(parse_args(["-h"]).unwrap_err(), ParseError::Help);
    }

    #[test]
    fn missing_value_is_invalid() {
        assert!(matches!(
            parse_args(["--runs"]).unwrap_err(),
            ParseError::Invalid(_)
        ));
    }

    #[test]
    fn bad_number_is_invalid() {
        assert!(matches!(
            parse_args(["--runs", "abc"]).unwrap_err(),
            ParseError::Invalid(_)
        ));
    }

    #[test]
    fn inject_at_empty_is_no_injections() {
        assert_eq!(parse_inject_at("").unwrap(), Vec::<f64>::new());
        assert_eq!(parse_inject_at("   ").unwrap(), Vec::<f64>::new());
    }

    #[test]
    fn inject_at_out_of_range_is_invalid() {
        assert!(matches!(
            parse_inject_at("1.5").unwrap_err(),
            ParseError::Invalid(_)
        ));
        assert!(matches!(
            parse_inject_at("0").unwrap_err(),
            ParseError::Invalid(_)
        ));
        assert_eq!(parse_inject_at("1.0").unwrap(), vec![1.0]);
    }

    #[test]
    fn inject_at_single_value() {
        assert_eq!(parse_inject_at("0.5").unwrap(), vec![0.5]);
    }
}
