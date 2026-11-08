# ADR-0315: A reference = one form, the `/` path from the library root + built-in `engram:/`

- 상태: 제안 (2026-10-31, basis: user decisions memos 101–103 (2026-10-15 「ㅇㅇ 지정한 폴더로 확정」) · PRD R3 · Δ1)
- 관련: ADR-0312 · ADR-0335 (place = identity) · ADR-0325 · TRD `docs/process/S25-preset-library/trd.md` §1-2 `LibPath` · §7 2 · research `docs/research/preset-asset-reference-base-survey-2026-10-14.md` (+ 2026-10-15 addendum)

## 맥락
In what form do we write a reference inside a preset (skill, instruction, settings, another preset)?

## 결정
1. A reference = one form, the `/` path from the very top of the library (e.g. `스킬/리뷰/review`) · no scope limit (R3).
2. `LibPath` rules: UTF-8 · the separator is only `/` · no leading or trailing `/` · no empty segment · `.` · `..` · `\` · `:` · no control characters. There is one root, so there is no root prefix (ADR-0312).
3. A built-in asset = a fixed fake path outside the library, `engram:/…` (Δ1). `LibPath` does not accept `:`, so the two do not mix.

## 거부한 대안
- **Relative to the preset file** (`./` — the shape of Claude Code plugins · Compose · Cargo) — it fits when the file lives inside a bundle, and does not fit a shared library that many presets share (memo 103 (a) · PRD R3).
- **Reference by name** (Claude Code plugin dependencies · Continue `owner/name`) — it holds only inside a namespace with a clear scope. In a folder browser, the same name in a different folder is common (memo 103 (b) · PRD R3).
- **Fixed ID files** (Unity · Godot) — a heavy engine model that breaks on outside moves (memo 103 (c)).
- **A URI form** (`lib://업무/...`) — URL host normalization mangles non-ASCII library names (memo 126 — rejected by a session · no record of user confirmation). (Basis for rejection: weak) Basis (main 2026-11-01 — reported · M98): the intent of memo 126 (a form that a standard function can parse) is filled by `LibPath` wrapped in `relative-path` · there is one root (ADR-0312), so there is no library name to put in the URI host area · whether host handling changes non-ASCII names was not measured.

## 근거
User reason (memo 102): a skill is shared by many presets (many-to-many) — placing it next to one preset leaves no place for the other presets · a preset contains presets · Engram is a folder browser, so an entry's identity is its place. Browser-shaped peers confirmed (Obsidian vault-root paths etc. — research addendum 2).

## 영향 / 불변식
- Validation, changing disk paths, and changing the prefix on move are handled by one `LibPath` type (wrapped in `relative-path` — TRD §1-2).
- If multiple roots are reopened, only the new root gets a prefix (ADR-0312 Decision 6).
- The key and case-insensitive comparison of the table looked up by path (`Catalog`) are handled by the platform `path_key` · `same_path` (ADR-0325) — `LibPath` strings are not compared as they are.
