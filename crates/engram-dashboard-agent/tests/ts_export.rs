//! Generating declaration derivatives — `cargo test -p engram-dashboard-agent` writes into `bindings/` (TRD §2-4).
//!
//! ★Explicit export (no reliance on implicit derive tests)★: ts-rs makes an `export_bindings_<type>` test per `#[ts(export)]` type, but
//!   `src-tauri/bindings/` relied on that implicit path and became a hand-maintained artifact that **never ran** locally (`0xc0000139`) or
//!   in CI (package excluded) (TRD §5 measured). Local side now works (`--test lib_unit` runs those tests and regenerates
//!   `src-tauri/bindings/`), but **CI is still blocked** (suite not registered · 30 known failures pending) and **no sync gate watches
//!   that directory** — so this file keeps calling the export directly, like protocol's `tests/ts_export.rs`.
//! ★Commit the generated files★ — the CI diff gate must watch this directory to catch vocabulary drift.

use engram_dashboard_agent::commands::{
    AgentCancelQueuedInputArgs, AgentCancelQueuedInputOk, AgentInterruptArgs, AgentInterruptOk,
    AgentListArgs, AgentListOk, AgentListQueuedInputsArgs, AgentListQueuedInputsOk, AgentMoveArgs,
    AgentMoveOk, AgentNewArgs, AgentNewOk, AgentRenameArgs, AgentRenameOk, AgentResolveArgs,
    AgentResolveOk, AgentSpawnArgs, AgentSpawnOk, LibraryFoldersArgs, LibraryFoldersOk,
    LibraryListArgs, LibraryListOk, LibraryNewFileArgs, LibraryNewFileOk, LibraryNewFolderArgs,
    LibraryNewFolderOk, LibraryReadArgs, LibraryReadOk, LibraryRefreshArgs, LibraryRefreshOk,
    LibraryRelinkArgs, LibraryRelinkOk, LibrarySetRootArgs, LibrarySetRootOk, LibraryStatusArgs,
    LibraryStatusOk, UsageGetArgs, UsageRefreshArgs, UsageRefreshOk, UsageVendorRow,
    CATALOG_VERSION,
};
use engram_dashboard_command::{catalog_json, command_specs, CommandSpec};
use ts_rs::TS;

/// **All declarations linked into** this test binary, sorted by name.
///
/// ★Not importing one module's `COMMAND_SPECS`★ — then a new declaration block inside agent would silently drop out of coverage and the
/// derived schema; a 「hand-written import」 would replace the 「hand-written number」 with the same failure mode (TRD §5).
/// ★Sorting makes the derived file deterministic★ — the linker's collection order is not fixed, and wobble would turn CI's bindings diff
///   gate red regardless of content.
fn linked_specs() -> Vec<&'static CommandSpec> {
    let mut specs: Vec<&'static CommandSpec> = command_specs().collect();
    specs.sort_unstable_by_key(|s| s.name);
    specs
}

