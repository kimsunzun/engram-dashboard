//! Transcript tap — a pure parser that reads the raw session JSONL left by the claude CLI **for measurement only** (ADR-0090).
//!
//! ## ★Standing (honest scope) — this is an experiment measurement tap, not a product feature★
//! The core decoder (`ClaudeStreamDecoder`) **discards** `cache_creation_input_tokens`/`cache_read_input_tokens` (where the real
//! context size lives) · `system/init` (model id) · compact-related raw lines. The core is a no-modification
//! target (ADR-0090 constraint) so this information cannot be obtained from the spawn path. Workaround: since **we control the session id**
//! (`--session-id`, ADR-0008), read the raw transcript the claude CLI leaves at `~/.claude/projects/<munged-cwd>/<session-id>.jsonl`
//! **best-effort** to supplement the measurements.
//!
//! ★ADR-0008 boundary — no product feature is built on a trace file★: this transcript is **for the experiment measurement tap only**.
//! ADR-0008 pins down "restore correctness depends only on the controlled sid; the trace file is best-effort — no feature expansion with it".
//! So this module **never fails the harness** on a missing file or parse failure (best-effort).
// ADR-0090

use std::path::{Path, PathBuf};

use super::record::{event_type_key, line_mentions_compact, parse_init_model};

/// The **real context footprint** tokens of one assistant turn. Looking only at claude usage `input_tokens` gives that turn's
/// incremental input, which does not reflect the cumulative context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealUsage {
    /// Non-cache input tokens (newly sent in that turn).
    pub input_tokens: u64,
    /// Cache-creation input tokens (the prompt written to the cache this time — part of the context).
    pub cache_creation_input_tokens: u64,
    /// Cache-read input tokens (the reused part of the previous context — most of the context).
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl RealUsage {
    /// Real context footprint = input + cache_creation + cache_read. The two cache terms are most of the true context size,
    /// so this sum is a ground-truth approximation of "the number of context tokens the model is looking at now".
    pub fn context_footprint(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptSummary {
    pub resolved_model: Option<String>,
    /// Holds **only assistant-message per-turn footprints** (appearance order = turn order). The aggregate usage
    ///   of a result line is not mixed in here.
    pub real_usage_series: Vec<RealUsage>,
    /// The top-level aggregate usage of a result line (the last one) — for diagnosis and records.
    pub aggregate_result_usage: Option<RealUsage>,
    /// Raw event-type/subtype histogram (produced by record::event_type_key).
    pub event_histogram: std::collections::BTreeMap<String, u64>,
    /// Lines caught as compact/summary-related, verbatim (before the cap — the caller caps with record::cap_response).
    pub compact_marker_lines: Vec<String>,
    /// Total number of lines parsed (non-empty).
    pub total_lines: usize,
}

/// ★cwd-munging not hardcoded★: claude puts the transcript under a directory name made by munging the cwd (e.g.
/// `C--Users-x-proj`). That rule may change by version, so we match only the file name by **recursive search** — absorbs
/// changes in the munging rule regardless.
pub fn locate_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = claude_projects_dir()?;
    let target = format!("{session_id}.jsonl");
    find_file_recursive(&projects, &target, 0)
}

fn claude_projects_dir() -> Option<PathBuf> {
    // Find home via std env — the dirs crate is not introduced (no-new-deps).
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

/// The depth cap is to prevent runaway — claude's structure is shallow, projects/<munged>/<sid>.jsonl, so 8 is enough.
fn find_file_recursive(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Exact case match — sid is a lowercase uuid.
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
    // If the token fields are all 0/absent it is a meaningless usage — e.g. an object with only geo/tier.
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

/// ★finding 4 warning★: this function **does not check whether the line is an assistant line** — it also extracts the top-level `usage` of a result line (aggregate ·
///   partial). If this function is used directly to build the running context series, an aggregate line overwrites the per-turn
///   footprint. The per-turn series must use `assistant_footprint_from_line`.
///
/// Measured schema (claude 2.1.170): an assistant line is `{"type":"assistant","message":{...,"usage":{
/// "input_tokens":N,"cache_creation_input_tokens":N,"cache_read_input_tokens":N,"output_tokens":N}}}`.
pub fn real_usage_from_line(line: &str) -> Option<RealUsage> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let usage = v
        .get("message")
        .and_then(|m| m.get("usage"))
        .or_else(|| v.get("usage"))?;
    real_usage_from_obj(usage)
}

/// Extracts **only the assistant message's per-turn footprint** — this is the true running series of "the context
///   the model is looking at now".
///
/// ★Why this distinction is load-bearing★: the last line of a claude transcript is often an **aggregate/partial usage** like `{"type":"result",...,
///   "usage":{"input_tokens":1200,...}}` — this input_tokens is only that turn's increment and not the cumulative context.
///   If this line is mixed into the running series,
///   right after a 49k footprint is logged the result line's 1200 overwrites series.last() and the driver misreads it as "the context dropped sharply from 49k→1200" (a fake
///   compaction + wrong per-turn usage). So the running series is built only from assistant message footprints.
pub fn assistant_footprint_from_line(line: &str) -> Option<RealUsage> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let usage = v.get("message").and_then(|m| m.get("usage"))?;
    real_usage_from_obj(usage)
}

/// Extracts the model id from the `message.model` of an assistant line (transcript parser). The transcript of the spawn path (stream-json
/// headless) **has no** `system/init` line (measured 2026-07-20, claude 2.1.170) and the model id is carried in the
/// assistant message's `message.model` — so parse_init_model alone cannot get the spawn-path model id,
/// and this auxiliary extractor is needed. `<synthetic>` (placeholder for synthetic messages) is not a real model so it is excluded.
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

/// `isCompactSummary:true` is the compaction summary marker of a claude transcript.
pub fn line_is_compact_marker(line: &str) -> bool {
    if line_mentions_compact(line) {
        return true;
    }
    // isCompactSummary comes as a boolean at top level or inside message.
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

/// Parsing is per line so one corrupted line does not break the whole (that line is absorbed into the non_json histogram).
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

    // ── Fixtures (excerpted and redacted from the real transcript form) ────────────────────────────────
    //   Modeled exactly on the real transcript structure — sensitive content replaced with harmless text.

    /// Terminal-mode model id source.
    const INIT_LINE: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\tmp\\ws","model":"claude-sonnet-4-5-20250929","session_id":"abc"}"#;

    /// stream-json headless assistant line — real smoke form.
    const STREAM_ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_h","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 3"}],"usage":{"input_tokens":3,"cache_creation_input_tokens":1129,"cache_read_input_tokens":31094,"output_tokens":4}},"uuid":"uh"}"#;

    const SYNTHETIC_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_s","model":"<synthetic>","role":"assistant","content":[]},"uuid":"us"}"#;

    const ASSISTANT_LINE_1: &str = r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"received 1"}],"usage":{"input_tokens":3561,"cache_creation_input_tokens":5048,"cache_read_input_tokens":20679,"output_tokens":6,"service_tier":"standard"}},"uuid":"u1"}"#;

    /// A later turn with a larger context (cache read increased).
    const ASSISTANT_LINE_2: &str = r#"{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"received 2"}],"usage":{"input_tokens":12,"cache_creation_input_tokens":8000,"cache_read_input_tokens":41000,"output_tokens":5}},"uuid":"u2"}"#;

    /// A line without usage.
    const USER_LINE: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"DOC-1: x"}]},"uuid":"u3"}"#;

    const COMPACT_LINE: &str = r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":[{"type":"text","text":"Summary of prior conversation"}]},"uuid":"u4"}"#;

    const COMPACT_BOUNDARY: &str = r#"{"type":"system","subtype":"compact_boundary","uuid":"u5"}"#;

    const ASSISTANT_49K: &str = r#"{"type":"assistant","message":{"id":"msg_big","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 20"}],"usage":{"input_tokens":8,"cache_creation_input_tokens":2000,"cache_read_input_tokens":47000,"output_tokens":5}},"uuid":"ubig"}"#;

    /// A result aggregate line right after the assistant.
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
        // A result line may have no cache terms.
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
