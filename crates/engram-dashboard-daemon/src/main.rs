//! engram-dashboard-daemon — 데몬 진입점(thin).
//!
//! 본체(단일 인스턴스 가드·data_dir·daemon.json·bind·토큰·manager 배선·accept loop·graceful
//! 종료)는 라이브러리(`lib.rs`)의 `run()` 에 있다. main 은 tokio 런타임만 띄우고 `run()` 을 부른다.
//! 이렇게 분리해 격리 하네스(`tests/ws_e2e.rs`)가 같은 기동 흐름(`start_test_server`)을 공유한다.

// ★콘솔 가시성 — 빌드모드 의존(왜 cfg_attr 인가)★:
//   릴리즈만 windows 서브시스템(콘솔 창 없음). 디버그는 속성을 안 붙여 **콘솔 앱**으로 둔다 →
//   discovery 가 WMI 로 데몬을 띄울 때 콘솔 창이 함께 떠 데몬 로그(RUST_LOG)를 그 창에서 본다(개발 편의).
//   discovery WMI(Win32_Process.Create)는 CREATE_NO_WINDOW 를 RV=21 로 거부해(ADR-0021 실측) **콘솔 앱을
//   windowless 로 못 만든다**. 그래서 릴리즈의 "창 없음"은 이 서브시스템 속성으로만 달성된다(WMI 플래그 X).
//   사용자 결정(2026-06-19): 디버그=창 있이(로그), 릴리즈=창 없이.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tokio::main]
async fn main() {
    if let Err(code) = engram_dashboard_daemon::run().await {
        std::process::exit(code);
    }
}