/// ★Do not hand-write the expected value★ — 「add declarations, fix the list number, passes」 lets the edit that creates the omission also
/// fix the assertion (the failure mode TRD §5 measured in `src-tauri/bindings/`). The expected value comes from **the declarations
/// themselves** ([`linked_specs`]): a missing export call leaves that type's `.ts` ungenerated and this comparison fails.
#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    std::fs::create_dir_all(out).expect("failed to create bindings/");
    for entry in std::fs::read_dir(out).expect("read bindings/") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_some_and(|ext| ext == "ts") {
            std::fs::remove_file(&path).expect("remove old binding");
        }
    }

    AgentListArgs::export_all_to(out).expect("failed to export the AgentListArgs bindings");
    AgentListOk::export_all_to(out).expect("failed to export the AgentListOk bindings");
    AgentResolveArgs::export_all_to(out).expect("failed to export the AgentResolveArgs bindings");
    AgentResolveOk::export_all_to(out).expect("failed to export the AgentResolveOk bindings");
    AgentSpawnArgs::export_all_to(out).expect("failed to export the AgentSpawnArgs bindings");
    AgentSpawnOk::export_all_to(out).expect("failed to export the AgentSpawnOk bindings");
    AgentNewArgs::export_all_to(out).expect("failed to export the AgentNewArgs bindings");
    AgentNewOk::export_all_to(out).expect("failed to export the AgentNewOk bindings");
    AgentRenameArgs::export_all_to(out).expect("failed to export the AgentRenameArgs bindings");
    AgentRenameOk::export_all_to(out).expect("failed to export the AgentRenameOk bindings");
    AgentMoveArgs::export_all_to(out).expect("failed to export the AgentMoveArgs bindings");
    AgentMoveOk::export_all_to(out).expect("failed to export the AgentMoveOk bindings");
    AgentListQueuedInputsArgs::export_all_to(out)
        .expect("failed to export the AgentListQueuedInputsArgs bindings");
    AgentListQueuedInputsOk::export_all_to(out)
        .expect("failed to export the AgentListQueuedInputsOk bindings");
    AgentCancelQueuedInputArgs::export_all_to(out)
        .expect("failed to export the AgentCancelQueuedInputArgs bindings");
    AgentCancelQueuedInputOk::export_all_to(out)
        .expect("failed to export the AgentCancelQueuedInputOk bindings");
    AgentInterruptArgs::export_all_to(out)
        .expect("failed to export the AgentInterruptArgs bindings");
    AgentInterruptOk::export_all_to(out).expect("failed to export the AgentInterruptOk bindings");
    UsageGetArgs::export_all_to(out).expect("failed to export the UsageGetArgs bindings");
    UsageVendorRow::export_all_to(out).expect("failed to export the UsageVendorRow bindings");
    UsageRefreshArgs::export_all_to(out).expect("failed to export the UsageRefreshArgs bindings");
    UsageRefreshOk::export_all_to(out).expect("failed to export the UsageRefreshOk bindings");
    LibraryStatusArgs::export_all_to(out).expect("failed to export the LibraryStatusArgs bindings");
    LibraryStatusOk::export_all_to(out).expect("failed to export the LibraryStatusOk bindings");
    LibrarySetRootArgs::export_all_to(out)
        .expect("failed to export the LibrarySetRootArgs bindings");
    LibrarySetRootOk::export_all_to(out).expect("failed to export the LibrarySetRootOk bindings");
    LibraryFoldersArgs::export_all_to(out)
        .expect("failed to export the LibraryFoldersArgs bindings");
    LibraryFoldersOk::export_all_to(out).expect("failed to export the LibraryFoldersOk bindings");
    LibraryRefreshArgs::export_all_to(out)
        .expect("failed to export the LibraryRefreshArgs bindings");
    LibraryRefreshOk::export_all_to(out).expect("failed to export the LibraryRefreshOk bindings");
    LibraryListArgs::export_all_to(out).expect("failed to export the LibraryListArgs bindings");
    LibraryListOk::export_all_to(out).expect("failed to export the LibraryListOk bindings");
    LibraryReadArgs::export_all_to(out).expect("failed to export the LibraryReadArgs bindings");
    LibraryReadOk::export_all_to(out).expect("failed to export the LibraryReadOk bindings");
    LibraryNewFolderArgs::export_all_to(out)
        .expect("failed to export the LibraryNewFolderArgs bindings");
    LibraryNewFolderOk::export_all_to(out)
        .expect("failed to export the LibraryNewFolderOk bindings");
    LibraryNewFileArgs::export_all_to(out)
        .expect("failed to export the LibraryNewFileArgs bindings");
    LibraryNewFileOk::export_all_to(out).expect("failed to export the LibraryNewFileOk bindings");
    LibraryRelinkArgs::export_all_to(out).expect("failed to export the LibraryRelinkArgs bindings");
    LibraryRelinkOk::export_all_to(out).expect("failed to export the LibraryRelinkOk bindings");

    let produced: Vec<String> = std::fs::read_dir(out)
        .expect("read bindings/")
        .filter_map(|entry| {
            let path = entry.expect("directory entry").path();
            (path.extension()? == "ts").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();

    let specs = linked_specs();
    assert!(
        !specs.is_empty(),
        "linker collection is empty — coverage is disarmed"
    );
    for spec in specs {
        for type_name in [spec.args_type, spec.ok_type] {
            assert!(
                produced.iter().any(|name| name == type_name),
                "{}: {type_name}.ts is missing — add that type to the export list above (produced: {produced:?})",
                spec.name
            );
        }
    }
}

/// Derived schema for the LLM (TRD §2-4 ②) — elements **byte-identical** to the registration packet's `help`.
///
/// Covers [`linked_specs`] too — printing from one module's list would drop other blocks' commands from the file entirely, worse than one
/// missing `.ts` (the schema would say that command does not exist).
#[test]
fn export_command_schema() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings");
    // Do not rely on ts-rs having run first — test order is not fixed.
    std::fs::create_dir_all(dir).expect("failed to create bindings/");
    let body = catalog_json(CATALOG_VERSION, &linked_specs());
    std::fs::write(format!("{dir}/commands.schema.json"), body)
        .expect("failed to write commands.schema.json");
}
