# Comment-preserving YAML editing — yaml-edit · yamlpatch measured (preset files)

| Item | Value |
|---|---|
| Status | Measurement complete · editor recommendation (awaiting main session confirmation) |
| Date | 2026-10-31 |
| Method | Spike — in a separate cargo project in the session scratch (outside the repository), took one set of preset samples and edited a fresh copy per operation, compared `diff -U1` with the original · re-read the result with serde-saphyr 2.0.0 and checked it against the expected value (the value obtained by reading the original and applying the same operation with serde_json) · a typed read (path string \| `{path, backend}` untagged) too · counted 14 hand-written comments as kept/lost/duplicated · counted line endings (CRLF/LF). rustc 1.95.0 · Windows 11 · MSVC |
| Versions | `yaml-edit` **0.3.5** (latest · 2026-10-22) — the same table was run on 0.3.2, which the TRD wrote, for comparison · `yamlpatch` **1.31.1** (latest stable · 2026-10-15 · 1.32.0-rc1 not looked at) + `yamlpath` 1.31.1 · re-read = `serde-saphyr` 2.0.0 (the TRD's reading crate) |
| Question | TRD `S25-preset-library/trd.md` §5-1 0b — when Engram edits a hand-written preset file (append to the end of a list · insert in the middle · remove · reorder · change a map field), do the hand comments · shape survive |
| Deliverable | Scratch (not kept) — this document is the canonical result |

## Conclusion

- **Pick = `yaml-edit` 0.3.5 — with three conditions attached.** Append at the end · insert in the middle · remove · change a tag (map field value · string↔map) · `description` · empty list · CRLF · Korean comments all keep the hand comments with no loss, and the re-read values are correct too. Conditions:
  1. **There is no API usable as is for reordering, and the natural route (re-inserting the removed node) produces YAML that cannot be re-read at a map entry** (`backend:` double indentation — what worked in 0.3.2 broke in 0.3.5). The workaround = strip the indentation from the removed node's text from the second line on and re-read and insert it — it worked in measurement (the end-of-line comment of that entry follows along). Re-creating it as a new value and inserting it is also correct but loses that entry's end-of-line comment.
  2. If inserted into a list whose source is `[]`, it remains in the flow shape `[x]` → swap it in with a new block `Sequence` (measured ✅).
  3. Behavior changes with each patch version (five operation results differed between 0.3.2 and 0.3.5 · comparison below) → pin it with `=0.3.5`, and put a gate in `format.rs` that **re-reads with serde-saphyr before writing and checks against the expected value** (if they diverge, do not write — what to do then is an implementation decision). The broken outputs caught this time were all of the kind this check catches.
- **`yamlpatch` 1.31.1 does not work.** (1) It has no insert-in-the-middle or reorder operation (replacing the whole list with `Replace` = loses 4 comments inside the list) (2) string → map is rejected as broken YAML, and **map → string silently writes a wrong value** (asked to remove the tag, it remained as a `- path: …` map, and `backend` · the end-of-line comment disappeared · also reproduced in a pure-ASCII minimal document) (3) removing the last remaining entry **panics** if its end-of-line comment contains Korean (`line-index` invalid offset) · if ASCII, it deletes the whole `settings:` key (4) appending at the end of a CRLF file mixes in LF (5) its dependency requires rustc **1.97** (`tree-sitter-iter` ≥ 1.28 · repository pin 1.95.0) — here it was built only with `--ignore-rust-version`.
- **Common limit of both crates — the comment above an entry being removed stays.** The end-of-line comment goes with it, but the comment line above the entry stays in place and looks like the next entry's comment. Neighboring comments are untouched. For the same reason, insert-in-the-middle of `yaml-edit` puts the new entry **under the next entry's above-line comment**. Whether to accept that is up to the user — otherwise `format.rs` would have to handle comment lines separately, and that API was not measured.
- The fallback in TRD 0b (rewrite with serde and lose hand comments) is not needed — with the three conditions above it stands on `yaml-edit`.

## Operation × crate

✅ = value · comments · shape all correct · △ = value correct but comment position or shape off (or a workaround needed) · ✗ = no operation · error · wrong value · broken output

| Operation | yaml-edit 0.3.5 | yamlpatch 1.31.1 |
|---|---|---|
| 1 Append at end — string (`skills`) | ✅ Right under the last entry (before the blank line · the end-of-file comment) | △ Attached **after** the blank line · right above the end-of-file comment (a blank line inside the list) — value correct |
| 1 Append at end — map `{path, backend: codex}` (`instructions`) | ✅ Block map · indentation correct | ✅ Same result |
| 2 Insert in the middle (after `instructions` [0]) | △ `insert(1, …)` — value correct · the new entry goes under the next entry's above-line comment, so that comment looks like the new entry's | ✗ No operation · replacing the whole list with `Replace` = loses 4 comments inside the list |
| 3 Remove — string (above-line comment + end-of-line comment) | △ The entry line + end-of-line comment disappear · **the above-line comment stays** · neighboring comments untouched | △ Same result (identical down to the characters) |
| 3 Remove — map entry (above-line comment + `path` end-of-line comment) | △ Same shape | △ Same result |
| 4 Reorder (`instructions` [1] to the end) | ✗ Does not work as is — removed node `push`/`insert` = **YAML that cannot be re-read** (`backend:` 6 columns) · moving a string node = loses the end-of-line comment · `set` at each position = comments stay in place and one is lost. **Workaround ✅** node text → strip 4 columns from the second line → re-read and `push` = value correct · end-of-line comment follows (the above-line comment stays) | ✗ No operation · `Remove` + `Append` = value correct · loses the end-of-line comment · replacing the whole list with `Replace` = loses 4 comments · `Replace` at each position = error (the string → map defect below) |
| 5 Change a tag value (`backend: claude` → `codex`) | ✅ End-of-line comment as is | ✅ Same result |
| 5 String → map (attach a tag) | △ Value correct · the end-of-line comment stays but moves to the dash line (map starts on the line after `-      # 커밋 끝 주석`) | ✗ Error — produces broken YAML and rejects (even in an ASCII minimal document) |
| 5 Map → string (remove the tag) | ✅ The `path` end-of-line comment stays on the same line (corrected in the 2nd measurement) | ✗ **Silently wrong value** — remains as a `- path: 지침/codex 보고 형식.md` map and `backend` · the end-of-line comment disappears |
| 6 Change `description` | ✅ End-of-line comment as is | ✅ Same result |
| Empty list `settings: []` → first entry | △ `push` = `[설정/첫 설정.yaml]` flow shape · ✅ if swapped in with a new block `Sequence` (key-line comment as is) | △ `Append` = error (flow list rejected) · `Replace` = becomes a block but the key-line comment moves to the entry line |
| Value-less `settings:` → first entry | ✅ Insert a new block `Sequence` · key-line comment as is | △ `Append` = error · `Replace` = the key-line comment goes to the entry line |
| Remove the last remaining entry → empty list | ✅ `settings: []` (that entry's end-of-line comment disappears with it) | ✗ **panic** (end-of-line comment in Korean) · if ASCII, deletes the `settings:` key entirely (yamlpath's 「빈 블록 부모를 위로 접는다」 design) |
| CRLF input (1 · 2 · 3 · 4 · 5 string→map · 6) | ✅ All results are CRLF only · stripping the CR gives bytes identical to the LF result (the natural route of 4 breaks the same as LF) | △ Append at end = LF mixed in (CRLF 30 + LF-only 1–2 — even one original line loses its CR) · remove · `description` = CRLF kept |
| Korean comments · values | ✅ No problem | ✗ Remove-last-remaining-entry panic (above) · otherwise no problem |
| Re-read (serde-saphyr 2.0.0) | Only the reorder without the workaround (re-inserting the node) fails — all the rest equal the expected value | Among those that produced output, only map → string differs (wrong value) |
| Untouched lines | Byte for byte (including quotes `"공용/팀 규칙.yaml"` · alignment whitespace — confirmed by diff) | Same |

**Representative output (diff)** — yaml-edit 0.3.5 reorder natural route (cannot be re-read · `comment intercepting the multiline text`):

```diff
     backend: claude          # 꼬리표 끝 주석
+  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
+      backend: codex
```

yamlpatch map → string (silently wrong value):

```diff
   # codex 쪽 보고 형식 — 꼬리표 달린 항목
-  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
-    backend: codex
+  - path: 지침/codex 보고 형식.md
```

## yaml-edit 0.3.2 comparison (the version the TRD wrote)

- `Document::from_str` rejects a file with a head comment (「stream-level comments … Use YamlFile::from_str()」) — 0.3.5 accepts both. The comparison table was run with `YamlFile::from_str` for both versions (the 0.3.5 result is the same as running with `Document`).
- Worse than 0.3.5: map append at end = `backend:` went to 0 columns and changed the value (re-read differs) · string → map = YAML that cannot be re-read · remove = the end-of-line comment is left as a line on its own · LF mixed into CRLF.
- Better than 0.3.5: moving a map node (re-inserting the removed node) worked → it broke in 0.3.5 (a regression).
- → 0.3.3 ~ 0.3.5 came out as three in the two days 10-21 ~ 10-22, and behavior changes a lot between versions. Exact version pinning + golden tests + a re-read check before writing are needed (conclusion condition 3).

## Crate facts

| | yaml-edit | yamlpatch |
|---|---|---|
| License | Apache-2.0 (crates.io metadata) · no LICENSE file at the GitHub repository root (GitHub detection = none) | MIT |
| Repository · activity | `jelmer/yaml-edit` · ★12 · 10 forks · 1 open issue · last push 2026-10-22 · ≥ 100 commits since 2026-08-01 (one-page cap of the API) | `crates/yamlpatch` of `zizmorcore/zizmor` (★6.8k · 172 open issues · last push 2026-10-30) · 3 commits to that folder since 2026-08-01 · released with the same version number as zizmor |
| Recent versions | 0.3.5 10-22 · 0.3.4 · 0.3.3 10-21 · 0.3.2 09-17 · 0.3.1 08-31 · 0.3.0 08-25 · 0.2.3 07-01 | 1.31.1 · 1.31.0 10-15 · 1.30.1 09-09 · 1.30.0 08-30 · 1.29.0 08-01 · (1.32.0-rc1 10-23) |
| Downloads · reverse dependencies | 249k total · 207k in the last 90 days · 21 reverse dependencies (wasmer-cli · lintian-brush · debian-lsp, etc.) | 400k total · 204k in the last 90 days · 7 reverse dependencies (zizmor, etc.) |
| MSRV · Windows build | Declared 1.70 · built with rustc 1.95.0 + MSVC (does not trip the rust-version check) | None declared · the dependency `tree-sitter-iter` 1.31.1 declares **1.97.0** → `cargo build` is rejected on rustc 1.95.0 · it builds with `--ignore-rust-version` (about 30 seconds · C compilation = tree-sitter · cc). The last version where `tree-sitter-iter` declared 1.88 = 1.27.0 → yamlpatch 1.27.0 would stand on 1.95 (not run) |
| Dependency weight (`cargo tree -e normal,build --target all` · unique including itself) | 7 — rowan · countme · hashbrown 0.14 · rustc-hash · text-size · base64 (default feature `binary` — can be turned off) | 45 — tree-sitter · tree-sitter-yaml (C) · cc · yaml_serde · libyaml-rs · regex · tracing · serde_json · indexmap, etc. |
| Already in the repository's `Cargo.lock`? | rowan · countme · text-size absent (newly brought in) | tree-sitter · tree-sitter-yaml · yaml_serde · line-index absent |

## Sample used in the measurement (LF · UTF-8)

As instructed, the list order is `presets` · `instructions` · `settings` · `skills` (in the TRD §1-2 example, `skills` is before `settings` — unrelated to editing behavior).

```yaml
# PR 리뷰 프리셋 — 손으로 쓴 머리 주석
# 두 번째 머리 줄 (Engram 이 고쳐도 남아야 한다)
kind: preset
version: 1
description: PR 리뷰 전용   # 한 줄 설명

presets:
  - 공용/기본 규칙.yaml      # 공용 바닥
  # 아래는 팀 공용
  - "공용/팀 규칙.yaml"

instructions:
  - 지침/리뷰 규칙.md
  # codex 쪽 보고 형식 — 꼬리표 달린 항목
  - path: 지침/codex 보고 형식.md   # codex 에이전트에만
    backend: codex
  - 지침/커밋 메시지.md      # 커밋 끝 주석
  - path: 지침/claude 전용.md
    backend: claude          # 꼬리표 끝 주석

settings:
  - 설정/claude opus.yaml    # 설정 줄 주석

skills:
  - 스킬/리뷰/review         # 리뷰 스킬
  # lint 위 주석 — 이 항목 몫
  - 스킬/리뷰/lint           # lint 끝 주석
  - 스킬/공용/commit

# 파일 끝 주석
```

Variants: CRLF = the same text with `\n` → `\r\n` · empty list = the two `settings` lines turned into `settings: []   # 설정 줄 주석` · no value = `settings:   # 설정 줄 주석`.

Operation targets: 1 = `스킬/공용/새 스킬` to `skills` · `{path: 지침/새 codex 지침.md, backend: codex}` to `instructions` / 2 = after `instructions` [0] put `지침/새 지침.md` / 3 = `skills` [1](lint) · `instructions` [1] (codex map) / 4 = `instructions` [1] to the end / 5 = `instructions` [3] `backend` → `codex` · [2] to `{path, backend: claude}` · [1] to a string / 6 = `PR 리뷰 · 보안 점검`.

## Notes

- One sample · one indentation shape (`  - ` indented 2 columns under the key). The shape with the dash at the same column as the key (0 columns `- x`) · hand-written flow lists · tabs · BOM · anchors · multiple documents were not run.
- The reorder workaround (stripping indentation) was measured on only one map entry of this sample (content column 4), LF only — generalizing to read the column from the node · CRLF was not done.
- The yamlpatch results were built on rustc 1.95 with `--ignore-rust-version` — not run at the declared MSRV 1.97. Older versions such as 1.27.0 were not run either.
- The expected value of the re-read check is the value from reading the original with serde-saphyr and applying the same operation with serde_json — it is a semantic comparison. Shape (comment position · blank lines · indentation) judgments were made by eyeballing the diff.
- A side fact of the typed read: if `settings:` (no value) is read as `#[serde(default)] Vec`, serde-saphyr 2.0.0 accepts it as an empty list (not an error).
- The route of deleting together the above-line comment of the removed entry (the comment node API of yaml-edit, etc.) was not measured.

## Spike code

Session scratch `C:\Users\kimsunzun\AppData\Local\Temp\claude\…\scratchpad\yaml-spike\` (outside the repository · not kept) — `src/main.rs` (operation table · re-read check · comment count · line-ending count) · `src/bin/dbg.rs` (narrowing down the yamlpatch defect — ASCII · Korean minimal documents). No code · dependency was added to the repository.

## 2nd measurement (2026-10-31)

| Item | Value |
|---|---|
| Occasion | Review finding — if the re-read check before writing (conclusion condition 3) often rejects on real file shapes, presets cannot be edited from Engram |
| Target | `yaml-edit` =0.3.5 only · re-read = serde-saphyr 2.0.0 · added `src/bin/shapes.rs` to the same scratch project (outside the repository) |
| Operations (10 per shape) | Append at end (string `skills` · map `instructions`) · insert in the middle · remove (string · map — both with above-line · end-of-line comments) · reorder (1st-round workaround — generalized the indentation columns from a fixed 4 to **the common indentation from the second line on**) · change a tag value · string → map · map → string · `description`. For reference, the natural reorder (re-inserting the removed node) was also run but left out of the count |
| Verdict | Gate rejection = re-read value ≠ expected value (or could not be read) · death = panic · `Err` · for the rest, the same line diff as the 1st round was filtered with automatic flags (comment disappeared · new indentation column · BOM · line ending · document marker · bracket count · changed line count) and then **all of it was eyeballed** (10 of 20 automatic flags were false positives — e.g. bracket count changing when inserting and removing a map entry · 4 △ that the automatic flags missed and 6 notation mixes were caught by eye) |

### Result — shape × 10 operations

✅ = value · comments · shape correct (the same result as the 1st-round baseline shape — the three common △ of the 1st round = insert in the middle goes under the next entry's above-line comment · removing leaves the above-line comment · in string → map the end-of-line comment goes to the dash line — remain in every block shape, so are not counted again) · △ = value correct but a shape divergence specific to that shape · ✗ = gate rejection · deaths are 0

| Shape | ✅ | △ | ✗ | Note |
|---|---|---|---|---|
| 1a Dash at 0 columns (`key:\n- a` · 0-column comment · map continuation 2 columns) | 10 | 0 | 0 | The new entry also has a 0-column dash · map continuation 2 columns. No normalization needed |
| 1b PyYAML `yaml.dump` default output as is (0 columns · sorted keys · non-ASCII `"\uXXXX"` · no comments) | 10 | (6) | 0 | Value · indentation correct. Newly written text (append at end · insert · string↔map · `description`) is UTF-8 as is → the notation mixes with the neighbors' `\u` escapes (6 in parentheses = notation only) |
| 1c 0 columns · no blank lines · a 0-column comment right after the end of the list | 10 | 0 | 0 | Append at end goes in **before** that comment |
| 2 List indented 4 columns (Go yaml.v3 default shape) | 10 | 0 | 0 | New entry 4 columns · map continuation 6 columns |
| 3a One-line flow `[a, b]` (including map entries `{path, backend}`) | 10 | 0 | 0 | Flow shape kept · key-line end-of-line comment kept. A new map is also `{…}` |
| 3b Multi-line flow (an end-of-line comment per entry · trailing comma) | 6 | 4 | 0 | △ = append at end (attached on **the same line** as the last entry · for a map `, {…}]` at the head of a new line) · insert in the middle (on the same line as the next entry) · reorder (the moved entry's end-of-line comment disappears + the same kind of divergence) |
| 3c Flow map entry in a block list (`- {path: …, backend: codex}`) | 7 | 3 | 0 | △ = map append at end · string → map is written as a **block map** (the neighbors are flow) · reorder loses the moved entry's end-of-line comment |
| 4 Quotes (a path containing single · double · `\"` · `\uC608` · `''` · `: ` · `#`) | 8 | 2 | 0 | △ = the quotes of **a value changed in place**, like the tag value · `description`, **are stripped** (`'claude'` → `codex` · `"PR 리뷰 전용"` → plain text). Untouched quoted entries are byte for byte |
| 5a BOM + LF | 10 | 0 | 0 | BOM kept · serde-saphyr reads the BOM |
| 5b BOM + CRLF | 10 | 0 | 0 | BOM kept · all results CRLF |
| 5c 0 columns + CRLF (the shape of PyYAML text-mode writing on Windows) | 10 | 0 | 0 | Same changed lines as 1a · all CRLF |
| 6 Tab inside a comment · tab before a comment · trailing whitespace · a blank line with only whitespace | 10 | 0 | 0 | Untouched lines byte for byte · an edited line carries its own tab/whitespace. Changed lines same as the 1st-round baseline |
| 7a Anchor/alias in another list (`&base` of `presets` → `*base` of `settings`) | 10 | 0 | 0 | Outside the editing target, so no effect |
| 7b Anchor on an entry of the list being edited (`instructions` [1] `&cx` · `*cx` on `skills`) | 7 | 2 | 1 | △ remove · reorder = yaml-edit **moves the anchor to the alias position and expands the content** (value correct · the alias line turns into three map lines · reorder has two copies of the end-of-line comment). ✗ map → string = the anchor attaches to the new string, so the value at the alias position also becomes a string |
| 7c Merge key (`- <<: *cx` + `path`) | 7 | 2 | 1 | Same shape as 7b · ✗ map → string = the merge target becomes a string and cannot be re-read (`YAML merge value must be mapping`) · change a tag value newly adds a `backend:` key, so ✅ |
| 8a Starts with `---` · ends with `...` | 10 | 0 | 0 | Marker kept · append at end goes before `...` |
| 8b Head comment → `--- # 주석` · `...` right after the last entry · a comment after it | 10 | 0 | 0 | Same |
| 8c `%YAML 1.2` + `---` | 10 | 0 | 0 | Directive kept |
| 9a No final newline (ends with the last list entry) | 10 | 0 | 0 | Append at end normal · the result also has no final newline |
| 9b 0 columns + no final newline | 10 | 0 | 0 | Same |
| 10a `description: \|` block text | 9 | 1 | 0 | △ If `description` is changed to one line, **one blank line after the block also disappears with it** (it is attached as the tail of the block text) |
| 10b Multi-line plain text `description` folded by PyYAML at 80 columns | 10 | 0 | 0 | The new value goes in as one line |
| **Total (22 shapes × 10 = 220)** | **204** | **14 (+6 notation)** | **2** | Deaths 0 · unreadable input 0 |

