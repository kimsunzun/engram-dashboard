# ADR-0174: Shell unit tests turn off the lib built-in target and are re-established as an explicit [[test]] target

- 상태: 확정 (2026-08-24, basis: measured — 217 tests proven to start via `mt.exe` manifest injection + 4 forms of link arguments measured)
- 관련: CLAUDE.md 「빌드·검증 명령」 · `src-tauri/Cargo.toml` `[lib]`/`[[test]]` · `src-tauri/build.rs` · ADR-0012 (module isolation · standalone harness)

## 맥락

The 217 `#[cfg(test)]` unit tests inside `src/**` of the shell package (`src-tauri`) **have never been run, either locally or in CI.** The lib built-in test target died instantly at the process load stage with `0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND`.

The cause is the link path of the manifest resource. That test exe statically imports `comctl32.dll!TaskDialogIndirect` through the Wry/muda/rfd chain, and that export exists only in comctl32 **v6**, and v6 is bound side-by-side only when the binary carries an `RT_MANIFEST` (`Microsoft.Windows.Common-Controls 6.0.0.0`). `tauri_build` does produce that manifest in `$OUT_DIR`, but `embed-resource` passes it only as `cargo:rustc-link-arg-bins=`, and cargo applies that argument only to `[[bin]]` targets. An exe that did not receive the manifest binds System32's v5.82, cannot find the entry point, and dies before `main`.

That instant death **was cited as the basis of 40-odd other decisions** — the `--exclude engram-dashboard` in the workspace regression, the placement rule "tests newly written for this package go in `tests/`", deferring the fix of known defects, even the decision that called `src-tauri/bindings/` a "hand-maintained generated artifact". That is, this was not about one test not running but **a premise the whole verification landscape leaned on**.

## 결정

Three legs are set up together.

1. `src-tauri/Cargo.toml` — turn off the lib built-in test target with `[lib] test = false` (+ `doctest = false`).
2. `src-tauri/Cargo.toml` — **re-declare** the same source as `[[test]] name = "lib_unit", path = "src/lib.rs"`.
3. `src-tauri/build.rs` — find the resource archive in `$OUT_DIR` and pass it as `cargo:rustc-link-arg-tests=`.

The key is number 2. Cargo's `-tests` link argument applies to **kind=test targets** (= the `[[test]]` declarations), and the lib built-in unit-test target is not among them. Declaring the same `src/lib.rs` once more as an explicit `[[test]]` makes it a target that receives that argument.

The command to call is the single `cargo test -p engram-dashboard --test lib_unit`.

## 거부한 대안

- **Plain `cargo:rustc-link-arg=` without a suffix** — failed in measurement. It reaches lib tests, but it also reaches the bin and overlaps with what it already received through `-bins` → `CVT1100: duplicate resource type:VERSION` → `LNK1123`. `cargo build --bin` breaks.
- **`cargo:rustc-link-arg-tests=` alone** (what the old `build.rs` comment proposed) — failed in measurement. Checking with a fake `.lib`, `--test <이름>` died with `LNK1181` while `--lib` linked fine — that is, that argument targets **only targets that were already fine**. `embed_resource::compile_for_everything` also emits only `-bins/-tests/-examples/-benches` and has no lib unit-test target, so it is meaningless for the same reason. **This decision does not *revive* this alternative** — the difference is that the argument is left as is and **a new target** that the argument reaches was created.
- **Inject via `.cargo/config.toml` rustflags** — it applies to all targets, hitting the same duplication conflict as above, and leaks to the whole workspace.
- **Remove the Wry chain from `run()` with `#[cfg(not(test))]`** — failed on code basis. `tauri::AppHandle` (= `AppHandle<Wry>`) appears in command signatures in 20+ places across 17 files, so `run()` alone is not the root.
- **Move the tests to `src-tauri/tests/`** (what the old `build.rs` comment called 「정공법」, the proper way) — failed on code basis. Integration targets do not have this problem, but moving them would require opening up the visibility of 17+ files because of private items touched by `commands::popout::tests`, `tray::core::tests`, `daemon_client::tests`, and so on. This is the cost of **widening the product API surface for the sake of tests**, and the adopted option gets the same with zero source changes.
- **A `build.rs` environment-variable switch (`ENGRAM_LIBTEST`)** — it worked in measurement, but it makes a non-standard command line, still conflicts when used with a package-wide `cargo test`, and triggers a rebuild every time the variable is toggled.
- **A wrapper script that injects the manifest with `mt.exe` after the build** — proven on real artifacts (see 「근거」 below), but the gate becomes a script instead of a cargo command and a Windows SDK dependency is added. **It was used only as a proof tool, not as the solution.**

