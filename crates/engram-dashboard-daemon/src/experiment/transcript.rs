//! 트랜스크립트 탭 — claude CLI 가 남기는 raw 세션 JSONL 을 **측정용**으로만 읽는 순수 파서 (ADR-0090).
//!
//! ## ★위상(정직 범위) — 이건 제품 기능이 아니라 실험 측정 탭이다★
//! 코어 decoder(`ClaudeStreamDecoder`)는 `cache_creation_input_tokens`/`cache_read_input_tokens`(실
//! 컨텍스트 크기가 사는 곳)·`system/init`(모델 id)·compact 관련 raw 라인을 **버린다**. 코어는 무수정
//! 대상이라(ADR-0090 제약) 이 정보를 스폰 경로에서 못 얻는다. 우회: **우리가 세션 id 를 통제**하므로
//! (`--session-id`, ADR-0008) claude CLI 가 `~/.claude/projects/<munged-cwd>/<session-id>.jsonl` 에 남기는
//! raw 트랜스크립트를 **best-effort 로** 읽어 측정치를 보강한다.
//!
//! ★ADR-0008 경계 — 추적 파일 위에 제품 기능을 짓지 않는다★: 이 트랜스크립트는 **실험 측정 탭 전용**이다.
//! ADR-0008 은 "복원 정확성은 통제-sid 에만 의존, 추적 파일은 best-effort — 이걸로 기능 확장 금지" 를
//! 못박는다. 그래서 이 모듈은 파일 부재/파싱 실패에 **절대 하네스를 실패시키지 않는다**(best-effort).
// ADR-0090

use std::path::{Path, PathBuf};

use super::record::{event_type_key, line_mentions_compact, parse_init_model};

/// 한 assistant 턴의 **실 컨텍스트 footprint** 토큰. claude usage 의 `input_tokens` 만 보면 그 턴의
/// 증분 입력이라 누적 컨텍스트를 반영 못 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealUsage {
    /// 비-캐시 입력 토큰(그 턴에 새로 보낸 것).
    pub input_tokens: u64,
    /// 캐시 생성 입력 토큰(이번에 캐시에 쓴 프롬프트 — 컨텍스트의 일부).
    pub cache_creation_input_tokens: u64,
    /// 캐시 읽기 입력 토큰(이전 컨텍스트 재사용분 — 컨텍스트의 대부분).
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl RealUsage {
    /// 실 컨텍스트 footprint = input + cache_creation + cache_read. 캐시 두 항이 진짜 컨텍스트 크기의
    /// 대부분이라, 이 합이 "지금 모델이 보고 있는 컨텍스트 토큰 수" 의 ground truth 근사다.
    pub fn context_footprint(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptSummary {
    pub resolved_model: Option<String>,
    /// **assistant 메시지 per-turn footprint 만** 담는다(등장 순서 = 턴 순서). result 라인의 집계
    ///   usage 는 여기 섞지 않는다.
    pub real_usage_series: Vec<RealUsage>,
    /// result 라인의 최상위 집계 usage(마지막 것) — 진단·기록용.
    pub aggregate_result_usage: Option<RealUsage>,
    /// raw event-type/subtype 히스토그램(record::event_type_key 로 산출).
    pub event_histogram: std::collections::BTreeMap<String, u64>,
    /// compact/summary 관련으로 걸린 라인 verbatim(캡 적용 전 — 호출자가 record::cap_response 로 캡).
    pub compact_marker_lines: Vec<String>,
    /// 파싱한 총 라인 수(비어 있지 않은).
    pub total_lines: usize,
}

/// ★cwd-munging 미하드코딩★: claude 는 cwd 를 munge 한 디렉토리명 아래에 트랜스크립트를 둔다(예:
/// `C--Users-x-proj`). 그 규칙은 버전에 따라 바뀔 수 있어 **재귀 검색**으로 파일명만 매칭한다 — munging
/// 규칙 변화에 무관하게 흡수.
pub fn locate_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = claude_projects_dir()?;
    let target = format!("{session_id}.jsonl");
    find_file_recursive(&projects, &target, 0)
}

fn claude_projects_dir() -> Option<PathBuf> {
    // std env 로 홈을 찾는다 — dirs crate 미도입(no-new-deps).
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    let dir = home.join(".claude").join("projects");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// 깊이 상한은 폭주 방지 — claude 구조는 projects/<munged>/<sid>.jsonl 로 얕아 8 이면 충분하다.
fn find_file_recursive(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // 대소문자 정확 매칭 — sid 는 소문자 uuid.
        if path.file_name().and_then(|n| n.to_str()) == Some(target) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_file_recursive(&sub, target, depth + 1) {
            return Some(found);
        }
    }
    None
}

fn real_usage_from_obj(usage: &serde_json::Value) -> Option<RealUsage> {
    let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let input = get("input_tokens");
    let cache_creation = get("cache_creation_input_tokens");
    let cache_read = get("cache_read_input_tokens");
    let output = get("output_tokens");
    // 토큰 필드가 전부 0/부재면 의미 없는 usage — 예: geo/tier 만 있는 객체.
    if input == 0 && cache_creation == 0 && cache_read == 0 && output == 0 {
        return None;
    }
    Some(RealUsage {
        input_tokens: input,
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
        output_tokens: output,
    })
}

/// ★finding 4 경고★: 이 함수는 **assistant 여부를 가리지 않는다** — result 라인의 최상위 `usage`(집계·
///   부분치)도 뽑는다. running context 계열을 만들 때 이 함수를 직접 쓰면 집계 라인이 per-turn
///   footprint 를 덮어쓴다. per-turn 계열은 `assistant_footprint_from_line` 을 써야 한다.
///
/// 실측 스키마(claude 2.1.170): assistant 라인은 `{"type":"assistant","message":{...,"usage":{
/// "input_tokens":N,"cache_creation_input_tokens":N,"cache_read_input_tokens":N,"output_tokens":N}}}`.
pub fn real_usage_from_line(line: &str) -> Option<RealUsage> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let usage = v
        .get("message")
        .and_then(|m| m.get("usage"))
        .or_else(|| v.get("usage"))?;
    real_usage_from_obj(usage)
}

