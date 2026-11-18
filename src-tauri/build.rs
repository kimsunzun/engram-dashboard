// Link the Windows manifest resource into the test targets too.
// Why: this shell's test exe goes through the Wry/muda/rfd chain and statically imports `comctl32.dll!TaskDialogIndirect`, which exists only in comctl32 v6, and v6 binds side-by-side only when the binary carries an `RT_MANIFEST` (`Microsoft.Windows.Common-Controls 6.0.0.0`). Without it the loader picks System32's v5.82, cannot find the entry point, and the process dies before starting with 0xC0000139 (STATUS_ENTRYPOINT_NOT_FOUND) — no test can run.
// `tauri_build::build()` alone is not enough: it produces the manifest as `$OUT_DIR/resource.lib`, but embed-resource passes it on only as `cargo:rustc-link-arg-bins=` (embed-resource 3.0.9 `src/lib.rs:444`), which cargo applies only to `[[bin]]`.
// Emitting split into `-bins` and `-tests` is the key: one suffixless `cargo:rustc-link-arg=` covering everything gives the bin the same resource twice and breaks the build with `CVT1100: duplicate resource type:VERSION` → `LNK1123` (measured). Here only `-tests` is added; the bin-side argument is untouched.
// Paired setting = `[lib] test = false` + `[[test]] name = "lib_unit"` in `Cargo.toml`: cargo's `-tests` means kind=test targets (those declared with `[[test]]`), not the lib's built-in unit-test target, so the same `src/lib.rs` is declared once more as an explicit `[[test]]`. Reverting any one of the three brings 0xC0000139 straight back.
fn main() {
    tauri_build::build();

    // Do not branch with `#[cfg(windows)]` — see the function head comment below (host cfg ≠ target OS).
    emit_resource_link_arg_for_tests();
}

