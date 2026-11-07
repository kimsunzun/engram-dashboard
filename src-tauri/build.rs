// Link the Windows manifest resource into **the test targets too**.
//
// Why it is needed: this shell's test exe goes through the Wry/muda/rfd chain and statically imports
//   `comctl32.dll!TaskDialogIndirect`. That export exists only in comctl32 **v6**, and v6 binds side-by-side
//   only when the binary carries an `RT_MANIFEST` (`Microsoft.Windows.Common-Controls 6.0.0.0`). Without
//   a manifest the loader picks System32's v5.82, cannot find the entry point, and the **process dies before
//   it even starts** with 0xC0000139 (STATUS_ENTRYPOINT_NOT_FOUND) — not a single test can run.
//
// Why tauri_build alone is not enough: `tauri_build::build()` does produce that manifest as `$OUT_DIR/resource.lib`,
//   but embed-resource passes it on only as `cargo:rustc-link-arg-bins=`
//   (embed-resource 3.0.9 `src/lib.rs:444`). cargo applies that argument only to `[[bin]]`.
//
// ★Emitting split into `-bins` and `-tests` is the key★ — if one suffixless `cargo:rustc-link-arg=`
//   covers everything at once, the bin receives the same resource **twice** and the build breaks with `CVT1100: duplicate resource type:VERSION` →
//   `LNK1123` (measured). Here only `-tests` is added, so the bin-side argument is left untouched.
//
// The paired setting = `[lib] test = false` + `[[test]] name = "lib_unit"` in `Cargo.toml`.
//   cargo's `-tests` means **kind=test targets** (= those declared with `[[test]]`), and the lib's built-in unit-test target
//   is not among them. So the same `src/lib.rs` is declared once more as an explicit `[[test]]` so that it
//   receives that argument. If any one of the three is reverted, 0xC0000139 comes straight back.
fn main() {
    tauri_build::build();

    // ★Do not branch with `#[cfg(windows)]`★ — see the comment on the function head below (host cfg ≠ target OS).
    emit_resource_link_arg_for_tests();
}

