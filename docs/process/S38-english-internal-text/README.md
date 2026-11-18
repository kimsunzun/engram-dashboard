# S38 — English-only internal text

Everything the user does not read on the app screen becomes English: logs, panic/assert messages, internal error strings, LLM-facing strings embedded in code, test names and messages, plus the Korean left over in docs and comments after S33. App screen text stays switchable through the S35 tables. `prompts/` and `.claude/` are on hold. Input: `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md`; previous pass: `docs/process/S33-english-translation/plan.md`.

- `prd.md` — edition 2, approved.
- `trd.md` — edition 2.1, Implemented.