/// Find the resource static library `tauri_build` left in `$OUT_DIR` and pass it on as a test-target link argument.
/// Emit only for a windows target, decided by a runtime environment variable, not `#[cfg(windows)]`: the build script is compiled for the host but this resource is a target artifact; treating them as the same makes a cross build to `*-pc-windows-msvc` from a non-Windows host drop this code, the emit silently skipped and 0xC0000139 back. So the function always compiles and only the fork is at runtime.
/// Scan by extension rather than hardcoding `resource.lib` (an internal name of tauri-build/embed-resource — an upstream rename would silently skip the link), but then the scan must also count: `$OUT_DIR` is not cleaned between builds and CI restores `target/` from cache, so after an upstream rename the stale and new files would both be linked (`CVT1100` → `LNK1123`), package-wide (`-tests`), breaking the 4 green integration targets together. So exactly 1 candidate passes; 0 or 2+ is a panic.
/// No "prefer exactly `resource.lib`" rule: after an upstream rename a stale `resource.lib` beside the new artifact would be silently picked, hiding the state the count check exists to catch.
/// The extension scan alone gives 2 candidates in release — the defect that killed the v0.3.0 release workflow twice (2026-09-22). `$OUT_DIR` legitimately gets non-resource `.lib` files: `tauri-build 2.6.3` `src/static_vcruntime.rs` writes an empty object overriding the hardcoded `msvcrt.lib` (82-byte COFF linker-directive stub, only section `.drectve`) to force the static CRT and puts that directory on the linker search path (`cargo:rustc-link-search=native=`) — a native-library search folder, so more non-resource `.lib` files may come; the stub is written with `create_new`, so once present it stays. It exists only in release because `tauri-build` `src/lib.rs:708` uses it only with `STATIC_VCRUNTIME=true`, which a bare `cargo build` does not supply and the tauri CLI packaging build (`npm run tauri -- build`) does (the string is embedded in the CLI's native binary — measured); the 11 debug OUT_DIRs hold only `resource.lib`, only the release OUT_DIR had `msvcrt.lib` alongside (measured 2026-09-22).
/// So candidates are filtered by content (`carries_resource_data`), not name — a name filter would collapse the "no name preference" clause; content survives upstream renames (a resource artifact carries resource data, a linker-directive stub does not). The "exactly 1" rule then applies to the survivors: 0 and 2+ remain a panic, and the panic messages below carry the meaning of those branches.
/// Do not shrink the content test to a `.rsrc` section name: it never appears in an msvc artifact (measured 2026-09-22). `resource.lib` is a `.lib` in name only — neither COFF nor archive but a Win32 `.res` file (link.exe accepts `.res` directly), which has no sections, so a `.rsrc`-only filter drops the real resource and gives a "0 candidates" panic. The real test is the `.res` null header; `.rsrc` is the second branch, for gnu.
/// An unreadable candidate panics — never silently skipped: skipping disables both count-check branches together, and if the unreadable one was the real resource the remaining stale one would pass as 1. A read failure means undecidable, not absent.
/// A panic, not a `cargo:warning`, on purpose: no resource at all means tauri_build could not even pass it to `[[bin]]`, so the release binary also ships without a manifest — a build that must not succeed silently, whose silent omission would become months of latent debt (the original defect survived exactly that way). A build script panic is loud and immediate, and CI builds this package on every push.
fn emit_resource_link_arg_for_tests() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR missing — cargo always provides it, so reaching here means the build environment itself is broken");

    let entries =
        std::fs::read_dir(&out_dir).unwrap_or_else(|e| panic!("failed to open OUT_DIR({out_dir}): {e}"));

    // Stage 1 = scan by name. Only regular files count as candidates (a directory named `*.lib`/`*.a` passed to the linker makes the diagnostics wrong). Accept both `.lib` and `.a`: embed-resource 3.0.9 names its output `resource.lib` for msvc and `libresource.a` for `*-pc-windows-gnu`, a supported configuration (tauri-build 2.6.3 branches on `CARGO_CFG_TARGET_ENV == "gnu"`) — `.lib` alone gives 0 candidates there and the panic below stops the build with the wrong assertion "no resource was created at all".
    let mut scanned: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("lib" | "a")))
        .collect();
    // The read_dir order is unspecified — sorting only fixes the listing order of the panic below (selection is unaffected: only exactly 1 passes).
    scanned.sort();

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in scanned {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "failed to read candidate file ({p}) in OUT_DIR({out_dir}): {e} — an unreadable candidate is not \
                 silently dropped. Dropping it would make that file as if it never existed, disabling both the \
                 0 and 2 branches of the count check below (if the unreadable one was the real resource, the one stale left \
                 passes as 1). A read failure means the decision cannot be made, not that the candidate is absent, so stop here.",
                p = path.display()
            )
        });
        if carries_resource_data(&bytes) {
            candidates.push(path);
        } else {
            skipped.push(path.display().to_string());
        }
    }

    let chosen = match candidates.len() {
        1 => candidates.remove(0),
        n => {
            let listed = candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let skipped = skipped.join(", ");
            panic!(
                "could not pin down a single manifest resource (.lib/.a) in OUT_DIR({out_dir}) — \
                 {n} candidates: [{listed}] (filtered out as non-resources by the content check: [{skipped}]). \
                 0 means tauri_build failed to produce the resource, so `[[bin]]` is also linked without a manifest \
                 (= even the release binary has no manifest). 2 or more means stale + new are linked \
                 together, CVT1100 (duplicate resource type:VERSION) → LNK1123, and all 5 test targets of this package \
                 break. Either way this build must not silently succeed. \
                 ★If 0 appeared on a gnu target, that is the place to widen the `.rsrc` branch of `carries_resource_data`, \
                 not the place to strip the content check★ — stripping it makes the msvcrt stub a candidate \
                 again and the release build stops again."
            )
        }
    };

    println!("cargo:rustc-link-arg-tests={}", chosen.display());
}

/// Decide from the candidate file's bytes whether it is a real resource artifact (name and extension are not looked at). Two accepted forms, with different verification status:
/// ① Win32 `.res` file — the real thing on the msvc path (measured 2026-09-22: the 13,656-byte `resource.lib` is this form, the 82-byte `msvcrt.lib` beside it is not). A `.res` always begins with a null resource entry — DataSize=0, HeaderSize=32, Type=ordinal 0, Name=ordinal 0 — the marker by which link.exe recognizes a 32-bit `.res`, not a value the producing tool can change at will.
/// ② COFF `.rsrc` section name — an object or archive carrying a resource embeds the section name as bytes; covers gnu's `libresource.a` (windres → ar) and a future cvtres-made COFF lib. [unverified — no gnu artifact in this worktree, 2026-09-22]. It widens rather than blocks, so a wrong guess shows loudly as a "0 candidates" panic, not a false detection.
fn carries_resource_data(bytes: &[u8]) -> bool {
    // The fixed leading 16 bytes of the `.res` null header.
    const RES_NULL_HEADER: [u8; 16] = [
        0x00, 0x00, 0x00, 0x00, // DataSize = 0
        0x20, 0x00, 0x00, 0x00, // HeaderSize = 32
        0xFF, 0xFF, 0x00, 0x00, // Type = ordinal 0
        0xFF, 0xFF, 0x00, 0x00, // Name = ordinal 0
    ];
    const COFF_RSRC_SECTION: &[u8] = b".rsrc";

    bytes.starts_with(&RES_NULL_HEADER)
        || bytes
            .windows(COFF_RSRC_SECTION.len())
            .any(|w| w == COFF_RSRC_SECTION)
}
