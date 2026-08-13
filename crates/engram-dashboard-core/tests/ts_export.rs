//! 선언 파생물 생성 — `cargo test -p engram-dashboard-core` 가 `bindings/` 에 쓴다(TRD §2-4).
//!
//! ★명시적 export 다(암묵 derive 테스트에 기대지 않는다)★: ts-rs 는 `#[ts(export)]` 타입마다
//!   `export_bindings_<타입>` 테스트를 자동으로 만드는데, `src-tauri/bindings/` 가 그 암묵 경로에 기댔다가
//!   로컬(`0xc0000139`)에서도 CI(패키지 제외)에서도 **한 번도 돌지 않는** 손관리 생성물이 됐다(TRD §5 실측).
//!   여기서는 protocol 의 `tests/ts_export.rs` 와 같은 형태로 export 를 직접 부른다.
//! ★생성물은 커밋한다★ — CI 의 diff 게이트가 이 디렉토리를 봐야 어휘 drift 를 잡는다.

use engram_dashboard_command::{catalog_json, command_specs, CommandSpec};
use engram_dashboard_core::agent::commands::{
    AgentListArgs, AgentListOk, AgentMoveArgs, AgentMoveOk, AgentNewArgs, AgentNewOk,
    AgentRenameArgs, AgentRenameOk, AgentSpawnArgs, AgentSpawnOk, CATALOG_VERSION,
};
use ts_rs::TS;

/// 이 테스트 바이너리에 **링크된 선언 전량**, 이름순.
///
/// ★한 모듈의 `COMMAND_SPECS` 를 import 하지 않는 것이 요점이다★ — 그러면 core 안에 선언 블록이 하나
/// 더 생겼을 때 그 블록이 커버리지에서도 파생 스키마에서도 조용히 빠진다. 「손으로 적은 숫자」를 없앤
/// 자리에 「손으로 적은 import」가 들어앉은 셈이라 같은 실패 모드가 그대로 남는다(TRD §5).
/// ★정렬은 파생 파일을 결정적으로 만들려는 것★ — 링커 수집 순서는 정해져 있지 않고, 흔들리면 CI 의
/// bindings diff 게이트가 내용과 무관하게 붉어진다.
fn linked_specs() -> Vec<&'static CommandSpec> {
    let mut specs: Vec<&'static CommandSpec> = command_specs().collect();
    specs.sort_unstable_by_key(|s| s.name);
    specs
}

/// ★기대값을 손으로 적지 않는다★ — 「선언을 늘리고 목록의 숫자도 함께 고치면 통과」가 되면, 누락을
/// 만드는 그 편집이 단언까지 함께 고쳐 무장 해제된다(TRD §5 가 `src-tauri/bindings/` 에서 실측한 실패
/// 모드를 이 파일이 그대로 재현하게 된다). 그래서 기대값은 **선언 자신**([`linked_specs`])에서 나온다:
/// 아래 export 호출을 빠뜨리면 그 타입의 `.ts` 가 안 생기고 이 대조가 터진다.
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

    // 전이 의존(AgentRow 등)은 export_all_to 가 따라 데려온다.
    AgentListArgs::export_all_to(out).expect("AgentListArgs 바인딩 export 실패");
    AgentListOk::export_all_to(out).expect("AgentListOk 바인딩 export 실패");
    AgentSpawnArgs::export_all_to(out).expect("AgentSpawnArgs 바인딩 export 실패");
    AgentSpawnOk::export_all_to(out).expect("AgentSpawnOk 바인딩 export 실패");
    AgentNewArgs::export_all_to(out).expect("AgentNewArgs 바인딩 export 실패");
    AgentNewOk::export_all_to(out).expect("AgentNewOk 바인딩 export 실패");
    AgentRenameArgs::export_all_to(out).expect("AgentRenameArgs 바인딩 export 실패");
    AgentRenameOk::export_all_to(out).expect("AgentRenameOk 바인딩 export 실패");
    AgentMoveArgs::export_all_to(out).expect("AgentMoveArgs 바인딩 export 실패");
    AgentMoveOk::export_all_to(out).expect("AgentMoveOk 바인딩 export 실패");

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

/// LLM 용 파생 스키마(TRD §2-4 ②) — 원소는 등록 패킷의 `help` 와 **바이트 동일**하다.
///
/// 담는 범위도 [`linked_specs`] 다 — 한 모듈의 목록으로 찍으면 다른 블록의 명령이 파생 파일에서
/// 통째로 빠지고, 그건 `.ts` 하나가 없는 것보다 나쁘다(스키마가 그 명령을 아예 없다고 말한다).
#[test]
fn export_command_schema() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings");
    // ts-rs 가 먼저 돌았는지에 기대지 않는다 — 테스트 순서는 정해져 있지 않다.
    std::fs::create_dir_all(dir).expect("bindings/ 생성 실패");
    let body = catalog_json(CATALOG_VERSION, &linked_specs());
    std::fs::write(format!("{dir}/commands.schema.json"), body)
        .expect("commands.schema.json 쓰기 실패");
}
