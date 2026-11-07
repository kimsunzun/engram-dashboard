# ADR-0255: Usage failure is traced by a classification word and the upstream original text, and the original text is carried only on the UI and LLM read paths

- 상태: 확정 (2026-09-27, basis: user decision + TRD §7 #11)
- 관련: TRD §7 #11 · §7 #20(neutralizing the `Unavailable` wording) · `base/src/logging/mod.rs`(masker) · step-log S21

## 맥락
We had to decide how to convey a usage query failure to the user and the LLM — mix error wordings from several sources into one string, or classify and also keep the original text.

## 결정
Failure tracing is a **classification word + the upstream original text**. For each abnormal state the wire carries `UsageStateDetail{kind, code, upstream}` (agent-side `ProbeFailure` · `UsageDetail` · `UpstreamText`). **The original text is carried only on the UI and the LLM read path (the `usage.get` row) and is not sent to the log** (masking · 200-character cut · masked `Debug`, amending the `ProbeError` secret rule).

## 거부한 대안 (user)
- Mixing the error wordings of several sources into one string without classification — 「오류 문구 원문이 있으면 보내야지 … 이상한거 다 섞어서 보내지 말고 타입은 보내야지」.

## 거부한 대안 (main)
- Also keeping the original text in the log — a log can be shared.

## 거부한 대안 (internal — §3)
- Making `kind` an enum — old shell deserialization breaks (#68).
- Giving each failure variant its own field (#70).

## 근거
User decision 2026-09-27 (「오류 문구 원문이 있으면 보내야지 … 유저가 최대한 추적할 수 있도록」) · TRD §7 #11.

## 영향 / 불변식
The masking rule covers only the token shapes the base masker knows (`base/src/logging/mod.rs:49`), and the original-text field masks **only named-field values**. What goes into the original text is decided by the CLI version, and this decision itself makes no policy judgment (the same marker as PRD D3). Implementation commits: `56776d6`, `c68b885`, `846a3e4`.
