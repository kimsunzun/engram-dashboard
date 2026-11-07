# Preserving hand-written comments when editing a preset YAML — Rust libraries · practice

- **Status:** light survey · 2026-10-18 · 1 collector (document type) + main-session grounding (checked against docs.rs `yaml-edit`) · no adversarial review · the libraries were not run
- **Question:** When the daemon adds, removes, or reorders items in a list (guidelines · skills · settings · presets) in a preset `.yaml`, rewriting the whole file with serde erases the comments the user wrote by hand (TRD T2). Is there a Rust library that preserves comments · is the practice to preserve them?

## Conclusion

- **The practice is 「지킨다」 (preserve).** When a tool that edits user files wipes comments, complaints follow — a complaint post about the Home Assistant UI editor erasing hand-written comments (cause = the YAML → JSON → YAML round trip) · openHAB 「Main UI widget editor deletes comments」. ruamel.yaml, the de facto standard on the Python side, exists to preserve comments · block shape · key order in a round trip. — likely
- **There is a usable candidate — `yaml-edit`** (jelmer · 0.3.2 · 2026-09-17 · Apache-2.0 · rowan lossless tree). 「parsing and editing YAML files while preserving formatting, comments, and whitespace」 (checked against docs.rs). The confirmed edits = Mapping `set` · `remove` · `rename_key` · Sequence `push` · `set(index)`. **We could not confirm from the docs inserting into the middle of a list · removing · reordering, or how the comment attached to a removed item is handled** → a spike with real preset files is needed before implementation. — uncertain (behavior) · certain (description text)
- **Comparison candidates:** `yamlpatch` (zizmor · MIT · 1.31.1 · 2026-10-15 — Add · Remove · Append · Replace · 「best-effort」 · 「not a substitute for comprehensive YAML processing libraries」) · `noyalib` `Document` (bytes that are not touched, comments included, are preserved · insert/remove unconfirmed · 0.0.x). Not usable: `nondestructive` (「This project is as of yet, incomplete!」).

## Sources

| Item | Source | Confidence |
|---|---|---|
| yaml-edit | https://docs.rs/yaml-edit · https://github.com/jelmer/yaml-edit | description certain · list editing uncertain |
| yamlpatch | https://github.com/zizmorcore/zizmor/tree/main/crates/yamlpatch | likely |
| noyalib | https://docs.rs/crate/noyalib/latest/source/examples/lossless_edit.rs | likely |
| nondestructive | https://docs.rs/nondestructive | certain |
| Home Assistant complaint | https://community.home-assistant.io/t/automation-editor-changes/159712 | certain (collector) |
| openHAB complaint | https://community.openhab.org/t/main-ui-widget-editor-deletes-comments/114084 | likely (title only) |
| ruamel.yaml | https://pypi.org/project/ruamel.yaml/ | certain (collector) |

## Gaps

- We ran none of the libraries. We could not find primary sources for the comment handling of VS Code jsonc `modify` · kubectl · Renovate.