## 근거

- **Cause confirmed = `mt.exe` injection experiment.** Copy the dying exe, plant only the manifest (`mt.exe -manifest app.manifest -outputresource:patched.exe;#1`), and run it: it **started normally and the 217 tests ran.** All other bytes identical — this closed it that the absent manifest is the only cause.
- **Why the four integration targets are fine.** The lib test compiles this crate as the **link root**, so `run()` and the `AppHandle<Wry>` commands all go in and the Wry chain comes along. Integration targets **consume `engram_dashboard_lib` as an rlib** and pull out only the symbols they need, so that chain drops out of the link. Measured: the four integration exes have 12 import DLLs and have no `comctl32`·`user32`·`ole32` at all (the lib test has 21). **They live not because the manifest is there but because they do not need it.**
- **Measured no effect on release.** The `-tests` argument reaches only kind=test targets. After the change, the exe built with `cargo build -p engram-dashboard` was confirmed to still have `.rsrc`, VERSIONINFO, and the Common-Controls 6.0 manifest. `Cargo.lock` is also unchanged, so all CI `--locked` steps are unaffected.
- **Adversarial review, 2 rounds · 4 people** (Claude 2 + GPT 2, cross-family). Round 1 BLOCK → round 2 FIX. The two that both families independently pointed out (a wait ceiling instead of `#[ignore]` · `libresource.a` for the gnu target) were reflected as is.

## 영향 / 불변식

- ★**The three legs must stand together**★ — if even one is missing, it falls **silently**. Delete `[[test]]` and all 217 tests evaporate; delete the `build.rs` emission and `0xC0000139` comes back as is; delete `[lib] test = false` and the dying target returns to being the default selection.
- **What guards those three is the CI gate** — `cargo test --locked -p engram-dashboard --test lib_unit -- export_bindings_` (8 tests). That those 8 run is itself proof that the three legs exist (`[[test]]` absent → cargo says "no test target named `lib_unit`", link argument absent → the exe dies before even starting). The same step brings `src-tauri/bindings/` into the generated-artifact sync gate for the first time.
- ★**`--lib` and `--all-targets` still die**★ — `[lib] test = false` removes the target only from the **default selection**, so an explicit call still picks the built-in target without the manifest. It is not a hole that a manifest can close, so **avoid it by call form**.
- **`src/lib.rs` is compiled twice** — cargo constantly emits a "present in multiple build targets" warning. It is a cosmetic matter, but "cleaning it up" by deleting `[[test]]` causes the regression above.
- **`build.rs` panics if it does not find exactly one resource archive** — with 0 it means the `[[bin]]` did not get the manifest either (= the release binary is already broken), and with 2 or more a stale and a new one are linked together and `CVT1100` breaks all five test targets of this package. **Either way it is a build that must not succeed silently.** It accepts both names, msvc (`resource.lib`) and gnu (`libresource.a`).
- **This decision does not turn the suite green** — 32 of the 217 fail, and all come from a single cause in `daemon_client::tests::` (the `app: None` short-circuit of `start_connection`). **It is not a product defect** (the production path is always `app: Some`). So the whole suite has not yet been registered in CI. What shape to fix that harness into is **undecided and a user decision**.
