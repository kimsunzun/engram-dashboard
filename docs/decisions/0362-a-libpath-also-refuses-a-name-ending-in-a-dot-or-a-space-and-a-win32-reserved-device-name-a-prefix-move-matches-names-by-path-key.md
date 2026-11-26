# ADR-0362: A LibPath also refuses a name ending in a dot or a space and a Win32 reserved device name; a prefix move matches names by path key

- 상태: 확정 (2026-11-13, 근거: main decisions from the S25 step-7 P1 code review (2026-11-11) · landed `42cede81` (P1) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` §12 「ADR-0315 amendment」)
- 관련: Amends ADR-0315 (decision 2, the LibPath name rules) · ADR-0325 (path key — the platform `path_key`) · ADR-0312 (one root) · ADR-0335 (place = identity; references follow a move)

## 맥락
ADR-0315 decision 2 lists what a `LibPath` (a library reference — the `/` path from the library root) refuses: a leading or trailing `/`, an empty name, `.` and `..`, `\`, `:` and control characters. The step-7 P1 review found two spellings it lets through that reach a different place on Windows than the one spelled:
- Win32 trims a trailing `.` or space from a name, so `rules/a.md.` opens `rules\a.md`, and `.. ` (dot, dot, space) names the root's parent, not an entry called `.. `.
- A Win32 reserved device name (`CON`, `NUL`, `COM1`, `nul.md`, `COM1.txt` …) makes `LibPath::to_path` open a device instead of a file.
The same review found that `replace_prefix` (the reference rewrite when a file or folder moves) compared leading names as strings, while ADR-0315 「영향」 says paths are compared through the platform `path_key` (ADR-0325), not as spelled. On Windows `Team/rules/a.md` is below `team/rules`, but a string match would leave it behind on a move.

## 결정
1. **`LibPath` also refuses a name that ends in `.` or a space**, on every OS.
2. **`LibPath` also refuses a Win32 reserved device name**, on every OS — the rule of platform `fs::is_reserved_device_name` (shared with platform `normalize_folder`): `CON` · `PRN` · `AUX` · `NUL` · `COM1`–`COM9` · `LPT1`–`LPT9` and the superscript `COM¹ COM² COM³ LPT¹ LPT² LPT³`, ASCII case-insensitive, with any extension and with spaces before it (`nul.md`, `Com1 .tar.gz`, `COM¹.md`). It is stricter than Win32 for a few spellings, never looser; `CONSOLE`, `nulx.md`, `com10.txt` are allowed.
3. **`replace_prefix` matches the leading names name by name by `path_key`**; the answer is the target as spelled followed by the rest of the reference as spelled.
Everything else in ADR-0315 decision 2 stays (UTF-8, `/` only, no root prefix, the refused characters and names listed there).

## 거부한 대안
- **Keep ADR-0315 decision 2 as written** — a stored reference could open a different file than it names (`a.md.` → `a.md`), climb out of the root (`.. `), or open a device (`NUL`). Measured by the P1 tests on the refused spellings (asset-library `path.rs` tests).
- **Match `replace_prefix` leading names as strings** — a move of `team/rules` would leave `Team/rules/a.md` pointing at the old place on Windows, although both name one folder; ADR-0315 「영향」 already requires path comparison through `path_key` (ADR-0325). Pinned by the P1 test `replace_prefix_matches_leading_names_by_path_key_and_keeps_both_spellings`.

## 근거
- Main decisions from the P1 code review (2026-11-11), recorded in the P1 commit `42cede81` ("LibPath also refuses names ending in '.' or a space and Win32 reserved device names; replace_prefix matches leading names by path_key … amending ADR-0315 decision 2 — ADR in the step-7 doc pass") and child TRD §12.
- Code: asset-library `path.rs` `LibPath::parse`, `LibPath::replace_prefix` and their tests; platform `fs::is_reserved_device_name`.

## 영향 / 불변식
- The rules hold on every OS, so a library made on one OS stays usable on Windows; the code carries no OS `cfg` for them.
- A reference written by hand with one of the refused spellings is refused when it is parsed (`PathError`), so it never opens a file or device it does not name.
- Every reference rewrite on a move or rename (9a / 9b) goes through `replace_prefix`, so it follows `path_key` like the catalog lookup.
