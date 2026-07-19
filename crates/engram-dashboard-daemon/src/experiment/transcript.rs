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
//! 못박는다. 그래서 이 모듈은 (a) 오직 `#[cfg(feature = "test-harness")]` 실험 경로에서만 쓰이고,
//! (b) 파일 부재/파싱 실패에 **절대 하네스를 실패시키지 않는다**(best-effort). 제품 코드는 이 모듈에
//! 의존하지 않는다(운영 빌드 미컴파일).
//!
//! ## 역할
//! - `locate_transcript`: `~/.claude/projects/` 아래를 **재귀 검색**해 `<session-id>.jsonl` 을 찾는다.
//!   cwd-munging 규칙을 하드코딩하지 않는다(claude 내부 규칙이 바뀌어도 재귀 검색이 흡수).
//! - 순수 파서들(전부 단위 테스트): raw 라인 → 실 usage(`real_usage_from_line`)·모델 id
//!   (`parse_init_model` 터미널 모드 / `model_from_assistant_line` stream-json headless)·event 히스토그램·
//!   compact 마커. record.rs 의 기존 파서(`parse_init_model`/`event_type_key`/`line_mentions_compact`)를
//!   재사용하고, cache 토큰 합산·assistant-model 파서를 여기 추가한다.
//! - `parse_transcript`: 파일 1개를 통째로 파싱해 `TranscriptSummary`(모델 id·per-assistant-turn 실
//!   usage 계열·히스토그램·compact 마커 라인)로 접는다.
// ADR-0090

use std::path::{Path, PathBuf};

use super::record::{event_type_key, line_mentions_compact, parse_init_model};

/// 한 assistant 턴의 **실 컨텍스트 footprint** 토큰. claude usage 는 `input_tokens` 만 노출하면 그 턴의
/// 증분 입력이라 누적 컨텍스트를 반영 못 한다 — 실제 컨텍스트 크기는 아래 세 항의 합이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealUsage {
    /// 비-캐시 입력 토큰(그 턴에 새로 보낸 것).
    pub input_tokens: u64,
    /// 캐시 생성 입력 토큰(이번에 캐시에 쓴 프롬프트 — 컨텍스트의 일부).
    pub cache_creation_input_tokens: u64,
    /// 캐시 읽기 입력 토큰(이전 컨텍스트 재사용분 — 컨텍스트의 대부분).
    pub cache_read_input_tokens: u64,
    /// 출력 토큰.
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

/// 트랜스크립트 파일 1개를 파싱한 요약(파일럿 산출물).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TranscriptSummary {
    /// 해석된 모델 id(있으면) — system/init(터미널) 또는 assistant.message.model(stream-json headless).
    pub resolved_model: Option<String>,
    /// ★finding 4★: **assistant 메시지 per-turn footprint 만** 담는 running 계열(등장 순서 = 턴 순서).
    ///   result 라인의 집계 usage 는 여기 섞지 않는다 — series.last()/max() 가 진짜 컨텍스트를 반영하게.
    ///   포화 진행 캘리브레이션의 핵심 데이터.
    pub real_usage_series: Vec<RealUsage>,
    /// ★finding 4★: result 라인의 최상위 집계 usage(마지막 것). running 계열과 **분리** 보관 —
    ///   진단·기록용이지 running context 를 덮지 않는다(집계 input_tokens 는 증분/부분치라 오독 유발).
    pub aggregate_result_usage: Option<RealUsage>,
    /// raw event-type/subtype 히스토그램(record::event_type_key 로 산출).
    pub event_histogram: std::collections::BTreeMap<String, u64>,
    /// compact/summary 관련으로 걸린 라인 verbatim(캡 적용 전 — 호출자가 record::cap_response 로 캡).
    pub compact_marker_lines: Vec<String>,
    /// 파싱한 총 라인 수(비어 있지 않은).
    pub total_lines: usize,
}

/// `~/.claude/projects/` 아래를 재귀 검색해 `<session-id>.jsonl` 의 절대 경로를 찾는다. 없으면 None
/// (best-effort — 호출자는 `transcript_available:false` 로 기록하고 문자 추정으로 폴백).
///
/// ★cwd-munging 미하드코딩★: claude 는 cwd 를 munge 한 디렉토리명 아래에 트랜스크립트를 둔다(예:
/// `C--Users-x-proj`). 그 규칙은 버전에 따라 바뀔 수 있어 **재귀 검색**으로 파일명만 매칭한다 — munging
/// 규칙 변화에 무관하게 흡수.
pub fn locate_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = claude_projects_dir()?;
    let target = format!("{session_id}.jsonl");
    find_file_recursive(&projects, &target, 0)
}

