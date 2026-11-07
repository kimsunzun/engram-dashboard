# S31 — spaced paths in the source-regex gates

Closes a gap where six CI source-regex checks miss paths with whitespace between segments, such as `tokio :: time`. It is a narrow gap that is only actually open in macro bodies rustfmt does not touch and in `rustfmt::skip` regions.

- Source: `docs/todo/doc-drift.md` 「게이트 정규식이 못 닫은 구멍」
- Documents: [`prd.md`](prd.md) · [`trd.md`](trd.md)
- Related ADRs: none (the source of truth for a gate's expected values = the target crate / file header)
- Timeline: `docs/process/step-log.md` `## S31`
