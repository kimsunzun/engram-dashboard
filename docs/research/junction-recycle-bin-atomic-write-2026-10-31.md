# Junction · OS Recycle Bin · atomic replace — library choice on the server side (investigation · medium)

- **Status:** medium · 2026-10-31 · 1 collector + main grounding + codex adversarial review (effort high) — verdict FIX. One branch of the server side of TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a (external library investigation per implementation part).
- **Occasion:** the preset library has to do (1) create · turn off a folder link (junction) (2) keep what was deleted in `.trash` and send it to the OS Recycle Bin (G45) (3) replace a file atomically (G38). For each, decide whether to write it ourselves or bring in a crate.
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = single primary source, no rebuttal · uncertain = guess · untested · contested = the review rebutted with counter-evidence.

## Conclusion

**Two decisions (main · 2026-10-31).**

- **Decision 1 — library file writes reuse `base::file::write_atomic`, and `sweep_temps` is not called for the library.** The `<파일>.engram-tmp` written in the TRD would be a second rule, so it is not used. Not calling `sweep_temps` is to keep G40 「자동 정리 없음」 (no automatic cleanup). The side that scans the library folder skips base's temporary name shape (`<이름>.tmp<pid>.<번호>`).
- **Decision 2 (M59) — permanent deletion only on an HRESULT settled as 「이 볼륨엔 휴지통이 없다/꺼져 있다」 (this volume has no Recycle Bin / it is off)** (for example `0x8027003A`, `RECYCLE_FORCE_NUKE` and the like). In use · access denied · too large · **all unknown codes stay in `.trash`**, and the original HRESULT is reported per item. The actual behavior on a missing volume · a turned-off volume · removable · network is measured in the implementation pilot.

**Choice per need.**

| Need | Choice | One-line reason | Runner-up |
|---|---|---|---|
| Create a junction | `create` of `junction` 2.1.0 | std's `junction_point` is unstable (#121709), and `symlink_dir` needs developer mode · privilege. Administrator rights are not needed on one's own folder (likely) | Direct call of raw `FSCTL_SET_REPARSE_POINT` (reimplementing the crate) |
| Turn off a junction | `std::fs::remove_dir` — **but first confirm it is a link by non-following metadata (reparse tag)** | `RemoveDirectoryW` removes only the link and leaves the target as is (certain). `junction::delete` leaves an empty folder, so unsuitable | — |
| OS Recycle Bin | `IFileOperation` **direct call** (not the `trash` crate) | Per-item results · HRESULT must be preserved for G45 to stand | `trash` 5.2.9 (below) |
| Atomic replace | `base::file::write_atomic` (on top of std `rename`) | Already exists and the shell settings store uses it. No new crate needed | `tempfile` 3.27.0 `persist` (unsuitable for a fixed name) |

**★Corrections the review forced (already reflected in the body below).**

1. 「`PreDeleteItem` 에서 `TSF_DELETE_RECYCLE_IF_POSSIBLE` 이 없으면 취소하니 조용한 영구 삭제가 막힌다」 (since it cancels when that flag is missing, silent permanent deletion is prevented) **was an overstatement** — that flag is only 「가능하면 휴지통」 (recycle if possible), not proof. Keep the guard but write it as **unverified**.
2. `0x8027003A` is not 「휴지통 없음」 (no Recycle Bin) but 「휴지통을 못 찾았거나 쓸 수 없음」 (the Recycle Bin could not be found or cannot be used). Do not let a three-code distinction authorize the destructive fallback path → decision 2.
3. `psiNewlyCreated == NULL` alone cannot establish 「영구 삭제됨」 (permanently deleted) → look at `hrDelete` and `GetAnyOperationsAborted` together.
4. The statement of the limits of the `trash` crate (「실패가 전부 Unknown」 (all failures are Unknown)) was excessive — the corrected edition is in the Recycle Bin section below.
5. `junction::exists` returns `false` for a broken junction, so it cannot be used alone as a removal guard.
6. The existing writer's directory sync is a no-op on Windows → state the durability claim accurately.

**TRD mismatch (found by main grounding).** The platform table row of TRD §1-6 (G12, 2nd edition) revived the OS Recycle Bin, but the bullet below it still remains as 「(걷음 2026-10-19) OS 휴지통 `fs::move_to_trash`」 (withdrawn 2026-10-19: OS Recycle Bin move-to-trash) → needs cleanup.

