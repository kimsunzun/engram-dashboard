//! 1차 로그 폴더를 못 쓰는 릴리즈 상황에서 **기동 실패 사유가 어디엔가 남는지** 본다.
//!
//! ★별도 프로세스가 필요하다★: 전역 subscriber 는 프로세스당 한 번뿐이고, 이 테스트는 `%TEMP%` 까지
//! 갈아끼운다(다른 테스트가 보는 임시 폴더를 건드리지 않으려고). 그래서 통합 테스트 파일 하나 =
//! 테스트 하나다.

use std::path::PathBuf;

use engram_dashboard_agent::logging::{init_logging_with_file, LogKind};

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

    // ★로그 하위 폴더 자리에 파일을 둔다★ = `create_dir_all` 이 반드시 실패하는 상태(읽기 전용
    //   볼륨·권한 없음과 같은 결과를 결정적으로 재현하는 방법).
    std::fs::write(data_dir.join("logs"), "이 자리는 파일이다").expect("막기");
    // 이 프로세스는 아직 스레드를 안 띄웠다(테스트 하나뿐) — 여기서만 안전하다.
    std::env::set_var("TMP", &temp_home);
    std::env::set_var("TEMP", &temp_home);

    let path = init_logging_with_file(&data_dir, LogKind::Daemon).expect("폴백으로라도 열려야");
    // 데몬이 그 직후 내는 바로 그 줄(= 기동 포기 사유).
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
