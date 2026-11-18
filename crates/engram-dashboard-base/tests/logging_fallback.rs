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
    std::fs::write(data_dir.join("logs"), "this spot is a file").expect("block");
    // This process has not started any thread yet (only one test) — it is safe only here.
    std::env::set_var("TMP", &temp_home);
    std::env::set_var("TEMP", &temp_home);

    let path = init_logging_with_file(&data_dir.join("logs"), LogKind::Daemon)
        .expect("must open, even via the fallback");
    // The very line the daemon emits right after (= the reason for giving up startup).
    tracing::error!(
        "cannot start the daemon because the data folder could not be prepared: (repro)"
    );

    assert!(
        path.starts_with(&temp_home),
        "must fall back to the temp folder: {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).expect("read log");
    println!("--- {} ---\n{body}", path.display());

    assert!(body.contains("==== engram daemon"), "header: {body}");
    assert!(
        body.contains("instead"),
        "the fallback reason must remain in the file: {body}"
    );
    assert!(
        body.contains("cannot start the daemon"),
        "the startup failure reason must remain in the file: {body}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
