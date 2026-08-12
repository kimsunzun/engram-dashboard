// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ADR-0132: 이 exe 는 argv 로 동사를 받지 않는다 — GUI init 앞에 동사 분기를 두면 첫 인자가 GUI 인자와
// 겹칠 때 창이 안 뜨고, 그 충돌을 동사가 늘 때마다 관리해야 한다. 제어 CLI 는 `engram` 실행파일 소유다.
fn main() {
    engram_dashboard_lib::run()
}