/// **assistant 메시지의 per-turn footprint 만** 뽑는다 — 이것이 "지금 모델이 보고 있는 컨텍스트" 의
///   진짜 running 계열이다.
///
/// ★왜 이 구분이 load-bearing 인가★: claude 트랜스크립트의 마지막 라인은 종종 `{"type":"result",...,
///   "usage":{"input_tokens":1200,...}}` 같은 **집계·부분 usage** 다 — 이 input_tokens 는 그 턴의 증분일
///   뿐 누적 컨텍스트가 아니다. 이 라인을 running 계열에 섞으면, 49k footprint 를 찍은 직후 result 라인의
///   1200 이 series.last() 를 덮어 드라이버가 "컨텍스트가 49k→1200 으로 급감" 으로 오독한다(가짜
///   compaction + 잘못된 per-turn usage). 그래서 running 계열은 assistant 메시지 footprint 로만 만든다.
pub fn assistant_footprint_from_line(line: &str) -> Option<RealUsage> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let usage = v.get("message").and_then(|m| m.get("usage"))?;
    real_usage_from_obj(usage)
}

/// assistant 라인의 `message.model` 에서 모델 id 를 뽑는다(트랜스크립트 파서). 스폰 경로(stream-json
/// headless)의 트랜스크립트에는 `system/init` 라인이 **없고**(실측 2026-07-20, claude 2.1.170) 모델 id 가
/// assistant 메시지의 `message.model` 에 실린다 — 그래서 parse_init_model 만으로는 스폰 경로 모델 id 를
/// 못 얻어 이 보조 추출기가 필요하다. `<synthetic>`(합성 메시지 placeholder)은 실제 모델이 아니라 제외.
pub fn model_from_assistant_line(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let model = v
        .get("message")
        .and_then(|m| m.get("model"))
        .and_then(|x| x.as_str())?;
    if model.is_empty() || model == "<synthetic>" {
        return None;
    }
    Some(model.to_string())
}

fn is_result_line(line: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(line.trim()) {
        Ok(v) => v.get("type").and_then(|t| t.as_str()) == Some("result"),
        Err(_) => false,
    }
}

/// `isCompactSummary:true` 는 claude 트랜스크립트의 압축 요약 마커다.
pub fn line_is_compact_marker(line: &str) -> bool {
    if line_mentions_compact(line) {
        return true;
    }
    // isCompactSummary 는 top-level 또는 message 안에 boolean 으로 온다.
    match serde_json::from_str::<serde_json::Value>(line.trim()) {
        Ok(v) => {
            v.get("isCompactSummary").and_then(|b| b.as_bool()) == Some(true)
                || v.get("message")
                    .and_then(|m| m.get("isCompactSummary"))
                    .and_then(|b| b.as_bool())
                    == Some(true)
        }
        Err(_) => line.contains("isCompactSummary"),
    }
}

