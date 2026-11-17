# S35 — UI language: English source, Korean on request

The app's UI text is English by default with Korean selectable through the LLM-only setting `ui.language`, applied live to every window, the tray, the refusal dialog, tab names and window titles; i18next does the lookup behind `t()`. Canonical ADRs: ADR-0372 (user decisions), ADR-0373 (runtime design), ADR-0349 (i18next behind `t()`). Reference: `docs/reference/structure/i18n.md`. Research: `docs/research/i18n-oss-library-reevaluation-2026-11-07.md`.

- `prd.md` — edition 2, approved.
- `trd.md` — edition 2.3, Implemented.
