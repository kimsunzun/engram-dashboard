# Library scan · file identity · path key — what to pick (server-side external library survey · medium)

- **Status:** medium · 2026-10-31 · collector 1 (worker-scout) + main grounding + codex adversarial review (effort high) — verdict **BLOCK** (one path key item — corrected by a main decision and applied below. Scan and file identity needed only FIX-level additions). TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a (external library survey per implementation part), server side — folder scan · file identity · path key.
- **Trigger:** The preset library scans folders to build a catalog (TRD §1-4 FsStore), and must tell apart whether it is the same file and whether it is the same path (TRD §1-6). Before writing it ourselves, check whether a usable crate exists.
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = a single primary source, no rebuttal · uncertain = estimate/unchecked · contested = the review rebutted it with counter-evidence.

## Conclusion

| Need | Pick | One-line reason | Next best |
|---|---|---|---|
| Folder scan | **walkdir 2.5.0** | Already in `Cargo.lock` (via the tauri-build build dependency), and even if raised to a direct dependency of asset-library, 0 new packages in the lock. With `filter_entry`, prune `.trash/` and exclude `*.engram-tmp` · dot names. TRD §1-4 already points to walkdir | recursion with std `read_dir` |
| File identity | **Windows = `volume_serial_number()` + `file_index()` of `winapi-util::file::information(handle)` · elsewhere = `dev`/`ino` of std unix `MetadataExt`** (inside the platform crate) | winapi-util 0.1.11 is already locked. The no-crate alternative = `windows` 0.58 + `Win32_Storage_FileSystem` with `GetFileInformationByHandle` (the TRD §1-6 plan) | file-id 0.2.3 |
| Path key | **hand-written `platform::fs::path_key`** — ★corrected by the review★ below | the crate that does that job does not match the NTFS rules | none |

★**Three corrections the review forced** (details in 「적대 리뷰 결과」 below)★

1. **Path key = main decision.** The collector's proposal (「문자마다 `to_uppercase()` 가 한 글자일 때만」) was **refuted** — it maps a supplementary-plane letter (e.g. U+10428 → U+10400) with a one-character mapping to make two names equal that NTFS distinguishes, so they collide. **Decision: uppercase per UTF-16 code unit, BMP units only (do not touch surrogates), with the OS uppercase mapping (`RtlUpcaseUnicodeChar` or `LCMapStringEx` invariant UPPERCASE — which one is chosen in the pilot). `same_path` = key match. The match with NTFS is measured in the implementation pilot.**
2. **File identity = prefer the 128-bit `FileIdInfo` (`GetFileInformationByHandleEx`) where supported**, and keep a documented fallback (volume serial number + 64-bit index). Volume serial number + 64-bit index may not be unique on ReFS. The pick above is based on 「잠금에 이미 있는 것」, and the nearest way to get even the 128-bit one is the file-id crate (does both) or platform calling `FileIdInfo` directly — chosen in the pilot.
3. **Scan = state the root symbolic link policy explicitly.** walkdir 2.5.0 has a `follow_root_links` default of true even with `follow_links(false)`, so it follows the root if it is a link. Decide the policy in code, including for partial-tree scans.

Two additions: in a folder with **Windows per-folder case sensitivity** (WSL `fsutil setCaseSensitiveInfo`) turned on, `A.md` and `a.md` can coexist, and unconditional uppercasing merges the two → document it as unsupported or follow the folder flag. **macOS** needs case folding + NFC/NFD for a faithful key, but `unicode-normalization` is not in the lock → leave it as a known gap (the 「그 밖 = 그대로 비교」 of TRD §1-6 is a case-sensitive comparison).

## 1. Folder scan

