# ADR-0333: Preset YAML editing preserves hand-written comments · shape — `yaml-edit =0.3.5` + three conditions

- 상태: 제안 (2026-10-31, basis: TRD decision T2 (2026-10-18 convention) · user 「없으면 관행대로 처리해」 · G48 (2026-10-31 「보통 메모 지킴은 맞는데 그걸 위해 별도로 노력할 필요는 없음 … 어쨋든 그 라이브러리에 맞게 하면 될것같음」) + main M61 (0b spike) · M68 (second measurement) · M73 (report) · spike `docs/research/yaml-comment-preserving-edit-spike-2026-10-31.md`)
- 관련: ADR-0338 · ADR-0318 · ADR-0336 (reading = serde-saphyr) · TRD §1-4 `format.rs` · §1-12 · §5 0b · §7 27 · earlier convention research `docs/research/yaml-comment-preserving-edit-2026-10-18.md`

## 맥락
When the daemon edits a preset file (attach · detach · reorder · tags), do the comments and shape a person wrote remain? The 0b spike measured two editors on a preset of the real shape with 14 hand-written comments and four lists (a new copy per operation · `diff` · re-reading with serde-saphyr to compare values · counting CRLF/LF).

## 결정
1. Reading = serde-saphyr · writing = `yaml-edit` **pinned with `=0.3.5`** (M61) — behavior changes with each patch release (between 0.3.2 and 0.3.5 the results of five operations changed · the three releases 0.3.3~0.3.5 came out within two days).
2. Three conditions (M61):
   ① **Reorder = detach + write out the detached node's text outdented by the node's original content column (CST offset) and re-read it and insert** (comments · blank lines are skipped — M73 · the old fixed column was wrong for the no-indent form · the 4-space form (M68), and 「둘째 줄부터 공통 들여쓰기」 was also replaced by M73) — there is no move API, and in 0.3.5 putting the detached node back as it is turns a map entry into YAML that cannot be re-read (a regression of what worked in 0.3.2). The workaround worked in measurement (the end-of-line comment follows too). Rebuilding with a new value and inserting gives the right value but loses that entry's end-of-line comment.
   ② **If the source is an empty flow list `[]`, replace it with a new block `Sequence` at the first item** — if inserted as is, it remains as the flow form `[x]`.
   ③ **Golden test + the gate of `format.rs`** — before writing, re-read the output with serde-saphyr and compare it with the intended value, and if it differs do not write and refuse (decision 4 — a stable code + the name of the anchor involved · no fallback to a serde rewrite · G48). This gate caught all the broken outputs seen in the spike.
3. The fallback place of TRD 0b (rewrite with serde and lose hand-written comments) is not used.
4. **If the write gate refuses, we refuse** — answer with a stable code + the name of the anchor involved as arguments, and do not fall back to a serde rewrite (G48 · U3 (a) — the main session reported 「별도로 노력할 필요는 없음」 translated this way). In the second measurement, all the gate refused was the editing of values written split by anchors · aliases (2/220 · 0/200 when there is no anchor — M68).
5. **Nothing more is done for comment preservation** (G48) — the three conditions are all (the write gate that compares values stays because it guards the correctness of values, not comments). Known gaps (accepted — G48): ① the comment lines **above** a detached item stay in place and read like the next item's comment · for a middle insert too, the new item goes in under the comment line above the next item (M61) ② reordering inside a flow list can lose the end-of-line comment of the moved item ③ editing a value in place can lose the quote form (U4 · M68 · M74).

**Closed question (G48):**
- When an item is detached, the comment line **above** it stays in place and reads like the next item's comment (common to both editors · the end-of-line comment goes along — M61) — accepted (decision 5 ①). The way to delete the line-above comment together (`yaml-edit`'s comment node API) was not measured and is not built (G48 「별도로 노력할 필요는 없음」).

## 거부한 대안
- **`yamlpatch` 1.31.1** (MIT · zizmor's `crates/yamlpatch`) — it needs rustc 1.97 (`tree-sitter-iter` — the repository pin is 1.95.0 · it was built only with `--ignore-rust-version`) · if an end-of-line comment contains Hangul, detaching the last remaining item panics (with ASCII it deletes the `settings:` key too) · for map → string it silently writes a wrong value · it has no insert-in-the-middle · reorder operations (replacing the whole list loses four comments inside the list) · LF gets mixed into a CRLF file · 45 dependencies (including tree-sitter-yaml C) (M61 · spike).
- **`yaml-edit` 0.3.2** (the version the TRD had written) — it refuses a document with a head comment · appending at the end of a map goes in at 0 spaces of indentation and changes the value · string → map becomes YAML that cannot be re-read · LF gets mixed into CRLF (spike). Raise to 0.3.5 and pin the version.
- **Rewrite by a serde round trip and accept losing comments** (the old T2 recommendation) — convention preserves them (§7 27). The spike passed, so it is not used as a fallback either (M61).
- **Rescue only the edits the write gate refused with a serde rewrite** (U3 (b) — with a notice that comments are lost · confirmation) — G48 (「별도로 노력할 필요는 없음」 — the main session reported it translated as 「물러날 길 없음」) · second measurement: the refusals are only the edits of values written split by anchors (2/220) and rescuing them would lose all the comments of the file (M68).

## 근거
- T2 · the 2026-10-18 convention (ruamel.yaml etc. — round-trip preservation is the reason they exist).
- Spike (2026-10-31): on `yaml-edit` 0.3.5, append at the end (string · map) · change a tag value · map → string · `description` · first item into a valueless `settings:` · detach the last remaining item → `settings: []` · CRLF (all CRLF, one kind) · Hangul comments · values = ✅, and untouched lines are byte for byte.
- Second measurement (M68 — 22 shapes × 10 operations): gate refusals 2/220 (0.9%) — only edits of values written split by anchors · aliases · 0/200 when there is no anchor · wrong values that passed the gate 0 · no-indent form (PyYAML default) · 4 spaces · one-line flow list · BOM/CRLF · tab · `---`/`...`/`%YAML` passed · the gate must compare values (a read that looks only at the schema passed three of the four refused).
- User decision G48.

## 영향 / 불변식
- When the version is raised, re-run the golden test and the gate of `format.rs` — the version pin (`=`) is a condition of this decision.
- License: the crates.io metadata says Apache-2.0 but there is no LICENSE file at the root of the repository (`jelmer/yaml-edit`) — noted (M61). Declared MSRV 1.70 · new packages = rowan · countme · text-size.
- Measurement scope: first = one sample · second = 22 shapes × 10 operations (M68 — no-indent form · 4 spaces · flow list · tab · BOM/CRLF · including anchors). Shape problems the gate cannot see (appending at the end of a multi-line flow list · the place of a middle insert · decision 5 ② ③) are written down as known (M68). The golden adds a head comment with 0 spaces between items and an inner item with a single key (M73).
- To change the editor, reopen this decision — reading (serde-saphyr) stays as it is and only the writing path changes.
