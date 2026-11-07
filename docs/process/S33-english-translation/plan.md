# S33 — English translation of existing comments and docs

Source = CLAUDE.md 「언어」 (S30): code comments and `docs/` md are English. This topic moves the existing Korean text over, a small batch at a time.

## Scope

- In: code comments (Rust · TS/TSX · scripts) and every md/html under `docs/`, archives included (`docs/process` · `docs/research` · `docs/decisions`).
- Out: string literals (UI text, logs, errors, test expectations) stay as they are. `.claude/` stays Korean (instructions the user reads).
- Out for now: Korean file names (307 ADRs · 2 `launch/*.bat`). Renaming is a separate, script-only batch at the end, because every inbound link moves with it.

## Rules every batch keeps

- Keep verbatim: code blocks · backtick spans · paths · identifiers · links · ADR numbers · user quotes in 「」 · names of `.claude/` sections · UI strings quoted from code.
- ADR files: the lint parses Korean tokens. Keep `- 상태:` and its vocab (`확정` · `제안` · `폐기` · `거부`), `- 관련:`, and the section headings (`## 맥락` · `## 결정` · `## 거부한 대안` · `## 근거` · `## 영향 / 불변식`). Translate everything else, H1 title included, then regenerate the index (`adr.mjs index --write`).
- Terms follow the glossary (`glossary.md`, built in the pilot).
- One batch = one commit. Translation commits go into `.git-blame-ignore-revs`.

## Checks per batch

- Mechanical: code tokens unchanged (comment-stripped compare) · code blocks, backtick spans, links unchanged · no Hangul left outside the keep list · ADR lint clean.
- Meaning: codex reads source and translation side by side and flags drift (negation · conditions · invariants · decisions).

## Order

Oldest first. Pilot = ADR-0001..0005, translated once by Sonnet and once by Opus to pick the model and measure tokens.