| Name | Version · date | License | Already in the lockfile? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| **walkdir** | 2.5.0 · 2024-03-01 | Unlicense/MIT | yes (only as the tauri-build build dependency — even raised to a runtime direct dependency, 0 new packages) | fits — `filter_entry` pruning | certain (`Cargo.lock` main checked + crates.io) | crates.io API · `Cargo.lock` |
| std `read_dir` recursion | — | — | — | next best — pruning · error handling written by hand | certain | — |
| jwalk | 0.9.0 | — | not checked | filtered out — depends on rayon/crossbeam, parallelism not needed | likely | crates.io API |
| ignore | 0.4.34 | — | not checked | filtered out — gitignore semantics + pulls in globset/regex-automata, so the regex-automata version may rise | likely | crates.io API |

- Root link: walkdir 2.5.0 has `follow_root_links` default true (codex · `src/lib.rs`) — correction 3 above.
- How walkdir handles **following junctions** (Windows junction) is **unverified** (only confirmed that by default it does not follow symbolic links) — a gap.

## 2. File identity

| Name | Version · date | License | Already in the lockfile? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| **winapi-util** `file::information` | 0.1.11 | — | yes (also satisfies the windows-sys range) | fits — obtained with `volume_serial_number()` · `file_index()`. But it wraps the 64-bit API (the ReFS limit — correction 2) | certain (docs.rs — the collector marked it as from memory, main checked · codex rechecked) | docs.rs `winapi_util::file::Information` |
| `windows` 0.58 + `Win32_Storage_FileSystem` | 0.58 | — | based on 0.58 (platform plan) | fits — no crate · can call `FileIdInfo` directly too | likely | TRD §1-6 |
| file-id (notify-rs) | 0.2.3 | — | not checked · windows-sys ^0.60.1 reuses the locked 0.60.2 | fits — 128-bit `FileIdInfo` (ReFS) + fallback | likely (collector) · codex agreed that it uses both APIs | notify-rs `file-id` `src/lib.rs` |
| std Windows `MetadataExt` | — | — | — | **cannot use** — `volume_serial_number`/`file_index` are unstable (`windows_by_handle`, rust-lang/rust#63010 open · as of rustc 1.95 stable) | certain (main · codex recheck) | rust-lang/rust#63010 · doc.rust-lang.org MetadataExt |
| same-file | 1.0.6 | — | not checked | filtered out — no storable identity value (comparison only) | likely (「아마」) | crates.io |

- Either way, keep it **inside the platform crate**. If winapi-util is picked, it is an OS crate, so it **must be added to the hand list of gate 5.**
- On the unix side, `dev`/`ino` of std `MetadataExt` are enough.

## 3. Path key (deciding same path under case differences)

- **NTFS rule:** comparison is ordinal uppercasing with the volume `$UpCase` table — 1:1 per UTF-16 unit, no full case folding (`ß`→`ss`), no normalization (collector · confidence likely). But that table is **BMP only**, and supplementary-plane letters stay distinguished (codex · docs.rs ntfs `upcase_table.rs` — this part refuted the collector's proposal).
- The OS `CompareStringOrdinal(bIgnoreCase)` is a comparison function, not a key (MS docs).

| Name | Version | Fit | Confidence | Source |
|---|---|---|---|---|
| **hand-written `path_key`** (corrected version — UTF-16 units · BMP only · OS uppercase mapping) | — | fits. Where `RtlUpcaseUnicodeChar` vs `LCMapStringEx` (invariant, UPPERCASE) diverge from `$UpCase` is **unchecked** — if wrong, the result may be a collision rather than 「못 찾음」, so it is a measurement target | correction direction = decision (main decision M60 following the review's refutation) · same as NTFS = not measured (measured in the pilot) · exact API = uncertain | MS docs · docs.rs ntfs |
| the collector's proposal: Rust `to_uppercase()` per character (only when it is a single character) | — | **contested — filtered out.** U+10428 → U+10400 is a one-character mapping, so two names that NTFS distinguishes become the same key | contested (refuted by the review) | doc.rust-lang.org char::to_uppercase · unicode.org n_10400 (Deseret) |
| unicase | 2.10.0 | filtered out — it is full case folding (`ß`→`ss`), so it differs from NTFS | certain (source) | unicase `src/unicode/mod.rs` |
| caseless | 0.2.2 | filtered out | likely | crates.io |
| icu_casemap | 2.4.0 | filtered out — heavy and Unicode 3.0 | likely | crates.io |
| git `strcasecmp` | — | filtered out — ASCII only | certain | git `dir.c:127` |
| dunce | 1.0.5 | already a direct dependency — used **only where canonicalize needs normalization** (not for the key) | certain | `Cargo.lock` |

- Location: `platform::fs::path_key`, **Windows only**. `same_path` = key match.
- **macOS:** APFS ignores both case and normalization by default, so a faithful key requires folding + NFC/NFD (`unicode-normalization` 0.1.26 — not in the lock). For now it is a **documented gap** (the APFS claim was not fetched again to confirm).
- **Per-folder case sensitivity (Windows):** see the added note in the corrections.

## Adversarial review result (적대 리뷰 결과)

| # | Severity | Finding | Handling |
|---|---|---|---|
| 1 | HIGH (BLOCK) | A key based on Rust per-character `to_uppercase` does not match NTFS — supplementary-plane letters become the same key, so different catalog items collide (wrong reference resolution) | **settled as a decision · applied** — the main decision above (UTF-16 units · BMP only · OS mapping · pilot measurement) |
| 2 | MED | Volume serial number + 64-bit index may not be unique on ReFS (MS `BY_HANDLE_FILE_INFORMATION` document) | **applied** — prefer the 128-bit `FileIdInfo` + a documented fallback. Which path (file-id crate / direct platform call) is decided in the pilot |
| 3 | MED | With Windows per-folder case sensitivity, `A.md` · `a.md` can coexist — unconditional uppercasing merges the two | **left** — decide at implementation between documenting as unsupported or honoring the folder flag (open) |
| 4 | MED | walkdir 2.5.0 `follow_root_links` default true | **applied** — state the root link policy (including partial trees). The content of the policy is decided at implementation |

codex recheck (agrees): winapi-util has `information`/`volume_serial_number`/`file_index` (wrapping the 64-bit API) · std `windows_by_handle` is still nightly.

## Gaps · measurements needed

- **Measure the path key against NTFS** (required in the implementation pilot): where `RtlUpcaseUnicodeChar` vs `LCMapStringEx` diverge from the actual volume `$UpCase`, and whether pairs of same names / different names match file system behavior for supplementary planes · surrogates · special characters (e.g. `ß` · `ı`).
- Whether the 128-bit identity is actually obtained on ReFS or in an environment where `FileIdInfo` works, and whether the fallback works on volumes where it does not.
- How walkdir handles Windows junctions · reparse points (unverified).
- The policy when a per-folder case-sensitive folder is met.
- Not surveyed: path key practice of .NET · VS Code. Not confirmed: re-verification of the APFS description. The full set of differences between Rust `to_uppercase` and `$UpCase` was not measured (the corrected version works around this comparison with the OS mapping).
- Whether to add winapi-util to the platform gate 5 hand list (when the chosen path is winapi-util).

## Sources

- crates.io API (walkdir 2.5.0 · jwalk 0.9.0 · ignore 0.4.34 · file-id 0.2.3 · same-file 1.0.6 · unicase 2.10.0 · caseless 0.2.2 · icu_casemap 2.4.0 · dunce 1.0.5 · unicode-normalization 0.1.26)
- `Cargo.lock` (walkdir 2.5.0 · winapi-util 0.1.11 — main checked)
- https://github.com/rust-lang/rust/issues/63010 (`windows_by_handle`) · doc.rust-lang.org MetadataExt
- docs.rs `winapi_util::file::Information`
- notify-rs `file-id` `src/lib.rs`
- MS docs `CompareStringOrdinal` · `BY_HANDLE_FILE_INFORMATION`
- unicase `src/unicode/mod.rs` · git `dir.c:127`
- doc.rust-lang.org `char::to_uppercase` · unicode.org n_10400 (Deseret) · docs.rs ntfs `upcase_table.rs` (cited by codex)
- walkdir 2.5.0 `src/lib.rs` (`follow_root_links`)