## 1. Junction

| Name | Version · date | License | Already in the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| `junction` (tesuji) | 2.1.0 · 2026-09-24 · about 1.6 million recent downloads | MIT | No (dependency `windows-sys` >=0.59,<=0.61 — the lock has 0.59/0.60/0.61 · whether `scopeguard` is locked is unconfirmed) | Fits | likely | crates.io API · local registry source |
| std `junction_point` | Unstable (#121709) | — | — | Cannot use | certain | `os/windows/fs.rs:720` |
| std `symlink_dir` | Stable | — | — | Needs developer mode · privilege → unsuitable | likely | rust 1.95.0 std |
| Raw FSCTL direct | — | — | The `windows` family is already present | The shape of reimplementing the crate | likely | Collector's judgment |

- **Creating:** `junction::create` = `create_dir` + `FSCTL_SET_REPARSE_POINT` (`IO_REPARSE_TAG_MOUNT_POINT`, `src/internals.rs:25-105`). The privilege-elevation path only when `PermissionDenied` (`helpers.rs:16-66`). ★**If the reparse setting fails it leaves an empty folder — the caller must clean it up.**
- **Turning off:** `std::fs::remove_dir`. std 1.95's rmdir = `RemoveDirectoryW` (`sys/fs/windows.rs:1382-1383`), and per MS docs it removes only the link and does not affect the target (certain — collector and codex recheck agree). `junction::delete` strips only the reparse data and leaves an empty folder.
- ★**Correction (review reflected):** `junction::exists` first calls `Path::exists` and follows the link, so it returns `false` for a broken junction. Also, since it opens with `share_mode(0)`, it may fail if something else has the folder open. The 「정션인가」 (is it a junction) check before removal is done with non-following metadata (the reparse tag of `symlink_metadata`).
- macOS: create as a symbolic link and turn off with `remove_file` (collector's description — untested).

## 2. OS Recycle Bin

**Choice: `IFileOperation` direct call.** Flags `FOF_NO_UI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE` (whether `FOF_ALLOWUNDO` is needed is undecided) + `IFileOperationProgressSink`:

- `PreDeleteItem` — if `dwFlags` lacks `TSF_DELETE_RECYCLE_IF_POSSIBLE` (0x80), it returns an error and cancels. ★**The assumption that this guard 「못 보내는 것을 미리 드러낸다」 (reveals in advance what cannot be sent) is unverified** (contested → review below). Electron (`platform_util_win.cc#L97`) and Qt (`qfilesystemengine_win.cpp#L764`) use the same guard, which supports the design but is not a guarantee.
- `PostDeleteItem` — collects `hrDelete` per item. If `psiNewlyCreated` is NULL it is a candidate for permanent deletion, but **do not conclude from that alone**.
- Cost: `Win32_UI_Shell` of `windows` 0.58 + COM implement macros · a dedicated STA thread.

| Name | Version · date | License | Already in the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| `IFileOperation` direct | On top of `windows` 0.58 | — | `windows` present (feature addition needed) | Fits — per-item HRESULT possible | likely (core assumption unverified) | MS docs |
| `trash` | 5.2.9 · 2026-09-13 · ★255 (Byron/trash-rs) | MIT | No (`windows` 0.62 → a third copy) | Partial — no per-item report | certain (source cross-check + codex) | Local source `src/windows.rs` |
| PowerToys Peek · .NET VB | — | — | — | `SHFileOperation` + `FOF_ALLOWUNDO` (+`WANTNUKEWARNING`, interactive) — different conditions from our windowless daemon | likely | codex recheck |
| Files app · Go `mono0x/trash` | — | — | — | Uses `IFileOperation` callbacks · flags — a precedent for direct calls | likely | codex recheck |

**Why `trash` 5.2.9 is not used (corrected edition).** The flags are `FOF_NO_UI|FOF_ALLOWUNDO|FOF_WANTNUKEWARNING` and there is no `FOFX_RECYCLEONDELETE` (`src/windows.rs:97`). It bundles `DeleteItem` and runs them with a single `PerformOperations`, and since there is no `FOFX_EARLYFAILURE`, even if it fails with some deleted, it is reported as one lump. ★**Review correction:** Windows errors are mapped to `Error::Os{code}` and `PerformOperations` failures are also propagated — **only in the case of 「성공했는데 aborted」 (succeeded but aborted)** are they lumped into `Error::Unknown` ("Some operations were aborted", `:110-115`, TODO "return the reason"). So 「전부 Unknown」 (all Unknown) was wrong, and still the **absence of per-item reporting** is what justifies direct COM. Beyond that: the behavior of `FOF_WANTNUKEWARNING` + `FOF_NO_UI` in a windowless daemon is unclear (trash-rs #55) · STA `CoInitializeEx` per thread panics at `RPC_E_CHANGED_MODE` (`:318-345`) · `CoCreateInstance` unwrap (`:95`).

**Error classification — read together with decision 2.**

- `0x8027003A` (`COPYENGINE_E_RECYCLE_BIN_NOT_FOUND`) = the Recycle Bin was **not found or cannot be used**. The collector proposed splitting this into three branches: 「휴지통 없음」 (no Recycle Bin), `0x80070020` (in use), `0x80070005` (access denied), but that is incomplete — a volume with recycling turned off gets `RECYCLE_FORCE_NUKE`, items that are too large · excessive paths each have their own codes, and sharing · access has `COPYENGINE_E_*_SRC/DEST` variants (`sherrors.h` L65–95).
- Handling: **preserve the original HRESULT and have an 「알 수 없음」 (unknown) category.** A three-code distinction does not authorize a destructive fallback (permanent deletion).
- There is no means of advance determination: `SHQueryRecycleBinW` looks only at content, and `GetDriveType` is only for filtering (a USB can look fixed, and a redirected network folder can also have a Recycle Bin). Resolve the containing volume with `GetVolumePathName`, and when uncertain handle it conservatively.
- `PerformOperations` can return success even after cancellation, so also look at `GetAnyOperationsAborted`. `FOFX_EARLYFAILURE` does not roll back items already finished.

## 3. Atomic replace

**Choice: reuse `base::file::write_atomic`.** `crates/engram-dashboard-base/src/file.rs` — `write_atomic` (L348) · `write_atomic_unless` · `copy_atomic` · `sweep_temps` (L537, deletes only temporaries of this process or a dead pid — called by agent persistence and shell state boot). Temporary name `<이름>.tmp<pid>.<번호>` (L501) → sync → `std::fs::rename` → on lock, retry with `OsHooks` → directory sync (ADR-0265 decision 4 · ADR-0291 R10/R11). The shell `settings/store.rs:87` uses it. The basis is decision 1.

| Name | Version · date | License | Already in the lock file? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|
| `base::file::write_atomic` | In the repository | — | Yes | Fits | certain (main confirmed the source) | `file.rs:348` |
| std `fs::rename` | 1.95 | — | Yes | Same-volume atomicity likely (no MS guarantee wording) | likely | `sys/fs/windows.rs:1311-1379` |
| `tempfile` | 3.27.0 | — | Not checked | `persist` = `SetFileAttributesW` + `MoveFileExW`. A fixed name works with `Builder` but is `create_new(true)`, so if a file left after shutdown exists, `AlreadyExists` → unsuitable for a fixed name | likely | Local source |
| `atomic-write-file` | 0.3.1 | — | Not checked | Random name (`rand`) | likely | docs.rs · `generic.rs` |
| `atomicwrites` | 0.4.4 · stagnant since 2024-09 | — | Not checked | Outdated | likely | crates.io |

- std 1.95's `rename` = `MoveFileExW(REPLACE_EXISTING)`, and on `ACCESS_DENIED` it falls back to `SetFileInformationByHandle(FileRenameInfoEx, REPLACE_IF_EXISTS|POSIX_SEMANTICS)`. There is no `WRITE_THROUGH`.
- `ReplaceFileW` preserves attributes · ACLs and fails if the target does not exist (likely — the collector wrote it from memory, docs not rechecked).
- ★**Durability statement correction (review):** the existing writer's directory sync is a no-op on Windows (`platform/src/fs.rs:125`) and ignores its result (`base/src/file.rs:482`). Do not write 「교체 뒤 디렉터리 항목까지 디스크에 내려갔다」 (the directory entry went down to disk after the replace), and state the guarantee as up to the temporary file sync + a same-volume rename.

## Adversarial review result (codex · effort high · FIX)

| Severity | Finding | Handling |
|---|---|---|
| HIGH | 「`TSF_DELETE_RECYCLE_IF_POSSIBLE` 없으면 거부 → 조용한 영구 삭제 방지」 (refuse if the flag is absent → prevents silent permanent deletion) is an overstatement | **Reflected** — keep the guard but mark it unverified. Measure turned-off Recycle Bins · oversized items · capacity · removable · network in the pilot |
| HIGH | Error classification incomplete (`0x8027003A` = none or unusable, `RECYCLE_FORCE_NUKE`, size · path codes, `*_SRC/DEST` variants) | **Settled as a decision** — M59: preserve the original HRESULT + an unknown category, permanent deletion only on a settled 「휴지통 없음/꺼짐」 (no Recycle Bin / off) |
| MED | 「이 항목을 휴지통에 넣을 수 있나」 (can this item be put in the Recycle Bin) has no means of advance determination | **Reflected** — no advance determination · conservative when uncertain |
| MED | `psiNewlyCreated == NULL` alone cannot establish permanent deletion · `GetAnyOperationsAborted` · EARLYFAILURE no rollback | **Reflected** — decide together with `hrDelete` |
| MED | `trash` limit overstated (actually `Error::Os{code}`, only success+aborted is Unknown) | **Reflected** — description corrected, and the absence of per-item reporting remains the basis for direct COM |
| MED | `junction::exists` returns `false` on a broken junction | **Reflected** — replaced by a non-following metadata check |
| MED | The existing writer's directory sync is a no-op on Windows · result ignored | **Reflected** — durability statement made accurate |

What codex rechecked: `RemoveDirectoryW` removes only the junction and does not affect the target · PowerToys Peek and .NET VB use `SHFileOperation` + `FOF_ALLOWUNDO` (interactive) · the Files app and Go `mono0x/trash` use `IFileOperation` callbacks · flags · no stronger advance determination was found.

## Gaps · measurements needed (implementation pilot)

- **Measuring the behavior of `IFileOperation`** — what the `PreDeleteItem` flag · `hrDelete` · `psiNewlyCreated` · `GetAnyOperationsAborted` actually emit on a FAT32 USB · a volume with the Recycle Bin turned off · an item that is too large · removable · network. The core assumption (the guard reveals in advance) is still unverified.
- Whether `FOF_ALLOWUNDO` is needed · the exact `windows` feature list.
- Resolving `windows-sys`/`scopeguard` of `junction` (`cargo tree` not run) · whether to add them to the lock file.
- Recheck of the `ReplaceFileW` docs (relies on memory).
- The macOS Recycle Bin was not investigated.
- Whether the library folder scan skips the base temporary name shape (the premise of decision 1) — to be cleaned up together with the TRD's `.engram-tmp` wording.

## Sources

- crates.io API: `junction` 2.1.0 · `trash` 5.2.9 · `tempfile` 3.27.0 · `atomic-write-file` 0.3.1 · `atomicwrites` 0.4.4
- Local cargo registry source: junction-2.1.0 `src/internals.rs:25-105` · `helpers.rs:16-66` · trash-5.2.9 `src/windows.rs:95, 97, 110-115, 318-345` · tempfile-3.27.0
- rust-lang/rust 1.95.0: `sys/fs/windows.rs` 1311-1445 (rename · 1382-1383 rmdir) · `os/windows/fs.rs:720` · issue #121709
- MS docs: `RemoveDirectoryW` · `IFileOperation::SetOperationFlags` · `IFileOperationProgressSink` `PreDeleteItem`/`PostDeleteItem` · `_TRANSFER_SOURCE_FLAGS` · `sherrors.h` L65–95 · docs.rs windows-sys `COPYENGINE_E_RECYCLE_BIN_NOT_FOUND`
- trash-rs issues #55 · #101 · atomic-write-file `generic.rs`
- Electron `platform_util_win.cc#L97` · Qt `qfilesystemengine_win.cpp#L764`
- Repository: `crates/engram-dashboard-base/src/file.rs` L348 · L482 · L501 · L537 · `platform/src/fs.rs:125` · `src-tauri` `settings/store.rs:87` · ADR-0265 decision 4 · ADR-0291 R10/R11 · TRD `docs/process/S25-preset-library/trd.md` §1-6 · §5-1
