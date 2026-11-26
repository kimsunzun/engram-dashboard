# ADR-0336: Server component choices — serde-saphyr · jsonschema · ureq · walkdir · junction · pulldown-cmark · sha2

- 상태: 제안 (2026-10-31, basis: 0a external library research (medium · codex adversarial review) · user B18 (2026-10-19 「구현전에 외부 라이브러리 쓸수있는거 다 조사하고 들어가」) + main M56 · M57 · M60 · M61 · M66 (report)) · 부분 폐기 by ADR-0358 (YAML rewrite row and serde-saphyr features) · 부분 폐기 by ADR-0359 (jsonschema row and ureq row)
- 관련: ADR-0318 (wrapping unit) · ADR-0335 (atomic write) · ADR-0332 (settings check) · ADR-0333 (YAML rewrite) · ADR-0322 (OS trash) · ADR-0324 (locking) · ADR-0325 (path key · file identity) · CLAUDE.md 「의존성 (변경 시 보고)」 · TRD §1-12 · §5 0a · research `docs/research/yaml-schema-https-crates-2026-10-31.md` · `docs/research/library-scan-file-identity-path-key-crates-2026-10-31.md` · `docs/research/markdown-frontmatter-hash-sync-crates-2026-10-31.md` · `docs/research/junction-recycle-bin-atomic-write-2026-10-31.md` · Amended by ADR-0358 (YAML rewrite row and serde-saphyr features) · Amended by ADR-0359 (jsonschema row and ureq row)

## 맥락
The server stage (G20) re-researched, for each part of the implementation, the third-party libraries it will use (§5 0a). `Cargo.lock` has no TLS stack, no YAML crate and no JSON schema checker at all, so whatever is chosen brings in new packages (research yaml-schema-https).

## 결정
| Use | Choice | Place · conditions |
|---|---|---|
| YAML · `.md` front matter **read** | `serde-saphyr` 2.0.0 (`default-features = false`, `["deserialize"]`) | asset-library `format` · for bool targets `strict_booleans: true` (it is the YAML 1.1 form, so `yes/no/on/off` is read as bool) |
| YAML **rewrite** | `yaml-edit =0.3.5` | ADR-0333 |
| JSON schema check | `jsonschema` **0.60.2** (`default-features = false`, `["resolve-file","idna"]`) — raised from the TRD's 0.59.0 | asset-library `validate` · the format validation policy is stated in code (both target schemas are draft-07) |
| Fetching schemas (HTTPS) | `ureq` 3.5.0 blocking (`default-features = false`, `["rustls","platform-verifier","gzip","win-system-proxy"]`) | the daemon's fetch port implementation (agent · asset-library know no network — ADR-0311) · state `RootCerts::PlatformVerifier` in code (with the feature alone the Mozilla roots are the default) · fix the read API by name and set a size cap · proxy = environment variables + `win-system-proxy` (only one HKCU manual proxy — PAC · WPAD not supported · turn off indirect `winreg` — new package · a T-62 target — main 2026-10-31 M66) |
| Folder scan | `walkdir` 2.5.0 (already in the lock — raising it to a direct dependency adds 0 new packages) | asset-library `store` · state in code the policy when the root is a link (`follow_root_links` default true) |
| Directory junction | `junction` 2.1.0 `create` · removal = confirm it is a link with one handle that opened the reparse point itself and delete with that handle (implemented 2026-11-01 — delete with the very handle that confirmed it · finding it again by path and deleting can delete a place swapped for something else between the check and the delete — S25 stage 2 code review) | platform `fs` · even on failure, the remaining empty folder is not deleted (implemented 2026-11-01 — it could delete a folder someone else made · where it is used is inside a volatile folder so it is cleared with that folder — S25 stage 2 code review) |
| Guidance `.md` excerpt | `pulldown-cmark` 0.13.6 (MIT · new) | skips the `MetadataBlock` subtree · the table is `ENABLE_TABLES` |
| `SKILL.md` front matter | hand-built split (codex `extract_frontmatter` rule as is) + serde-saphyr · **carries over all of codex's repair rules** | a strict check alone would wrongly block skills that codex loads |
| Edition hash (`rev`) | `sha2` 0.10 (SHA-256 · in the lock — new to the daemon binary) | hash raw bytes only · the attachment list is a length-prefixed canonical encoding (without it `["ab","c"]` · `["a","bc"]` collide) · does not go through `std::hash::Hash` |
| Atomic write · OS trash · locking · file identity | respectively `base::file::write_atomic` · `IFileOperation` direct · std `Mutex`+`Condvar` · ADR-0325 | ADR-0335 · ADR-0322 · ADR-0324 · ADR-0325 · direct `IFileOperation` for the OS trash is the default and the choice against the `trash` crate is measured again at implementation (ADR-0326 decision 5) |

If TOML is needed, reuse the existing `toml` 1.1.2 · `toml_edit` 0.25.

