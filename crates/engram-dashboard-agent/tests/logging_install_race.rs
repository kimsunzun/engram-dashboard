//! subscriber 설치에 **진** 호출이 파일을 만들지도, 보존 정리를 돌리지도 않는지 본다.
//!
//! 진 호출이 파일을 먼저 열면 아무도 쓰지 않을 빈 파일이 남고, 그걸 만들며 돌린 보존 정리가 **진짜
//! 로그 하나를 지운다** — 그 빈 파일이 상한 한 자리를 영구히 차지한다.
//!
//! ★별도 프로세스가 필요하다★: 전역 subscriber 는 프로세스당 한 번뿐이라, 남이 먼저 깐 상태를
//! 만들려면 이 파일이 그 프로세스를 통째로 소유해야 한다.

use engram_dashboard_agent::logging::{init_logging_with_file, LogKind};

#[test]
fn losing_the_subscriber_install_creates_and_prunes_nothing() {
    let data_dir = std::env::temp_dir().join(format!("engram-loginstall-{}", std::process::id()));
    let logs = data_dir.join("logs");
    std::fs::create_dir_all(&logs).expect("logs dir");
    // 상한(10)을 넘겨 둔다 — 보존 정리가 돌면 반드시 지워진다.
    for i in 0..15 {
        std::fs::write(logs.join(format!("daemon-20260101-{i:06}-1.log")), "x").expect("write");
    }

    // 남이 먼저 깐 subscriber.
    tracing_subscriber::fmt().try_init().expect("선점해야");

    let got = init_logging_with_file(&data_dir, LogKind::Daemon);

    let mut left: Vec<String> = std::fs::read_dir(&logs)
        .expect("read")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    println!("returned={got:?}\nfiles={left:#?}");

    assert!(got.is_none(), "설치에 졌으면 경로를 광고하면 안 된다");
    assert_eq!(left.len(), 15, "새 파일도 삭제도 없어야: {left:?}");

    let _ = std::fs::remove_dir_all(&data_dir);
}
