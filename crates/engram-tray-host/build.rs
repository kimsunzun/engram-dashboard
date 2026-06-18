// 왜: standalone 트레이 exe 에 Common Controls v6 의존을 선언하는 애플리케이션 매니페스트를
// 임베드한다. 트레이 메뉴(muda/tray-icon, 특히 PredefinedMenuItem)가 TaskDialogIndirect 를
// 참조하는데, 이 함수는 comctl32 v6 export 다. 매니페스트가 없으면 윈도가 comctl32 v5 를 로드
// → export 부재로 "프로시저 시작 지점 TaskDialogIndirect 을 찾을 수 없습니다" 로드 타임 실패.
// Tauri 앱(src-tauri)은 이 매니페스트를 자동 임베드해 문제없었지만 standalone 바이너리는 수동.
// embed-manifest 의 new_manifest(...) 는 기본으로 Common Controls v6 의존 + DPI 인식을 포함한다.
fn main() {
    #[cfg(windows)]
    {
        use embed_manifest::{embed_manifest, new_manifest};
        // CARGO_CFG_WINDOWS 가드: 윈도 타깃 빌드에서만 임베드.
        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            embed_manifest(new_manifest("Engram.TrayHost")).expect("manifest 임베드 실패");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
