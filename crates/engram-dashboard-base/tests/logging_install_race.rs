//! Checks that a call that **lost** the subscriber install neither creates a file nor runs retention cleanup.
//!
//! If the losing call opened the file first, an empty file nobody writes would remain, and the retention cleanup run while making it
//!   **deletes one real
//! log** — that empty file permanently takes one slot of the cap.
//!
//! ★A separate process is needed★: the global subscriber is once per process, so to create a state where someone else installed first
//! this file must own that process whole.

use engram_dashboard_base::logging::{init_logging_with_file, LogKind};

#[test]
fn losing_the_subscriber_install_creates_and_prunes_nothing() {
    let data_dir = std::env::temp_dir().join(format!("engram-loginstall-{}", std::process::id()));
    let logs = data_dir.join("logs");
    std::fs::create_dir_all(&logs).expect("logs dir");
    // Leave it over the cap (10) — it is certain to be deleted when retention cleanup runs.
    for i in 0..15 {
        std::fs::write(logs.join(format!("daemon-20260101-{i:06}-1.log")), "x").expect("write");
    }

    // A subscriber installed first by someone else.
    tracing_subscriber::fmt()
        .try_init()
        .expect("must claim first");

    let got = init_logging_with_file(&logs, LogKind::Daemon);

    let mut left: Vec<String> = std::fs::read_dir(&logs)
        .expect("read")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    println!("returned={got:?}\nfiles={left:#?}");

    assert!(
        got.is_none(),
        "if it lost the install, it must not advertise the path"
    );
    assert_eq!(left.len(), 15, "no new file and no deletion: {left:?}");

    let _ = std::fs::remove_dir_all(&data_dir);
}
