// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ★CLI 겸용 분기(설계 §5 · ADR-0014 방향) — load-bearing 순서★: 이 판정이 Tauri/GUI init 보다
    //   먼저여야 창·트레이·single-instance 플러그인을 절대 건드리지 않는다(먼저가 아니면 CLI 호출에서
    //   창이 뜬다). 스폰된 에이전트가 자기 exe 를 재실행해 A→B 메시지·spawn 을 하는 통로다.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(first) = args.first() {
        if engram_dashboard_lib::cli::is_cli_verb(first) {
            let code = engram_dashboard_lib::cli::run_cli(&args);
            std::process::exit(code);
        }
    }

    engram_dashboard_lib::run()
}