Side measurements:

- **Editing an entry that has an anchor itself** (change a tag value on [1] of 7b · 7c) = **2/2 gate rejections** — the value at the alias position changes along with it, so it differs from the expected value (only that entry changes). 7a (an entry without an anchor) is ✅.
- **A new value that needs quotes** (`yes` · `123` · `null` · `스킬/a: b` · `#스킬` · ` 앞공백` · `[스킬]` · `PR: 리뷰 #1`) = 8/8 ✅ on the baseline · quote shapes — written with single quotes only when needed.
- **Multi-line value** (`"첫 줄\n둘째 줄"`) = written as a `|-` block ✅ (the end-of-line comment stays on the key line).
- **Natural reorder (the route not to be used)** = ✅ on 0-column shapes (1a · 1b · 1c · 5c · 9b) and the one-line flow (3a) · cannot be re-read on all 2-column/4-column block shapes (12 shapes + baseline) · on multi-line flow · flow map · anchor shapes the value is correct but the end-of-line comment is lost or doubled. **The generalized workaround was correct in value on all 22 shapes** (0 · 2 · 4 columns · flow, all).
- **Precision of the gate** — all 4 gate rejections (2 in the count + 2 on the side) were outputs whose value had actually changed (0 false rejections). Conversely, there was no case of an output with a wrong value slipping through the gate (the ✅ · △ outputs all passed the typed read too). **3 of the 4 rejections passed the typed read** — a gate that only checked schema conformance would have missed them (the value at the alias position changed into a correctly shaped one). A value comparison is needed. **But the shape divergences of △ are not seen by the gate** — the gate compares only values.

