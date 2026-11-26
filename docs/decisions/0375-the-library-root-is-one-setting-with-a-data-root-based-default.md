# ADR-0375: The library root is one setting whose default value is the data-root-based place, stored as the word default and resolved from the data root on every start

- 상태: 확정 (2026-11-17, 근거: S25 step-9a TRD §8 (c) U-D1 — user 2026-11-17 「A,B라고하면 당연히 B지」 and 「그냥 동일룰이라잖아. 기본 라이브러리라는 개념은 없어. 단지 디폴트 경로가 그거일 뿐이지」, PRD+TRD review PASS 2026-11-17 (full)) · 부분 폐기 by ADR-0381 (decision 4 first start order) · 부분 폐기 by ADR-0382 (first start order) · 부분 폐기 by ADR-0400 (decisions 1, 2 and 4 one root setting)
- 관련: Amends ADR-0312 (decision 5 recording the root) · ADR-0366 (decision 3: `set_root` and the first start go through `change_root`) · ADR-0376 (`library.json` follows the base data-file rules) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §2 · §3.1 · §3.2 · §6 · §7 U1 · §9 U1 · §8 (c) U-D1 · parent TRD `docs/process/S25-preset-library/trd.md` §1-4 「Root · default path」 · Amended by ADR-0381 (decision 4 first start order) · Amended by ADR-0382 (first start order) · Amended by ADR-0400 (decisions 1, 2 and 4 one root setting)

## 맥락
ADR-0312 decision 5 says that at first start, with no root, an empty `<배포판>\library` is created and recorded as the root. Step 9a makes the root persistent (`<data>\daemon\state\library.json`) and so has to decide what is recorded. Recording the absolute path of that place ties the library to where the distribution was at first start. The user decided (U-D1) that there is no separate 「default library」 concept: the library root is one setting, and the data-root-based place is only its default value.

## 결정
1. **The library root is one setting whose default value is the data-root-based default place** (`DataLayout::default_library_root`). There is no separate 「default library」 concept.
2. **Persisted shape**: `library.json` holds `{"schema_version": 1, "root": "default" | "<absolute normalized path>"}`. `"default"` means the setting holds its default value; any other value is an absolute path the user set. `"default"` can never be read as a path (it is not absolute). Normalization = platform `normalize_folder`.
3. **The default value is resolved on every start** from the data-root origin to that run's `default_root`: release default ⇒ `<data root's parent>\library`; otherwise ⇒ `<data root>\library`; never guessed from a directory name (ADR-0312 decision 5, parent L236).
4. **First start writes the default value** (`"root": "default"`), not the absolute path.
5. **`set_root` to the default place stores the default value**: a path whose `path_key` equals this run's `default_root` is stored as `"default"` (the same as never changed); any other path is stored absolute.
6. **The folder's existence never changes the setting.** Deleting the folder, or re-creating it at the configured place, leaves the setting as it is. A configured place missing at start is `missing` and is not re-created — the same rule whether the setting holds its default value or an explicit path (ADR-0312 decision 5, ADR-0366 decision 3, unchanged). Only a missing `library.json` triggers the first-start step.

**Clause that changes:**
- **ADR-0312 decision 5** — 「record it as the root」 becomes 「write the setting at its default value」: what is recorded is the word `default`, resolved from the data root on every start, not the created folder's absolute path. The rest of decision 5 stays: an empty folder is created at the default place at first start; development and test follow the `DataLayout` rule and `ENGRAM_DATA_DIR` isolation (G35); a creation failure falls back to the unset state (M24-2); 「라이브러리 폴더 바꾸기」 (R9) stays.

## 거부한 대안
- **Record the absolute path at first start** (ADR-0312 decision 5 as written) — a moved distribution reads `missing`, and a copy of the distribution shares the original's library. The user chose the setting form (U-D1). Basis: descriptive (weak), 사용자 직접 기각 (U-D1).
- **Treat a re-created or re-chosen default place as an explicit path** — the user ruled that there is one rule and no default-library concept; the default place is only the setting's default value (「그냥 동일룰이라잖아. 기본 라이브러리라는 개념은 없어. 단지 디폴트 경로가 그거일 뿐이지」). Choosing the default place again is the same as never changing it (TRD §2, §8 (c)). Basis: descriptive (weak), 사용자 직접 기각 (U-D1).

## 근거
- User decision U-D1 (2026-11-17), quoted in the 상태 line; recorded in the step-9a TRD §8 (c) and applied in §2, §3.1, §3.2, §6, §7 U1 and §9 U1.
- The step-9a PRD+TRD review (full, Claude lead + codex) passed on `43f64c4f`. The U-D1 wording entered in draft 3 after that PASS and was closed by main's diff check, not re-reviewed (TRD §13).

## 영향 / 불변식
- **User-visible**: moving the distribution (or data) folder keeps the library — while the setting holds its default value, it is found in the moved folder; a copy of the folder uses its own `library`. A root the user set elsewhere stays at its absolute path (TRD §6).
- G39 still holds: a distribution rebuild deletes `<dist>\library` (ADR-0312 「영향」).
- `library.json`'s shape check refuses a document whose `root` is neither `"default"` nor an absolute path string (TRD §2).
- `DataLayout` carries the data-root origin and computes `default_library_root()` without creating anything; the origin comes from the branch `data_dir::default_data_dir` already takes, returned by a sibling function (TRD §2, B22). The shell's copy (`src-tauri/src/discovery/layout.rs`) is unaffected.
- `status` answers `root_setting` = `default` (the file says `"default"` or is not written) · `explicit`, absent when `library.json` could not be read (TRD §3.1).
- Tests that must hold (TRD §9 U1): the first start writes the default value, not a path; the default value resolves to the new place after the data folder moves while an explicit root stays the absolute path; `set_root` to a path whose `path_key` equals the default place stores the default value; a configured root that vanished is `missing`, not re-created, the setting unchanged. QA mutations: the first start writing the absolute path; `set_root` to the default place storing it absolute; the default place guessed from a name.
- Implemented in step 9a U1 (asset-library `config.rs`, daemon `data_dir.rs`); `docs/reference/structure/asset-library.md` describes the root setting from U1 on.
