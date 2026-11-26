# ADR-0358: Preset YAML edits rewrite the whole file with serde-saphyr and do not keep hand-written comments (yaml-edit dropped)

- 상태: 확정 (2026-11-11, 근거: user 「그냥 주석 없어도 되니깐 표준으로 하자.」 — S25 step 7 child TRD `docs/process/S25-preset-library/trd-asset-library.md` decision U6)
- 관련: Supersedes ADR-0333 · G48 (2026-10-31) · Amends ADR-0336 (YAML rewrite row and serde-saphyr features) · Amends ADR-0338 (decision 6 writer)

## 맥락
ADR-0333 chose `yaml-edit =0.3.5` to keep hand-written comments and shape when the daemon edits a preset file. While writing the step-7 child TRD the license gate came up: the published `yaml-edit` package and its upstream repository carry no license text (the crates.io metadata says Apache-2.0), so the repository's license gate (`ci.yml`, `scripts/licenses/`) fails unless the crate is listed as synthesized. `yaml-edit` is used in one place only — the preset edit writer (`format::edit` in asset-library). Facts shown to the user (crates.io, 2026-11-11): created 2025-08, about 296k downloads, 26 reverse dependencies, one owner. G48 had already said comment preservation is not worth separate effort.

## 결정
1. Every preset edit — attach · detach · reorder · tag change · the screen's [Save] (`preset.save`) · the LLM/CLI `preset.*` operations — reads the file with serde-saphyr, applies the change, and writes the **whole file** with serde-saphyr's serializer. Hand-written comments, blank lines and quoting style are not kept.
2. `yaml-edit` is not a dependency. ADR-0333's three conditions (reorder detour, empty flow list, the write gate tied to `yaml-edit`) and its refusal code fall away with it.

## 거부한 대안
- **Keep `yaml-edit` and list it in `scripts/licenses/synthesized-crates.txt`** (the `siphasher` precedent) — the user chose not to depend on a small, single-maintainer crate whose package ships no license text, for a feature (comments) the user does not need (약함 · 사용자 직접 기각).
- **Hold the dependency until upstream adds a license file** — no date; the edit path (step 7 P4, step 9a edits, `preset.save`) would stay blocked (사용자 직접 기각).
- **`yamlpatch`** — rejected in ADR-0333 on measurement (rustc 1.97 requirement, a panic on Hangul end-of-line comments, wrong values on map → string, no middle insert or reorder).

## 근거
- User decision 2026-11-11 (U6) with the facts above.
- serde-saphyr 2.0.0 is already the reader (ADR-0336) and provides `to_string` / `to_string_with_options` under its `serialize` feature (`src/ser/api.rs`), so no new crate is added.
- G48 (2026-10-31): 「보통 메모 지킴은 맞는데 그걸 위해 별도로 노력할 필요는 없음」.

## 영향 / 불변식
- ADR-0336: the YAML rewrite row becomes serde-saphyr's serializer, and the serde-saphyr feature list gains `serialize` (it enabled only `deserialize`).
- ADR-0338 decision 6: a save is still computed from the current file on disk and still answers `NOT_FOUND` for a moved, renamed or deleted preset; only the writer changes, and the `yaml-edit` write-gate refusal code is gone.
- No new license-gate entries (`rowan` · `countme` · `text-size` are not added).
- A rewrite must not lose data other than formatting: keys Engram does not model must survive an edit. How (write from the parsed document tree, not from a typed struct) is the step-7 TRD's call; its tests must cover an unknown key surviving an edit.
- To bring comment preservation back, reopen this decision — reading stays as it is and only the writer changes.
