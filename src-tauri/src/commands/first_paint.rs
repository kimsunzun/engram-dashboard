use tauri::WebviewWindow;

/// 창 웹뷰가 첫 화면(탭바 + 활성 탭)을 그렸다고 알린다 — `invoke('webview_painted', { generation })`, 페이지 읽기마다
/// 한 번(`src/theme/firstPaint.ts`). `generation` = 그 페이지가 받은 채택 번호(`spare:adopted` 몸 · `window_role` 답 —
/// TRD S21-slot-docking §3-9 I26) · 받은 적 없으면 `null` 또는 생략. 셸이 숨긴 채 지어 기다리던 창이면 그 창을 보이고(보일지
/// 아직 정하지 않은 창이면 알림을 받아 둔다 · 맡긴 예열 창은 그 채택 번호를 실은 알림만 받는다), 아니면(다시 읽기 ·
/// 맡기기 전 예열 창 · 이미 보임 · 숨기기가 기다림을 거둔 창) 아무 일도 하지 않는다 — 실패가 없다. 웹뷰 안 배선이라 버스
/// 명령이 아니다. 동기 명령이라 메인 스레드에서 돈다(보이기는 메인 스레드 몫이다 — `crate::first_paint`).
// ADR-0299
#[tauri::command]
pub fn webview_painted(window: WebviewWindow, generation: Option<u32>) {
    crate::first_paint::reveal_painted(&window, generation);
}
