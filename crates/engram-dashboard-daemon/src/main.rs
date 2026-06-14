//! engram-dashboard-daemon — 데몬 진입점(thin).
//!
//! 본체(단일 인스턴스 가드·data_dir·daemon.json·bind·토큰·manager 배선·accept loop·graceful
//! 종료)는 라이브러리(`lib.rs`)의 `run()` 에 있다. main 은 tokio 런타임만 띄우고 `run()` 을 부른다.
//! 이렇게 분리해 격리 하네스(`tests/ws_e2e.rs`)가 같은 기동 흐름(`start_test_server`)을 공유한다.

#[tokio::main]
async fn main() {
    if let Err(code) = engram_dashboard_daemon::run().await {
        std::process::exit(code);
    }
}
