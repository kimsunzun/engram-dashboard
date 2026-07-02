// KNOWN-ISSUE(선재): `cargo test -p engram-dashboard --lib`는 0xC0000139(ENTRYPOINT_NOT_FOUND)로
// 기동 실패한다. 원인 = tauri_build 가 manifest 리소스(resource.lib — comctl32 v6 활성화)를
// `rustc-link-arg-bins`(bin 전용)로만 링크 → 테스트 exe 는 manifest 없이 구버전 comctl32 v5.82
// (TaskDialogIndirect 없음)를 로드해 로더 단계에서 죽음. `rustc-link-arg-tests`로 같은 리소스를
// 링크하는 우회는 실측 탈락 — 이 패키지에 통합 테스트 타깃(tests/)이 없어 cargo 가 instruction 자체를
// 거부한다("does not have a test target"). 정공법 = lib 내 WS 클라이언트 테스트(T1/T2/T4)를 비-tauri
// 테스트 크레이트 또는 tests/ 통합 테스트로 이전(백로그, step-log 2026-07-02 참조).
fn main() {
    tauri_build::build()
}