/// `~/.claude/projects` 경로(HOME/USERPROFILE 기반). 신규 의존성 없이 env 로 홈을 찾는다.
fn claude_projects_dir() -> Option<PathBuf> {
    // Windows=USERPROFILE, unix=HOME. 둘 다 std env 로 조회(dirs crate 미도입 — no-new-deps).
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

/// `dir` 아래를 깊이 제한 재귀로 훑어 파일명이 `target` 인 첫 파일을 찾는다. 깊이 상한은 폭주 방지
/// (claude 구조는 projects/<munged>/<sid>.jsonl 로 얕다 — 상한 8 이면 충분).
fn find_file_recursive(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // 파일명 직접 매칭(대소문자 정확 — sid 는 소문자 uuid).
        if path.file_name().and_then(|n| n.to_str()) == Some(target) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    // 이 레벨에서 못 찾으면 하위 디렉토리로(BFS-ish: 얕은 곳 우선).
    for sub in subdirs {
        if let Some(found) = find_file_recursive(&sub, target, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// usage 객체 하나에서 RealUsage 를 뽑는다(공통). 세 cache 항 중 누락은 0 흡수. 전부 0/부재면 None.
fn real_usage_from_obj(usage: &serde_json::Value) -> Option<RealUsage> {
    let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let input = get("input_tokens");
    let cache_creation = get("cache_creation_input_tokens");
    let cache_read = get("cache_read_input_tokens");
    let output = get("output_tokens");
    // 토큰 필드가 전부 0/부재면 의미 없는 usage(예: geo/tier만 있는 객체) — None.
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

/// raw 트랜스크립트 라인에서 실 usage 를 뽑는다(assistant 라인의 `message.usage` 또는 최상위 `usage`).
/// usage 객체가 없거나 토큰 필드가 하나도 없으면 None. 세 cache 항 중 누락은 0 으로 흡수.
///
/// ★finding 4 경고★: 이 함수는 **assistant 여부를 가리지 않는다** — result 라인의 최상위 `usage`(집계·
///   부분치)도 뽑는다. 그래서 running context 계열(real_usage_series)을 만들 때 이 함수를 직접 쓰면
///   집계 라인이 per-turn footprint 를 덮어쓴다. per-turn 계열은 `assistant_footprint_from_line` 을
///   써야 한다(아래). 이 함수는 하위호환·단일 usage 추출 유틸로만 남긴다.
///
/// 실측 스키마(claude 2.1.170): assistant 라인은 `{"type":"assistant","message":{...,"usage":{
/// "input_tokens":N,"cache_creation_input_tokens":N,"cache_read_input_tokens":N,"output_tokens":N}}}`.
pub fn real_usage_from_line(line: &str) -> Option<RealUsage> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    // usage 는 message.usage(assistant) 또는 최상위 usage(result 라인)에 있다.
    let usage = v
        .get("message")
        .and_then(|m| m.get("usage"))
        .or_else(|| v.get("usage"))?;
    real_usage_from_obj(usage)
}

/// ★finding 4 fix★: **assistant 메시지의 per-turn footprint 만** 뽑는다(`type=="assistant"` 이고
///   `message.usage` 가 있는 라인). 이것이 "지금 모델이 보고 있는 컨텍스트" 의 진짜 running 계열이다.
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
    // "<synthetic>" 는 실제 모델 id 가 아님(합성 메시지) — 제외.
    if model.is_empty() || model == "<synthetic>" {
        return None;
    }
    Some(model.to_string())
}

/// 라인이 최상위 집계 `type=="result"` 인가(finding 4 — running 계열과 분리할 집계 usage 소스).
fn is_result_line(line: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(line.trim()) {
        Ok(v) => v.get("type").and_then(|t| t.as_str()) == Some("result"),
        Err(_) => false,
    }
}

/// 라인이 compact/summary 마커인지 — record::line_mentions_compact(type/subtype/문자열 스캔)에 더해
/// `isCompactSummary:true`(claude 트랜스크립트의 압축 요약 마커)를 본다.
pub fn line_is_compact_marker(line: &str) -> bool {
    if line_mentions_compact(line) {
        return true;
    }
    // isCompactSummary 는 top-level 또는 message 안에 boolean 으로 온다 — JSON 파싱해 확인, 실패 시
    //   문자열 fallback.
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

/// 트랜스크립트 파일 1개를 통째로 파싱해 요약으로 접는다. best-effort: 읽기 실패면 None(호출자 폴백).
/// 파싱은 라인 독립이라 손상 라인 하나가 전체를 깨지 않는다(그 라인은 non_json 히스토그램으로 흡수).
pub fn parse_transcript(path: &Path) -> Option<TranscriptSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(parse_transcript_str(&content))
}

/// 문자열 본문(줄바꿈 구분 JSONL)을 파싱 — 파일 IO 를 뗀 순수 코어(단위 테스트가 이걸 직접 호출).
pub fn parse_transcript_str(content: &str) -> TranscriptSummary {
    let mut summary = TranscriptSummary::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        summary.total_lines += 1;

        // event 히스토그램(raw 타입/subtype).
        *summary
            .event_histogram
            .entry(event_type_key(line))
            .or_insert(0) += 1;

        // 모델 id(최초 승리 — 이후 라인이 덮지 않게). 소스 2종: (1) system/init(터미널 모드 트랜스크립트),
        //   (2) assistant.message.model(stream-json headless 트랜스크립트 — 스폰 경로 실측 소스). 둘 다 시도.
        if summary.resolved_model.is_none() {
            if let Some(model) = parse_init_model(line).or_else(|| model_from_assistant_line(line))
            {
                summary.resolved_model = Some(model);
            }
        }

        // ★finding 4 fix★: running 계열은 **assistant 메시지 footprint 만** 쌓는다(result 집계 usage 는
        //   여기 안 섞음 — series.last()/max() 오염 방지). result 라인의 집계 usage 는 분리 필드에 최신치로.
        if let Some(u) = assistant_footprint_from_line(line) {
            summary.real_usage_series.push(u);
        } else if is_result_line(line) {
            if let Some(u) = real_usage_from_line(line) {
                summary.aggregate_result_usage = Some(u);
            }
        }

        // compact 마커(verbatim 캡처 — 호출자가 cap_response 로 캡해 기록).
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
    //   실측 스키마(claude 2.1.170): assistant.message.usage 에 4개 토큰 항. 모델 id 소스는 2종 —
    //   터미널 모드는 system/init.model, stream-json headless 는 assistant.message.model(2026-07-20 스모크
    //   실측). 아래 라인들은 실 트랜스크립트 구조를 그대로 본떴다(민감 내용은 무해 텍스트로 치환).

    /// system/init 라인(터미널 모드 모델 id 소스).
    const INIT_LINE: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\tmp\\ws","model":"claude-sonnet-4-5-20250929","session_id":"abc"}"#;

    /// stream-json headless assistant 라인 — 실 스모크 형태(message.model + 실 usage). 모델 id 가 여기 산다.
    const STREAM_ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_h","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 3"}],"usage":{"input_tokens":3,"cache_creation_input_tokens":1129,"cache_read_input_tokens":31094,"output_tokens":4}},"uuid":"uh"}"#;

    /// 합성 메시지(`<synthetic>` model — 실제 모델 아님, 제외돼야).
    const SYNTHETIC_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_s","model":"<synthetic>","role":"assistant","content":[]},"uuid":"us"}"#;

    /// assistant 라인 — 실 usage(input + cache_creation + cache_read + output).
    const ASSISTANT_LINE_1: &str = r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"received 1"}],"usage":{"input_tokens":3561,"cache_creation_input_tokens":5048,"cache_read_input_tokens":20679,"output_tokens":6,"service_tier":"standard"}},"uuid":"u1"}"#;

    /// assistant 라인 2 — 컨텍스트가 더 큰 후속 턴(캐시 read 증가).
    const ASSISTANT_LINE_2: &str = r#"{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"received 2"}],"usage":{"input_tokens":12,"cache_creation_input_tokens":8000,"cache_read_input_tokens":41000,"output_tokens":5}},"uuid":"u2"}"#;

    /// user 라인 — usage 없음(계열에 안 들어가야).
    const USER_LINE: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"DOC-1: x"}]},"uuid":"u3"}"#;

    /// compact 요약 마커 라인(isCompactSummary).
    const COMPACT_LINE: &str = r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":[{"type":"text","text":"Summary of prior conversation"}]},"uuid":"u4"}"#;

    /// system/compact_boundary 라인(type/subtype 스캔으로 잡힘).
    const COMPACT_BOUNDARY: &str = r#"{"type":"system","subtype":"compact_boundary","uuid":"u5"}"#;

    /// ★finding 4 픽스처★: 49k footprint 를 찍은 assistant 라인.
    const ASSISTANT_49K: &str = r#"{"type":"assistant","message":{"id":"msg_big","model":"claude-sonnet-4-6","role":"assistant","content":[{"type":"text","text":"received 20"}],"usage":{"input_tokens":8,"cache_creation_input_tokens":2000,"cache_read_input_tokens":47000,"output_tokens":5}},"uuid":"ubig"}"#;

    /// ★finding 4 픽스처★: assistant 직후의 result 집계 라인 — 최상위 usage 에 input_tokens:1200 만.
    ///   이 라인이 running 계열을 덮으면 49k→1200 가짜 급감이 된다(그래서 계열에서 제외돼야 함).
    const RESULT_1200: &str =
        r#"{"type":"result","subtype":"success","usage":{"input_tokens":1200,"output_tokens":34}}"#;

    #[test]
    fn real_usage_sums_cache_fields() {
        let u = real_usage_from_line(ASSISTANT_LINE_1).unwrap();
        assert_eq!(u.input_tokens, 3561);
        assert_eq!(u.cache_creation_input_tokens, 5048);
        assert_eq!(u.cache_read_input_tokens, 20679);
        assert_eq!(u.output_tokens, 6);
        // 실 컨텍스트 footprint = 세 입력 항의 합(누적 컨텍스트 근사).
        assert_eq!(u.context_footprint(), 3561 + 5048 + 20679);
    }

    #[test]
    fn real_usage_footprint_dwarfs_bare_input() {
        // ★파일럿 발견의 핵심★: 실 footprint 가 bare input_tokens 보다 훨씬 크다(캐시가 대부분).
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
        // result 라인은 cache 항이 없을 수 있다 — 누락은 0 으로 흡수, input/output 만으로도 usage.
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
        // stream-json headless 트랜스크립트는 assistant.message.model 에 모델 id 를 실는다(스모크 실측).
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
        // system/init 이 없는 stream-json headless 트랜스크립트에서도 모델 id 를 뽑아야 한다(assistant fallback).
        //   <synthetic> 이 먼저 와도 실제 모델이 승리한다(합성은 스킵).
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
            "", // 빈 줄(무시돼야).
        ]
        .join("\n");
        let s = parse_transcript_str(&content);

        // 모델 id.
        assert_eq!(
            s.resolved_model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        // 실 usage 계열 = assistant 2턴(user/init/compact 라인은 usage 없음 → 제외).
        assert_eq!(s.real_usage_series.len(), 2);
        assert_eq!(
            s.real_usage_series[0].context_footprint(),
            3561 + 5048 + 20679
        );
        assert_eq!(
            s.real_usage_series[1].context_footprint(),
            12 + 8000 + 41000
        );
        // 컨텍스트 계열이 증가(포화 진행 방향).
        assert!(
            s.real_usage_series[1].context_footprint() > s.real_usage_series[0].context_footprint()
        );
        // compact 마커 2건(isCompactSummary + compact_boundary).
        assert_eq!(s.compact_marker_lines.len(), 2);
        // 히스토그램: system/init 1, assistant 2, user 2, system/compact_boundary 1.
        assert_eq!(s.event_histogram.get("assistant"), Some(&2));
        assert_eq!(s.event_histogram.get("user"), Some(&2));
        assert_eq!(s.event_histogram.get("system/init"), Some(&1));
        assert_eq!(s.event_histogram.get("system/compact_boundary"), Some(&1));
        // 빈 줄 제외한 총 라인.
        assert_eq!(s.total_lines, 6);
    }

    #[test]
    fn assistant_footprint_excludes_result_aggregate() {
        // assistant_footprint_from_line 은 assistant 만 — result 집계 라인은 None.
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
        // ★finding 4 regression★: 49k assistant → 1200 result 순서. running 계열은 49k 만 담고,
        //   집계 1200 은 별도 필드로 — series.last() 가 1200 으로 덮이면 안 된다(가짜 compaction 방지).
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
        // 집계는 분리 필드에 보존(진단용).
        assert_eq!(
            s.aggregate_result_usage.map(|u| u.input_tokens),
            Some(1200),
            "result 집계 usage 는 분리 필드에"
        );
        // max footprint 도 49k(1200 이 최댓값을 낮추지 않음).
        let max = s
            .real_usage_series
            .iter()
            .map(|u| u.context_footprint())
            .max();
        assert_eq!(max, Some(8 + 2000 + 47000));
    }

    #[test]
    fn parse_transcript_missing_file_is_none() {
        // 존재하지 않는 경로 → None(best-effort 폴백 계약).
        let p = std::env::temp_dir().join("engram-nonexistent-transcript-xyz.jsonl");
        assert_eq!(parse_transcript(&p), None);
    }

    #[test]
    fn locate_transcript_absent_session_is_none() {
        // 랜덤 uuid 형태 sid 는 존재하지 않으므로 None(패닉 없이). projects 디렉토리 자체가 없어도 None.
        let missing = "00000000-dead-beef-0000-000000000000";
        assert_eq!(locate_transcript(missing), None);
    }
}
