# ADR-0325: Path key = per-UTF-16-unit folding with the OS uppercase table · file identity = 128-bit first — both in platform `fs`

- 상태: 제안 (2026-10-31, basis: main M38 (re-review B5 · F11) · M60 (0a codex review BLOCK kept · reported) · research `docs/research/library-scan-file-identity-path-key-crates-2026-10-31.md`)
- 관련: ADR-0315 · ADR-0324 · ADR-0336 (walkdir) · CLAUDE.md 「플랫폼 중립」 · ADR-0266 (the operating-OS branch is inside platform) · TRD `docs/process/S25-preset-library/trd.md` §1-6 · §1-4 `catalog.rs` · flow ① scan tag · flow ② 「찾을 수 없음」 · §5-2 step 2 · T-74

## 맥락
Windows does not distinguish case. `items` and `used_by` of `Catalog` are tables looked up by path, but the pairwise comparison `same_path(a,b)` cannot make a table key, so references that differ only in case looked like 「찾을 수 없음」 on Windows (M38). To re-parse only changed files, the scan compares the path, modification time, size and file identity as a tag (TRD flow ①).

## 결정
1. Add `path_key(path)` to platform `fs` — **`same_path(a,b)` == the keys are the same**. `items` and `used_by` of `Catalog` are looked up by this key, and the places that compare paths (the agent-side reverse lookup · the correct attachment in path fixing) compare with `same_path` (M38).
2. Windows key = **per UTF-16 code unit · BMP units only (surrogates left as they are) · the OS uppercase mapping** (`RtlUpcaseUnicodeChar` or `LCMapStringEx` invariant UPPERCASE — chosen in the pilot). Other OSes = as is (M60).
3. Folders that distinguish case per folder (WSL `setCaseSensitiveInfo`) are out of scope, and we write that down (M60).
4. File identity (the scan tag) = **128-bit `FileIdInfo` first · volumes that do not support it fall back to 64-bit (volume serial number + file index)** (M60 — the 64-bit value may not be unique on ReFS). The way to obtain it (a direct platform call · `file-id` · `winapi-util`) is chosen in the pilot (research).
5. File identity is used only for the scan tag — it is not a relink candidate (G38 · T-74 — that would reopen R19 「후보 제안 없음」).

## 거부한 대안
- **Rust `char::to_uppercase`** (inside the collector — only when it is a one-character mapping) — it merges supplementary-plane names (U+10428 → U+10400 · the `$UpCase` table of NTFS is BMP only, so it tells the two apart) (M60 · codex review BLOCK kept).
- **Only the pairwise comparison `same_path`** — it cannot make a table key (M38).
- **`unicase`** — it is full folding (`ß` → `ss`), so it differs from NTFS · **`icu_casemap`** — heavy (Basis for rejection: weak) Basis (main 2026-11-01 — reported · M98): it is the Unicode standard case mapping, so for the same reason as `char::to_uppercase` (it merges supplementary-plane names — NTFS folds only BMP) it disagrees with the OS uppercase table of Decision 2 (not verified — compare in the step 2 pilot) · the weight was not measured. · **the git `strcasecmp` shape** — ASCII only · **`CompareStringOrdinal`** — it is a comparison function, not a key (research).
- **`volume_serial_number` · `file_index` of std Windows `MetadataExt`** — unstable (rust-lang/rust#63010 · still nightly in 1.95) · **`same-file`** — there is no identity value to store (comparison only) (research).

## 근거
- Research library-scan (medium · codex review BLOCK → corrected in M60): key direction = decision · agreement with NTFS = not measured · the exact API = uncertain. MS `BY_HANDLE_FILE_INFORMATION` docs — the 64-bit index may not be unique on ReFS.

## 영향 / 불변식
- If the key rule changes, change it together with `same_path` — that the two are the same is the invariant.
- **Pilot measurement**: where do `RtlUpcaseUnicodeChar` · `LCMapStringEx` diverge from NTFS `$UpCase` (supplementary planes · surrogates · `ß` · `ı`) · is a 128-bit identity actually obtained on ReFS · does the fallback work on volumes that do not support it.
- macOS APFS needs folding + NFC/NFD normalization, but `unicode-normalization` is not in the lock — a documented gap (research · 「플랫폼 중립」).
- If `winapi-util` is chosen, add it to platform gate 5 (the manual list of OS crates) (research).