### Normalization once — is it worth trying

| Normalization | Target | 10 operations after normalization | Cost (change to the user's shape) |
|---|---|---|---|
| 0 columns → 2-column indentation (character level · top-level list only) | 1a · 1b · 5c | 10/10 ✅ | **All** list lines of the file (24~30 lines) change — but the 0-column shape is already 10/10 without normalization, so nothing is gained |
| Flow → block (rebuilding the list as a new block `Sequence` with yaml-edit) | 3a · 3b | 10/10 ✅ | In 3b, **7 of 11 comments inside the flow disappear** (per-entry end-of-line · above-line comments) · quotes of untouched entries are also stripped (`"공용/팀 규칙.yaml"` → plain text) · a one-line list becomes multiple lines. 3a has only a key-line end-of-line comment so loses no comments (but 3a was 10/10 to begin with) |
| Anchors | 7b · 7c | Not tried | The only option is to expand the aliases — sharing becomes copying (the value is the same). The rejection comes not from a shape that normalization can remove (indentation · flow) but from **the operation of editing a shared value** itself |

### Conclusion (material for the U3 decision — the decision is the user's)

- **Gate rejections are rare — 2 of 220 (0.9%), both when editing an entry that an anchor is attached to.** On the 20 shapes without anchors (0 columns · 4 columns · flow · quotes · BOM · CRLF · tabs · markers · directives · no final newline · block text), **0 of 200**. Looking at only the anchor shapes (7b · 7c), including side measurements, 4 of 22 (map → string on an entry with an anchor · change a tag value on that entry).
- The 「실제 파일 꼴에서 자주 거절」 (frequent rejection on real file shapes) that the review worried about did not appear in this sample. Instead, **the △ that the gate cannot see (value correct, shape diverges) concentrate in shapes** — multi-line flow lists (4/10) · flow map entries (3/10) · in-place change of quoted values (2/10) · removing/moving an anchored entry (the anchor moves to the alias position · 2/10) · block text `description` (1/10).
- Holding the three fallbacks against the data:
  - **Reject (do not write, notify)** — the only case caught in this sample is editing an anchored shared value. In that case whatever is edited, the alias side changes with it, so it does not fit the user intent of 「그 항목만」 (only that entry) to begin with.
  - **Rewrite with serde** — it loses all 14 hand comments of this sample and expands the anchors. The loss is too large to justify saving 2 rejections (the rarer the rejections, the rarer the cases this fallback protects).
  - **Edit after normalization** — it has no use in reducing rejections (the rejection shapes are not indentation · flow). As for reducing △, flow → block for multi-line flow removes 4 cases but loses 7/11 of that list's in-flow comments · quotes.
- Condition 1 of the 1st-round conclusion (the reorder workaround) **stands on all shapes if generalized to read the columns from the node** — the fixed 4-column workaround is wrong on 0-column · 4-column shapes (this time only the generalized one was run).

### 1st-round table correction

- The 「`path` 줄 끝 주석이 새 줄에 남음」 of 「5 맵 → 문자열」 (5 map → string) in the 1st-round table differs from the actual output — the end-of-line comment stays on **the same line** (`- 지침/codex 보고 형식.md   # codex 에이전트에만`). The 1st-round output file (`out_035/yaml-edit/op5c_map_to_plain.yaml`) and this round's output are the same. The verdict (✅) stands.

### Notes (2nd round)

- The samples are hand-made samples built as instructed (not a collection of real-use preset files). The PyYAML output was **reproduced by hand** because this PC has no Python (`yaml.dump` defaults = `sort_keys=True` · `allow_unicode=False` · 0 columns · width 80 — paths are short, so no folding).
- Multiple documents (two or more `---`) · files mixing 0 and 2 columns · tab indentation (forbidden in YAML) · duplicate keys · `!!tag` were not run.
- For anchors · merge keys, only the shape where the anchor is attached to a single `instructions` [1] was measured. For normalization 「흐름 → 블록」 (flow → block), only one kind — rebuilding the list with yaml-edit — was measured (no character-level conversion).
- The expected value of the pre-write check is the same as in the 1st round (the value from reading the original + the same operation applied with serde_json). If Engram builds the expected value from a typed model, it would be comparing against the value with the aliases expanded, so the verdict for anchor shapes is the same.

### Spike code (2nd round)

The same scratch `yaml-spike\src\bin\shapes.rs` (shape generation · operations · gate comparison · automatic flags · LCS line diff · two normalizations) — output `out2\<꼴>\<연산>.yaml` · `out2\diffs.txt`. No code · dependency was added to the repository.