## 거부한 대안
- **YAML read**: `serde_yaml` 0.9 — archived · `serde_yml` — archived · deprecated shell · RUSTSEC-2025-0068 (unsound) · `noyalib` — same author 0.x · `serde_norway` — updates stalled · `saphyr-serde` — placeholder · `yaml-rust2` — no serde · `marked-yaml` — low activity · `saphyr` 0.3.3 — runner-up (no serde) (research yaml-schema-https). `yaml_serde` 0.10.7 was not compared — no decision.
- **JSON schema**: `boon` 0.6.1 — runner-up · `valico` — outdated (same research). `jsonschema` default features — they pull in reqwest · rustls · aws-lc.
- **HTTPS**: `reqwest` 0.13.5 — runner-up: the default TLS (rustls + aws-lc-rs) requires NASM on Windows. ★「async 라서 탈락」 was too much (blocking · ring forms also exist)★ — the ground was narrowed to the dependency count and no runtime needed, and it is measured again if it runs on tokio or daemon HTTPS grows. `native-tls` · `schannel` — differ per platform (the collector's judgment) (same research · codex review correction).
- **Folder scan**: `jwalk` — depends on rayon · crossbeam · parallelism is not needed · `ignore` — gitignore semantics + it pulls in globset · regex-automata (research library-scan).
- **Junction**: std `junction_point` — unstable (#121709) · `symlink_dir` — developer mode · privileges needed · `junction::delete` — strips only the reparse data so an empty folder remains · `junction::exists` — follows the link and returns `false` for a broken junction · it can fail on an open folder so it cannot be used as a removal guard (research junction-recycle-bin).
- **Markdown**: `comrak` — AST-based · heavy · `markdown-rs` — slow (uncertain · collector alone) (research markdown-frontmatter-hash-sync).
- **Front matter**: `gray_matter` — pulls in yaml-rust2 and its rules differ from codex's · `yaml-front-matter` — 2021 · depends on serde_yaml (same research).
- **Edition hash**: `xxhash-rust` — BSL-1.0 · new · `DefaultHasher` — unstable across Rust releases (same research · the description of `rustc-hash` · `fxhash` is unverified).
- **Atomic write crates**: `tempfile` — because it is `create_new`, if a leftover file with the same name exists it fails with `AlreadyExists` · `atomic-write-file` — random names · `atomicwrites` — stalled since 2024-09 (research junction-recycle-bin). The existing base is used (ADR-0335).

## 근거
- Research yaml-schema-https: serde-saphyr = line · column + excerpt in errors · duplicate keys rejected by default · budget cap (billion-laughs) · no unsafe · fuzzing · `Spanned<T>` · MSRV 1.89 / jsonschema = turning off the default features keeps reqwest · TLS out · automatic draft detection · `instance_path()` / ureq = runs on a plain daemon thread so it keeps the async-import gate and its dependency tree is small · TLS = ring (only a C compiler · no NASM · CMake needed). All three certain.
- Research library-scan: walkdir = certain (already in the lock).
- Research junction-recycle-bin: the `windows-sys` scope of junction 2.1.0 (2026-09-24 · MIT) is already in the lock · `RemoveDirectoryW` deletes only the link (certain).
- Research markdown-frontmatter-hash-sync: the codex front matter rule = `codex-rs/skills/src/parser.rs` (certain — the latest version is the same) · sha2 = certain.

## 영향 / 불변식
- New packages: serde-saphyr · jsonschema · ureq + a new TLS stack (rustls · ring) · pulldown-cmark · yaml-edit (rowan · countme · text-size) · winreg (`win-system-proxy` of `ureq` — M66). The line to add to CLAUDE.md 「의존성」 is reported at implementation.
- `junction` is added to the hand list of the OS-crate dependency gate (`/qa` 4g · CI `platform gate 5`). The async-import gate · the HTTPS client are decided at the schema-fetch stage (M17) — that `win-system-proxy` of `ureq` is not caught by that gate (it counts direct dependencies only) is likely.
- **To confirm (remaining items of the research)**: the licenses of the three crates (serde-saphyr · jsonschema · ureq) were not in the research input — before bringing them in, check `Cargo.toml` · LICENSE · a test that the claude schema (draft-07 + `$defs`) passes in `jsonschema` 0.60.2 · the 304 and over-size behavior of `ureq` · the Windows system proxy reads only one HKCU HTTP proxy (PAC · WPAD not supported) · serde-saphyr's `Spanned` loses position inside untagged · flatten (if the `kind` discrimination is built with an untagged enum, no error position comes out) · edge differences between serde-saphyr and codex's serde_yaml — a golden with the two parsers side by side.
- **Left as an implementation judgment**: whether to take the edition hash with `sha1` (already in the daemon graph so 0 new packages) — it depends on the premise that `rev` is not for security (research).
- The TRD §1-12 table has received this choice (checked 2026-10-31 — `jsonschema` 0.60.2 · the HTTPS field = `ureq` + `win-system-proxy` · the `trash` row = 「거부 — 구현 때 다시 잰다」).
