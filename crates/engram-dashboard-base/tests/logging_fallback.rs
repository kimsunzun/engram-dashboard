//! Checks, in the release situation where the primary log folder cannot be used, **whether the startup failure reason is left somewhere**.
//!
//! ★A separate process is needed★: the global subscriber is once per process, and this test even swaps `%TEMP%`
//! (so as not to touch the temp folder other tests see). So one integration test file =
//! one test.

use std::path::PathBuf;

use engram_dashboard_base::logging::{init_logging_with_file, LogKind};

#[test]
fn daemon_failure_reason_survives_when_the_data_dir_cannot_hold_logs() {
    let root: PathBuf = std::env::temp_dir().join(format!(
        "engram-logfallback-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let data_dir = root.join("data");
    let temp_home = root.join("temp");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&temp_home).expect("temp home");

    // ★Put a file where the log subfolder goes★ = a state where `create_dir_all` must fail (a way to deterministically
    //   reproduce the same result as a read-only volume · no permission).
    std::fs::write(data_dir.join("logs"), "이 자리는 파일이다").expect("막기");
    // This process has not started any thread yet (only one test) — it is safe only here.
    std::env::set_var("TMP", &temp_home);
    std::env::set_var("TEMP", &temp_home);

    let path = init_logging_with_file(&data_dir.join("logs"), LogKind::Daemon)
        .expect("폴백으로라도 열려야");
    // The very line the daemon emits right after (= the reason for giving up startup).
    tracing::error!("데이터 폴더를 준비하지 못해 데몬을 시작할 수 없음: (재현)");

    assert!(
        path.starts_with(&temp_home),
        "임시 폴더로 물러나야: {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).expect("로그 읽기");
    println!("--- {} ---\n{body}", path.display());

    assert!(body.contains("==== engram daemon"), "머리글: {body}");
    assert!(body.contains("대신"), "폴백 사유가 파일에 남아야: {body}");
    assert!(
        body.contains("데몬을 시작할 수 없음"),
        "기동 실패 사유가 파일에 남아야: {body}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