/// 파싱은 라인 독립이라 손상 라인 하나가 전체를 깨지 않는다(그 라인은 non_json 히스토그램으로 흡수).
pub fn parse_transcript(path: &Path) -> Option<TranscriptSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(parse_transcript_str(&content))
}

pub fn parse_transcript_str(content: &str) -> TranscriptSummary {
    let mut summary = TranscriptSummary::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        summary.total_lines += 1;

        *summary
            .event_histogram
            .entry(event_type_key(line))
            .or_insert(0) += 1;

        if summary.resolved_model.is_none() {
            if let Some(model) = parse_init_model(line).or_else(|| model_from_assistant_line(line))
            {
                summary.resolved_model = Some(model);
            }
        }

        if let Some(u) = assistant_footprint_from_line(line) {
            summary.real_usage_series.push(u);
        } else if is_result_line(line) {
            if let Some(u) = real_usage_from_line(line) {
                summary.aggregate_result_usage = Some(u);
            }
        }

        if line_is_compact_marker(line) {
            summary.compact_marker_lines.push(line.to_string());
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 픽스처(실 트랜스크립트 형태에서 발췌·redact) ────────────────────────────────
    //   실 트랜스크립트 구조를 그대로 본떴다 — 민감 내용은 무해 텍스트로 치환.

    /// 터미널 모드 모델 id 소스.
    const INIT_LINE: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\tmp\\ws","model":"claude-sonnet-4-5-20250929","session_id":"abc"}"#;

    /// stream-json headless assistant 라인 — 실 스모크 형태.
    const STREAM_ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_h","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 3"}],"usage":{"input_tokens":3,"cache_creation_input_tokens":1129,"cache_read_input_tokens":31094,"output_tokens":4}},"uuid":"uh"}"#;

    const SYNTHETIC_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_s","model":"<synthetic>","role":"assistant","content":[]},"uuid":"us"}"#;

    const ASSISTANT_LINE_1: &str = r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"received 1"}],"usage":{"input_tokens":3561,"cache_creation_input_tokens":5048,"cache_read_input_tokens":20679,"output_tokens":6,"service_tier":"standard"}},"uuid":"u1"}"#;

    /// 컨텍스트가 더 큰 후속 턴(캐시 read 증가).
    const ASSISTANT_LINE_2: &str = r#"{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"received 2"}],"usage":{"input_tokens":12,"cache_creation_input_tokens":8000,"cache_read_input_tokens":41000,"output_tokens":5}},"uuid":"u2"}"#;

    /// usage 없는 라인.
    const USER_LINE: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"DOC-1: x"}]},"uuid":"u3"}"#;

    const COMPACT_LINE: &str = r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":[{"type":"text","text":"Summary of prior conversation"}]},"uuid":"u4"}"#;

    const COMPACT_BOUNDARY: &str = r#"{"type":"system","subtype":"compact_boundary","uuid":"u5"}"#;

    const ASSISTANT_49K: &str = r#"{"type":"assistant","message":{"id":"msg_big","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 20"}],"usage":{"input_tokens":8,"cache_creation_input_tokens":2000,"cache_read_input_tokens":47000,"output_tokens":5}},"uuid":"ubig"}"#;

    /// assistant 직후의 result 집계 라인.
    const RESULT_1200: &str =
        r#"{"type":"result","subtype":"success","usage":{"input_tokens":1200,"output_tokens":34}}"#;

    #[test]
    fn real_usage_sums_cache_fields() {
        let u = real_usage_from_line(ASSISTANT_LINE_1).unwrap();
        assert_eq!(u.input_tokens, 3561);
        assert_eq!(u.cache_creation_input_tokens, 5048);
        assert_eq!(u.cache_read_input_tokens, 20679);
        assert_eq!(u.output_tokens, 6);
        assert_eq!(u.context_footprint(), 3561 + 5048 + 20679);
    }

    #[test]
    fn real_usage_footprint_dwarfs_bare_input() {
        let u = real_usage_from_line(ASSISTANT_LINE_2).unwrap();
        assert_eq!(u.input_tokens, 12);
        assert_eq!(u.context_footprint(), 12 + 8000 + 41000);
        assert!(
            u.context_footprint() > u.input_tokens * 100,
            "실 컨텍스트는 bare input 의 수백 배(캐시 항)"
        );
    }

    #[test]
    fn real_usage_none_when_no_usage() {
        assert_eq!(real_usage_from_line(USER_LINE), None);
        assert_eq!(real_usage_from_line("not json"), None);
    }

    #[test]
    fn real_usage_handles_missing_cache_fields() {
        // result 라인은 cache 항이 없을 수 있다.
        let line = r#"{"type":"result","usage":{"input_tokens":1200,"output_tokens":34}}"#;
        let u = real_usage_from_line(line).unwrap();
        assert_eq!(u.input_tokens, 1200);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert_eq!(u.context_footprint(), 1200);
    }

    #[test]
    fn compact_marker_detects_variants() {
        assert!(line_is_compact_marker(COMPACT_LINE), "isCompactSummary");
        assert!(
            line_is_compact_marker(COMPACT_BOUNDARY),
            "system/compact_boundary"
        );
        assert!(!line_is_compact_marker(ASSISTANT_LINE_1), "일반 assistant");
        assert!(!line_is_compact_marker(USER_LINE), "일반 user");
    }

    #[test]
    fn model_from_assistant_line_extracts_stream_json_model() {
        assert_eq!(
            model_from_assistant_line(STREAM_ASSISTANT_LINE).as_deref(),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn model_from_assistant_line_skips_synthetic_and_non_assistant() {
        assert_eq!(
            model_from_assistant_line(SYNTHETIC_LINE),
            None,
            "<synthetic> 제외"
        );
        assert_eq!(model_from_assistant_line(USER_LINE), None, "user 라인 제외");
        assert_eq!(
            model_from_assistant_line(INIT_LINE),
            None,
            "system 라인 제외"
        );
        assert_eq!(model_from_assistant_line("not json"), None);
    }

    #[test]
    fn parse_transcript_resolves_model_from_stream_assistant() {
        let content = [SYNTHETIC_LINE, STREAM_ASSISTANT_LINE].join("\n");
        let s = parse_transcript_str(&content);
        assert_eq!(s.resolved_model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn parse_transcript_folds_full_file() {
        let content = [
            INIT_LINE,
            ASSISTANT_LINE_1,
            USER_LINE,
            ASSISTANT_LINE_2,
            COMPACT_LINE,
            COMPACT_BOUNDARY,
            "",
        ]
        .join("\n");
        let s = parse_transcript_str(&content);

        assert_eq!(
            s.resolved_model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(s.real_usage_series.len(), 2);
        assert_eq!(
            s.real_usage_series[0].context_footprint(),
            3561 + 5048 + 20679
        );
        assert_eq!(
            s.real_usage_series[1].context_footprint(),
            12 + 8000 + 41000
        );
        assert!(
            s.real_usage_series[1].context_footprint() > s.real_usage_series[0].context_footprint()
        );
        assert_eq!(s.compact_marker_lines.len(), 2);
        assert_eq!(s.event_histogram.get("assistant"), Some(&2));
        assert_eq!(s.event_histogram.get("user"), Some(&2));
        assert_eq!(s.event_histogram.get("system/init"), Some(&1));
        assert_eq!(s.event_histogram.get("system/compact_boundary"), Some(&1));
        assert_eq!(s.total_lines, 6);
    }

    #[test]
    fn assistant_footprint_excludes_result_aggregate() {
        let a = assistant_footprint_from_line(ASSISTANT_49K).unwrap();
        assert_eq!(a.context_footprint(), 8 + 2000 + 47000);
        assert_eq!(
            assistant_footprint_from_line(RESULT_1200),
            None,
            "result 집계 라인은 per-turn footprint 아님"
        );
    }

    #[test]
    fn result_aggregate_does_not_clobber_running_context() {
        let content = [ASSISTANT_49K, RESULT_1200].join("\n");
        let s = parse_transcript_str(&content);
        assert_eq!(
            s.real_usage_series.len(),
            1,
            "assistant 1턴만 running 계열에"
        );
        assert_eq!(
            s.real_usage_series.last().unwrap().context_footprint(),
            8 + 2000 + 47000,
            "running context 는 49k 유지(1200 이 덮지 않음)"
        );
        assert_eq!(
            s.aggregate_result_usage.map(|u| u.input_tokens),
            Some(1200),
            "result 집계 usage 는 분리 필드에"
        );
        let max = s
            .real_usage_series
            .iter()
            .map(|u| u.context_footprint())
            .max();
        assert_eq!(max, Some(8 + 2000 + 47000));
    }

    #[test]
    fn parse_transcript_missing_file_is_none() {
        let p = std::env::temp_dir().join("engram-nonexistent-transcript-xyz.jsonl");
        assert_eq!(parse_transcript(&p), None);
    }

    #[test]
    fn locate_transcript_absent_session_is_none() {
        let missing = "00000000-dead-beef-0000-000000000000";
        assert_eq!(locate_transcript(missing), None);
    }
}