/// Find the resource static library that `tauri_build` left in `$OUT_DIR` and pass it on as a test-target link argument.
///
/// ★Emit only for a windows **target**, and the decision is a runtime environment variable, not `#[cfg(windows)]`★ — the build
/// script is compiled for the **host**, but this resource is a **target** artifact. If the two are treated as the same,
/// a cross build to `*-pc-windows-msvc` from a non-Windows host makes this code vanish entirely, the emit
/// is silently skipped, and 0xC0000139 comes straight back. So the function is always compiled and only the fork is put at runtime.
///
/// The reason for scanning by extension instead of hardcoding the name (`resource.lib`) = that name is an internal
/// matter of tauri-build/embed-resource, so if upstream changes it the link silently does not happen. But if we only scan and **do not
/// count**, that flexibility becomes a risk: `$OUT_DIR` is not cleaned between builds and CI restores `target/` from cache,
/// so if an upstream rename ever happened, the stale one + the new one would be linked **both**, giving `CVT1100` (duplicate
/// resource type:VERSION) → `LNK1123`. That argument is package-wide (`-tests`), so the integration
/// targets that are green now, 4 of them, break together. So only **exactly 1 candidate** passes, and 0 or 2 or more are rejected with a
/// **panic**.
///
/// ★No preference of the form "if a name is exactly `resource.lib`, pick it" is kept★ — with that preference, after an upstream
/// rename a **stale `resource.lib`** left next to the new artifact would be silently picked, hiding again the very state
/// the count check just above was meant to catch.
///
/// ★But keeping **only** the extension scan gives 2 candidates in release — this is the defect that killed the v0.3.0 release workflow
/// twice (2026-09-22)★. `$OUT_DIR` legitimately receives `.lib` files that are **not** resources:
/// `src/static_vcruntime.rs` of `tauri-build 2.6.3` writes there **an empty object to override the hardcoded
/// `msvcrt.lib`** (82-byte COFF, a linker-directive stub whose only section is `.drectve`) in order to force the static CRT,
/// and puts that directory on the linker search path with `cargo:rustc-link-search=native=`.
/// That is, `$OUT_DIR` is not a resource-only folder but a **native-library search folder**, and more non-resource
/// `.lib` files may arrive in future. Moreover that stub is written with `create_new`, so once it exists it stays.
///
/// ★That it exists only in release and not in debug is why this defect went uncaught until now★ — that stub is used in `tauri-build`
/// `src/lib.rs:708` **only when the `STATIC_VCRUNTIME=true` environment variable is present**. A bare `cargo build` does not
/// supply that value and the tauri CLI's packaging build (`npm run tauri -- build`) does (that string is embedded in the CLI's native
/// binary — measured). So in this worktree too, the 11 debug OUT_DIRs hold
/// only `resource.lib`, and only the release OUT_DIR had `msvcrt.lib` alongside it (measured 2026-09-22).
///
/// ★So candidates are filtered by **content**, not by name — `carries_resource_data`★. Filtering by name would collapse
/// the "no name preference" clause just above (it would pick the stale `resource.lib` again). Content does not follow
/// when upstream changes the file name — a resource artifact carries resource data, and a linker-directive
/// stub does not. After filtering, the existing "exactly 1" rule is applied unchanged to **the survivors**: 0 and
/// 2 or more are still a panic, and the meaning of those two branches lives in the panic messages below.
///
/// ★Do not **shrink** it to "it is a resource so it must have a `.rsrc` section name" — that string never
/// appears in an msvc artifact (measured 2026-09-22)★. `resource.lib` is a `.lib` in name only; it is neither a COFF nor an archive but a
/// **Win32 `.res` file** (link.exe accepts `.res` directly). A `.res` has no concept of sections, so
/// `.rsrc` cannot exist in it — filtering on `.rsrc` alone **drops the real resource and gives a "0 candidates" panic**.
/// The real test is the `.res` null header, and `.rsrc` is the second branch, for gnu.
///
/// ★If a candidate cannot be read, panic — do not silently skip it★. Skipping makes that file as if it had never
/// existed, and both branches of the count check are disabled together: if the unreadable one was the real resource, instead of standing at "0" the **remaining
/// stale one passes as 1**. A read failure means undecidable, not that the candidate is absent.
///
/// ★That it is a panic and not a `cargo:warning` is intentional★ — having no resource at all means tauri_build could not even pass it to `[[bin]]`,
/// so **the release binary also ships without a manifest**. This is a build that must not succeed silently.
/// Moreover a silent omission here becomes months of latent debt (the original defect this file fixes survived exactly that
/// way). A build script panic is loud and immediate, and CI builds this package on every push.
fn emit_resource_link_arg_for_tests() {
    // Decide the target OS. The value is passed by cargo to the build script, so it is accurate in cross builds too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR 부재 — cargo 가 늘 주는 값이라 여기 닿으면 빌드 환경 자체가 깨진 것이다");

    let entries =
        std::fs::read_dir(&out_dir).unwrap_or_else(|e| panic!("OUT_DIR({out_dir}) 열기 실패: {e}"));

    // Stage 1 = scan by name.
    // Only regular files count as candidates — passing a directory whose name ends in `.lib`/`.a` to the linker makes the diagnostics wrong.
    // ★Accept both `.lib` and `.a`★ — even within the same version, embed-resource 3.0.9 names its output `resource.lib` for msvc and
    //   `libresource.a` for `*-pc-windows-gnu`. gnu is also a supported configuration (tauri-build 2.6.3 branches separately on
    //   `CARGO_CFG_TARGET_ENV == "gnu"`), so looking at `.lib` alone gives 0 candidates on that target,
    //   and the panic below stops the build with the **wrong assertion** that "no resource was created at all".
    let mut scanned: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("lib" | "a")))
        .collect();
    // The read_dir order is unspecified — sorting only fixes the **listing order** of the panic below (it has nothing to do with selection:
    //   only exactly 1 passes, so there is nothing to choose).
    scanned.sort();

    // Stage 2 = filter by content. The typical case filtered out here is the `msvcrt.lib` stub (see the head comment).
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in scanned {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "OUT_DIR({out_dir}) 의 후보 파일({p}) 읽기 실패: {e} — 읽지 못한 후보를 조용히 \
                 버리지 않는다. 버리면 그 파일이 처음부터 없었던 것이 되어 아래 개수 검사의 \
                 0개·2개 갈래가 함께 무력해진다(못 읽은 것이 진짜 리소스였다면 남은 stale 하나가 \
                 1개로 통과한다). 읽기 실패는 판정 불능이지 후보 부재가 아니므로 여기서 세운다.",
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
                "OUT_DIR({out_dir}) 에서 manifest 리소스(.lib/.a)를 하나로 특정하지 못했다 — \
                 후보 {n}개: [{listed}] (내용 검사에서 리소스 아님으로 걸러낸 것: [{skipped}]). \
                 0개면 tauri_build 가 리소스를 만들지 못한 것이라 `[[bin]]` 도 manifest 없이 \
                 링크된다(= 릴리즈 바이너리까지 manifest 없음). 2개 이상이면 stale + 신규가 함께 \
                 링크돼 CVT1100(duplicate resource type:VERSION) → LNK1123 으로 이 패키지 테스트 \
                 타깃 5개가 전부 깨진다. 어느 쪽이든 조용히 성공하면 안 되는 빌드다. \
                 ★gnu 타깃에서 0개가 났다면 `carries_resource_data` 의 `.rsrc` 갈래를 넓힐 \
                 자리이지 내용 검사를 걷어낼 자리가 아니다★ — 걷어내면 msvcrt stub 이 도로 \
                 후보가 되어 릴리즈가 다시 선다."
            )
        }
    };

    println!("cargo:rustc-link-arg-tests={}", chosen.display());
}

/// Decide from the candidate file's **bytes** whether it is a real resource artifact — the name and extension are not looked at.
///
/// There are two accepted forms and their **verification status differs**.
///
/// ① **Win32 `.res` file** — the real thing on the msvc path, measured in this worktree (2026-09-22:
///    `resource.lib` of 13,656 bytes is this form, while `msvcrt.lib` of 82 bytes in the same folder is not). A `.res`
///    always begins with a "null resource entry" — DataSize=0, HeaderSize=32, Type=ordinal 0,
///    Name=ordinal 0. That is the marker by which link.exe recognizes a 32-bit `.res`, so it is not a value the producing tool can
///    change at will.
/// ② **COFF `.rsrc` section name** — when an object or archive carries a resource, the section name is embedded as bytes.
///    This covers gnu's `libresource.a` (windres → ar) and the case where upstream later
///    emits a real COFF lib via cvtres. ★**Unverified**★ — there is no gnu artifact in this worktree, so it could not be measured (2026-09-22).
///    It is a branch that **widens**, not one that blocks, so if it is wrong it shows up loudly as a "0 candidates" panic, not as a false detection.
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
