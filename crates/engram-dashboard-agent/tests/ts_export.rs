//! Generating declaration derivatives — `cargo test -p engram-dashboard-agent` writes into `bindings/` (TRD §2-4).
//!
//! ★It is an explicit export (does not rely on implicit derive tests)★: ts-rs automatically makes, per `#[ts(export)]` type,
//!   an `export_bindings_<타입>` test, but `src-tauri/bindings/` relied on that implicit path and
//!   became a hand-maintained artifact that **never ran** either locally (`0xc0000139`) or in CI (package excluded) (TRD §5 measured).
//!   ★Updated 2026-08-24★: the local side is resolved — with `--test lib_unit` standing, those implicit tests run and
//!   `src-tauri/bindings/` is regenerated automatically every time that suite runs. **The CI side is still blocked** (that suite
//!   is not registered · 30 known failures pending) **and there is still no sync gate that watches that directory**. So the policy of this file of not relying on the implicit path
//!   stays — here we call the export directly, in the same form as
//!   protocol's `tests/ts_export.rs`.
//! ★Commit the generated files★ — the CI diff gate must watch this directory to catch vocabulary drift.

use engram_dashboard_agent::commands::{
    AgentCancelQueuedInputArgs, AgentCancelQueuedInputOk, AgentInterruptArgs, AgentInterruptOk,
    AgentListArgs, AgentListOk, AgentListQueuedInputsArgs, AgentListQueuedInputsOk, AgentMoveArgs,
    AgentMoveOk, AgentNewArgs, AgentNewOk, AgentRenameArgs, AgentRenameOk, AgentResolveArgs,
    AgentResolveOk, AgentSpawnArgs, AgentSpawnOk, UsageGetArgs, UsageRefreshArgs, UsageRefreshOk,
    UsageVendorRow, CATALOG_VERSION,
};
use engram_dashboard_command::{catalog_json, command_specs, CommandSpec};
use ts_rs::TS;

/// **All declarations linked into** this test binary, sorted by name.
///
/// ★The point is not importing one module's `COMMAND_SPECS`★ — then when one more declaration block appears inside agent, that
/// block silently drops out of both coverage and the derived schema. Where 「손으로 적은 숫자」 (a hand-written number) was removed,
/// a 「손으로 적은 import」 (hand-written import) sits in its place, so the same failure mode remains (TRD §5).
/// ★The sorting is to make the derived file deterministic★ — the linker's collection order is not fixed, and if it wobbles, CI's
/// bindings diff gate goes red regardless of content.
fn linked_specs() -> Vec<&'static CommandSpec> {
    let mut specs: Vec<&'static CommandSpec> = command_specs().collect();
    specs.sort_unstable_by_key(|s| s.name);
    specs
}

/// ★Do not hand-write the expected value★ — if it became 「선언을 늘리고 목록의 숫자도 함께 고치면 통과」 (add declarations and fix the number in the list too, and it passes), the edit that
/// creates the omission also fixes the assertion and disarms it (this file would reproduce exactly the failure mode TRD §5 measured in `src-tauri/bindings/`).
/// So the expected value comes from **the declarations themselves** ([`linked_specs`]):
/// if the export call below is left out, the `.ts` of that type is not generated and this comparison blows up.
#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    std::fs::create_dir_all(out).expect("bindings/ 생성 실패");
    for entry in std::fs::read_dir(out).expect("bindings/ 조회") {
        let path = entry.expect("디렉터리 항목").path();
        if path.extension().is_some_and(|ext| ext == "ts") {
            std::fs::remove_file(&path).expect("옛 바인딩 삭제");
        }
    }

    // Transitive dependencies (AgentRow etc.) are brought along by export_all_to.
    AgentListArgs::export_all_to(out).expect("AgentListArgs 바인딩 export 실패");
    AgentListOk::export_all_to(out).expect("AgentListOk 바인딩 export 실패");
    AgentResolveArgs::export_all_to(out).expect("AgentResolveArgs 바인딩 export 실패");
    AgentResolveOk::export_all_to(out).expect("AgentResolveOk 바인딩 export 실패");
    AgentSpawnArgs::export_all_to(out).expect("AgentSpawnArgs 바인딩 export 실패");
    AgentSpawnOk::export_all_to(out).expect("AgentSpawnOk 바인딩 export 실패");
    AgentNewArgs::export_all_to(out).expect("AgentNewArgs 바인딩 export 실패");
    AgentNewOk::export_all_to(out).expect("AgentNewOk 바인딩 export 실패");
    AgentRenameArgs::export_all_to(out).expect("AgentRenameArgs 바인딩 export 실패");
    AgentRenameOk::export_all_to(out).expect("AgentRenameOk 바인딩 export 실패");
    AgentMoveArgs::export_all_to(out).expect("AgentMoveArgs 바인딩 export 실패");
    AgentMoveOk::export_all_to(out).expect("AgentMoveOk 바인딩 export 실패");
    AgentListQueuedInputsArgs::export_all_to(out)
        .expect("AgentListQueuedInputsArgs 바인딩 export 실패");
    AgentListQueuedInputsOk::export_all_to(out)
        .expect("AgentListQueuedInputsOk 바인딩 export 실패");
    AgentCancelQueuedInputArgs::export_all_to(out)
        .expect("AgentCancelQueuedInputArgs 바인딩 export 실패");
    AgentCancelQueuedInputOk::export_all_to(out)
        .expect("AgentCancelQueuedInputOk 바인딩 export 실패");
    AgentInterruptArgs::export_all_to(out).expect("AgentInterruptArgs 바인딩 export 실패");
    AgentInterruptOk::export_all_to(out).expect("AgentInterruptOk 바인딩 export 실패");
    UsageGetArgs::export_all_to(out).expect("UsageGetArgs 바인딩 export 실패");
    UsageVendorRow::export_all_to(out).expect("UsageVendorRow 바인딩 export 실패");
    UsageRefreshArgs::export_all_to(out).expect("UsageRefreshArgs 바인딩 export 실패");
    UsageRefreshOk::export_all_to(out).expect("UsageRefreshOk 바인딩 export 실패");

    let produced: Vec<String> = std::fs::read_dir(out)
        .expect("bindings/ 조회")
        .filter_map(|entry| {
            let path = entry.expect("디렉터리 항목").path();
            (path.extension()? == "ts").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();

    let specs = linked_specs();
    assert!(
        !specs.is_empty(),
        "링커 수집이 비었다 — 커버리지가 무장 해제된다"
    );
    for spec in specs {
        for type_name in [spec.args_type, spec.ok_type] {
            assert!(
                produced.iter().any(|name| name == type_name),
                "{}: {type_name}.ts 가 없다 — 위 export 목록에 그 타입을 더할 것 (생성된 것: {produced:?})",
                spec.name
            );
        }
    }
}

/// Derived schema for the LLM (TRD §2-4 ②) — the elements are **byte-identical** to the `help` of the registration packet.
///
/// What it covers is also [`linked_specs`] — if printed from one module's list, the commands of other blocks drop out of the derived file
/// whole, and that is worse than one `.ts` missing (the schema says that command does not exist at all).
#[test]
fn export_command_schema() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings");
    // Do not rely on ts-rs having run first — test order is not fixed.
    std::fs::create_dir_all(dir).expect("bindings/ 생성 실패");
    let body = catalog_json(CATALOG_VERSION, &linked_specs());
    std::fs::write(format!("{dir}/commands.schema.json"), body)
        .expect("commands.schema.json 쓰기 실패");
}
